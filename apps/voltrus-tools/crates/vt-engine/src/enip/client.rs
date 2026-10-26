//! Reads a Logix controller's tags and CIP attributes over EtherNet/IP,
//! writes them, and sends the inspector's explicit messages.
//!
//! Tags are batched into Multiple_Service_Packets by `mt-enip`; a tag the
//! device refuses is read on its own from then on, so one bad name costs one
//! point, not the poll. Attribute points sharing an attribute (fields of one
//! assembly) cost one Get_Attribute_Single per poll.

use std::time::{Duration, Instant};

use mt_enip::{
    CipDataType, CipTag, CipValue, EnipClient, EnipConfig, PacketRecord, ProtocolClient,
};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    CipResponse, CipType, EnipAddress, EnipClientSpec, LogLevel, PointInfo, PointText, Quality,
    Reading, SessionState, Stats, ValueFrame,
};

use super::plants::short_name;
use super::{cip_type_of, decode, device_type_name, encode, traffic_frame};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(50);
/// How often captured frames reach the traffic view between polls.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// Timeouts in a row that make the client drop the session and reconnect.
const TIMEOUTS_BEFORE_RECONNECT: u32 = 3;
/// Points a browse-everything client reads at most.
const BROWSE_LIMIT: usize = 500;
/// Elements of each browsed array that become points.
const ARRAY_PREVIEW: u16 = 16;
const GET_ATTRIBUTE_SINGLE: u8 = 0x0E;

