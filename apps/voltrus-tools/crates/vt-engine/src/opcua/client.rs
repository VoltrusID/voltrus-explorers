//! Reads an OPC UA server: signs in on the endpoint matching the spec's security policy and mode
//! (anonymous, user name or user certificate), monitors the spec's variables in one subscription,
//! and on the app's request browses the address space, reads a node's attributes, monitors more
//! variables, writes typed values and calls methods.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mt_opcua::{OpcUaConfig, OpcUaProtocolClient, UaQuality, UaValue, UserIdentity};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, OpcuaArgument, OpcuaAttribute, OpcuaAuth, OpcuaBrowseNode,
    OpcuaClientSpec, OpcuaEvent, OpcuaMethodInfo, OpcuaNode, OpcuaNodeClass, OpcuaRequest,
    OpcuaScalar, PointInfo, PointText, Quality, Reading, SessionState, Stats, ValueFrame,
    opcua_host_port, opcua_writable_type, parse_opcua_value,
};

use super::{clock, traffic};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Values reach the app at most this often, however fast the server publishes.
const FLUSH: Duration = Duration::from_millis(100);
/// How often the client checks that its connection is still up.
const HEALTH: Duration = Duration::from_secs(1);
/// Longest a connection attempt (discovery, secure channel, session) may take.
const CONNECT_WAIT: Duration = Duration::from_secs(15);
/// Longest one request (browse, read, monitor, write, call) may take before it is answered with
/// an error, so a stalled server never freezes the session.
const REQUEST_WAIT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// A subscription silent this long is backed up by a direct read of the values.
const QUIET: Duration = Duration::from_secs(3);
/// Fastest publishing interval the engine asks for.
pub(crate) const MIN_PUBLISHING_MS: u32 = 50;

/// A monitored variable.
struct Point {
    node_id: String,
    data_type: String,
    info: PointInfo,
}

/// A server the client signed in to and what it monitors there.
struct Link {
    client: OpcUaProtocolClient,
    subscription: u32,
    points: Vec<Point>,
    by_node: HashMap<String, usize>,
}

/// The latest data change of a point.
#[derive(Clone, Default)]
struct PointState {
    number: Option<f64>,
    text: String,
    quality: Option<UaQuality>,
    status: String,
    source_ms: u64,
}

impl PointState {
    fn reading(&self) -> Reading {
        match (self.quality, self.number) {
            (Some(UaQuality::Good | UaQuality::Uncertain), Some(value)) if value.is_finite() => {
                Reading {
                    value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: Quality::Good,
                }
            }
            // Text and other non-numbers: the value is in the payload.
            (Some(UaQuality::Good | UaQuality::Uncertain), None) => Reading {
                value: f64::NAN,
                raw: [0; 4],
                raw_len: 0,
                quality: Quality::Good,
            },
            _ => Reading::bad(),
        }
    }

    /// The value, its status and source time, for the inspector.
    fn payload(&self) -> String {
        let mut text = format!("{}\n{}", self.text, self.status);
        if self.source_ms > 0 {
            text.push_str(&format!(" · source {} UTC", clock(self.source_ms)));
        }
        text
    }
}

