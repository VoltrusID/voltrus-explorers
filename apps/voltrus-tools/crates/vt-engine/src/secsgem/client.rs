//! A GEM host: connects to equipment over HSMS, establishes communications,
//! reads its status variables and equipment constants as points, writes the
//! constants, and reports the events and alarms it sends.

use std::time::{Duration, Instant};

use mt_secsgem::sim::model::like;
use mt_secsgem::{
    Connector, EcInfo, EventLink, GemHost, HostEvent, HsmsConfig, IdFormat, Identity, Item,
    LogEntry, ReportDef, SecsError, SvInfo,
};
use tokio::sync::{broadcast, mpsc};
use tokio::time::MissedTickBehavior;
use vt_protocol::{
    LogLevel, PointInfo, Quality, Reading, SECSGEM_MAX_REPORT_VIDS, SecsGemClientSpec, SessionSpec,
    SessionState, Stats, ValueFrame,
};

use super::{clip, eac_error, frame, is_boolean, point, type_of, value_of};
use crate::Control;
use crate::cnc::{Texts, retry, until_closed};
use crate::emit::{Emitter, now_ms};

/// Fastest poll the engine runs, whatever the spec asks for.
const MIN_POLL: Duration = Duration::from_millis(100);
/// The report the events subscription defines.
const REPORT_ID: u64 = 1;
/// How the host keeps an idle link alive.
const LINKTEST_MS: u64 = 30_000;
const LOST: &str = "The connection to the equipment was lost";

/// A write that came in while a poll was on the wire, done after it.
struct Deferred {
    point: u32,
    value: f64,
}

/// The points of the session: status variables, then equipment constants.
struct Layout {
    svids: Vec<u64>,
    /// Name and units of each status variable.
    sv_names: Vec<(String, String)>,
    ecids: Vec<u64>,
    ec_names: Vec<(String, String)>,
    /// Allowed values of each constant, when the equipment names a range.
    ranges: Vec<Option<(f64, f64)>>,
    /// The last item of each constant, whose type a write takes.
    last: Vec<Option<Item>>,
    /// Which constants take a number.
    writable: Vec<bool>,
}

enum Poll {
    /// What the equipment answered for every point (`None`: nothing), and
    /// the requests that failed.
    Done {
        items: Vec<Option<Item>>,
        failed: Vec<String>,
    },
    /// The link is gone.
    Lost,
}

/// What one poll showed.
struct Polled {
    readings: Vec<Reading>,
    texts: Vec<String>,
    /// Requests that failed and ids the equipment could not provide.
    failed: Vec<String>,
}