pub(crate) async fn run(
    spec: EnipClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let config = config(&spec);
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;
    let mut plan: Option<Plan> = None;

    'session: loop {
        out.state(SessionState::Connecting);
        let connected = tokio::select! {
            result = EnipClient::connect(config.clone()) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let client = match connected {
            Ok(client) => client,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                if !retry(&mut control, attempt, &error.to_string(), out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        let (packets_tx, mut packets) = mpsc::unbounded_channel::<PacketRecord>();
        client.set_packet_sink(packets_tx).await;
        out.state(SessionState::Connected);
        match client.get_identity().await {
            Ok(identity) => out.log(
                LogLevel::Info,
                format!(
                    "Connected to {} · {} · {}, revision {}.{}, serial 0x{:08X}",
                    if identity.product_name.is_empty() {
                        "a device"
                    } else {
                        identity.product_name.as_str()
                    },
                    identity.vendor_name,
                    device_type_name(identity.device_type),
                    identity.revision_major,
                    identity.revision_minor,
                    identity.serial_number
                ),
            ),
            Err(error) => out.log(
                LogLevel::Warn,
                format!(
                    "Connected to {}; its Identity object did not answer: {error}",
                    spec.host
                ),
            ),
        }
        if plan.is_none() {
            let planned = if spec.points.is_empty() {
                match browse(&client, &config, out).await {
                    Ok(planned) => planned,
                    Err(error) => {
                        out.log(
                            LogLevel::Error,
                            format!("Browsing the tags failed: {error}"),
                        );
                        let _ = client.disconnect().await;
                        out.state(SessionState::Closed {
                            error: Some(format!("Browsing the tags failed: {error}")),
                        });
                        out.stats(stats);
                        return;
                    }
                }
            } else {
                spec.points
                    .iter()
                    .map(|point| Planned {
                        name: point.name.trim().to_owned(),
                        address: point.address.clone(),
                        unit: point.unit.clone(),
                        hint: None,
                    })
                    .collect()
            };
            plan = Some(Plan::new(planned));
        }
        let Some(plan) = plan.as_mut() else {
            break 'session;
        };
        attempt = 0;

        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut flush = tokio::time::interval(TRAFFIC_TICK);
        flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut timeouts = 0;
        let mut traffic = Vec::new();
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let started = Instant::now();
                    let mut deferred = Vec::new();
                    let polled = {
                        let read = poll_once(&client, plan);
                        tokio::pin!(read);
                        loop {
                            tokio::select! {
                                result = &mut read => break Some(result),
                                command = control.recv() => match command {
                                    None | Some(Control::Close) => break None,
                                    Some(command) => deferred.push(command),
                                },
                            }
                        }
                    };
                    let Some(polled) = polled else {
                        let _ = client.disconnect().await;
                        break 'session;
                    };
                    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
                    stats.polls += 1;
                    stats.last_latency_us = latency_us;
                    match polled {
                        Ok(outcomes) => {
                            timeouts = 0;
                            let failures: Vec<&str> = outcomes
                                .iter()
                                .filter_map(|outcome| match outcome {
                                    Outcome::Failed(error) => Some(error.as_str()),
                                    Outcome::Value { .. } => None,
                                })
                                .collect();
                            if !failures.is_empty() {
                                stats.errors += 1;
                            }
                            for (index, error) in plan.newly_failed(&outcomes) {
                                out.log(LogLevel::Warn, format!("{}: {error}", plan.planned[index].address));
                            }
                            if plan.learn(&outcomes) {
                                out.points(plan.points());
                            }
                            let texts = plan.texts(&outcomes);
                            out.values(frame(&outcomes, latency_us));
                            if !texts.is_empty() {
                                out.payloads(texts);
                            }
                        }
                        Err(error) => {
                            stats.errors += 1;
                            if plan.announced {
                                out.values(ValueFrame {
                                    at_ms: now_ms(),
                                    latency_us,
                                    readings: vec![Reading::bad(); plan.planned.len()],
                                });
                            }
                            let lost = !error.contains("Timed out") || {
                                timeouts += 1;
                                timeouts >= TIMEOUTS_BEFORE_RECONNECT
                            };
                            out.log(if lost { LogLevel::Error } else { LogLevel::Warn }, error.clone());
                            if lost {
                                for command in deferred {
                                    refuse(command, out);
                                }
                                drain(&mut packets, &mut traffic);
                                stats.frames += traffic.len() as u64;
                                out.traffic(std::mem::take(&mut traffic));
                                out.stats(stats);
                                let _ = client.disconnect().await;
                                attempt = 1;
                                if !retry(&mut control, attempt, &error, out).await {
                                    break 'session;
                                }
                                continue 'session;
                            }
                        }
                    }
                    for command in deferred {
                        handle(command, &client, plan, out).await;
                    }
                    drain(&mut packets, &mut traffic);
                    stats.frames += traffic.len() as u64;
                    out.traffic(std::mem::take(&mut traffic));
                    out.stats(stats);
                }
                _ = flush.tick() => {
                    drain(&mut packets, &mut traffic);
                    stats.frames += traffic.len() as u64;
                    out.traffic(std::mem::take(&mut traffic));
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        let _ = client.disconnect().await;
                        break 'session;
                    }
                    Some(command) => handle(command, &client, plan, out).await,
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

fn config(spec: &EnipClientSpec) -> EnipConfig {
    let host = spec.host.trim();
    EnipConfig {
        id: String::new(),
        // `mt-enip` connects to `host:port`: IPv6 literals need brackets.
        host: if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        },
        port: spec.port,
        poll_interval_ms: spec.poll_ms,
        route_path: Some(spec.route_path.trim().to_owned()).filter(|route| !route.is_empty()),
    }
}

fn drain(
    packets: &mut mpsc::UnboundedReceiver<PacketRecord>,
    traffic: &mut Vec<vt_protocol::TrafficFrame>,
) {
    while let Ok(record) = packets.try_recv() {
        traffic.push(traffic_frame(&record));
    }
}

/// A point as the session reads it.
#[derive(Debug, Clone)]
struct Planned {
    name: String,
    address: EnipAddress,
    unit: String,
    /// The type a browse reported, until a read tells.
    hint: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Source {
    /// Index into [`Plan::tags`].
    Tag(usize),
    /// Index into [`Plan::attributes`], and where the value sits in its bytes.
    Attribute {
        read: usize,
        offset: usize,
        data_type: CipType,
    },
}

/// What one poll found for a point.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Value {
        value: f64,
        /// Text behind a value that is no number (STRING, structures).
        text: Option<String>,
        /// The type a tag's value came back as.
        learned: Option<String>,
    },
    Failed(String),
}

struct Plan {
    planned: Vec<Planned>,
    sources: Vec<Source>,
    tags: Vec<CipTag>,
    /// Per tag: read on its own since it failed in a batch.
    isolated: Vec<bool>,
    /// Distinct (class, instance, attribute) read by attribute points.
    attributes: Vec<(u16, u32, u16)>,
    /// Per point: the type label shown, and whether the point is a number.
    labels: Vec<Option<String>>,
    numeric: Vec<bool>,
    failed: Vec<bool>,
    texts: Vec<String>,
    /// The points went out to the app.
    announced: bool,
}

impl Plan {
    fn new(planned: Vec<Planned>) -> Self {
        let mut sources = Vec::with_capacity(planned.len());
        let mut tags = Vec::new();
        let mut attributes: Vec<(u16, u32, u16)> = Vec::new();
        let mut labels = Vec::with_capacity(planned.len());
        for point in &planned {
            match &point.address {
                EnipAddress::Tag { tag } => {
                    sources.push(Source::Tag(tags.len()));
                    tags.push(CipTag {
                        name: tag.clone(),
                        data_type: CipDataType::Dint,
                        element_count: 1,
                        template_id: None,
                    });
                    labels.push(point.hint.clone());
                }
                EnipAddress::Attribute {
                    class,
                    instance,
                    attribute,
                    offset,
                    data_type,
                } => {
                    let key = (*class, *instance, *attribute);
                    let read = attributes
                        .iter()
                        .position(|seen| *seen == key)
                        .unwrap_or_else(|| {
                            attributes.push(key);
                            attributes.len() - 1
                        });
                    sources.push(Source::Attribute {
                        read,
                        offset: usize::from(*offset),
                        data_type: *data_type,
                    });
                    labels.push(Some(data_type.name().to_owned()));
                }
            }
        }
        let count = planned.len();
        Self {
            isolated: vec![false; tags.len()],
            planned,
            sources,
            tags,
            attributes,
            labels,
            numeric: vec![true; count],
            failed: vec![false; count],
            texts: vec![String::new(); count],
            announced: false,
        }
    }

    /// Take the types the poll's values came back as. True when the app must
    /// get the points (again): the first time, or a label changed.
    fn learn(&mut self, outcomes: &[Outcome]) -> bool {
        let mut changed = !self.announced;
        for (index, outcome) in outcomes.iter().enumerate() {
            if let Outcome::Value {
                learned: Some(label),
                text,
                ..
            } = outcome
            {
                let numeric = text.is_none();
                if self.labels[index].as_deref() != Some(label.as_str())
                    || self.numeric[index] != numeric
                {
                    self.labels[index] = Some(label.clone());
                    self.numeric[index] = numeric;
                    changed = true;
                }
            }
        }
        self.announced = true;
        changed
    }

    /// Points that failed this poll but not the last: worth a log line once.
    fn newly_failed(&mut self, outcomes: &[Outcome]) -> Vec<(usize, String)> {
        let mut fresh = Vec::new();
        for (index, outcome) in outcomes.iter().enumerate() {
            let failed = matches!(outcome, Outcome::Failed(_));
            if failed
                && !self.failed[index]
                && let Outcome::Failed(error) = outcome
            {
                fresh.push((index, error.clone()));
            }
            self.failed[index] = failed;
        }
        fresh
    }

    /// The texts behind values that changed since the last poll.
    fn texts(&mut self, outcomes: &[Outcome]) -> Vec<PointText> {
        let mut changed = Vec::new();
        for (index, outcome) in outcomes.iter().enumerate() {
            if let Outcome::Value {
                text: Some(text), ..
            } = outcome
                && self.texts[index] != *text
            {
                self.texts[index].clone_from(text);
                changed.push(PointText {
                    point: u32::try_from(index).unwrap_or(u32::MAX),
                    text: text.clone(),
                });
            }
        }
        changed
    }

    fn points(&self) -> Vec<PointInfo> {
        self.planned
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let address = point.address.to_string();
                let label = self.labels[index].clone();
                let boolean = label.as_deref() == Some(CipType::Bool.name());
                PointInfo {
                    id: u32::try_from(index).unwrap_or(u32::MAX),
                    name: if point.name.is_empty() {
                        match &point.address {
                            EnipAddress::Tag { tag } => short_name(tag).to_owned(),
                            EnipAddress::Attribute { .. } => address.clone(),
                        }
                    } else {
                        point.name.clone()
                    },
                    address,
                    data_type: label.unwrap_or_else(|| "—".to_owned()),
                    unit: point.unit.clone(),
                    writable: self.numeric[index],
                    boolean,
                    states: Vec::new(),
                }
            })
            .collect()
    }
}