pub(crate) async fn run(
    spec: OpcuaClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    super::init_pki();
    if opcua_host_port(&spec.endpoint).is_none() {
        let message = format!(
            "“{}” is not an OPC UA endpoint (opc.tcp://host:port)",
            spec.endpoint
        );
        out.log(LogLevel::Error, message.clone());
        out.state(SessionState::Closed {
            error: Some(message),
        });
        return;
    }
    let mut wanted: Vec<OpcuaNode> = spec.nodes.clone();
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;
    // The client's certificate, made before the connect wait starts: a new key can take longer
    // than the wait.
    out.state(SessionState::Connecting);
    let prepared = tokio::select! {
        prepared = OpcUaProtocolClient::prepare_certificate() => prepared,
        () = until_closed(&mut control, out) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
    };
    if let Err(error) = prepared {
        out.log(LogLevel::Error, error.clone());
        out.state(SessionState::Closed { error: Some(error) });
        return;
    }

    'session: loop {
        out.state(SessionState::Connecting);
        let (value_tx, mut value_rx) = mpsc::unbounded_channel::<UaValue>();
        let connected = tokio::select! {
            result = tokio::time::timeout(CONNECT_WAIT, connect(&spec, value_tx, out, &mut stats)) => {
                result.unwrap_or_else(|_| Err(format!("{} did not answer within 15 s", spec.endpoint)))
            }
            () = until_closed(&mut control, out) => break 'session,
        };
        let mut link = match connected {
            Ok(link) => link,
            Err(error) if refused(&error) => {
                // Retrying cannot fix a refused identity or a security setting the server lacks.
                out.log(LogLevel::Error, error.clone());
                out.state(SessionState::Closed { error: Some(error) });
                return;
            }
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
        let Ok((points, results)) =
            tokio::time::timeout(REQUEST_WAIT, add(&mut link, &wanted, spec.sampling_ms, out))
                .await
        else {
            let error = format!(
                "{} did not describe the variables within 10 s",
                spec.endpoint
            );
            stats.errors += 1;
            out.log(LogLevel::Error, error.clone());
            link.client.close().await;
            attempt += 1;
            if !retry(&mut control, attempt, &error, out).await {
                break 'session;
            }
            continue 'session;
        };
        // Variables the server no longer has stay saved, but are not points of this run.
        out.points(points);
        if results.iter().any(|(_, result)| result.is_err()) {
            out.opcua(OpcuaEvent::Monitored(results));
        }
        out.state(SessionState::Connected);
        let mut states = vec![PointState::default(); link.points.len()];
        let mut dirty = vec![false; link.points.len()];
        out.values(frame(&states, 0));
        out.stats(stats);

        let mut flush = tokio::time::interval(FLUSH);
        flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut health = tokio::time::interval(HEALTH);
        health.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut changed = false;
        // When the subscription has been quiet, values are read directly now and then, so a
        // change the subscription did not deliver still shows.
        let mut last_heard = Instant::now();
        loop {
            tokio::select! {
                Some(value) = value_rx.recv() => {
                    tracing::trace!(node = %value.node_id, "data change");
                    stats.frames += 1;
                    last_heard = Instant::now();
                    if let Some(&index) = link.by_node.get(&value.node_id) {
                        states[index] = PointState {
                            number: value.number,
                            text: value.text,
                            quality: Some(value.quality),
                            status: value.status,
                            source_ms: value.source_ms,
                        };
                        dirty[index] = true;
                        changed = true;
                    }
                }
                _ = flush.tick(), if changed => {
                    changed = false;
                    stats.polls += 1;
                    out.values(frame(&states, stats.last_latency_us));
                    let texts: Vec<PointText> = dirty
                        .iter_mut()
                        .enumerate()
                        .filter(|(_, dirty)| **dirty)
                        .map(|(index, dirty)| {
                            *dirty = false;
                            PointText {
                                point: u32::try_from(index).unwrap_or(u32::MAX),
                                text: states[index].payload(),
                            }
                        })
                        .collect();
                    out.payloads(texts);
                    out.stats(stats);
                }
                _ = health.tick() => {
                    tracing::trace!("health tick");
                    if !link.client.is_alive() {
                        let error = format!("Lost the session with {}", spec.endpoint);
                        stats.errors += 1;
                        out.log(LogLevel::Error, error.clone());
                        out.values(ValueFrame {
                            at_ms: now_ms(),
                            latency_us: 0,
                            readings: vec![Reading::bad(); states.len()],
                        });
                        out.stats(stats);
                        link.client.close().await;
                        attempt = 1;
                        if !retry(&mut control, attempt, &error, out).await {
                            break 'session;
                        }
                        continue 'session;
                    }
                    if !link.points.is_empty() && last_heard.elapsed() >= QUIET {
                        tracing::trace!("reading the values directly");
                        last_heard = Instant::now();
                        let ids: Vec<String> = link.points.iter().map(|point| point.node_id.clone()).collect();
                        let read = match tokio::time::timeout(REQUEST_WAIT, link.client.read_values(&ids)).await {
                            Ok(Ok(read)) => read,
                            Ok(Err(error)) => {
                                out.log(LogLevel::Warn, format!("Reading the values directly failed: {error}"));
                                Vec::new()
                            }
                            Err(_) => {
                                // A session that answers nothing is as good as lost: start over.
                                let error = format!("{} stopped answering", spec.endpoint);
                                stats.errors += 1;
                                out.log(LogLevel::Error, error.clone());
                                out.values(ValueFrame {
                                    at_ms: now_ms(),
                                    latency_us: 0,
                                    readings: vec![Reading::bad(); states.len()],
                                });
                                out.stats(stats);
                                link.client.close().await;
                                attempt = 1;
                                if !retry(&mut control, attempt, &error, out).await {
                                    break 'session;
                                }
                                continue 'session;
                            }
                        };
                        tracing::trace!(count = read.len(), "read the values directly");
                        let before = changed;
                        {
                            for (index, value) in read.into_iter().enumerate() {
                                let fresh = PointState {
                                    number: value.number,
                                    text: value.text,
                                    quality: Some(value.quality),
                                    status: value.status,
                                    source_ms: value.source_ms,
                                };
                                if let Some(state) = states.get_mut(index)
                                    && (state.number != fresh.number || state.text != fresh.text || state.quality != fresh.quality)
                                {
                                    *state = fresh;
                                    dirty[index] = true;
                                    changed = true;
                                }
                            }
                        }
                        if changed && !before {
                            out.log(LogLevel::Info, "The subscription was quiet; read the values directly");
                        }
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        link.client.close().await;
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        let text = match link.points.get(usize::try_from(point).unwrap_or(usize::MAX)) {
                            Some(found) if found.info.boolean => (value != 0.0).to_string(),
                            _ => value.to_string(),
                        };
                        let wrote = tokio::time::timeout(REQUEST_WAIT, write(&link, point, &text, out, &mut stats)).await;
                        if wrote.is_err() {
                            stats.errors += 1;
                            out.log(LogLevel::Warn, "The write got no answer within 10 s");
                            out.written(point, Err("No answer within 10 s".into()));
                        }
                        out.stats(stats);
                    }
                    Some(Control::Opcua(request)) => {
                        tracing::trace!(?request, "request");
                        let asked = request.clone();
                        let handled = tokio::time::timeout(
                            REQUEST_WAIT,
                            handle(&mut link, &spec, &mut wanted, request, out, &mut stats),
                        )
                        .await;
                        tracing::trace!(answered = handled.is_ok(), "request done");
                        let added = handled.unwrap_or_else(|_| {
                            stats.errors += 1;
                            out.log(LogLevel::Warn, format!("No answer within 10 s to {asked:?}"));
                            refuse(asked, "No answer within 10 s", out);
                            0
                        });
                        if added > 0 {
                            states.resize(link.points.len(), PointState::default());
                            dirty.resize(link.points.len(), false);
                        }
                        out.stats(stats);
                    }
                    // Clients serve nothing, so there is nothing to hold.
                    Some(_) => {}
                },
            }
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Report a lost server and wait before the next attempt. Returns `false` when the app closed the
/// session while waiting.
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

/// Wait for the app to close the session while there is no server, answering what it asks.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => out.written(point, Err("Not connected".into())),
            Some(Control::Opcua(request)) => refuse(request, "Not connected", out),
            Some(_) => {}
        }
    }
}