pub(crate) async fn run(
    spec: SecsGemClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let poll = Duration::from_millis(spec.poll_ms).max(MIN_POLL);
    let address = SessionSpec::SecsGemClient(spec.clone()).address();
    let hsms = HsmsConfig {
        device_id: spec.device_id,
        t3_ms: spec.timeout_ms,
        connect_timeout_ms: spec.timeout_ms,
        linktest_interval_ms: Some(LINKTEST_MS),
        // The session's own backoff spaces the attempts (0.5 s doubling to
        // 10 s); T5's 10 s default on top of it would only slow a reconnect.
        t5_ms: 500,
        ..HsmsConfig::default()
    };
    let (log_tx, mut log) = broadcast::channel::<LogEntry>(512);
    let connector = Connector::new(hsms).with_log(log_tx);
    let mut stats = Stats::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let connected = tokio::select! {
            result = establish(&connector, &address, &spec, out) => result,
            () = until_closed(&mut control, out, "Not connected") => break 'session,
        };
        let (host, mut layout, identity) = match connected {
            Ok(connected) => connected,
            Err(reason) => {
                attempt += 1;
                stats.errors += 1;
                out.stats(stats);
                out.log(LogLevel::Error, reason.clone());
                drain(&mut log, &mut stats, out);
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        attempt = 0;
        let mut events = host.subscribe();
        subscribe_events(&host, &spec, &layout, out).await;

        // The first poll tells what type each point has.
        let started = Instant::now();
        let Poll::Done { items, failed } = poll_once(&host, &layout).await else {
            out.log(LogLevel::Error, LOST);
            host.disconnect();
            if !retry(&mut control, 1, LOST, out).await {
                break 'session;
            }
            continue 'session;
        };
        let points = points(&mut layout, &items);
        out.points(points.clone());
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Connected to {address}: {}, {} status variable(s), {} equipment constant(s)",
                match &identity {
                    Some(id) => format!("{} {}", id.mdln.trim(), id.softrev.trim()),
                    None => "equipment that gave no model".to_owned(),
                },
                layout.svids.len(),
                layout.ecids.len()
            ),
        );
        let mut seen = Texts::new(points.len());
        let mut failing: Vec<String> = Vec::new();
        report(
            &mut stats,
            started,
            polled(&mut layout, &items, failed),
            &mut failing,
            &mut seen,
            out,
        );
        drain(&mut log, &mut stats, out);

        let mut deferred: Vec<Deferred> = Vec::new();
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker.reset();

        let lost = 'link: loop {
            for job in std::mem::take(&mut deferred) {
                let result = write(&host, &layout, job.point, job.value, out).await;
                if let Err(error) = &result {
                    out.log(LogLevel::Warn, error.clone());
                }
                out.written(job.point, result);
            }
            if !host.connection().is_selected() {
                break 'link match host.connection().close_reason() {
                    Some(error) => format!("{LOST}: {error}"),
                    None => LOST.to_owned(),
                };
            }
            tokio::select! {
                _ = ticker.tick() => {
                    let started = Instant::now();
                    match poll_once(&host, &layout).await {
                        Poll::Lost => break 'link LOST.to_owned(),
                        Poll::Done { items, failed } => report(
                            &mut stats,
                            started,
                            polled(&mut layout, &items, failed),
                            &mut failing,
                            &mut seen,
                            out,
                        ),
                    }
                }
                entry = log.recv() => if let Ok(entry) = entry {
                    stats.frames += 1;
                    out.traffic(vec![frame(&entry)]);
                },
                event = events.recv() => match event {
                    Ok(HostEvent::Closed { reason }) => {
                        break 'link match reason {
                            Some(reason) if !reason.is_empty() => format!("{LOST}: {reason}"),
                            _ => "The equipment closed the connection".to_owned(),
                        };
                    }
                    Ok(event) => log_event(&event, out),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break 'link LOST.to_owned(),
                },
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        host.disconnect();
                        drain(&mut log, &mut stats, out);
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => deferred.push(Deferred { point, value }),
                    Some(_) => {}
                },
            }
        };
        out.log(LogLevel::Error, lost.clone());
        out.stats(stats);
        host.disconnect();
        drain(&mut log, &mut stats, out);
        if !retry(&mut control, 1, &lost, out).await {
            break 'session;
        }
    }
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Pass the messages logged so far to the traffic view.
fn drain(log: &mut broadcast::Receiver<LogEntry>, stats: &mut Stats, out: &Emitter) {
    let mut frames = Vec::new();
    while let Ok(entry) = log.try_recv() {
        frames.push(frame(&entry));
    }
    stats.frames += frames.len() as u64;
    out.traffic(frames);
}

fn report(
    stats: &mut Stats,
    started: Instant,
    polled: Polled,
    failing: &mut Vec<String>,
    seen: &mut Texts,
    out: &Emitter,
) {
    let latency_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
    stats.polls += 1;
    stats.last_latency_us = latency_us;
    if !polled.failed.is_empty() {
        stats.errors += 1;
    }
    if polled.failed != *failing {
        if !polled.failed.is_empty() {
            out.log(
                LogLevel::Warn,
                format!(
                    "The equipment could not provide: {}",
                    polled.failed.join(", ")
                ),
            );
        } else if !failing.is_empty() {
            out.log(LogLevel::Info, "Every value reads again");
        }
        failing.clone_from(&polled.failed);
    }
    let changed = seen.changed(&polled.texts);
    if !changed.is_empty() {
        out.payloads(changed);
    }
    out.values(ValueFrame {
        at_ms: now_ms(),
        latency_us,
        readings: polled.readings,
    });
    out.stats(*stats);
}

