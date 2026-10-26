//! Subscribes to a NATS server through `mt-nats`'s client. Every concrete
//! subject it receives becomes a point (or several: the numeric fields of a
//! JSON message); raw messages go to the traffic monitor and server events to
//! the log. A write publishes the value to the point's subject. Publishing
//! with headers, request/reply and JetStream browsing come in as
//! [`NatsRequest`]s and run beside the subscriptions, so a slow reply never
//! holds up the values.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_nats::connection::State;
use futures_util::StreamExt as _;
use mt_nats::{NatsClient, NatsClientConfig, NatsClientTlsConfig};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, NatsAuth, NatsClientSpec, NatsConsumer, NatsEvent, NatsHeader, NatsReply,
    NatsRequest, NatsStoredMessage, NatsStream, NatsStreamDetail, PathStep, PointInfo, PointText,
    Quality, Reading, SessionSpec, SessionState, Stats, TrafficFrame, ValueFrame, parse_json_path,
};

use super::{frame, header_lines, header_map, headers_of, payload_text, url_host};
use crate::Control;
use crate::emit::{Emitter, now_ms};
use crate::mqtt::values::{self, Extracted, Sample};
use crate::mqtt::{format_number, preview};

/// How often the app sees what arrived.
pub(crate) const TICK: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// How long the client may try to reconnect on its own before the session
/// drops it and starts over.
const LOST_AFTER: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT_MS: u64 = 5000;
/// Longest a request may wait for its reply.
pub const MAX_REQUEST_MS: u64 = 60_000;
/// How long a fetch waits for a consumer's first message.
const FETCH_WAIT_MS: u64 = 1000;
/// Points one session keeps at most: `>` on a busy server could otherwise
/// grow without end. Later subjects still show in the traffic monitor.
const MAX_POINTS: usize = 5_000;