/// Answer a request that cannot be served now, so the app stops waiting for it.
fn refuse(request: OpcuaRequest, reason: &str, out: &Emitter) {
    let reason = reason.to_owned();
    match request {
        OpcuaRequest::Browse { node_id } => out.opcua(OpcuaEvent::Children {
            parent: node_id,
            result: Err(reason),
        }),
        OpcuaRequest::Attributes { node_id } => out.opcua(OpcuaEvent::Attributes {
            node_id,
            result: Err(reason),
        }),
        OpcuaRequest::Monitor { nodes } => out.opcua(OpcuaEvent::Monitored(
            nodes
                .into_iter()
                .map(|node| (node.node_id, Err(reason.clone())))
                .collect(),
        )),
        OpcuaRequest::Write { point, .. } => out.written(point, Err(reason)),
        OpcuaRequest::Method { method_id } => out.opcua(OpcuaEvent::Method {
            method_id,
            result: Err(reason),
        }),
        OpcuaRequest::Call { method_id, .. } => out.opcua(OpcuaEvent::Called {
            method_id,
            result: Err(reason),
        }),
        OpcuaRequest::Endpoints { .. } => {}
    }
}

/// Whether a connection failed because the server refused who or how the client signs in, which
/// another attempt cannot change.
fn refused(error: &str) -> bool {
    [
        "BadIdentityTokenRejected",
        "BadIdentityTokenInvalid",
        "BadUserAccessDenied",
        "BadSecurityPolicyRejected",
        "BadCertificateInvalid",
        "Cannot use the user certificate",
    ]
    .iter()
    .any(|status| error.contains(status))
}