/// Connect, select the session, establish communications and name the
/// points. The error is what to tell the user.
async fn establish(
    connector: &Connector,
    address: &str,
    spec: &SecsGemClientSpec,
    out: &Emitter,
) -> Result<(GemHost, Layout, Option<Identity>), String> {
    let host = GemHost::connect_with(connector, address, IdFormat::U4)
        .await
        .map_err(|error| error.to_string())?;
    match setup(&host, spec, out).await {
        Ok((layout, identity)) => Ok((host, layout, identity)),
        Err(error) => {
            host.disconnect();
            Err(error.to_string())
        }
    }
}

async fn setup(
    host: &GemHost,
    spec: &SecsGemClientSpec,
    out: &Emitter,
) -> Result<(Layout, Option<Identity>), SecsError> {
    let (ack, identity) = host.establish_communications().await?;
    if !ack.is_ok() {
        return Err(SecsError::UnexpectedReply {
            call: "S1F13",
            detail: format!(
                "the equipment denied communication (COMMACK {})",
                ack.code()
            ),
        });
    }
    // Names are best effort: equipment that does not list them still
    // answers value requests for the ids the user typed.
    let names: Vec<SvInfo> = match host.status_variable_names(&spec.svids).await {
        Ok(names) => names,
        Err(error) => {
            out.log(
                LogLevel::Warn,
                format!("The equipment did not name its status variables: {error}"),
            );
            Vec::new()
        }
    };
    let constants: Vec<EcInfo> = match host.equipment_constant_names(&spec.ecids).await {
        Ok(constants) => constants,
        Err(error) => {
            out.log(
                LogLevel::Warn,
                format!("The equipment did not name its equipment constants: {error}"),
            );
            Vec::new()
        }
    };
    let (svids, sv_names) = if names.is_empty() {
        let ids = spec.svids.clone();
        let names = ids
            .iter()
            .map(|id| (format!("SV {id}"), String::new()))
            .collect();
        (ids, names)
    } else {
        (
            names.iter().map(|sv| sv.id).collect(),
            names
                .iter()
                .map(|sv| {
                    (
                        display_name("SV", sv.id, &sv.name),
                        sv.units.trim().to_owned(),
                    )
                })
                .collect(),
        )
    };
    let (ecids, ec_names, ranges) = if constants.is_empty() {
        let ids = spec.ecids.clone();
        let names = ids
            .iter()
            .map(|id| (format!("EC {id}"), String::new()))
            .collect();
        let ranges = vec![None; ids.len()];
        (ids, names, ranges)
    } else {
        (
            constants.iter().map(|ec| ec.id).collect(),
            constants
                .iter()
                .map(|ec| {
                    (
                        display_name("EC", ec.id, &ec.name),
                        ec.units.trim().to_owned(),
                    )
                })
                .collect(),
            constants
                .iter()
                .map(|ec| Some((ec.min.as_f64()?, ec.max.as_f64()?)))
                .collect(),
        )
    };
    Ok((
        Layout {
            last: vec![None; ecids.len()],
            writable: vec![false; ecids.len()],
            svids,
            sv_names,
            ecids,
            ec_names,
            ranges,
        },
        identity,
    ))
}

fn display_name(prefix: &str, id: u64, name: &str) -> String {
    match name.trim() {
        "" => format!("{prefix} {id}"),
        name => name.to_owned(),
    }
}