/// Whether an error from `mt-enip` means the session itself is gone (or the
/// device stopped answering), rather than the device refusing one request.
fn link_lost(error: &str) -> bool {
    [
        "Not connected",
        "socket failed",
        "connection closed",
        "Timed out",
        "encapsulation error",
        "Unexpected encapsulation reply",
    ]
    .iter()
    .any(|sign| error.contains(sign))
}

/// What a tag's value shows as: a number, or a text for strings and raw
/// structures, with the type it came back as.
fn tag_outcome(value: &CipValue) -> Outcome {
    match (cip_type_of(value), value.to_f64()) {
        (Some(data_type), Some(number)) => Outcome::Value {
            value: number,
            text: None,
            learned: Some(data_type.name().to_owned()),
        },
        _ => Outcome::Value {
            value: f64::NAN,
            text: Some(value.display_value()),
            learned: Some(
                match value {
                    CipValue::String(_) => "STRING",
                    CipValue::Array(_) => "Array",
                    _ => "Structure",
                }
                .to_owned(),
            ),
        },
    }
}

/// Read every point once. `Err` when the session is lost.
async fn poll_once(client: &EnipClient, plan: &mut Plan) -> Result<Vec<Outcome>, String> {
    let mut tag_results: Vec<Option<Outcome>> = vec![None; plan.tags.len()];
    let batch: Vec<usize> = (0..plan.tags.len())
        .filter(|index| !plan.isolated[*index])
        .collect();
    if !batch.is_empty() {
        let tags: Vec<CipTag> = batch
            .iter()
            .map(|index| plan.tags[*index].clone())
            .collect();
        match client.read(&tags).await {
            Ok(values) => {
                for (index, value) in batch.iter().zip(&values) {
                    tag_results[*index] = Some(tag_outcome(value));
                }
            }
            Err(error) => {
                let error = error.to_string();
                if link_lost(&error) {
                    return Err(error);
                }
                // Someone in the batch was refused: find who, one by one.
                for index in &batch {
                    plan.isolated[*index] = true;
                }
            }
        }
    }
    for index in 0..plan.tags.len() {
        if tag_results[index].is_some() {
            continue;
        }
        match client.read(std::slice::from_ref(&plan.tags[index])).await {
            Ok(values) => {
                plan.isolated[index] = false;
                tag_results[index] = values.first().map(tag_outcome);
            }
            Err(error) => {
                let error = error.to_string();
                if link_lost(&error) {
                    return Err(error);
                }
                tag_results[index] = Some(Outcome::Failed(strip(&error)));
            }
        }
    }

    let mut attribute_results: Vec<Result<Vec<u8>, String>> =
        Vec::with_capacity(plan.attributes.len());
    for (class, instance, attribute) in &plan.attributes {
        match client
            .generic_cip(
                GET_ATTRIBUTE_SINGLE,
                *class,
                *instance,
                Some(*attribute),
                Vec::new(),
            )
            .await
        {
            Ok(bytes) => attribute_results.push(Ok(bytes)),
            Err(error) => {
                let error = error.to_string();
                if link_lost(&error) {
                    return Err(error);
                }
                attribute_results.push(Err(strip(&error)));
            }
        }
    }

    Ok(plan
        .sources
        .iter()
        .map(|source| match *source {
            Source::Tag(index) => tag_results[index]
                .clone()
                .unwrap_or_else(|| Outcome::Failed("No value came back".into())),
            Source::Attribute {
                read,
                offset,
                data_type,
            } => match &attribute_results[read] {
                Ok(bytes) => match decode(data_type, bytes, offset) {
                    Some(value) => Outcome::Value {
                        value,
                        text: None,
                        learned: None,
                    },
                    None => Outcome::Failed(format!(
                        "The attribute has {} bytes: no {} at offset {offset}",
                        bytes.len(),
                        data_type.name()
                    )),
                },
                Err(error) => Outcome::Failed(error.clone()),
            },
        })
        .collect())
}