/// Gives sessions without a name distinct ones in the server's list.
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn run(
    spec: NatsClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let filters: Vec<Filter> = spec
        .subscriptions
        .iter()
        .map(|subscription| Filter {
            subject: subscription.subject.trim().to_owned(),
            queue: subscription.queue.trim().to_owned(),
            path: parse_json_path(&subscription.json_path)
                .ok()
                .filter(|path| !path.is_empty()),
        })
        .collect();
    let name = if spec.name.trim().is_empty() {
        format!(
            "voltrus-tools-{}-{}",
            std::process::id(),
            NEXT_CLIENT.fetch_add(1, Ordering::Relaxed)
        )
    } else {
        spec.name.trim().to_owned()
    };
    let url = format!(
        "{}://{}",
        if spec.tls { "tls" } else { "nats" },
        SessionSpec::NatsClient(spec.clone()).address()
    );
    let config = config(&spec, &name);

    // Subjects seen in an earlier connection of this session keep their ids.
    out.points(Vec::new());
    let mut subjects = Subjects::default();
    let mut stats = Stats::default();
    let mut reported = Stats::default();
    let mut traffic = Vec::new();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let connected = tokio::select! {
            result = NatsClient::connect(&config) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let client = match connected {
            Ok(client) => client,
            Err(error) => {
                attempt += 1;
                stats.errors += 1;
                out.log(LogLevel::Error, format!("{url}: {error}"));
                out.stats(stats);
                if !retry(&mut control, attempt, &error, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        let info = client.inner().server_info();
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Connected to {url} as {name}: {} {}, messages up to {} bytes{}",
                if info.server_name.is_empty() {
                    "NATS server"
                } else {
                    info.server_name.as_str()
                },
                info.version,
                info.max_payload,
                if info.headers { ", headers" } else { "" },
            ),
        );
        traffic.push(frame(
            Direction::Rx,
            format!("INFO {}", info.server_name),
            format!(
                "version {} · max payload {}",
                info.version, info.max_payload
            ),
            0,
        ));
        traffic.push(frame(
            Direction::Tx,
            format!("CONNECT {name}"),
            match &spec.auth {
                NatsAuth::None => "no auth".to_owned(),
                auth => format!("{} auth", auth.name()),
            },
            0,
        ));

        let mut subscribers = Vec::with_capacity(filters.len());
        for (index, filter) in filters.iter().enumerate() {
            let subscribed = if filter.queue.is_empty() {
                client.inner().subscribe(filter.subject.clone()).await
            } else {
                client
                    .inner()
                    .queue_subscribe(filter.subject.clone(), filter.queue.clone())
                    .await
            };
            match subscribed {
                Ok(subscriber) => {
                    traffic.push(frame(
                        Direction::Tx,
                        format!("SUB {}", filter.subject),
                        if filter.queue.is_empty() {
                            String::new()
                        } else {
                            format!("queue {}", filter.queue)
                        },
                        0,
                    ));
                    out.log(
                        LogLevel::Info,
                        if filter.queue.is_empty() {
                            format!("Subscribed to {}", filter.subject)
                        } else {
                            format!(
                                "Subscribed to {} in queue group {}",
                                filter.subject, filter.queue
                            )
                        },
                    );
                    subscribers.push(subscriber.map(move |message| (index, message)));
                }
                Err(error) => {
                    stats.errors += 1;
                    out.log(
                        LogLevel::Warn,
                        format!("Subscribing to {} failed: {error}", filter.subject),
                    );
                }
            }
        }
        let watching = !subscribers.is_empty();
        let mut messages = futures_util::stream::select_all(subscribers);
        let mut down_since: Option<Instant> = None;

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = messages.next(), if watching => match message {
                    Some((index, message)) => {
                        stats.frames += 1;
                        subjects.receive(&message, index, filters.get(index), &mut traffic, out);
                    }
                    None => {
                        stats.errors += 1;
                        let reason = format!("Lost the connection to {url}");
                        out.log(LogLevel::Error, reason.clone());
                        subjects.flush(&mut stats, out);
                        out.traffic(std::mem::take(&mut traffic));
                        attempt = 1;
                        if !retry(&mut control, attempt, &reason, out).await {
                            break 'session;
                        }
                        continue 'session;
                    }
                },
                _ = ticker.tick() => {
                    subjects.flush(&mut stats, out);
                    out.traffic(std::mem::take(&mut traffic));
                    if stats != reported {
                        out.stats(stats);
                        reported = stats;
                    }
                    // The client reconnects on its own for a while; the
                    // session reports it and starts over if that fails.
                    match client.inner().connection_state() {
                        State::Connected => {
                            if down_since.take().is_some() {
                                out.state(SessionState::Connected);
                                out.log(LogLevel::Info, format!("Reconnected to {url}"));
                            }
                        }
                        State::Pending | State::Disconnected => match down_since {
                            None => {
                                down_since = Some(Instant::now());
                                stats.errors += 1;
                                out.log(LogLevel::Warn, format!("Lost the connection to {url}; reconnecting"));
                                out.state(SessionState::Reconnecting {
                                    attempt: 1,
                                    retry_in_ms: 0,
                                    reason: format!("Lost the connection to {url}"),
                                });
                            }
                            Some(since) if since.elapsed() > LOST_AFTER => {
                                let reason = format!("{url} did not come back");
                                out.log(LogLevel::Error, reason.clone());
                                attempt = 1;
                                if !retry(&mut control, attempt, &reason, out).await {
                                    break 'session;
                                }
                                continue 'session;
                            }
                            Some(_) => {}
                        },
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        let _ = tokio::time::timeout(Duration::from_millis(500), client.inner().flush()).await;
                        traffic.push(frame(Direction::Tx, "UNSUB all".into(), "closing".into(), 0));
                        subjects.flush(&mut stats, out);
                        out.traffic(std::mem::take(&mut traffic));
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        let result = subjects.write(point, value, &client, &mut traffic).await;
                        out.written(point, result);
                    }
                    Some(Control::Publish { topic, payload, .. }) => {
                        let result = publish(&client, &topic, payload.as_bytes(), &[], &mut traffic).await;
                        out.published(topic, result);
                    }
                    Some(Control::Nats(request)) => {
                        handle(request, &client, &mut traffic, &mut stats, out).await;
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

fn config(spec: &NatsClientSpec, name: &str) -> NatsClientConfig {
    let auth = match &spec.auth {
        NatsAuth::None => mt_nats::NatsAuth::None,
        NatsAuth::Token { token } => mt_nats::NatsAuth::Token {
            token: token.clone(),
        },
        NatsAuth::UserPassword { username, password } => mt_nats::NatsAuth::UserPass {
            username: username.clone(),
            password: password.clone(),
        },
        NatsAuth::Nkey { seed } => mt_nats::NatsAuth::Nkey {
            seed: seed.trim().to_owned(),
        },
        NatsAuth::Credentials { creds, .. } => mt_nats::NatsAuth::Credentials {
            creds: creds.clone(),
        },
    };
    NatsClientConfig {
        host: url_host(&spec.host),
        port: spec.port,
        name: name.to_owned(),
        auth,
        tls: NatsClientTlsConfig {
            enabled: spec.tls,
            ..NatsClientTlsConfig::default()
        },
        connect_timeout_ms: CONNECT_TIMEOUT_MS,
        ..NatsClientConfig::default()
    }
}

/// Report a lost server and wait before the next attempt. Returns `false`
/// when the app closed the session while waiting.
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

/// Wait for the app to close the session while there is no server, refusing
/// writes, publishes and requests in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(Control::Publish { topic, .. }) => {
                out.published(topic, Err("Not connected".into()));
            }
            Some(Control::Nats(request)) => refuse(request, "Not connected", out),
            Some(_) => {}
        }
    }
}

/// Answer a request that cannot run with `reason`.
pub(crate) fn refuse(request: NatsRequest, reason: &str, out: &Emitter) {
    match request {
        NatsRequest::Publish { subject, .. } => out.published(subject, Err(reason.to_owned())),
        NatsRequest::Request { subject, .. } => out.nats(NatsEvent::Reply {
            subject,
            result: Err(reason.to_owned()),
        }),
        NatsRequest::Streams => out.nats(NatsEvent::Streams(Err(reason.to_owned()))),
        NatsRequest::Stream { name, .. } => out.nats(NatsEvent::Stream {
            name,
            result: Err(reason.to_owned()),
        }),
        NatsRequest::Fetch {
            stream, consumer, ..
        } => out.nats(NatsEvent::Fetched {
            stream,
            consumer,
            result: Err(reason.to_owned()),
        }),
    }
}

async fn publish(
    client: &NatsClient,
    subject: &str,
    payload: &[u8],
    headers: &[NatsHeader],
    traffic: &mut Vec<TrafficFrame>,
) -> Result<(), String> {
    if !vt_protocol::valid_subject(subject) {
        return Err(format!("“{subject}” is not a subject to publish to"));
    }
    let pairs: Vec<(String, String)> = headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    if pairs.is_empty() {
        client.publish(subject, payload).await?;
    } else {
        client
            .publish_with_headers(subject, &pairs, payload)
            .await?;
    }
    traffic.push(frame(
        Direction::Tx,
        format!(
            "{} {subject}",
            if pairs.is_empty() { "PUB" } else { "HPUB" }
        ),
        if pairs.is_empty() {
            preview(payload)
        } else {
            format!(
                "{} header{} · {}",
                pairs.len(),
                if pairs.len() == 1 { "" } else { "s" },
                preview(payload)
            )
        },
        payload.len(),
    ));
    Ok(())
}

/// Run one of the app's requests: publishes at once, everything that waits
/// for the server on its own task.
async fn handle(
    request: NatsRequest,
    client: &NatsClient,
    traffic: &mut Vec<TrafficFrame>,
    stats: &mut Stats,
    out: &Emitter,
) {
    match request {
        NatsRequest::Publish {
            subject,
            payload,
            headers,
        } => {
            let result = publish(client, &subject, payload.as_bytes(), &headers, traffic).await;
            if result.is_err() {
                stats.errors += 1;
            }
            out.published(subject, result);
        }
        NatsRequest::Request {
            subject,
            payload,
            headers,
            timeout_ms,
        } => {
            if !vt_protocol::valid_subject(&subject) {
                out.nats(NatsEvent::Reply {
                    result: Err(format!("“{subject}” is not a subject to send a request to")),
                    subject,
                });
                return;
            }
            traffic.push(frame(
                Direction::Tx,
                format!("REQ {subject}"),
                preview(payload.as_bytes()),
                payload.len(),
            ));
            let client = client.inner().clone();
            let out = out.clone();
            tokio::spawn(async move {
                let result = request_reply(&client, &subject, payload, &headers, timeout_ms).await;
                match &result {
                    Ok(reply) => out.traffic(vec![frame(
                        Direction::Rx,
                        format!("REPLY {subject}"),
                        format!(
                            "{:.1} ms · {}",
                            f64::from(reply.latency_us) / 1000.0,
                            preview(reply.payload.as_bytes())
                        ),
                        usize::try_from(reply.bytes).unwrap_or(usize::MAX),
                    )]),
                    Err(error) => out.log(LogLevel::Warn, format!("Request to {subject}: {error}")),
                }
                out.nats(NatsEvent::Reply { subject, result });
            });
        }
        NatsRequest::Streams => {
            traffic.push(frame(
                Direction::Tx,
                "JS STREAM.LIST".into(),
                String::new(),
                0,
            ));
            let client = client.clone();
            let out = out.clone();
            tokio::spawn(async move {
                let result = client
                    .js_streams()
                    .await
                    .map(|streams| streams.into_iter().map(stream_from).collect::<Vec<_>>())
                    .map_err(|error| jetstream_error(&error));
                if let Ok(streams) = &result {
                    out.traffic(vec![frame(
                        Direction::Rx,
                        "JS STREAM.LIST".into(),
                        format!(
                            "{} stream{}",
                            streams.len(),
                            if streams.len() == 1 { "" } else { "s" }
                        ),
                        0,
                    )]);
                }
                out.nats(NatsEvent::Streams(result));
            });
        }
        NatsRequest::Stream { name, messages } => {
            traffic.push(frame(
                Direction::Tx,
                format!("JS STREAM.INFO {name}"),
                String::new(),
                0,
            ));
            let client = client.clone();
            let out = out.clone();
            tokio::spawn(async move {
                let result = stream_detail(&client, &name, messages).await;
                out.nats(NatsEvent::Stream { name, result });
            });
        }
        NatsRequest::Fetch {
            stream,
            consumer,
            batch,
        } => {
            traffic.push(frame(
                Direction::Tx,
                format!("JS CONSUMER.MSG.NEXT {stream} {consumer}"),
                format!("batch {batch}"),
                0,
            ));
            let client = client.clone();
            let out = out.clone();
            tokio::spawn(async move {
                let result = client
                    .js_consumer_next(&stream, &consumer, u64::from(batch.max(1)), FETCH_WAIT_MS)
                    .await
                    .map(|messages| messages.into_iter().map(stored_from).collect::<Vec<_>>())
                    .map_err(|error| jetstream_error(&error));
                if let Ok(messages) = &result {
                    out.traffic(vec![frame(
                        Direction::Rx,
                        format!("JS fetched {stream} {consumer}"),
                        format!(
                            "{} message{} acknowledged",
                            messages.len(),
                            if messages.len() == 1 { "" } else { "s" }
                        ),
                        0,
                    )]);
                }
                out.nats(NatsEvent::Fetched {
                    stream,
                    consumer,
                    result,
                });
            });
        }
    }
}

async fn request_reply(
    client: &async_nats::Client,
    subject: &str,
    payload: String,
    headers: &[NatsHeader],
    timeout_ms: u64,
) -> Result<NatsReply, String> {
    let timeout = Duration::from_millis(timeout_ms.clamp(1, MAX_REQUEST_MS));
    let mut request = async_nats::Request::new()
        .payload(payload.into_bytes().into())
        .timeout(Some(timeout));
    if !headers.is_empty() {
        request = request.headers(header_map(headers)?);
    }
    let started = Instant::now();
    // The client's own timeout covers the reply; this one also covers a
    // connection that stalls before the request is sent.
    let reply = tokio::time::timeout(
        timeout + Duration::from_millis(250),
        client.send_request(subject.to_owned(), request),
    )
    .await
    .map_err(|_| format!("No reply within {} ms", timeout.as_millis()))?
    .map_err(|error| match error.kind() {
        async_nats::RequestErrorKind::NoResponders => {
            format!("No one is listening on {subject}")
        }
        async_nats::RequestErrorKind::TimedOut => {
            format!("No reply within {} ms", timeout.as_millis())
        }
        _ => format!("Request failed: {error}"),
    })?;
    let latency = started.elapsed();
    Ok(NatsReply {
        subject: reply.subject.to_string(),
        payload: payload_text(&reply.payload),
        headers: headers_of(reply.headers.as_ref()),
        bytes: u32::try_from(reply.payload.len()).unwrap_or(u32::MAX),
        latency_us: u32::try_from(latency.as_micros()).unwrap_or(u32::MAX),
    })
}

async fn stream_detail(
    client: &NatsClient,
    name: &str,
    messages: u32,
) -> Result<NatsStreamDetail, String> {
    let stream = client
        .js_stream_info(name)
        .await
        .map_err(|error| jetstream_error(&error))?
        .ok_or_else(|| format!("No stream {name}"))?;
    let consumers = client
        .js_stream_consumers(name)
        .await
        .map_err(|error| jetstream_error(&error))?;
    let stored = client
        .js_stream_tail(name, u64::from(messages))
        .await
        .map_err(|error| jetstream_error(&error))?;
    Ok(NatsStreamDetail {
        stream: stream_from(stream),
        consumers: consumers
            .into_iter()
            .map(|consumer| NatsConsumer {
                name: consumer.name,
                durable: consumer.durable,
                kind: consumer.deliver_kind,
                pending: consumer.pending,
                ack_pending: consumer.num_ack_pending,
                redelivered: consumer.num_redelivered,
            })
            .collect(),
        messages: stored.into_iter().map(stored_from).collect(),
    })
}

/// JetStream errors as a person reads them: a server without JetStream
/// answers no one, so the request times out.
fn jetstream_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("no responders") || lower.contains("timed out") {
        "JetStream is not enabled on this server (no answer from $JS.API)".to_owned()
    } else {
        error.to_owned()
    }
}

