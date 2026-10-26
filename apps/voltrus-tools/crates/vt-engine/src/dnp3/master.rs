//! Polls a DNP3 outstation over TCP: an integrity poll on connect and every
//! integrity period, class 1-3 event polls between them, every point the
//! outstation reports (named ones first) with its flags as quality, controls
//! by select-before-operate or direct operate, and the outstation's internal
//! indications acted on: time written when it asks, a restart cleared and
//! followed by an integrity poll, an event overflow recovered.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::Duration;

use mt_dnp3::objects::group_variation_name;
use mt_dnp3::{
    ControlObject, ControlRequest, Crob, Dnp3Client, Dnp3Config, PollResponse, TrafficRecord,
};
use mt_protocol::ProtocolError;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    DNP3_MAX_ADDRESS, DNP3_MAX_CONTROL_INDEX, Dnp3Action, Dnp3ControlMode, Dnp3Crob, Dnp3Iin,
    Dnp3MasterSpec, Dnp3Operation, Dnp3PointKind, Dnp3PointName, LogLevel, PointInfo, PointText,
    Quality, Reading, SessionState, Stats, ValueFrame,
};

use super::{data_type, flags_good, flags_text, iin_names, iin_of, point_label, traffic, utc};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Fastest event poll and integrity poll the engine runs.
const MIN_EVENT_POLL: Duration = Duration::from_millis(100);
const MIN_INTEGRITY_POLL: Duration = Duration::from_secs(1);
/// Timeouts in a row that make the master drop the link and connect again.
const TIMEOUTS_BEFORE_RECONNECT: u32 = 3;
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// How often the app sees the frames exchanged.
const TRAFFIC_TICK: Duration = Duration::from_millis(250);

const IIN1_NEED_TIME: u8 = 0x10;
const IIN1_DEVICE_RESTART: u8 = 0x80;
const IIN2_EVENT_OVERFLOW: u8 = 0x08;

/// What the master learned about one point.
struct Point {
    kind: Dnp3PointKind,
    index: u16,
    name: String,
    unit: String,
    value: Option<f64>,
    flags: Option<u8>,
    /// The static variation the outstation reports it in; 0 until it has.
    variation: u8,
    /// Time stamp of the last event that carried one.
    event_at: Option<u64>,
    /// The status text last sent to the app.
    status: String,
}

impl Point {
    fn new(kind: Dnp3PointKind, index: u16, name: String, unit: String) -> Self {
        Self {
            kind,
            index,
            name,
            unit,
            value: None,
            flags: None,
            variation: 0,
            event_at: None,
            status: String::new(),
        }
    }

    fn good(&self) -> bool {
        self.value.is_some() && flags_good(self.kind, self.flags)
    }

    fn reading(&self) -> Reading {
        match self.value {
            Some(value) => Reading {
                value,
                raw: [0; 4],
                raw_len: 0,
                quality: if flags_good(self.kind, self.flags) {
                    Quality::Good
                } else {
                    Quality::Bad
                },
            },
            None => Reading::bad(),
        }
    }

    /// What the inspector shows behind the value: object, flags, last event.
    fn status_text(&self) -> String {
        let mut lines = Vec::with_capacity(3);
        if self.variation != 0 {
            let group = self.kind.group();
            lines.push(format!(
                "g{group}v{} · {}",
                self.variation,
                group_variation_name(group, self.variation)
            ));
        }
        lines.push(format!("Flags: {}", flags_text(self.kind, self.flags)));
        if let Some(at) = self.event_at {
            lines.push(format!("Last event: {}", utc(at)));
        }
        lines.join("\n")
    }

    fn label(&self) -> String {
        format!("{} {}", point_label(self.kind, self.index), self.name)
    }
}

/// Every point of the outstation the session knows, in the order reported
/// to the app: named points first, then the others as they appear.
struct Points {
    list: Vec<Point>,
    by_key: HashMap<(Dnp3PointKind, u16), usize>,
}

/// What one response changed.
#[derive(Default)]
struct Taken {
    added: Vec<PointInfo>,
    statuses: Vec<PointText>,
    events: usize,
}

impl Points {
    fn new(names: &[Dnp3PointName]) -> Self {
        let mut points = Self {
            list: Vec::with_capacity(names.len()),
            by_key: HashMap::with_capacity(names.len()),
        };
        for name in names {
            let key = (name.kind, name.index);
            if points.by_key.contains_key(&key) {
                continue;
            }
            points.by_key.insert(key, points.list.len());
            let label = match name.name.trim() {
                "" => format!("{} {}", name.kind.label(), name.index),
                text => text.to_owned(),
            };
            points
                .list
                .push(Point::new(name.kind, name.index, label, name.unit.clone()));
        }
        points
    }

    fn info(&self, id: usize) -> PointInfo {
        let point = &self.list[id];
        PointInfo {
            id: u32::try_from(id).unwrap_or(u32::MAX),
            name: point.name.clone(),
            address: point_label(point.kind, point.index),
            data_type: data_type(point.kind).into(),
            unit: point.unit.clone(),
            writable: point.kind.writable(),
            boolean: point.kind.is_binary(),
            states: Vec::new(),
        }
    }

    fn infos(&self) -> Vec<PointInfo> {
        (0..self.list.len()).map(|id| self.info(id)).collect()
    }