fn frame(outcomes: &[Outcome], latency_us: u32) -> ValueFrame {
    ValueFrame {
        at_ms: now_ms(),
        latency_us,
        readings: outcomes
            .iter()
            .map(|outcome| match outcome {
                Outcome::Value { value, .. } => Reading {
                    value: *value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: Quality::Good,
                },
                Outcome::Failed(_) => Reading::bad(),
            })
            .collect(),
    }
}

/// Every controller and program tag the device lists, as points: arrays by
/// their first elements, structures by their atomic members.
async fn browse(
    client: &EnipClient,
    config: &EnipConfig,
    out: &Emitter,
) -> Result<Vec<Planned>, String> {
    let tags = client.browse().await.map_err(|error| error.to_string())?;
    let mut template_ids: Vec<u16> = tags.iter().filter_map(|tag| tag.template_id).collect();
    template_ids.sort_unstable();
    template_ids.dedup();
    let templates = if template_ids.is_empty() {
        Vec::new()
    } else {
        match mt_enip::read_templates(config, &template_ids).await {
            Ok(templates) => templates,
            Err(error) => {
                out.log(
                    LogLevel::Warn,
                    format!("Structure definitions could not be read, so structure tags are left out: {error}"),
                );
                Vec::new()
            }
        }
    };
    let mut planned = Vec::new();
    let mut push = |tag: String, hint: Option<&str>| {
        planned.push(Planned {
            name: String::new(),
            address: EnipAddress::Tag { tag },
            unit: String::new(),
            hint: hint.map(str::to_owned),
        });
    };
    for tag in &tags {
        match tag.template_id {
            Some(id) => {
                let Some(template) = templates.iter().find(|template| template.id == id) else {
                    continue;
                };
                if tag.element_count > 1 {
                    continue;
                }
                for member in &template.members {
                    // Hidden host words of packed BOOLs, and nested arrays.
                    if member.name.starts_with("ZZZZZZZZZZ")
                        || member.name.starts_with("__")
                        || member.array_elements > 1
                    {
                        continue;
                    }
                    push(
                        format!("{}.{}", tag.name, member.name),
                        Some(member.data_type.as_str()),
                    );
                }
            }
            None if tag.element_count > 1 => {
                for element in 0..tag.element_count.min(ARRAY_PREVIEW) {
                    push(
                        format!("{}[{element}]", tag.name),
                        Some(tag.data_type.as_str()),
                    );
                }
            }
            None => push(tag.name.clone(), Some(tag.data_type.as_str())),
        }
    }
    if planned.len() > BROWSE_LIMIT {
        out.log(
            LogLevel::Warn,
            format!(
                "The controller lists {} values; reading the first {BROWSE_LIMIT}. Edit the connection to pick the tags to read.",
                planned.len()
            ),
        );
        planned.truncate(BROWSE_LIMIT);
    }
    out.log(
        LogLevel::Info,
        format!(
            "Browsed {} tags: reading {} values",
            tags.len(),
            planned.len()
        ),
    );
    Ok(planned)
}