/// Define a report of the status variables, link it to the events the user
/// listed and enable them; enable the alarm reports. A refusal is logged and
/// the rest carries on.
async fn subscribe_events(
    host: &GemHost,
    spec: &SecsGemClientSpec,
    layout: &Layout,
    out: &Emitter,
) {
    if !spec.events.is_empty() {
        let vids: Vec<u64> = layout
            .svids
            .iter()
            .take(SECSGEM_MAX_REPORT_VIDS)
            .copied()
            .collect();
        let mut linked = true;
        if !vids.is_empty() {
            let count = vids.len();
            match host
                .define_reports(&[ReportDef {
                    rptid: REPORT_ID,
                    vids,
                }])
                .await
            {
                Ok(ack) if ack.is_ok() => out.log(
                    LogLevel::Info,
                    format!("Defined report {REPORT_ID} of {count} status variable(s)"),
                ),
                Ok(ack) => {
                    linked = false;
                    out.log(
                        LogLevel::Warn,
                        format!("The equipment refused the report: {ack:?}"),
                    );
                }
                Err(error) => {
                    linked = false;
                    out.log(
                        LogLevel::Warn,
                        format!("Defining the report failed: {error}"),
                    );
                }
            }
            if linked {
                let links: Vec<EventLink> = spec
                    .events
                    .iter()
                    .map(|ceid| EventLink {
                        ceid: *ceid,
                        rptids: vec![REPORT_ID],
                    })
                    .collect();
                match host.link_event_reports(&links).await {
                    Ok(ack) if ack.is_ok() => {}
                    Ok(ack) => out.log(
                        LogLevel::Warn,
                        format!("The equipment refused to link the report to the events: {ack:?}"),
                    ),
                    Err(error) => out.log(
                        LogLevel::Warn,
                        format!("Linking the report to the events failed: {error}"),
                    ),
                }
            }
        }
        match host.enable_events(true, &spec.events).await {
            Ok(ack) if ack.is_ok() => out.log(
                LogLevel::Info,
                format!("Subscribed to {} collection event(s)", spec.events.len()),
            ),
            Ok(ack) => out.log(
                LogLevel::Warn,
                format!("The equipment refused to enable the events: {ack:?}"),
            ),
            Err(error) => out.log(
                LogLevel::Warn,
                format!("Enabling the events failed: {error}"),
            ),
        }
    }
    if spec.alarms {
        match host.enable_alarms(true, None).await {
            Ok(ack) if ack.is_ok() => out.log(LogLevel::Info, "Alarm reports are on"),
            Ok(ack) => out.log(
                LogLevel::Warn,
                format!("The equipment refused to enable alarm reports: {ack:?}"),
            ),
            Err(error) => out.log(
                LogLevel::Warn,
                format!("Enabling alarm reports failed: {error}"),
            ),
        }
    }
}

/// What the equipment sent by itself, in the log.
fn log_event(event: &HostEvent, out: &Emitter) {
    match event {
        HostEvent::EventReport(report) => {
            let values: Vec<String> = report
                .reports
                .iter()
                .flat_map(|report| report.values.iter().map(Item::to_sml))
                .collect();
            out.log(
                LogLevel::Info,
                if values.is_empty() {
                    format!("Event {} reported", report.ceid)
                } else {
                    format!(
                        "Event {} reported: {}",
                        report.ceid,
                        clip(&values.join(", "), 160)
                    )
                },
            );
        }
        HostEvent::Alarm(alarm) => {
            if alarm.is_set() {
                out.log(
                    LogLevel::Warn,
                    format!(
                        "Alarm {} set (category {}): {}",
                        alarm.alid,
                        alarm.category(),
                        alarm.altx.trim()
                    ),
                );
            } else {
                out.log(
                    LogLevel::Info,
                    format!("Alarm {} cleared: {}", alarm.alid, alarm.altx.trim()),
                );
            }
        }
        HostEvent::Terminal(text) => out.log(
            LogLevel::Info,
            format!(
                "Terminal message {}: {}",
                text.tid,
                clip(text.text.trim(), 160)
            ),
        ),
        HostEvent::S9(error) => out.log(
            LogLevel::Warn,
            format!("The equipment reported an error: {error}"),
        ),
        HostEvent::Unhandled(message) => out.log(
            LogLevel::Warn,
            format!(
                "The equipment sent {}, which this host does not handle",
                message.name()
            ),
        ),
        HostEvent::Closed { .. } => {}
    }
}

/// Read every status variable and equipment constant.
async fn poll_once(host: &GemHost, layout: &Layout) -> Poll {
    let mut items: Vec<Option<Item>> = Vec::with_capacity(layout.svids.len() + layout.ecids.len());
    let mut failed = Vec::new();
    // An empty id list means "all" on the wire, so it is never sent.
    if !layout.svids.is_empty() {
        match host.status_variables(&layout.svids).await {
            Ok(list) => items.extend((0..layout.svids.len()).map(|index| list.get(index).cloned())),
            Err(error) => {
                if !host.connection().is_selected() {
                    return Poll::Lost;
                }
                failed.push(format!("status variables ({error})"));
                items.extend(layout.svids.iter().map(|_| None));
            }
        }
    }
    if !layout.ecids.is_empty() {
        match host.equipment_constants(&layout.ecids).await {
            Ok(list) => items.extend((0..layout.ecids.len()).map(|index| list.get(index).cloned())),
            Err(error) => {
                if !host.connection().is_selected() {
                    return Poll::Lost;
                }
                failed.push(format!("equipment constants ({error})"));
                items.extend(layout.ecids.iter().map(|_| None));
            }
        }
    }
    Poll::Done { items, failed }
}