fn stream_from(info: mt_nats::NatsStreamInfo) -> NatsStream {
    NatsStream {
        name: info.name,
        subjects: info.subjects,
        messages: info.messages,
        bytes: info.bytes,
        consumers: info.consumers,
        first_seq: info.first_seq,
        last_seq: info.last_seq,
        storage: info.storage,
    }
}

fn stored_from(message: mt_nats::NatsStoredMessage) -> NatsStoredMessage {
    // mt-nats hands payloads over as text, hex-prefixed `0x…` when binary.
    let payload = match message.payload.strip_prefix("0x") {
        Some(hex) if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            super::clip(&format!("binary {}", hex.to_ascii_uppercase()))
        }
        _ => super::clip(&message.payload),
    };
    NatsStoredMessage {
        sequence: message.sequence,
        subject: message.subject,
        payload,
        at_ms: u64::try_from(message.timestamp_ms).unwrap_or(0),
    }
}

struct Filter {
    subject: String,
    queue: String,
    /// The subscription's JSON path, parsed.
    path: Option<Vec<PathStep>>,
}

/// Where a point's value comes from and goes to.
struct Origin {
    /// The subject writes publish to.
    subject: String,
    boolean: bool,
    writable: bool,
}

/// The points found so far and what changed since the app last heard.
#[derive(Default)]
struct Subjects {
    ids: HashMap<(String, String), u32>,
    origins: Vec<Origin>,
    readings: Vec<Reading>,
    added: Vec<PointInfo>,
    texts: HashMap<u32, String>,
    changed: bool,
    full: bool,
}