/// Act on a command while connected.
async fn handle(command: Control, client: &EnipClient, plan: &mut Plan, out: &Emitter) {
    match command {
        Control::Write { point, value } => {
            let result = write(client, plan, point, value).await;
            match &result {
                Ok(()) => out.log(
                    LogLevel::Info,
                    format!(
                        "Wrote {value} to {}",
                        plan.planned
                            .get(point as usize)
                            .map_or(String::new(), |p| p.address.to_string())
                    ),
                ),
                Err(error) => out.log(LogLevel::Warn, error.clone()),
            }
            out.written(point, result);
        }
        Control::CipRequest(request) => {
            let started = Instant::now();
            let result = client
                .generic_cip(
                    request.service,
                    request.class,
                    request.instance,
                    request.attribute,
                    request.data.clone(),
                )
                .await
                .map(|data| CipResponse {
                    request,
                    data,
                    latency_us: u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX),
                })
                .map_err(|error| strip(&error.to_string()));
            out.cip_reply(result);
        }
        // Clients hold nothing and publish nothing.
        _ => {}
    }
}

/// `mt-enip` wraps errors as `Protocol error: …` and the like: the app
/// shows the reason.
fn strip(error: &str) -> String {
    ["Protocol error: ", "Read failed: ", "Write failed: "]
        .iter()
        .find_map(|prefix| error.strip_prefix(prefix))
        .unwrap_or(error)
        .to_owned()
}