fn latency(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX)
}

/// Sign in and create the subscription the points are monitored in.
async fn connect(
    spec: &OpcuaClientSpec,
    values: mpsc::UnboundedSender<UaValue>,
    out: &Emitter,
    stats: &mut Stats,
) -> Result<Link, String> {
    let identity = match &spec.auth {
        OpcuaAuth::Anonymous => UserIdentity::Anonymous,
        OpcuaAuth::UserName { username, password } => UserIdentity::UserName {
            user: username.clone(),
            password: password.clone(),
        },
        OpcuaAuth::Certificate {
            certificate,
            private_key,
        } => UserIdentity::Certificate {
            certificate: PathBuf::from(certificate),
            private_key: PathBuf::from(private_key),
        },
    };
    let config = OpcUaConfig {
        endpoint_url: spec.endpoint.trim().to_owned(),
        security_policy: spec.policy.name().to_owned(),
        security_mode: spec.mode.key().to_owned(),
        username: None,
        password: None,
        poll_interval_ms: u64::from(spec.publishing_ms),
    };
    out.traffic(vec![traffic(
        Direction::Tx,
        "Connect",
        format!(
            "{} · {} · {} · {}",
            spec.endpoint,
            spec.policy.name(),
            spec.mode.name(),
            spec.auth.label()
        ),
    )]);
    let started = Instant::now();
    let mut client = OpcUaProtocolClient::new();
    client.connect_as(config, &identity).await?;
    stats.last_latency_us = latency(started);
    out.traffic(vec![traffic(
        Direction::Rx,
        "Session active",
        "GetEndpoints · OpenSecureChannel · CreateSession · ActivateSession",
    )]);
    out.log(
        LogLevel::Info,
        format!(
            "Signed in to {} ({} / {}, {})",
            spec.endpoint,
            spec.policy.name(),
            spec.mode.name(),
            spec.auth.label().to_lowercase()
        ),
    );
    if let Ok(namespaces) = client.read_namespaces().await {
        out.log(
            LogLevel::Info,
            format!(
                "Namespaces: {}",
                namespaces
                    .iter()
                    .map(|namespace| format!("{} {}", namespace.index, namespace.uri))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ),
        );
    }
    let publishing = spec.publishing_ms.max(MIN_PUBLISHING_MS);
    let subscription = client
        .subscribe_values(
            publishing,
            Arc::new(move |value: UaValue| {
                let _ = values.send(value);
            }),
        )
        .await?;
    out.traffic(vec![traffic(
        Direction::Tx,
        "CreateSubscription",
        format!("publishing every {publishing} ms · subscription {subscription}"),
    )]);
    Ok(Link {
        client,
        subscription,
        points: Vec::new(),
        by_node: HashMap::new(),
    })
}