    fn get(&self, point: u32) -> Option<&Point> {
        self.list.get(usize::try_from(point).ok()?)
    }

    /// Take a response's points: add the ones not seen before (sorted by
    /// type and index), update values, flags and event times, and log binary
    /// events and quality changes.
    fn take(&mut self, response: &PollResponse, out: &Emitter) -> Taken {
        let mut taken = Taken::default();
        let mut new: Vec<(Dnp3PointKind, u16)> = response
            .points
            .iter()
            .filter_map(|value| {
                Dnp3PointKind::from_group(value.group).map(|kind| (kind, value.index))
            })
            .filter(|key| !self.by_key.contains_key(key))
            .collect();
        new.sort_by_key(|(kind, index)| {
            (
                Dnp3PointKind::ALL.iter().position(|each| each == kind),
                *index,
            )
        });
        new.dedup();
        for (kind, index) in new {
            let id = self.list.len();
            self.by_key.insert((kind, index), id);
            self.list.push(Point::new(
                kind,
                index,
                format!("{} {index}", kind.label()),
                String::new(),
            ));
            taken.added.push(self.info(id));
        }
        let mut touched = Vec::new();
        for value in &response.points {
            let Some(kind) = Dnp3PointKind::from_group(value.group) else {
                continue;
            };
            let Some(&id) = self.by_key.get(&(kind, value.index)) else {
                continue;
            };
            let point = &mut self.list[id];
            let was_good = point.good();
            let had_value = point.value.is_some();
            let before = point.value;
            point.value = Some(value.value);
            point.flags = value.flags;
            if value.event {
                taken.events += 1;
                if value.time.is_some() {
                    point.event_at = value.time;
                }
                if kind.is_binary() && before != Some(value.value) {
                    let when = value
                        .time
                        .map(|at| format!(" at {}", utc(at)))
                        .unwrap_or_default();
                    out.log(
                        LogLevel::Info,
                        format!(
                            "{} → {}{when}",
                            point.label(),
                            if value.value != 0.0 { "ON" } else { "OFF" }
                        ),
                    );
                }
            } else {
                point.variation = value.variation;
            }
            if had_value && was_good != point.good() {
                if point.good() {
                    out.log(LogLevel::Info, format!("{} is good again", point.label()));
                } else {
                    out.log(
                        LogLevel::Warn,
                        format!("{}: {}", point.label(), flags_text(kind, point.flags)),
                    );
                }
            }
            touched.push(id);
        }
        touched.sort_unstable();
        touched.dedup();
        for id in touched {
            let point = &mut self.list[id];
            let status = point.status_text();
            if status != point.status {
                point.status.clone_from(&status);
                taken.statuses.push(PointText {
                    point: u32::try_from(id).unwrap_or(u32::MAX),
                    text: status,
                });
            }
        }
        taken
    }

    fn frame(&self, latency_us: u32) -> ValueFrame {
        ValueFrame {
            at_ms: now_ms(),
            latency_us,
            readings: self.list.iter().map(Point::reading).collect(),
        }
    }

    fn bad_frame(&self) -> ValueFrame {
        ValueFrame {
            at_ms: now_ms(),
            latency_us: 0,
            readings: vec![Reading::bad(); self.list.len()],
        }
    }
}

/// One exchange with the outstation, queued.
#[derive(Debug)]
enum Job {
    Poll {
        integrity: bool,
        classes: [bool; 3],
        /// Asked for by the app: answered with `Dnp3Done`.
        asked: Option<Dnp3Action>,
    },
    Write {
        point: u32,
        value: f64,
    },
    Operate {
        point: u32,
        operation: Dnp3Operation,
        mode: Dnp3ControlMode,
        asked: Dnp3Action,
    },
    /// Time sync, freeze, restarts.
    Command(Dnp3Action),
    /// Write the time because the outstation asked (IIN1.4).
    AutoTimeSync,
    /// Clear IIN1.7 after a restart.
    ClearRestart,
}

impl Job {
    fn integrity() -> Self {
        Self::Poll {
            integrity: true,
            classes: [false; 3],
            asked: None,
        }
    }

    fn events() -> Self {
        Self::Poll {
            integrity: false,
            classes: [true; 3],
            asked: None,
        }
    }

    /// The job behind a control from the app, if it is one this session runs;
    /// the others are answered here.
    fn of(control: Control, out: &Emitter) -> Option<Self> {
        match control {
            Control::Write { point, value } => Some(Self::Write { point, value }),
            Control::Dnp3(action) => match action {
                Dnp3Action::IntegrityPoll => Some(Self::Poll {
                    integrity: true,
                    classes: [false; 3],
                    asked: Some(action),
                }),
                Dnp3Action::ClassPoll { class } => {
                    if (1..=3).contains(&class) {
                        let mut classes = [false; 3];
                        classes[usize::from(class) - 1] = true;
                        Some(Self::Poll {
                            integrity: false,
                            classes,
                            asked: Some(action),
                        })
                    } else {
                        out.dnp3_done(action, Err(format!("There is no event class {class}")));
                        None
                    }
                }
                Dnp3Action::Operate {
                    point,
                    operation,
                    mode,
                } => Some(Self::Operate {
                    point,
                    operation,
                    mode,
                    asked: action,
                }),
                Dnp3Action::SetOnline { .. } => {
                    out.dnp3_done(
                        action,
                        Err("Only an outstation simulator takes its points offline".into()),
                    );
                    None
                }
                Dnp3Action::TimeSync
                | Dnp3Action::FreezeCounters
                | Dnp3Action::ColdRestart
                | Dnp3Action::WarmRestart => Some(Self::Command(action)),
            },
            Control::Relinquish { point } => {
                out.written(
                    point,
                    Err("DNP3 has no command priorities to relinquish".into()),
                );
                None
            }
            // Masters serve nothing, so there is nothing to hold or publish.
            _ => None,
        }
    }

