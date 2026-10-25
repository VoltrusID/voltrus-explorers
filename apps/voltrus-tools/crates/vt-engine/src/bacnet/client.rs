//! Reads a BACnet/IP device: finds it with a directed Who-Is when its
//! instance is unknown, reads its object list, subscribes to change-of-value
//! notifications where the device takes them and polls the rest with
//! ReadPropertyMultiple, and commands points at the session's priority.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mt_bacnet::{BacnetClient, BacnetConfig, BacnetObjectInfo, BacnetValue};
use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    BacnetClientSpec, Direction, LogLevel, PointInfo, Quality, Reading, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

use super::{Object, ObjectKind, StatusFlags, describe_frame, parse_ip, resolve, traffic};
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(100);
/// Timeouts in a row that make the client drop the device and find it again.
const TIMEOUTS_BEFORE_RECONNECT: u32 = 3;
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// How long a directed Who-Is waits for the I-Am.
const WHO_IS_WAIT: Duration = Duration::from_secs(2);
/// Lifetime asked for every COV subscription; renewed at 80 % of it.
const COV_LIFETIME_S: u32 = 300;
/// A subscription the device has not answered within this is left to polling.
const COV_ANSWER_WAIT: Duration = Duration::from_secs(3);
/// Subscriber process identifier of every subscription: each session has its
/// own socket, so the subscriber address already tells them apart.
const COV_PROCESS: u32 = 0x5654;
/// Subscriptions sent at once, so invoke IDs never wrap among those in flight.
const COV_BATCH: usize = 64;
/// Change-of-value updates reach the app at most this often.
const COV_FLUSH: Duration = Duration::from_millis(200);
/// Largest BACnet/IP datagram (Annex J).
const MAX_DATAGRAM: usize = 1500;

const PRESENT_VALUE: &str = "presentValue";
const STATUS_FLAGS: &str = "statusFlags";

/// A device the client reached and the points it found on it.
struct Link {
    client: BacnetClient,
    device: SocketAddr,
    instance: u32,
    objects: Vec<Object>,
    points: Vec<PointInfo>,
}

/// What the client last learned about one point.
#[derive(Clone, Default)]
struct PointState {
    value: Option<f64>,
    flags: StatusFlags,
    /// Why the last read of the point failed.
    error: Option<String>,
    /// The device sends this point's changes; it is not polled.
    subscribed: bool,
}

impl PointState {
    fn reading(&self) -> Reading {
        match self.value {
            Some(value) if self.error.is_none() && !self.flags.fault => Reading {
                value,
                raw: [0; 4],
                raw_len: 0,
                quality: Quality::Good,
            },
            _ => Reading::bad(),
        }
    }
}