/// Describe and monitor `nodes`; the ones the server takes become points after those the link
/// has. Returns their points and each node's outcome.
async fn add(
    link: &mut Link,
    nodes: &[OpcuaNode],
    sampling_ms: u32,
    out: &Emitter,
) -> (Vec<PointInfo>, Vec<(String, Result<(), String>)>) {
    if nodes.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let ids: Vec<String> = nodes.iter().map(|node| node.node_id.clone()).collect();
    out.traffic(vec![traffic(
        Direction::Tx,
        "Read",
        format!(
            "NodeClass, DisplayName, DataType, UserAccessLevel, EngineeringUnits of {} node{}",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        ),
    )]);
    let described = match link.client.describe_variables(&ids).await {
        Ok(described) => described,
        Err(error) => {
            out.log(LogLevel::Error, error.clone());
            return (
                Vec::new(),
                ids.into_iter().map(|id| (id, Err(error.clone()))).collect(),
            );
        }
    };
    let mut results = Vec::new();
    let mut fresh: Vec<(&OpcuaNode, mt_opcua::VariableInfo)> = Vec::new();
    for (node, info) in nodes.iter().zip(described) {
        match info {
            Ok(info)
                if link.by_node.contains_key(&info.node_id)
                    || fresh.iter().any(|(_, other)| other.node_id == info.node_id) =>
            {
                results.push((node.node_id.clone(), Err("Already a point".to_owned())));
            }
            Ok(info) => fresh.push((node, info)),
            Err(error) => results.push((node.node_id.clone(), Err(error))),
        }
    }
    if fresh.is_empty() {
        report_refusals(&results, out);
        return (Vec::new(), results);
    }
    let items: Vec<(String, f64)> = fresh
        .iter()
        .map(|(_, info)| (info.node_id.clone(), f64::from(sampling_ms)))
        .collect();
    out.traffic(vec![traffic(
        Direction::Tx,
        "CreateMonitoredItems",
        format!("{} items · sampling every {sampling_ms} ms", items.len()),
    )]);
    let monitored = match link.client.monitor(link.subscription, &items).await {
        Ok(monitored) => monitored,
        Err(error) => vec![Err(error); items.len()],
    };
    let mut added = Vec::new();
    for ((node, info), outcome) in fresh.into_iter().zip(monitored) {
        match outcome {
            Ok(_revised_sampling) => {
                let id = link.points.len();
                let point = PointInfo {
                    id: u32::try_from(id).unwrap_or(u32::MAX),
                    name: [node.name.trim(), info.display_name.as_str()]
                        .into_iter()
                        .find(|name| !name.is_empty())
                        .unwrap_or(info.node_id.as_str())
                        .to_owned(),
                    address: info.node_id.clone(),
                    data_type: if info.data_type.is_empty() {
                        "Unknown".to_owned()
                    } else {
                        info.data_type.clone()
                    },
                    unit: info.unit.clone(),
                    writable: info.writable && opcua_writable_type(&info.data_type),
                    boolean: info.data_type == "Boolean",
                    states: Vec::new(),
                };
                link.by_node.insert(info.node_id.clone(), id);
                link.points.push(Point {
                    node_id: info.node_id,
                    data_type: info.data_type,
                    info: point.clone(),
                });
                added.push(point);
                results.push((node.node_id.clone(), Ok(())));
            }
            Err(error) => results.push((node.node_id.clone(), Err(error))),
        }
    }
    let refused = results.iter().filter(|(_, result)| result.is_err()).count();
    out.traffic(vec![traffic(
        Direction::Rx,
        "CreateMonitoredItemsResponse",
        format!("{} monitored · {refused} refused", added.len()),
    )]);
    report_refusals(&results, out);
    (added, results)
}

fn report_refusals(results: &[(String, Result<(), String>)], out: &Emitter) {
    for (node_id, result) in results {
        if let Err(error) = result {
            out.log(LogLevel::Warn, format!("Not monitoring {node_id}: {error}"));
        }
    }
}

fn frame(states: &[PointState], latency_us: u32) -> ValueFrame {
    ValueFrame {
        at_ms: now_ms(),
        latency_us,
        readings: states.iter().map(PointState::reading).collect(),
    }
}

fn scalar_json(value: OpcuaScalar) -> serde_json::Value {
    match value {
        OpcuaScalar::Boolean(on) => serde_json::Value::Bool(on),
        OpcuaScalar::Int(n) => serde_json::json!(n),
        OpcuaScalar::UInt(n) => serde_json::json!(n),
        OpcuaScalar::Float(n) => serde_json::json!(n),
        OpcuaScalar::Text(text) => serde_json::Value::String(text),
    }
}

/// Write `text`, parsed as the point's data type, and report the outcome.
async fn write(link: &Link, point: u32, text: &str, out: &Emitter, stats: &mut Stats) {
    let Some(target) = link
        .points
        .get(usize::try_from(point).unwrap_or(usize::MAX))
    else {
        out.written(point, Err(format!("No point {point}")));
        return;
    };
    if !target.info.writable {
        out.written(
            point,
            Err(format!("{} is read-only for this user", target.info.name)),
        );
        return;
    }
    let value = match parse_opcua_value(&target.data_type, text) {
        Ok(value) => scalar_json(value),
        Err(error) => {
            out.written(point, Err(error));
            return;
        }
    };
    out.traffic(vec![traffic(
        Direction::Tx,
        "Write",
        format!("{} = {value} ({})", target.node_id, target.data_type),
    )]);
    let started = Instant::now();
    let result = link
        .client
        .write_value(&target.node_id, &target.data_type, &value)
        .await;
    stats.last_latency_us = latency(started);
    out.traffic(vec![traffic(
        Direction::Rx,
        "WriteResponse",
        match &result {
            Ok(()) => "Good".to_owned(),
            Err(error) => error.clone(),
        },
    )]);
    match &result {
        Ok(()) => out.log(
            LogLevel::Info,
            format!("Wrote {} = {value}", target.info.name),
        ),
        Err(error) => {
            stats.errors += 1;
            out.log(LogLevel::Warn, format!("{}: {error}", target.info.name));
        }
    }
    out.written(point, result);
}