    /// Answer a job that will not run because the link is gone.
    fn refuse(self, out: &Emitter) {
        match self {
            Self::Write { point, .. } => out.written(point, Err("Not connected".into())),
            Self::Operate { point, asked, .. } => {
                out.written(point, Err("Not connected".into()));
                out.dnp3_done(asked, Err("Not connected".into()));
            }
            Self::Command(action)
            | Self::Poll {
                asked: Some(action),
                ..
            } => out.dnp3_done(action, Err("Not connected".into())),
            _ => {}
        }
    }
}

/// What the master keeps about the link between exchanges.
struct Station {
    iin: Option<Dnp3Iin>,
    time_sync_tried: bool,
    restart_handled: bool,
    overflow_handled: bool,
    timeouts: u32,
}

impl Station {
    fn new() -> Self {
        Self {
            iin: None,
            time_sync_tried: false,
            restart_handled: false,
            overflow_handled: false,
            timeouts: 0,
        }
    }
}

pub(crate) async fn run(
    spec: Dnp3MasterSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    if let Err(error) = validate(&spec) {
        fail(out, error);
        return;
    }
    let mut points = Points::new(&spec.points);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;
    let integrity_every = Duration::from_millis(spec.integrity_ms).max(MIN_INTEGRITY_POLL);
    let events_every =
        (spec.event_ms > 0).then(|| Duration::from_millis(spec.event_ms).max(MIN_EVENT_POLL));

    'session: loop {
        out.state(SessionState::Connecting);
        let (tap, mut records) = mpsc::unbounded_channel::<TrafficRecord>();
        let connected = tokio::select! {
            result = connect(&spec, tap) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        flush(&mut records, &mut stats, out);
        let (client, address) = match connected {
            Ok(link) => link,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.log(LogLevel::Error, error.clone());
                out.stats(stats);
                if !retry(&mut control, attempt, &error, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        attempt = 0;
        client
            .set_response_timeout(Duration::from_millis(spec.timeout_ms.max(100)))
            .await;
        out.points(points.infos());
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Connected to outstation {} at {address} as master {}",
                spec.outstation_address, spec.master_address
            ),
        );

        let mut station = Station::new();
        let mut queue: VecDeque<Job> = VecDeque::new();
        let mut integrity = tokio::time::interval(integrity_every);
        integrity.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut events = events_every.map(|every| {
            let mut ticker = tokio::time::interval_at(Instant::now() + every, every);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticker
        });
        let mut traffic_tick = tokio::time::interval(TRAFFIC_TICK);
        traffic_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            let job = match queue.pop_front() {
                Some(job) => job,
                None => tokio::select! {
                    _ = integrity.tick() => Job::integrity(),
                    _ = tick(events.as_mut()) => Job::events(),
                    _ = traffic_tick.tick() => {
                        flush(&mut records, &mut stats, out);
                        out.stats(stats);
                        continue;
                    }
                    command = control.recv() => match command {
                        None | Some(Control::Close) => {
                            let _ = mt_protocol::ProtocolClient::disconnect(&client).await;
                            flush(&mut records, &mut stats, out);
                            break 'session;
                        }
                        Some(command) => match Job::of(command, out) {
                            Some(job) => job,
                            None => continue,
                        },
                    },
                },
            };
            // An explicit integrity poll restarts the integrity period.
            if matches!(
                job,
                Job::Poll {
                    integrity: true,
                    ..
                }
            ) {
                integrity.reset();
            }
            let mut follow = Vec::new();
            let exchange = execute(
                &client,
                job,
                &mut points,
                &mut follow,
                &spec,
                out,
                &mut stats,
            );
            let Some(result) = or_close(exchange, &mut control, &mut queue, out).await else {
                let _ = mt_protocol::ProtocolClient::disconnect(&client).await;
                flush(&mut records, &mut stats, out);
                break 'session;
            };
            // A control's feedback comes with the event poll it queued: run it first.
            for job in follow.into_iter().rev() {
                queue.push_front(job);
            }
            match result {
                Ok(()) => {
                    station.timeouts = 0;
                    let iin = iin_of(&client.last_iin().await);
                    react(&spec, &mut station, iin, &mut queue, out);
                }
                Err(error) => {
                    stats.errors += 1;
                    let lost = match &error {
                        ProtocolError::Timeout(_) => {
                            station.timeouts += 1;
                            station.timeouts >= TIMEOUTS_BEFORE_RECONNECT
                        }
                        other => connection_lost(other),
                    };
                    out.log(
                        if lost {
                            LogLevel::Error
                        } else {
                            LogLevel::Warn
                        },
                        error.to_string(),
                    );
                    if lost {
                        out.values(points.bad_frame());
                        for job in queue.drain(..) {
                            job.refuse(out);
                        }
                        flush(&mut records, &mut stats, out);
                        out.stats(stats);
                        let _ = mt_protocol::ProtocolClient::disconnect(&client).await;
                        attempt = 1;
                        if !retry(&mut control, attempt, &error.to_string(), out).await {
                            break 'session;
                        }
                        continue 'session;
                    }
                }
            }
            flush(&mut records, &mut stats, out);
            out.stats(stats);
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

async fn tick(ticker: Option<&mut tokio::time::Interval>) {
    match ticker {
        Some(ticker) => {
            ticker.tick().await;
        }
        None => std::future::pending().await,
    }
}

fn validate(spec: &Dnp3MasterSpec) -> Result<(), String> {
    if spec.master_address > DNP3_MAX_ADDRESS || spec.outstation_address > DNP3_MAX_ADDRESS {
        return Err(format!(
            "Link addresses run from 0 to {DNP3_MAX_ADDRESS}; above that they are reserved"
        ));
    }
    if spec.master_address == spec.outstation_address {
        return Err("The master and the outstation need different link addresses".into());
    }
    if spec.host.trim().is_empty() {
        return Err("Enter the outstation's address".into());
    }
    Ok(())
}

fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

/// Errors that mean the TCP link is gone, not that the outstation refused.
fn connection_lost(error: &ProtocolError) -> bool {
    match error {
        ProtocolError::ConnectionFailed(_) | ProtocolError::NotConnected => true,
        ProtocolError::ReadFailed(message) | ProtocolError::WriteFailed(message) => {
            message.contains("Read frame")
                || message.contains("Send ")
                || message.contains("start bytes")
                || message.contains("Invalid LENGTH")
        }
        _ => false,
    }
}

/// Resolve the outstation and open the link (TCP, then RESET LINK STATES).
async fn connect(
    spec: &Dnp3MasterSpec,
    tap: mpsc::UnboundedSender<TrafficRecord>,
) -> Result<(Dnp3Client, String), String> {
    let address = crate::bacnet::resolve(&spec.host, spec.port).await?;
    let host = match address.ip() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    let config = Dnp3Config {
        id: format!("outstation {}", spec.outstation_address),
        host,
        port: address.port(),
        master_address: spec.master_address,
        outstation_address: spec.outstation_address,
        poll_interval_ms: spec.sample_ms(),
    };
    Dnp3Client::connect_with_tap(config, Some(tap))
        .await
        .map(|client| (client, address.to_string()))
        .map_err(|error| match error {
            ProtocolError::ConnectionFailed(reason) | ProtocolError::Timeout(reason) => {
                format!("Cannot reach the outstation at {address}: {reason}")
            }
            other => other.to_string(),
        })
}

/// Report the frames exchanged since the last flush.
fn flush(records: &mut mpsc::UnboundedReceiver<TrafficRecord>, stats: &mut Stats, out: &Emitter) {
    let mut frames = Vec::new();
    while let Ok(record) = records.try_recv() {
        frames.push(traffic(&record));
    }
    stats.frames += frames.len() as u64;
    out.traffic(frames);
}

/// Run `exchange` unless the app closes the session first (`None`).
/// Controls that arrive meanwhile are queued behind it.
async fn or_close<T>(
    exchange: impl Future<Output = T>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    queue: &mut VecDeque<Job>,
    out: &Emitter,
) -> Option<T> {
    tokio::pin!(exchange);
    loop {
        tokio::select! {
            result = &mut exchange => return Some(result),
            command = control.recv() => match command {
                None | Some(Control::Close) => return None,
                Some(command) => {
                    if let Some(job) = Job::of(command, out) {
                        queue.push_back(job);
                    }
                }
            },
        }
    }
}

/// Act on the outstation's internal indications: report changes, write the
/// time when asked, clear a restart and poll everything again, recover from
/// an event buffer overflow.
fn react(
    spec: &Dnp3MasterSpec,
    station: &mut Station,
    iin: Dnp3Iin,
    queue: &mut VecDeque<Job>,
    out: &Emitter,
) {
    if station.iin != Some(iin) {
        let before = station.iin.unwrap_or_default();
        let raised = Dnp3Iin {
            iin1: iin.iin1 & !before.iin1,
            iin2: iin.iin2 & !before.iin2,
        };
        let problems: Vec<&str> = raised
            .set()
            .iter()
            .filter(|bit| bit.problem)
            .map(|bit| bit.name)
            .collect();
        if !problems.is_empty() {
            out.log(
                LogLevel::Warn,
                format!("Outstation indicates {}", problems.join(", ")),
            );
        } else if station.iin.is_none() {
            out.log(
                LogLevel::Info,
                format!("Internal indications: {}", iin_names(iin)),
            );
        }
        station.iin = Some(iin);
        out.dnp3_iin(iin);
    }

    if iin.iin1 & IIN1_NEED_TIME != 0 {
        if spec.time_sync && !station.time_sync_tried {
            station.time_sync_tried = true;
            queue.push_back(Job::AutoTimeSync);
        }
    } else {
        station.time_sync_tried = false;
    }
    if iin.iin1 & IIN1_DEVICE_RESTART != 0 {
        if !station.restart_handled {
            station.restart_handled = true;
            queue.push_back(Job::ClearRestart);
            queue.push_back(Job::integrity());
        }
    } else {
        station.restart_handled = false;
    }
    if iin.iin2 & IIN2_EVENT_OVERFLOW != 0 {
        if !station.overflow_handled {
            station.overflow_handled = true;
            out.log(
                LogLevel::Warn,
                "The outstation's event buffer overflowed: polling everything again",
            );
            queue.push_back(Job::integrity());
        }
    } else {
        station.overflow_handled = false;
    }
}

/// Run one job. Errors that the outstation answered (a refused control, an
/// unsupported function) are reported to the app here and come back `Ok`;
/// the error returned is the exchange itself failing.
async fn execute(
    client: &Dnp3Client,
    job: Job,
    points: &mut Points,
    follow: &mut Vec<Job>,
    spec: &Dnp3MasterSpec,
    out: &Emitter,
    stats: &mut Stats,
) -> Result<(), ProtocolError> {
    match job {
        Job::Poll {
            integrity,
            classes,
            asked,
        } => {
            let started = Instant::now();
            let result = if integrity {
                client.integrity_poll().await
            } else {
                client
                    .poll_classes(classes[0], classes[1], classes[2])
                    .await
            };
            let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
            stats.polls += 1;
            stats.last_latency_us = latency_us;
            match result {
                Ok(response) => {
                    for issue in &response.issues {
                        out.log(
                            LogLevel::Warn,
                            format!("Response not fully decoded: {issue}"),
                        );
                    }
                    let taken = points.take(&response, out);
                    if !taken.added.is_empty() {
                        if integrity && asked.is_none() && taken.added.len() == points.list.len() {
                            out.log(
                                LogLevel::Info,
                                format!("The outstation reports {} points", taken.added.len()),
                            );
                        }
                        out.points_added(taken.added);
                    }
                    out.values(points.frame(latency_us));
                    if !taken.statuses.is_empty() {
                        out.payloads(taken.statuses);
                    }
                    if let Some(action) = asked {
                        let static_points = response.points.len().saturating_sub(taken.events);
                        out.dnp3_done(
                            action,
                            Ok(if integrity {
                                format!("{static_points} static values, {} events", taken.events)
                            } else {
                                format!(
                                    "{} event{}",
                                    taken.events,
                                    if taken.events == 1 { "" } else { "s" }
                                )
                            }),
                        );
                    }
                    Ok(())
                }
                Err(error) => {
                    if let Some(action) = asked {
                        out.dnp3_done(action, Err(error.to_string()));
                    }
                    if connection_lost(&error) || matches!(error, ProtocolError::Timeout(_)) {
                        Err(error)
                    } else {
                        // Refused in the IIN: the link is fine.
                        out.log(LogLevel::Warn, error.to_string());
                        Ok(())
                    }
                }
            }
        }
        Job::Write { point, value } => {
            let operation = match write_operation(points, point, value) {
                Ok(operation) => operation,
                Err(error) => {
                    out.written(point, Err(error));
                    return Ok(());
                }
            };
            operate(
                client,
                points,
                point,
                operation,
                spec.control_mode,
                None,
                follow,
                out,
            )
            .await
        }
        Job::Operate {
            point,
            operation,
            mode,
            asked,
        } => {
            operate(
                client,
                points,
                point,
                operation,
                mode,
                Some(asked),
                follow,
                out,
            )
            .await
        }
        Job::Command(action) => {
            let (result, done) = match action {
                Dnp3Action::TimeSync => (client.time_sync().await, "Time written"),
                Dnp3Action::FreezeCounters => (client.freeze_counters().await, "Counters frozen"),
                Dnp3Action::ColdRestart => {
                    (client.cold_restart().await, "The outstation is restarting")
                }
                Dnp3Action::WarmRestart => {
                    (client.warm_restart().await, "The outstation is restarting")
                }
                _ => return Ok(()),
            };
            report(action, result, done, out)
        }
        Job::AutoTimeSync => {
            let result = client.time_sync().await;
            if result.is_ok() {
                out.log(
                    LogLevel::Info,
                    "Wrote the time: the outstation asked for it (IIN1.4)",
                );
            }
            silent(result, "Time sync", out)
        }
        Job::ClearRestart => {
            out.log(
                LogLevel::Warn,
                "The outstation restarted (IIN1.7): clearing it and polling everything again",
            );
            silent(
                client.clear_restart().await,
                "Clearing the restart indication",
                out,
            )
        }
    }
}

/// Report a station command's outcome; only a failed exchange is returned.
fn report(
    action: Dnp3Action,
    result: Result<(), ProtocolError>,
    done: &str,
    out: &Emitter,
) -> Result<(), ProtocolError> {
    match result {
        Ok(()) => {
            out.log(LogLevel::Info, format!("{}: {done}", action.label()));
            out.dnp3_done(action, Ok(done.to_owned()));
            Ok(())
        }
        Err(error) => {
            out.dnp3_done(action, Err(error.to_string()));
            exchange_failure(error, out)
        }
    }
}

/// An automatic command's outcome: logged, nothing for the app to answer.
fn silent(
    result: Result<(), ProtocolError>,
    what: &str,
    out: &Emitter,
) -> Result<(), ProtocolError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let error = exchange_failure(error, out);
            if let Err(error) = &error {
                out.log(LogLevel::Warn, format!("{what}: {error}"));
            }
            error
        }
    }
}