async fn write(client: &EnipClient, plan: &Plan, point: u32, value: f64) -> Result<(), String> {
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    let source = *plan
        .sources
        .get(index)
        .ok_or_else(|| format!("No point {point}"))?;
    match source {
        Source::Tag(tag) => {
            if !plan.numeric[index] {
                return Err("Only numbers and BOOLs are written from here".into());
            }
            let known = plan.labels[index].as_deref().and_then(CipType::parse);
            if let Some(data_type) = known {
                encode(data_type, value)?;
            } else if !value.is_finite() {
                return Err("Set a number".into());
            }
            let value = match known {
                Some(CipType::Bool) => CipValue::Bool(value != 0.0),
                _ => CipValue::Lreal(value),
            };
            client
                .write(&plan.tags[tag], value)
                .await
                .map_err(|error| strip(&error.to_string()))
        }
        Source::Attribute {
            read,
            offset,
            data_type,
        } => {
            let encoded = encode(data_type, value)?;
            let (class, instance, attribute) = plan.attributes[read];
            // Set_Attribute_Single takes the whole attribute: read it, change
            // this value's bytes, write it back.
            let mut bytes = client
                .generic_cip(
                    GET_ATTRIBUTE_SINGLE,
                    class,
                    instance,
                    Some(attribute),
                    Vec::new(),
                )
                .await
                .map_err(|error| strip(&error.to_string()))?;
            let end = offset + encoded.len();
            if bytes.len() < end {
                return Err(format!(
                    "The attribute has {} bytes: no {} at offset {offset}",
                    bytes.len(),
                    data_type.name()
                ));
            }
            bytes[offset..end].copy_from_slice(&encoded);
            client
                .set_attribute_single(class, instance, attribute, bytes)
                .await
                .map_err(|error| strip(&error.to_string()))
        }
    }
}

/// Answer a command that arrived while there is no session.
fn refuse(command: Control, out: &Emitter) {
    match command {
        Control::Write { point, .. } => out.written(point, Err("Not connected".into())),
        Control::CipRequest(_) => out.cip_reply(Err("Not connected".into())),
        _ => {}
    }
}

/// Report a lost session and wait before the next attempt. Returns `false`
/// when the app closed the session while waiting.
async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let wait = backoff(attempt);
    out.state(SessionState::Reconnecting {
        attempt,
        retry_in_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        reason: reason.to_owned(),
    });
    tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = until_closed(control, out) => false,
    }
}