pub(crate) async fn run(
    spec: BacnetClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let Some(bind) = parse_ip(&spec.bind) else {
        fail(out, format!("Cannot bind {}: not an IP address", spec.bind));
        return;
    };
    if !BacnetClientSpec::valid_priority(spec.priority) {
        fail(
            out,
            format!(
                "Write priority {} is not 1 to 16 (6 is reserved)",
                spec.priority
            ),
        );
        return;
    }
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let connected = tokio::select! {
            result = connect(&spec, bind, out, &mut stats) => result,
            () = until_closed(&mut control, out) => break 'session,
        };
        let link = match connected {
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
        out.points(link.points.clone());
        out.state(SessionState::Connected);

        let mut states = vec![PointState::default(); link.objects.len()];
        let mut cov = if spec.cov {
            Cov::open(bind, out).await
        } else {
            None
        };
        if let Some(cov) = &mut cov {
            let all: Vec<usize> = (0..link.objects.len()).collect();
            cov.subscribe(&link, &all, out, &mut stats).await;
        }

        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut flush = tokio::time::interval(COV_FLUSH);
        flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut changed = false;
        let mut timeouts = 0;
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Some(cov) = &mut cov {
                        cov.maintain(&link, &mut states, out, &mut stats).await;
                    }
                    let polled: Vec<usize> = (0..states.len())
                        .filter(|&point| !states[point].subscribed)
                        .collect();
                    let mut deferred = Vec::new();
                    let started = Instant::now();
                    let Some(result) =
                        poll_or_close(&link, &polled, &mut control, &mut deferred, out, &mut stats).await
                    else {
                        close(&link, cov.as_ref()).await;
                        break 'session;
                    };
                    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
                    stats.polls += 1;
                    stats.last_latency_us = latency_us;
                    match result {
                        Ok(results) => {
                            timeouts = 0;
                            apply_poll(&link, &polled, results, &mut states, out);
                        }
                        Err(error) => {
                            stats.errors += 1;
                            for &point in &polled {
                                states[point].error = Some(error.to_string());
                            }
                            let lost = match &error {
                                ProtocolError::Timeout(_) => {
                                    timeouts += 1;
                                    timeouts >= TIMEOUTS_BEFORE_RECONNECT
                                }
                                ProtocolError::ConnectionFailed(_) | ProtocolError::NotConnected => true,
                                _ => false,
                            };
                            out.log(if lost { LogLevel::Error } else { LogLevel::Warn }, error.to_string());
                            if lost {
                                out.values(ValueFrame {
                                    at_ms: now_ms(),
                                    latency_us,
                                    readings: vec![Reading::bad(); states.len()],
                                });
                                for (point, _) in deferred {
                                    out.written(point, Err("Not connected".into()));
                                }
                                out.stats(stats);
                                close(&link, cov.as_ref()).await;
                                attempt = 1;
                                if !retry(&mut control, attempt, &error.to_string(), out).await {
                                    break 'session;
                                }
                                continue 'session;
                            }
                        }
                    }
                    out.values(frame(&states, latency_us));
                    changed = false;
                    let mut wrote = false;
                    for (point, value) in deferred {
                        wrote |= command(&link, point, value, spec.priority, out, &mut stats).await;
                    }
                    if wrote {
                        ticker.reset_immediately();
                    }
                    out.stats(stats);
                }
                _ = flush.tick(), if changed => {
                    out.values(frame(&states, 0));
                    changed = false;
                }
                received = receive(cov.as_ref(), &mut buffer) => {
                    if let (Some(cov), Ok((len, source))) = (&mut cov, received)
                        && source.ip() == link.device.ip()
                    {
                        stats.frames += 1;
                        changed |= cov.handle(&link, &buffer[..len], source, &mut states, out).await;
                    }
                }
                command_in = control.recv() => match command_in {
                    None | Some(Control::Close) => {
                        close(&link, cov.as_ref()).await;
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        if command(&link, point, Some(value), spec.priority, out, &mut stats).await {
                            ticker.reset_immediately();
                        }
                        out.stats(stats);
                    }
                    Some(Control::Relinquish { point }) => {
                        if command(&link, point, None, spec.priority, out, &mut stats).await {
                            ticker.reset_immediately();
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

fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

/// Report a lost device and wait before the next attempt. Returns `false`
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

/// Wait for the app to close the session while there is no device, refusing
/// writes in the meantime.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. } | Control::Relinquish { point }) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(_) => {}
        }
    }
}

/// `addr` the way `mt-bacnet` parses a host: IPv6 in brackets.
fn host_text(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

/// Find the device, reach it, read its object list and its points' names and
/// units.
async fn connect(
    spec: &BacnetClientSpec,
    bind: IpAddr,
    out: &Emitter,
    stats: &mut Stats,
) -> Result<Link, String> {
    let device = resolve(&spec.host, spec.port).await?;
    let instance = match spec.device_instance {
        Some(instance) => instance,
        None => {
            let local = SocketAddr::new(bind, 0);
            out.traffic(vec![traffic(
                Direction::Tx,
                "Who-Is",
                format!("directed to {device}"),
                0,
            )]);
            let found = mt_bacnet::who_is(local, None, vec![device], WHO_IS_WAIT)
                .await
                .map_err(|error| format!("Who-Is to {device}: {error}"))?;
            let answer = found
                .iter()
                .find(|found| {
                    found.ip == device.ip().to_string() && found.port == device.port()
                })
                .or(found.first())
                .ok_or_else(|| {
                    format!(
                        "No I-Am from {device}: is a BACnet device there? Enter its device instance to skip discovery"
                    )
                })?;
            let vendor = answer.vendor_name.clone().unwrap_or_default();
            stats.frames += 2;
            out.traffic(vec![traffic(
                Direction::Rx,
                format!("I-Am device {}", answer.device_instance),
                format!("{vendor} · {}:{}", answer.ip, answer.port),
                0,
            )]);
            out.log(
                LogLevel::Info,
                format!(
                    "Device {} answered the Who-Is from {}:{} ({vendor})",
                    answer.device_instance, answer.ip, answer.port
                ),
            );
            answer.device_instance
        }
    };

    let host = host_text(device.ip());
    let local = SocketAddr::new(bind, spec.local_port);
    let client = BacnetClient::connect_from(
        BacnetConfig {
            id: format!("device {instance}"),
            ip: host.clone(),
            port: device.port(),
            device_id: instance,
            poll_interval_ms: spec.poll_ms,
        },
        local,
    )
    .await
    .map_err(|error| error.to_string())?;
    let bound = client
        .local_addr()
        .map_or_else(|| local.to_string(), |address| address.to_string());
    out.log(
        LogLevel::Info,
        format!("Reached device {instance} at {device} from {bound}"),
    );

    out.traffic(vec![traffic(
        Direction::Tx,
        "ReadProperty",
        format!("Device {instance} objectList"),
        0,
    )]);
    let listed = mt_bacnet::browse_objects(&host, device.port(), instance)
        .await
        .map_err(|error| format!("Reading the object list of device {instance}: {error}"))?;
    let objects: Vec<Object> = listed.iter().filter_map(point_object).collect();
    stats.frames += 2;
    out.traffic(vec![traffic(
        Direction::Rx,
        "ComplexAck ReadProperty",
        format!("{} objects, {} of them points", listed.len(), objects.len()),
        0,
    )]);
    let skipped = listed.len() - objects.len();
    out.log(
        LogLevel::Info,
        if skipped == 0 {
            format!("Device {instance} lists {} points", objects.len())
        } else {
            format!(
                "Device {instance} lists {} points ({skipped} other objects not shown)",
                objects.len()
            )
        },
    );

    let points = describe_points(&client, &objects, out, stats).await?;
    Ok(Link {
        client,
        device,
        instance,
        objects,
        points,
    })
}

/// The objects that become points: inputs, outputs and values.
fn point_object(info: &BacnetObjectInfo) -> Option<Object> {
    ObjectKind::from_name(&info.object_type).map(|kind| Object {
        kind,
        instance: info.object_instance,
    })
}

/// Name every point after its object-name and give analog points their units.
async fn describe_points(
    client: &BacnetClient,
    objects: &[Object],
    out: &Emitter,
    stats: &mut Stats,
) -> Result<Vec<PointInfo>, String> {
    let mut tags = Vec::with_capacity(objects.len() * 2);
    for object in objects {
        tags.push(object.tag("objectName"));
        if object.kind.is_analog() {
            tags.push(object.tag("units"));
        }
    }
    out.traffic(vec![read_request(
        client,
        objects.len(),
        tags.len(),
        "objectName, units",
    )]);
    let results = client
        .read_each(&tags)
        .await
        .map_err(|error| format!("Reading point names: {error}"))?;
    stats.frames += 2;
    out.traffic(vec![read_answer(client, &results, None)]);
    let mut results = results.into_iter();
    let mut points = Vec::with_capacity(objects.len());
    for (id, object) in objects.iter().enumerate() {
        let name = match results.next() {
            Some(Ok(BacnetValue::String(name))) if !name.trim().is_empty() => name,
            _ => object.label(),
        };
        let unit = if object.kind.is_analog() {
            match results.next() {
                Some(Ok(units)) => units_text(&units),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        points.push(PointInfo {
            id: u32::try_from(id).unwrap_or(u32::MAX),
            name,
            address: object.label(),
            data_type: object.kind.label().into(),
            unit,
            writable: object.kind.writable(),
            boolean: object.kind.is_binary(),
        });
    }
    Ok(points)
}

/// The units property (an enumeration) as shown next to values.
fn units_text(units: &BacnetValue) -> String {
    let number = units.as_f64();
    if number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&number) {
        mt_bacnet::units_label(number as u32)
    } else {
        String::new()
    }
}

fn read_request(
    client: &BacnetClient,
    objects: usize,
    properties: usize,
    what: &str,
) -> TrafficFrame {
    let plural = if objects == 1 { "" } else { "s" };
    if client.reads_multiple() {
        traffic(
            Direction::Tx,
            "ReadPropertyMultiple",
            format!("{objects} object{plural} · {what}"),
            0,
        )
    } else {
        traffic(
            Direction::Tx,
            format!("ReadProperty ×{properties}"),
            format!("{objects} object{plural} · {what}"),
            0,
        )
    }
}

fn read_answer(
    client: &BacnetClient,
    results: &[Result<BacnetValue, String>],
    latency_us: Option<u32>,
) -> TrafficFrame {
    let errors = results.iter().filter(|result| result.is_err()).count();
    let mut detail = format!("{} values", results.len() - errors);
    if errors > 0 {
        detail.push_str(&format!(
            " · {errors} error{}",
            if errors == 1 { "" } else { "s" }
        ));
    }
    if let Some(latency) = latency_us {
        detail.push_str(&format!(" · {:.1} ms", f64::from(latency) / 1000.0));
    }
    let service = if client.reads_multiple() {
        "ComplexAck ReadPropertyMultiple"
    } else {
        "ComplexAck ReadProperty"
    };
    traffic(Direction::Rx, service, detail, 0)
}

/// Read the polled points' present value and status flags, unless the app
/// closes the session first (`None`). Writes that arrive meanwhile wait in
/// `deferred`. With every point subscribed, one property of the device object
/// is read so a device that went away is still noticed.
async fn poll_or_close(
    link: &Link,
    polled: &[usize],
    control: &mut mpsc::UnboundedReceiver<Control>,
    deferred: &mut Vec<(u32, Option<f64>)>,
    out: &Emitter,
    stats: &mut Stats,
) -> Option<Result<Vec<Result<BacnetValue, String>>, ProtocolError>> {
    let tags = if polled.is_empty() {
        vec![mt_bacnet::BacnetObject {
            name: format!("Device {}", link.instance),
            object_type: "device".into(),
            object_instance: link.instance,
            property: "systemStatus".into(),
        }]
    } else {
        polled
            .iter()
            .flat_map(|&point| {
                let object = link.objects[point];
                [object.tag(PRESENT_VALUE), object.tag(STATUS_FLAGS)]
            })
            .collect()
    };
    out.traffic(vec![if polled.is_empty() {
        traffic(
            Direction::Tx,
            "ReadProperty",
            format!("Device {} systemStatus · every point on COV", link.instance),
            0,
        )
    } else {
        read_request(
            &link.client,
            polled.len(),
            tags.len(),
            "presentValue, statusFlags",
        )
    }]);
    let started = Instant::now();
    let read = link.client.read_each(&tags);
    tokio::pin!(read);
    let result = loop {
        tokio::select! {
            result = &mut read => break result,
            command = control.recv() => match command {
                None | Some(Control::Close) => return None,
                Some(Control::Write { point, value }) => deferred.push((point, Some(value))),
                Some(Control::Relinquish { point }) => deferred.push((point, None)),
                Some(_) => {}
            },
        }
    };
    let latency = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
    stats.frames += 2;
    out.traffic(vec![match &result {
        Ok(results) => read_answer(&link.client, results, Some(latency)),
        Err(error) => traffic(Direction::Rx, "No answer", error.to_string(), 0),
    }]);
    Some(if polled.is_empty() {
        // The heartbeat's value is not a point.
        result.map(|_| Vec::new())
    } else {
        result
    })
}

/// Take a poll's answers into the points' states, logging what changed.
fn apply_poll(
    link: &Link,
    polled: &[usize],
    results: Vec<Result<BacnetValue, String>>,
    states: &mut [PointState],
    out: &Emitter,
) {
    let mut results = results.into_iter();
    for &point in polled {
        let (Some(value), Some(flags)) = (results.next(), results.next()) else {
            break;
        };
        let state = &mut states[point];
        let before = state.clone();
        match value {
            Ok(value) => match number(&value) {
                Some(value) => {
                    state.value = Some(value);
                    state.error = None;
                }
                None => {
                    state.error = Some(format!("present value {value:?} is not a number"));
                }
            },
            Err(error) => state.error = Some(error),
        }
        // A device may lack statusFlags on some objects: then they stay normal.
        if let Ok(BacnetValue::String(text)) = flags
            && let Some(flags) = StatusFlags::parse(&text)
        {
            state.flags = flags;
        }
        report_change(link, point, &before, state, out);
    }
}

/// A present value as a number: reals as they are, binary states 0 and 1,
/// multi-state values from 1.
fn number(value: &BacnetValue) -> Option<f64> {
    let number = value.as_f64();
    number.is_finite().then_some(number)
}

fn report_change(
    link: &Link,
    point: usize,
    before: &PointState,
    after: &PointState,
    out: &Emitter,
) {
    let name = || {
        format!(
            "{} {}",
            link.objects[point].label(),
            link.points[point].name
        )
    };
    if before.error != after.error {
        match &after.error {
            Some(error) => out.log(LogLevel::Warn, format!("{}: {error}", name())),
            None if before.error.is_some() => {
                out.log(LogLevel::Info, format!("{} reads again", name()))
            }
            None => {}
        }
    }
    if before.flags != after.flags {
        let level = if after.flags == StatusFlags::default() {
            LogLevel::Info
        } else {
            LogLevel::Warn
        };
        out.log(level, format!("{}: {}", name(), after.flags.describe()));
    }
}

fn frame(states: &[PointState], latency_us: u32) -> ValueFrame {
    ValueFrame {
        at_ms: now_ms(),
        latency_us,
        readings: states.iter().map(PointState::reading).collect(),
    }
}

/// The present-value write of `value` to a point of `kind`: Real for analog
/// objects, Enumerated for binary ones, Unsigned (from 1) for multi-state
/// ones; NULL relinquishes.
pub(crate) fn write_value(kind: ObjectKind, value: Option<f64>) -> Result<BacnetValue, String> {
    let Some(value) = value else {
        return Ok(BacnetValue::Null);
    };
    if !value.is_finite() {
        return Err("Write a number".into());
    }
    if kind.is_analog() {
        let real = value as f32;
        if !real.is_finite() {
            return Err(format!("{value} does not fit a BACnet Real"));
        }
        return Ok(BacnetValue::Real(real));
    }
    if kind.is_binary() {
        return if value == 0.0 || value == 1.0 {
            Ok(BacnetValue::Enum(u32::from(value == 1.0)))
        } else {
            Err("Write 1 for active, 0 for inactive".into())
        };
    }
    if value.fract() == 0.0 && (1.0..=f64::from(u32::MAX)).contains(&value) {
        Ok(BacnetValue::Unsigned(value as u32))
    } else {
        Err("Multi-state values are whole numbers from 1".into())
    }
}

fn value_text(value: &BacnetValue) -> String {
    match value {
        BacnetValue::Real(real) => format!("Real {real}"),
        BacnetValue::Enum(state) => format!("Enumerated {state}"),
        BacnetValue::Unsigned(state) => format!("Unsigned {state}"),
        BacnetValue::Null => "NULL".into(),
        other => format!("{other:?}"),
    }
}

/// Write `value` (or relinquish, `None`) to `point` at `priority` and report
/// the outcome. Returns whether the device took it.
async fn command(
    link: &Link,
    point: u32,
    value: Option<f64>,
    priority: u8,
    out: &Emitter,
    stats: &mut Stats,
) -> bool {
    let Some(&object) = usize::try_from(point)
        .ok()
        .and_then(|index| link.objects.get(index))
    else {
        out.written(point, Err(format!("No point {point}")));
        return false;
    };
    if !object.kind.writable() {
        out.written(
            point,
            Err(format!(
                "{} is an input: it takes no writes",
                object.label()
            )),
        );
        return false;
    }
    let typed = match write_value(object.kind, value) {
        Ok(typed) => typed,
        Err(error) => {
            out.written(point, Err(error));
            return false;
        }
    };
    let relinquish = matches!(typed, BacnetValue::Null);
    out.traffic(vec![traffic(
        Direction::Tx,
        "WriteProperty",
        format!(
            "{} presentValue = {} @ priority {priority}",
            object.label(),
            value_text(&typed)
        ),
        0,
    )]);
    let result = link
        .client
        .write_with_priority(&object.tag(PRESENT_VALUE), typed, Some(priority))
        .await
        .map_err(|error| error.to_string());
    stats.frames += 2;
    match &result {
        Ok(()) => {
            out.traffic(vec![traffic(
                Direction::Rx,
                "SimpleAck WriteProperty",
                object.label(),
                0,
            )]);
            out.log(
                LogLevel::Info,
                if relinquish {
                    format!("{} relinquished at priority {priority}", object.label())
                } else {
                    format!(
                        "{} commanded to {} at priority {priority}",
                        object.label(),
                        value.map_or_else(String::new, |value| value.to_string())
                    )
                },
            );
        }
        Err(error) => {
            stats.errors += 1;
            out.traffic(vec![traffic(
                Direction::Rx,
                "Error WriteProperty",
                error.clone(),
                0,
            )]);
            out.log(LogLevel::Warn, format!("{}: {error}", object.label()));
        }
    }
    let took = result.is_ok();
    out.written(point, result);
    took
}

/// Stop the device's notifications and let the link go.
async fn close(link: &Link, cov: Option<&Cov>) {
    if let Some(cov) = cov {
        cov.cancel(link).await;
    }
    let _ = link.client.disconnect().await;
}

async fn receive(cov: Option<&Cov>, buffer: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
    match cov {
        Some(cov) => cov.socket.recv_from(buffer).await,
        None => std::future::pending().await,
    }
}

/// Change-of-value subscriptions, on a socket of their own: the device
/// notifies the address that subscribed.
struct Cov {
    socket: UdpSocket,
    /// Subscriptions sent and not answered yet: invoke ID → point.
    pending: Vec<(u8, usize)>,
    pending_since: Instant,
    /// When the subscriptions are renewed.
    renew_at: Instant,
    /// Points the device notifies, for renewal and cancellation.
    subscribed: Vec<usize>,
}

impl Cov {
    async fn open(bind: IpAddr, out: &Emitter) -> Option<Self> {
        match UdpSocket::bind(SocketAddr::new(bind, 0)).await {
            Ok(socket) => Some(Self {
                socket,
                pending: Vec::new(),
                pending_since: Instant::now(),
                renew_at: Instant::now() + renewal(),
                subscribed: Vec::new(),
            }),
            Err(error) => {
                out.log(
                    LogLevel::Warn,
                    format!("No socket for COV notifications ({error}): polling every point"),
                );
                None
            }
        }
    }

    /// Ask the device to notify changes of `points`. Answers arrive in
    /// [`Self::handle`]; points stay polled until the device agrees.
    async fn subscribe(&mut self, link: &Link, points: &[usize], out: &Emitter, stats: &mut Stats) {
        for chunk in points.chunks(COV_BATCH) {
            let mut frames = Vec::with_capacity(chunk.len());
            for &point in chunk {
                let object = link.objects[point];
                let invoke = mt_bacnet::transport::next_invoke_id();
                let Ok(frame) = mt_bacnet::build_subscribe_cov_frame(
                    invoke,
                    COV_PROCESS,
                    object.kind.name(),
                    object.instance,
                    false,
                    COV_LIFETIME_S,
                ) else {
                    continue;
                };
                if self.socket.send_to(&frame, link.device).await.is_ok() {
                    self.pending.retain(|(_, waiting)| *waiting != point);
                    self.pending.push((invoke, point));
                    stats.frames += 1;
                    let (summary, detail) = describe_frame(&frame);
                    frames.push(traffic(Direction::Tx, summary, detail, frame.len()));
                }
            }
            out.traffic(frames);
        }
        self.pending_since = Instant::now();
    }

    /// Give up on unanswered subscriptions and renew the others when due.
    async fn maintain(
        &mut self,
        link: &Link,
        states: &mut [PointState],
        out: &Emitter,
        stats: &mut Stats,
    ) {
        if !self.pending.is_empty() && self.pending_since.elapsed() >= COV_ANSWER_WAIT {
            let unanswered = self.pending.len();
            for (_, point) in self.pending.drain(..) {
                if states[point].subscribed {
                    // A renewal went unanswered: poll the point again.
                    states[point].subscribed = false;
                    self.subscribed.retain(|subscribed| *subscribed != point);
                }
            }
            out.log(
                LogLevel::Warn,
                format!("{unanswered} COV subscriptions went unanswered: those points are polled"),
            );
        }
        if Instant::now() >= self.renew_at {
            self.renew_at = Instant::now() + renewal();
            let points = self.subscribed.clone();
            self.subscribe(link, &points, out, stats).await;
        }
    }

    /// Take one datagram from the device: a subscription's answer or a
    /// notification. Returns whether a point's value or flags changed.
    async fn handle(
        &mut self,
        link: &Link,
        frame: &[u8],
        source: SocketAddr,
        states: &mut [PointState],
        out: &Emitter,
    ) -> bool {
        let (summary, detail) = describe_frame(frame);
        out.traffic(vec![traffic(Direction::Rx, summary, detail, frame.len())]);
        if let Some(packet) = mt_bacnet::parse_cov_packet(frame) {
            if let Some(invoke) = packet.confirmed_invoke_id
                && let Some((to, ack)) = mt_bacnet::cov::simple_ack_for(frame, source, invoke, 1)
            {
                let _ = self.socket.send_to(&ack, to).await;
            }
            return notification(link, &packet.notification, states, out);
        }
        let Some(apdu) = mt_bacnet::transport::apdu_of_frame(frame) else {
            return false;
        };
        let (Some(&kind), Some(&invoke)) = (apdu.first(), apdu.get(1)) else {
            return false;
        };
        let Some(at) = self
            .pending
            .iter()
            .position(|(waiting, _)| *waiting == invoke)
        else {
            return false;
        };
        let (_, point) = self.pending.swap_remove(at);
        let object = link.objects[point];
        match kind >> 4 {
            // SimpleAck: the device notifies this point from now on.
            2 => {
                if !states[point].subscribed {
                    states[point].subscribed = true;
                    self.subscribed.push(point);
                }
            }
            // Error, Reject or Abort: no COV for this object; keep polling it.
            _ => {
                if states[point].subscribed {
                    states[point].subscribed = false;
                    self.subscribed.retain(|subscribed| *subscribed != point);
                }
                let (_, reason) = describe_frame(frame);
                out.log(
                    LogLevel::Info,
                    format!(
                        "{} takes no COV subscription ({reason}): polled",
                        object.label()
                    ),
                );
            }
        }
        if self.pending.is_empty() {
            out.log(
                LogLevel::Info,
                format!(
                    "{} of {} points on COV, {} polled",
                    self.subscribed.len(),
                    link.objects.len(),
                    link.objects.len() - self.subscribed.len()
                ),
            );
        }
        false
    }

    /// Cancel every subscription; the device may be gone, so nobody waits.
    async fn cancel(&self, link: &Link) {
        for &point in &self.subscribed {
            let object = link.objects[point];
            if let Ok(frame) = mt_bacnet::build_cancel_cov_frame(
                mt_bacnet::transport::next_invoke_id(),
                COV_PROCESS,
                object.kind.name(),
                object.instance,
            ) {
                let _ = self.socket.send_to(&frame, link.device).await;
            }
        }
    }
}

fn renewal() -> Duration {
    mt_bacnet::renewal_delay(COV_LIFETIME_S).unwrap_or(Duration::from_secs(240))
}

/// Take a notification's present value and status flags into the point it
/// names. Returns whether anything changed.
fn notification(
    link: &Link,
    notification: &mt_bacnet::CovNotification,
    states: &mut [PointState],
    out: &Emitter,
) -> bool {
    let Some(kind) = ObjectKind::from_name(&notification.monitored_object_type) else {
        return false;
    };
    let named = Object {
        kind,
        instance: notification.monitored_object_instance,
    };
    let Some(point) = link.objects.iter().position(|object| *object == named) else {
        return false;
    };
    let state = &mut states[point];
    let before = state.clone();
    for value in &notification.values {
        match value.property_name.as_str() {
            PRESENT_VALUE => {
                if let Some(number) = number(&value.value) {
                    state.value = Some(number);
                    state.error = None;
                }
            }
            STATUS_FLAGS => {
                if let Some(flags) = StatusFlags::parse(&value.raw_value) {
                    state.flags = flags;
                }
            }
            _ => {}
        }
    }
    report_change(link, point, &before, state, out);
    state.value != before.value || state.flags != before.flags || state.error != before.error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_take_the_datatype_of_the_object() {
        assert!(matches!(
            write_value(ObjectKind::AnalogValue, Some(21.5)),
            Ok(BacnetValue::Real(real)) if real == 21.5
        ));
        assert!(matches!(
            write_value(ObjectKind::BinaryOutput, Some(1.0)),
            Ok(BacnetValue::Enum(1))
        ));
        assert!(matches!(
            write_value(ObjectKind::BinaryValue, Some(0.0)),
            Ok(BacnetValue::Enum(0))
        ));
        assert!(matches!(
            write_value(ObjectKind::MultiStateValue, Some(3.0)),
            Ok(BacnetValue::Unsigned(3))
        ));
        assert!(matches!(
            write_value(ObjectKind::AnalogOutput, None),
            Ok(BacnetValue::Null)
        ));
    }

    #[test]
    fn writes_outside_the_datatype_are_refused() {
        assert!(write_value(ObjectKind::BinaryOutput, Some(2.0)).is_err());
        assert!(write_value(ObjectKind::BinaryOutput, Some(0.5)).is_err());
        // Multi-state values count from 1.
        assert!(write_value(ObjectKind::MultiStateOutput, Some(0.0)).is_err());
        assert!(write_value(ObjectKind::MultiStateOutput, Some(2.5)).is_err());
        assert!(write_value(ObjectKind::AnalogValue, Some(f64::NAN)).is_err());
        // Finite as f64, infinite as a BACnet Real.
        assert!(write_value(ObjectKind::AnalogValue, Some(1e300)).is_err());
    }

    #[test]
    fn a_faulted_or_unread_point_is_bad() {
        let good = PointState {
            value: Some(21.0),
            ..PointState::default()
        };
        assert_eq!(good.reading().quality, Quality::Good);
        // In alarm or out of service, the value is still the device's value.
        let alarmed = PointState {
            flags: StatusFlags {
                in_alarm: true,
                out_of_service: true,
                ..StatusFlags::default()
            },
            ..good.clone()
        };
        assert_eq!(alarmed.reading().quality, Quality::Good);
        let faulted = PointState {
            flags: StatusFlags {
                fault: true,
                ..StatusFlags::default()
            },
            ..good.clone()
        };
        assert_eq!(faulted.reading().quality, Quality::Bad);
        let failed = PointState {
            error: Some("object / unknown-object".into()),
            ..good
        };
        assert_eq!(failed.reading().quality, Quality::Bad);
        assert_eq!(PointState::default().reading().quality, Quality::Bad);
    }

    #[test]
    fn units_show_as_symbols() {
        assert_eq!(units_text(&BacnetValue::Float64(62.0)), "°C");
        assert_eq!(units_text(&BacnetValue::Float64(98.0)), "%");
        // no-units
        assert_eq!(units_text(&BacnetValue::Float64(95.0)), "");
        assert_eq!(units_text(&BacnetValue::String("x".into())), "");
    }
}