/// The points: what each is called and what type its first value had.
fn points(layout: &mut Layout, items: &[Option<Item>]) -> Vec<PointInfo> {
    let mut points = Vec::new();
    for (index, id) in layout.svids.iter().enumerate() {
        let item = items.get(index).and_then(Option::as_ref);
        let (name, unit) = &layout.sv_names[index];
        points.push(point(
            points.len(),
            name.clone(),
            format!("SVID {id}"),
            item.map_or("", type_of),
            unit,
            false,
            item.is_some_and(is_boolean),
        ));
    }
    for (index, id) in layout.ecids.iter().enumerate() {
        let item = items
            .get(layout.svids.len() + index)
            .and_then(Option::as_ref);
        let numeric = item.is_some_and(|item| item.len() == 1 && item.as_f64().is_some());
        layout.writable[index] = numeric;
        layout.last[index] = item.cloned();
        let (name, unit) = &layout.ec_names[index];
        points.push(point(
            points.len(),
            name.clone(),
            format!("ECID {id}"),
            item.map_or("", type_of),
            unit,
            numeric,
            item.is_some_and(is_boolean),
        ));
    }
    points
}

/// Readings and texts of a poll; remembers each constant's item for writes.
fn polled(layout: &mut Layout, items: &[Option<Item>], mut failed: Vec<String>) -> Polled {
    let mut readings = Vec::with_capacity(items.len());
    let mut texts = Vec::with_capacity(items.len());
    let mut refused: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let name = match index.checked_sub(layout.svids.len()) {
            None => &layout.sv_names[index].0,
            Some(slot) => &layout.ec_names[slot].0,
        };
        match item {
            Some(item) => {
                let value = value_of(item);
                if value.reading.quality == Quality::Bad {
                    refused.push(name.clone());
                }
                readings.push(value.reading);
                texts.push(value.text);
                if let Some(slot) = index.checked_sub(layout.svids.len())
                    && layout.writable[slot]
                {
                    layout.last[slot] = Some(item.clone());
                }
            }
            None => {
                readings.push(Reading::bad());
                texts.push(String::new());
            }
        }
    }
    failed.extend(refused);
    Polled {
        readings,
        texts,
        failed,
    }
}

async fn write(
    host: &GemHost,
    layout: &Layout,
    point: u32,
    number: f64,
    out: &Emitter,
) -> Result<(), String> {
    let index = usize::try_from(point).map_err(|_| format!("No point {point}"))?;
    let slot = index
        .checked_sub(layout.svids.len())
        .filter(|slot| *slot < layout.ecids.len())
        .ok_or_else(|| {
            "Status variables are read-only; only equipment constants can be written".to_owned()
        })?;
    let id = layout.ecids[slot];
    if !layout.writable[slot] {
        return Err(format!("Equipment constant {id} is not a single number"));
    }
    if !number.is_finite() {
        return Err("Enter a number".into());
    }
    let range = layout.ranges[slot];
    if let Some((min, max)) = range
        && (number < min || number > max)
    {
        return Err(eac_error(mt_secsgem::Eac::OutOfRange, id, range));
    }
    let item = layout.last[slot]
        .as_ref()
        .and_then(|template| like(template, number))
        .ok_or_else(|| format!("{number} does not fit the type of equipment constant {id}"))?;
    out.log(
        LogLevel::Info,
        format!("Writing equipment constant {id} = {number}"),
    );
    match host.set_equipment_constants(&[(id, item)]).await {
        Ok(eac) if eac.is_ok() => Ok(()),
        Ok(eac) => Err(eac_error(eac, id, range)),
        Err(error) => Err(error.to_string()),
    }
}