/// Wait for the app to close the session while there is none, refusing
/// writes and requests meanwhile.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(command) => refuse(command, out),
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Planned {
        Planned {
            name: String::new(),
            address: EnipAddress::Tag { tag: name.into() },
            unit: String::new(),
            hint: None,
        }
    }

    #[test]
    fn attribute_points_share_one_read_per_attribute() {
        let field = |offset: u16, data_type| Planned {
            name: String::new(),
            address: EnipAddress::Attribute {
                class: 4,
                instance: 100,
                attribute: 3,
                offset,
                data_type,
            },
            unit: String::new(),
            hint: None,
        };
        let plan = Plan::new(vec![
            tag("Speed"),
            field(0, CipType::Real),
            tag("Program:Main.Run"),
            field(4, CipType::Dint),
        ]);
        assert_eq!(plan.tags.len(), 2);
        assert_eq!(plan.attributes, [(4, 100, 3)]);
        let points = plan.points();
        assert_eq!(points[2].name, "Run");
        assert_eq!(points[2].address, "Program:Main.Run");
        // Tag types are unknown until read; attribute types are given.
        assert_eq!(points[0].data_type, "—");
        assert_eq!(points[3].data_type, "DINT");
    }

    #[test]
    fn learning_types_reannounces_only_on_change() {
        let mut plan = Plan::new(vec![tag("Speed"), tag("Name"), tag("Missing")]);
        let outcomes = vec![
            tag_outcome(&CipValue::Real(1.5)),
            tag_outcome(&CipValue::String("Line 4".into())),
            Outcome::Failed("CIP error 0x04".into()),
        ];
        assert!(plan.learn(&outcomes), "the first poll announces the points");
        let points = plan.points();
        assert_eq!(points[0].data_type, "REAL");
        assert!(points[0].writable);
        assert_eq!(points[1].data_type, "STRING");
        assert!(
            !points[1].writable,
            "strings are not written from the inspector"
        );
        assert!(!plan.learn(&outcomes));
        assert!(plan.learn(&[
            tag_outcome(&CipValue::Real(1.5)),
            tag_outcome(&CipValue::String("Line 4".into())),
            tag_outcome(&CipValue::Bool(true)),
        ]));
        assert!(plan.points()[2].boolean);
        // Texts go out when they change.
        assert_eq!(plan.texts(&outcomes).len(), 1);
        assert!(plan.texts(&outcomes).is_empty());
        // A failure is logged once until the point recovers.
        assert_eq!(plan.newly_failed(&outcomes).len(), 1);
        assert!(plan.newly_failed(&outcomes).is_empty());
    }

    #[test]
    fn transport_failures_are_told_from_refusals() {
        assert!(link_lost(
            "Read failed: Timed out waiting for the device's reply"
        ));
        assert!(link_lost(
            "Read failed: EtherNet/IP connection closed by the device"
        ));
        assert!(link_lost(
            "Read failed: EtherNet/IP encapsulation error 0x0064: Invalid session handle"
        ));
        assert!(!link_lost(
            "Read failed: reading tag 'Nope': CIP error 0x04: Path segment error (extended status 0x0000)"
        ));
        assert_eq!(
            strip("Protocol error: CIP error 0x14: Attribute not supported"),
            "CIP error 0x14: Attribute not supported"
        );
    }

    #[test]
    fn ipv6_hosts_are_bracketed_for_the_session() {
        let spec = EnipClientSpec {
            host: "fe80::1".into(),
            route_path: " 1,0 ".into(),
            ..EnipClientSpec::default()
        };
        let config = config(&spec);
        assert_eq!(config.host, "[fe80::1]");
        assert_eq!(config.route_path.as_deref(), Some("1,0"));
        assert_eq!(super::config(&EnipClientSpec::default()).route_path, None);
    }
}