impl Subjects {
    fn receive(
        &mut self,
        message: &async_nats::Message,
        index: usize,
        filter: Option<&Filter>,
        traffic: &mut Vec<TrafficFrame>,
        out: &Emitter,
    ) {
        let subject = message.subject.as_str();
        let headers = headers_of(message.headers.as_ref());
        let mut detail = String::new();
        if let Some(reply) = &message.reply {
            detail.push_str(&format!("reply {reply} · "));
        }
        if !headers.is_empty() {
            detail.push_str(&format!(
                "{} header{} · ",
                headers.len(),
                if headers.len() == 1 { "" } else { "s" }
            ));
        }
        detail.push_str(&preview(&message.payload));
        traffic.push(frame(
            Direction::Rx,
            format!(
                "{} {subject}",
                if headers.is_empty() { "MSG" } else { "HMSG" }
            ),
            detail,
            message.payload.len(),
        ));
        // The subscription it came through, short enough for the address
        // column: `sub 2`, or `sub 2 · workers` in a queue group.
        let address = match filter {
            Some(filter) if !filter.queue.is_empty() => {
                format!("sub {} · {}", index + 1, filter.queue)
            }
            _ => format!("sub {}", index + 1),
        };
        let path = filter.and_then(|filter| filter.path.as_deref());
        // A `Unit` header (as the simulator sends) names the value's unit.
        let unit = headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(super::server::UNIT_HEADER))
            .map(|header| header.value.clone());
        let prefix = if headers.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", header_lines(&headers))
        };
        for mut found in values::plain(&message.payload, path) {
            if found.path.is_empty()
                && let Some(unit) = &unit
            {
                found.unit.clone_from(unit);
            }
            if !prefix.is_empty() {
                found.text.insert_str(0, &prefix);
            }
            let name = if found.path.is_empty() {
                subject.to_owned()
            } else {
                format!("{subject} › {}", found.path)
            };
            let key = (subject.to_owned(), found.path.clone());
            let writable = found.path.is_empty()
                && matches!(found.sample, Sample::Number(_) | Sample::Bool(_));
            self.update(key, name, &address, found, writable, out);
        }
    }

    /// Store one value, adding its point the first time.
    fn update(
        &mut self,
        key: (String, String),
        name: String,
        address: &str,
        found: Extracted,
        writable: bool,
        out: &Emitter,
    ) {
        let boolean = matches!(found.sample, Sample::Bool(_));
        let id = match self.ids.get(&key) {
            Some(id) => *id,
            None => {
                if self.origins.len() >= MAX_POINTS {
                    if !self.full {
                        self.full = true;
                        out.log(
                            LogLevel::Warn,
                            format!(
                                "{MAX_POINTS} points reached; new subjects only show in the traffic monitor. Narrow the subscriptions to see them."
                            ),
                        );
                    }
                    return;
                }
                let id = u32::try_from(self.origins.len()).unwrap_or(u32::MAX);
                self.added.push(PointInfo {
                    id,
                    name,
                    address: address.to_owned(),
                    data_type: found.data_type.clone(),
                    unit: found.unit.clone(),
                    writable,
                    boolean,
                    states: Vec::new(),
                });
                self.origins.push(Origin {
                    subject: key.0.clone(),
                    boolean,
                    writable,
                });
                self.readings.push(Reading::bad());
                self.ids.insert(key, id);
                id
            }
        };
        let Some(reading) = usize::try_from(id)
            .ok()
            .and_then(|index| self.readings.get_mut(index))
        else {
            return;
        };
        reading.value = match found.sample {
            Sample::Number(value) => value,
            Sample::Bool(flag) => f64::from(u8::from(flag)),
            Sample::Text(_) => f64::NAN,
        };
        reading.quality = Quality::Good;
        self.texts.insert(id, found.text);
        self.changed = true;
    }

    /// Publish a value to a whole-message point's subject.
    async fn write(
        &self,
        point: u32,
        value: f64,
        client: &NatsClient,
        traffic: &mut Vec<TrafficFrame>,
    ) -> Result<(), String> {
        let origin = usize::try_from(point)
            .ok()
            .and_then(|index| self.origins.get(index))
            .ok_or_else(|| format!("No point {point}"))?;
        if !origin.writable {
            return Err(
                "Only whole-message values can be written; publish to the subject instead".into(),
            );
        }
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let payload = if origin.boolean {
            (if value != 0.0 { "true" } else { "false" }).to_owned()
        } else {
            format_number(value)
        };
        publish(client, &origin.subject, payload.as_bytes(), &[], traffic).await
    }

    /// Tell the app what arrived since the last tick.
    fn flush(&mut self, stats: &mut Stats, out: &Emitter) {
        if !self.added.is_empty() {
            out.points_added(std::mem::take(&mut self.added));
        }
        if self.changed {
            self.changed = false;
            stats.polls += 1;
            out.values(ValueFrame {
                at_ms: now_ms(),
                latency_us: stats.last_latency_us,
                readings: self.readings.clone(),
            });
        }
        if !self.texts.is_empty() {
            let mut texts: Vec<PointText> = self
                .texts
                .drain()
                .map(|(point, text)| PointText { point, text })
                .collect();
            texts.sort_by_key(|text| text.point);
            out.payloads(texts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_binary_payloads_read_as_hex() {
        let stored = |payload: &str| {
            stored_from(mt_nats::NatsStoredMessage {
                sequence: 7,
                subject: "a.b".into(),
                payload: payload.into(),
                timestamp_ms: -5,
            })
        };
        assert_eq!(stored("0xc328").payload, "binary C328");
        assert_eq!(stored("0x").payload, "0x", "not hex: kept as text");
        assert_eq!(stored("0xzz").payload, "0xzz");
        assert_eq!(stored("21.5").payload, "21.5");
        assert_eq!(stored("1").at_ms, 0, "a time before 1970 reads as 0");
    }

    #[test]
    fn jetstream_silence_reads_as_jetstream_missing() {
        assert!(jetstream_error("request timed out").contains("not enabled"));
        assert!(jetstream_error("nats: no responders").contains("not enabled"));
        assert_eq!(jetstream_error("stream not found"), "stream not found");
    }
}