/// Pass on a failed exchange; log a refusal and carry on.
fn exchange_failure(error: ProtocolError, out: &Emitter) -> Result<(), ProtocolError> {
    if connection_lost(&error) || matches!(error, ProtocolError::Timeout(_)) {
        Err(error)
    } else {
        out.log(LogLevel::Warn, error.to_string());
        Ok(())
    }
}

/// The control a plain write stands for: 1 latches a binary output on, 0
/// off; an analog output takes the value.
fn write_operation(points: &Points, point: u32, value: f64) -> Result<Dnp3Operation, String> {
    let target = points
        .get(point)
        .ok_or_else(|| format!("No point {point}"))?;
    match target.kind {
        Dnp3PointKind::BinaryOutput => {
            if value == 1.0 {
                Ok(Dnp3Operation::Crob(Dnp3Crob::new(
                    vt_protocol::Dnp3CrobCode::LatchOn,
                )))
            } else if value == 0.0 {
                Ok(Dnp3Operation::Crob(Dnp3Crob::new(
                    vt_protocol::Dnp3CrobCode::LatchOff,
                )))
            } else {
                Err("Write 1 to latch the output on, 0 to latch it off".into())
            }
        }
        Dnp3PointKind::AnalogOutput => Ok(Dnp3Operation::Analog { value }),
        kind => Err(format!(
            "{} is an input: DNP3 commands binary and analog outputs",
            point_label(kind, target.index)
        )),
    }
}