/// Serve one request of the app. Returns how many points joined.
async fn handle(
    link: &mut Link,
    spec: &OpcuaClientSpec,
    wanted: &mut Vec<OpcuaNode>,
    request: OpcuaRequest,
    out: &Emitter,
    stats: &mut Stats,
) -> usize {
    match request {
        OpcuaRequest::Browse { node_id } => {
            out.traffic(vec![traffic(Direction::Tx, "Browse", node_id.clone())]);
            let started = Instant::now();
            let result = link.client.browse_level(&node_id).await.map(|children| {
                children
                    .into_iter()
                    .map(|child| OpcuaBrowseNode {
                        node_id: child.node_id,
                        browse_name: child.browse_name,
                        display_name: child.display_name,
                        class: OpcuaNodeClass::from_name(&child.node_class),
                        data_type: child.data_type,
                        has_children: child.has_children,
                    })
                    .collect::<Vec<_>>()
            });
            stats.last_latency_us = latency(started);
            out.traffic(vec![traffic(
                Direction::Rx,
                "BrowseResponse",
                match &result {
                    Ok(children) => format!("{} references", children.len()),
                    Err(error) => error.clone(),
                },
            )]);
            if let Err(error) = &result {
                stats.errors += 1;
                out.log(LogLevel::Warn, format!("Browse {node_id}: {error}"));
            }
            out.opcua(OpcuaEvent::Children {
                parent: node_id,
                result,
            });
            0
        }
        OpcuaRequest::Attributes { node_id } => {
            out.traffic(vec![traffic(
                Direction::Tx,
                "Read",
                format!("attributes of {node_id}"),
            )]);
            let result = link.client.read_attributes(&node_id).await.map(|read| {
                read.into_iter()
                    .map(|attribute| OpcuaAttribute {
                        value: attribute_text(&attribute.attribute, &attribute.value),
                        name: attribute.attribute,
                        status: attribute.status,
                    })
                    .collect::<Vec<_>>()
            });
            out.traffic(vec![traffic(
                Direction::Rx,
                "ReadResponse",
                match &result {
                    Ok(attributes) => format!("{} attributes", attributes.len()),
                    Err(error) => error.clone(),
                },
            )]);
            out.opcua(OpcuaEvent::Attributes { node_id, result });
            0
        }
        OpcuaRequest::Monitor { nodes } => {
            let (added, results) = add(link, &nodes, spec.sampling_ms, out).await;
            for node in &nodes {
                let accepted = results
                    .iter()
                    .any(|(id, result)| id == &node.node_id && result.is_ok());
                if accepted {
                    wanted.push(node.clone());
                }
            }
            let count = added.len();
            if count > 0 {
                out.log(
                    LogLevel::Info,
                    format!(
                        "Monitoring {count} more variable{}",
                        if count == 1 { "" } else { "s" }
                    ),
                );
                out.points_added(added);
            }
            out.opcua(OpcuaEvent::Monitored(results));
            count
        }
        OpcuaRequest::Write { point, text } => {
            write(link, point, &text, out, stats).await;
            0
        }
        OpcuaRequest::Method { method_id } => {
            let result = link
                .client
                .method_arguments(&method_id)
                .await
                .map(|(inputs, outputs)| OpcuaMethodInfo {
                    inputs: inputs.into_iter().map(argument).collect(),
                    outputs: outputs.into_iter().map(argument).collect(),
                });
            out.opcua(OpcuaEvent::Method { method_id, result });
            0
        }
        OpcuaRequest::Call {
            object_id,
            method_id,
            arguments,
        } => {
            out.traffic(vec![traffic(
                Direction::Tx,
                "Call",
                format!("{method_id}({})", arguments.join(", ")),
            )]);
            let started = Instant::now();
            let result = call(link, &object_id, &method_id, &arguments).await;
            stats.last_latency_us = latency(started);
            out.traffic(vec![traffic(
                Direction::Rx,
                "CallResponse",
                match &result {
                    Ok(outputs) => format!("Good → {}", outputs.join(", ")),
                    Err(error) => error.clone(),
                },
            )]);
            match &result {
                Ok(outputs) => out.log(
                    LogLevel::Info,
                    format!("Called {method_id} → {}", outputs.join(", ")),
                ),
                Err(error) => {
                    stats.errors += 1;
                    out.log(LogLevel::Warn, format!("Call {method_id}: {error}"));
                }
            }
            out.opcua(OpcuaEvent::Called { method_id, result });
            0
        }
        // Discovery needs no session: the engine answers it.
        OpcuaRequest::Endpoints { .. } => 0,
    }
}