/// The control request for `operation` on `point`, checked against the point.
pub(crate) fn control_request(
    kind: Dnp3PointKind,
    index: u16,
    operation: Dnp3Operation,
) -> Result<ControlRequest, String> {
    if index > DNP3_MAX_CONTROL_INDEX {
        return Err(format!(
            "{} is above index {DNP3_MAX_CONTROL_INDEX}, the highest a control can address",
            point_label(kind, index)
        ));
    }
    let object = match (kind, operation) {
        (Dnp3PointKind::BinaryOutput, Dnp3Operation::Crob(crob)) => {
            if crob.count == 0 {
                return Err("A CROB operates at least once".into());
            }
            ControlObject::Crob(Crob {
                code: crob.code.code(),
                count: crob.count,
                on_time_ms: crob.on_ms,
                off_time_ms: crob.off_ms,
            })
        }
        (Dnp3PointKind::AnalogOutput, Dnp3Operation::Analog { value }) => {
            let single = value as f32;
            if !value.is_finite() || !single.is_finite() {
                return Err(format!(
                    "{value} does not fit an analog output block (float32)"
                ));
            }
            ControlObject::AnalogOutput(single)
        }
        (Dnp3PointKind::BinaryOutput, Dnp3Operation::Analog { .. }) => {
            return Err("A binary output takes a CROB: latch, pulse, trip or close".into());
        }
        (Dnp3PointKind::AnalogOutput, Dnp3Operation::Crob(_)) => {
            return Err("An analog output takes a value, not a CROB".into());
        }
        (kind, _) => {
            return Err(format!(
                "{} is an input: DNP3 commands binary and analog outputs",
                point_label(kind, index)
            ));
        }
    };
    Ok(ControlRequest { index, object })
}

/// Command an output and report the outcome; a successful control queues
/// an event poll so its feedback shows at once.
#[allow(clippy::too_many_arguments)]
async fn operate(
    client: &Dnp3Client,
    points: &Points,
    point: u32,
    operation: Dnp3Operation,
    mode: Dnp3ControlMode,
    asked: Option<Dnp3Action>,
    follow: &mut Vec<Job>,
    out: &Emitter,
) -> Result<(), ProtocolError> {
    let answer = |result: Result<(), String>| {
        if let Some(action) = asked {
            out.dnp3_done(
                action,
                result
                    .clone()
                    .map(|()| "The outstation accepted it".to_owned()),
            );
        }
        out.written(point, result);
    };
    let Some(target) = points.get(point) else {
        answer(Err(format!("No point {point}")));
        return Ok(());
    };
    let request = match control_request(target.kind, target.index, operation) {
        Ok(request) => request,
        Err(error) => {
            answer(Err(error));
            return Ok(());
        }
    };
    let label = target.label();
    let what = match operation {
        Dnp3Operation::Crob(crob) => crob.code.name().to_lowercase(),
        Dnp3Operation::Analog { value } => format!("set to {value}"),
    };
    let result = match mode {
        Dnp3ControlMode::SelectBeforeOperate => client.select_before_operate(&request).await,
        Dnp3ControlMode::DirectOperate => client.direct_operate(&request).await,
    };
    match result {
        Ok(()) => {
            out.log(
                LogLevel::Info,
                format!("{label}: {what} ({})", mode.name().to_lowercase()),
            );
            answer(Ok(()));
            follow.push(Job::events());
            Ok(())
        }
        Err(error) => {
            out.log(LogLevel::Warn, format!("{label}: {error}"));
            answer(Err(error.to_string()));
            if connection_lost(&error) || matches!(error, ProtocolError::Timeout(_)) {
                Err(error)
            } else {
                Ok(())
            }
        }
    }
}