fn argument(argument: mt_opcua::ArgumentInfo) -> OpcuaArgument {
    OpcuaArgument {
        name: argument.name,
        data_type: argument.data_type,
        description: argument.description,
    }
}

/// Type each argument by the method's definition and call it.
async fn call(
    link: &Link,
    object_id: &str,
    method_id: &str,
    arguments: &[String],
) -> Result<Vec<String>, String> {
    let (inputs, _) = link.client.method_arguments(method_id).await?;
    if inputs.len() != arguments.len() {
        return Err(format!(
            "The method takes {} argument{}, not {}",
            inputs.len(),
            if inputs.len() == 1 { "" } else { "s" },
            arguments.len()
        ));
    }
    let typed = inputs
        .iter()
        .zip(arguments)
        .map(|(input, text)| {
            parse_opcua_value(&input.data_type, text)
                .map(|value| (input.data_type.clone(), scalar_json(value)))
                .map_err(|error| format!("{}: {error}", input.name))
        })
        .collect::<Result<Vec<_>, _>>()?;
    link.client.call_method(object_id, method_id, &typed).await
}

/// An attribute's value as text; access levels with what they allow.
fn attribute_text(name: &str, value: &serde_json::Value) -> String {
    match (name, value) {
        ("AccessLevel" | "UserAccessLevel", serde_json::Value::Number(level)) => {
            let level = level.as_u64().unwrap_or(0);
            let mut allows = Vec::new();
            if level & 0x01 != 0 {
                allows.push("read");
            }
            if level & 0x02 != 0 {
                allows.push("write");
            }
            if level & 0x04 != 0 {
                allows.push("history read");
            }
            format!(
                "{level} ({})",
                if allows.is_empty() {
                    "none".to_owned()
                } else {
                    allows.join(", ")
                }
            )
        }
        (_, serde_json::Value::Null) => String::new(),
        (_, serde_json::Value::String(text)) => text.clone(),
        // Lists of text (a method's arguments) read as a list.
        (_, serde_json::Value::Array(items)) if items.iter().all(serde_json::Value::is_string) => {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        }
        (_, other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels_read_as_words() {
        assert_eq!(
            attribute_text("AccessLevel", &serde_json::json!(3)),
            "3 (read, write)"
        );
        assert_eq!(
            attribute_text("UserAccessLevel", &serde_json::json!(1)),
            "1 (read)"
        );
        assert_eq!(
            attribute_text("AccessLevel", &serde_json::json!(0)),
            "0 (none)"
        );
        assert_eq!(
            attribute_text("DisplayName", &serde_json::json!("Level")),
            "Level"
        );
        assert_eq!(attribute_text("Value", &serde_json::Value::Null), "");
        assert_eq!(
            attribute_text(
                "InputArguments",
                &serde_json::json!(["Recipe: String", "Quantity: Double"])
            ),
            "Recipe: String, Quantity: Double"
        );
    }

    #[test]
    fn uncertain_values_count_and_text_rides_in_the_payload() {
        let uncertain = PointState {
            number: Some(4.5),
            text: "4.5".into(),
            quality: Some(UaQuality::Uncertain),
            status: "UncertainLastUsableValue".into(),
            source_ms: 1_000,
        };
        assert_eq!(uncertain.reading().quality, Quality::Good);
        assert_eq!(uncertain.reading().value, 4.5);
        assert_eq!(
            uncertain.payload(),
            "4.5\nUncertainLastUsableValue · source 00:00:01.000 UTC"
        );
        let text = PointState {
            number: None,
            text: "Pale ale".into(),
            quality: Some(UaQuality::Good),
            status: "Good".into(),
            source_ms: 0,
        };
        assert!(text.reading().value.is_nan());
        assert_eq!(text.reading().quality, Quality::Good);
        assert_eq!(text.payload(), "Pale ale\nGood");
        let bad = PointState {
            quality: Some(UaQuality::Bad),
            ..uncertain
        };
        assert_eq!(bad.reading().quality, Quality::Bad);
        assert_eq!(PointState::default().reading().quality, Quality::Bad);
    }
}