/// Report a lost outstation and wait before the next attempt. Returns
/// `false` when the app closed the session while waiting.
async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let exponent = attempt.saturating_sub(1).min(5);
    let wait = (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF);
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

/// Wait for the app to close the session while there is no link, refusing
/// commands in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(command) => {
                if let Some(job) = Job::of(command, out) {
                    job.refuse(out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_dnp3::Dnp3PointValue;
    use mt_dnp3::application::Iin;
    use vt_protocol::Dnp3CrobCode;

    fn value(group: u8, index: u16, value: f64, flags: Option<u8>, event: bool) -> Dnp3PointValue {
        Dnp3PointValue {
            group,
            variation: if event { 7 } else { 5 },
            index,
            value,
            event,
            flags,
            time: event.then_some(1_700_000_000_000),
        }
    }

    fn emitter() -> (Emitter, async_channel::Receiver<vt_protocol::Event>) {
        let (sender, receiver) = async_channel::unbounded();
        (Emitter::new(vt_protocol::SessionId(1), sender), receiver)
    }

    #[test]
    fn named_points_come_first_and_new_ones_are_sorted() {
        let (out, _events) = emitter();
        let mut points = Points::new(&[Dnp3PointName {
            kind: Dnp3PointKind::BinaryOutput,
            index: 0,
            name: "Breaker".into(),
            unit: String::new(),
        }]);
        let response = PollResponse {
            iin: Iin::default(),
            points: vec![
                value(30, 1, 2.0, Some(0x01), false),
                value(1, 4, 1.0, Some(0x81), false),
                value(30, 0, 1.0, Some(0x01), false),
                value(10, 0, 1.0, Some(0x81), false),
                value(99, 0, 1.0, None, false),
            ],
            issues: Vec::new(),
        };
        let taken = points.take(&response, &out);
        let added: Vec<&str> = taken
            .added
            .iter()
            .map(|info| info.address.as_str())
            .collect();
        // Inputs before outputs, by index; an unknown group is no point.
        assert_eq!(added, ["BI 4", "AI 0", "AI 1"]);
        assert_eq!(taken.added[0].id, 1, "ids continue after the named point");
        assert_eq!(points.list[0].value, Some(1.0));
        assert!(points.info(0).writable && points.info(0).boolean);
        assert!(!points.info(2).writable);
        assert_eq!(taken.statuses.len(), 4);
        // Nothing new the second time, and unchanged statuses are not resent.
        let again = points.take(&response, &out);
        assert!(again.added.is_empty() && again.statuses.is_empty());
    }

    #[test]
    fn flags_make_quality_and_events_keep_their_time() {
        let (out, _events) = emitter();
        let mut points = Points::new(&[]);
        let first = PollResponse {
            iin: Iin::default(),
            points: vec![value(30, 0, 12.0, Some(0x01), false)],
            issues: Vec::new(),
        };
        points.take(&first, &out);
        assert_eq!(points.list[0].reading().quality, Quality::Good);
        let lost = PollResponse {
            iin: Iin::default(),
            points: vec![value(30, 0, 12.5, Some(0x04), true)],
            issues: Vec::new(),
        };
        let taken = points.take(&lost, &out);
        assert_eq!(taken.events, 1);
        let reading = points.list[0].reading();
        assert_eq!((reading.value, reading.quality), (12.5, Quality::Bad));
        assert!(taken.statuses[0].text.contains("OFFLINE, COMM_LOST"));
        assert!(
            taken.statuses[0]
                .text
                .contains("Last event: 2023-11-14 22:13:20.000 UTC")
        );
        // The static variation stays the one the static value came in.
        assert!(taken.statuses[0].text.starts_with("g30v5"));
    }

    #[test]
    fn controls_are_checked_against_the_point() {
        let latch = Dnp3Operation::Crob(Dnp3Crob::new(Dnp3CrobCode::LatchOn));
        let request = control_request(Dnp3PointKind::BinaryOutput, 3, latch).unwrap();
        assert_eq!(request.index, 3);
        assert!(
            matches!(request.object, ControlObject::Crob(crob) if crob.code == 0x03 && crob.count == 1)
        );
        let trip = Dnp3Operation::Crob(Dnp3Crob::new(Dnp3CrobCode::Trip));
        assert!(matches!(
            control_request(Dnp3PointKind::BinaryOutput, 0, trip).unwrap().object,
            ControlObject::Crob(crob) if crob.code == 0x81 && crob.on_time_ms == 1000
        ));
        let set = Dnp3Operation::Analog { value: 61.5 };
        assert!(matches!(
            control_request(Dnp3PointKind::AnalogOutput, 1, set).unwrap().object,
            ControlObject::AnalogOutput(value) if value == 61.5
        ));
        // Wrong object for the point, inputs, out of range, unencodable.
        assert!(control_request(Dnp3PointKind::AnalogOutput, 0, latch).is_err());
        assert!(control_request(Dnp3PointKind::BinaryOutput, 0, set).is_err());
        assert!(
            control_request(Dnp3PointKind::BinaryInput, 0, latch)
                .unwrap_err()
                .contains("input")
        );
        assert!(
            control_request(Dnp3PointKind::BinaryOutput, 256, latch)
                .unwrap_err()
                .contains("255")
        );
        assert!(
            control_request(
                Dnp3PointKind::AnalogOutput,
                0,
                Dnp3Operation::Analog { value: 1e300 }
            )
            .is_err()
        );
        let none = Dnp3Operation::Crob(Dnp3Crob {
            count: 0,
            ..Dnp3Crob::new(Dnp3CrobCode::PulseOn)
        });
        assert!(control_request(Dnp3PointKind::BinaryOutput, 0, none).is_err());
    }

    #[test]
    fn plain_writes_latch_binary_outputs_and_set_analog_ones() {
        let points = Points::new(&[
            Dnp3PointName {
                kind: Dnp3PointKind::BinaryOutput,
                index: 0,
                name: String::new(),
                unit: String::new(),
            },
            Dnp3PointName {
                kind: Dnp3PointKind::AnalogOutput,
                index: 0,
                name: String::new(),
                unit: String::new(),
            },
            Dnp3PointName {
                kind: Dnp3PointKind::AnalogInput,
                index: 0,
                name: String::new(),
                unit: String::new(),
            },
        ]);
        assert_eq!(
            points.list[0].name, "Binary output 0",
            "a blank name gets the type's"
        );
        assert!(
            matches!(write_operation(&points, 0, 1.0), Ok(Dnp3Operation::Crob(crob)) if crob.code == Dnp3CrobCode::LatchOn)
        );
        assert!(
            matches!(write_operation(&points, 0, 0.0), Ok(Dnp3Operation::Crob(crob)) if crob.code == Dnp3CrobCode::LatchOff)
        );
        assert!(write_operation(&points, 0, 0.5).is_err());
        assert!(
            matches!(write_operation(&points, 1, 7.25), Ok(Dnp3Operation::Analog { value }) if value == 7.25)
        );
        assert!(
            write_operation(&points, 2, 1.0)
                .unwrap_err()
                .contains("AI 0 is an input")
        );
        assert!(write_operation(&points, 9, 1.0).is_err());
    }

    #[test]
    fn indications_trigger_time_sync_restart_handling_and_recovery_once() {
        let (out, events) = emitter();
        let spec = Dnp3MasterSpec::default();
        let mut station = Station::new();
        let mut queue = VecDeque::new();
        let asking = Dnp3Iin {
            iin1: IIN1_NEED_TIME | IIN1_DEVICE_RESTART,
            iin2: IIN2_EVENT_OVERFLOW,
        };
        react(&spec, &mut station, asking, &mut queue, &out);
        let jobs: Vec<String> = queue.iter().map(|job| format!("{job:?}")).collect();
        assert_eq!(jobs.len(), 4, "{jobs:?}");
        assert!(jobs[0].starts_with("AutoTimeSync") && jobs[1].starts_with("ClearRestart"));
        // Still set on the next response: nothing more until they clear.
        queue.clear();
        react(&spec, &mut station, asking, &mut queue, &out);
        assert!(queue.is_empty());
        react(&spec, &mut station, Dnp3Iin::default(), &mut queue, &out);
        react(
            &spec,
            &mut station,
            Dnp3Iin {
                iin1: IIN1_NEED_TIME,
                iin2: 0,
            },
            &mut queue,
            &out,
        );
        assert_eq!(queue.len(), 1);
        // Without time sync the master leaves the clock alone.
        let manual = Dnp3MasterSpec {
            time_sync: false,
            ..Dnp3MasterSpec::default()
        };
        let mut station = Station::new();
        queue.clear();
        react(
            &manual,
            &mut station,
            Dnp3Iin {
                iin1: IIN1_NEED_TIME,
                iin2: 0,
            },
            &mut queue,
            &out,
        );
        assert!(queue.is_empty());
        // The app heard about each change of the indications.
        let mut reported = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event.kind, vt_protocol::EventKind::Dnp3Iin(_)) {
                reported += 1;
            }
        }
        assert_eq!(reported, 4);
    }

    #[test]
    fn link_errors_are_told_apart_from_refusals() {
        assert!(connection_lost(&ProtocolError::ReadFailed(
            "Read frame header failed: early eof".into()
        )));
        assert!(connection_lost(&ProtocolError::WriteFailed(
            "DNP3 select failed: Send ASDU failed: broken pipe".into()
        )));
        assert!(!connection_lost(&ProtocolError::ReadFailed(
            "DNP3 integrity poll refused by the outstation: parameter error (IIN2.2)".into()
        )));
        assert!(!connection_lost(&ProtocolError::WriteFailed(
            "DNP3 operate rejected by the outstation: LOCAL".into()
        )));
    }

    #[test]
    fn specs_with_clashing_or_reserved_addresses_are_refused() {
        assert!(validate(&Dnp3MasterSpec::default()).is_ok());
        let same = Dnp3MasterSpec {
            master_address: 10,
            ..Dnp3MasterSpec::default()
        };
        assert!(validate(&same).is_err());
        let reserved = Dnp3MasterSpec {
            outstation_address: 0xFFFD,
            ..Dnp3MasterSpec::default()
        };
        assert!(validate(&reserved).is_err());
    }
}
