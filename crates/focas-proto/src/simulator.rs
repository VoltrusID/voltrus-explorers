//! A CNC simulator that speaks FOCAS/Ethernet.
//!
//! [`FocasSimulator`] listens on a TCP port (8193 by default), accepts the open handshake and
//! answers the calls of this crate's client (and pyfanuc) from a [`CncModel`]. The model is shared:
//! [`FocasSimulator::with_model`] lets a UI change it while clients are connected, and a
//! background task advances it in real time so a started cycle moves the axes.
//!
//! Every call the client implements is answered with the framing and payload layout the reference
//! documents. Where the reference has no capture of an answer (macro write), the simulator
//! uses the same sub-packet layout as the calls that are captured.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::client::{DEFAULT_PORT, read_frame};
use crate::codec::{
    CHANNEL_CONTROL, DEVICE_PMC, Frame, FrameType, SubRequest, SubResponse, decode_var_request,
    encode_scaled, encode_var_response,
};
use crate::error::ew;
use crate::functions::*;
use crate::model::{CncModel, CncProfile, pmc_area_size};
use crate::types::*;

/// How to start a simulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatorConfig {
    /// Address to bind; `127.0.0.1` keeps the simulator local, `0.0.0.0` opens it to the network.
    pub bind_addr: String,
    /// TCP port; 0 picks a free one (see [`FocasSimulator::port`]).
    pub port: u16,
    pub profile: CncProfile,
    /// How often the model advances in real time; `None` leaves time to the caller
    /// ([`FocasSimulator::advance`]), which is what tests want.
    pub tick: Option<Duration>,
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        SimulatorConfig {
            bind_addr: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            profile: CncProfile::Series31iB,
            tick: Some(Duration::from_millis(100)),
        }
    }
}

struct Shared {
    model: Mutex<CncModel>,
    requests: AtomicU64,
    sessions: AtomicU64,
}

impl Shared {
    fn model(&self) -> std::sync::MutexGuard<'_, CncModel> {
        self.model.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A running simulator. Dropping it without [`FocasSimulator::stop`] leaves the tasks running
/// until the runtime ends.
pub struct FocasSimulator {
    addr: SocketAddr,
    shared: Arc<Shared>,
    shutdown: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl FocasSimulator {
    /// Bind and start serving.
    pub async fn start(config: SimulatorConfig) -> io::Result<Self> {
        Self::start_with_model(config.clone(), CncModel::new(config.profile)).await
    }

    /// Like [`FocasSimulator::start`] with a prepared model (its `sysinfo` is what clients read).
    pub async fn start_with_model(config: SimulatorConfig, model: CncModel) -> io::Result<Self> {
        let listener = TcpListener::bind((config.bind_addr.as_str(), config.port)).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            model: Mutex::new(model),
            requests: AtomicU64::new(0),
            sessions: AtomicU64::new(0),
        });
        let (shutdown, shutdown_rx) = watch::channel(false);

        let mut tasks = Vec::new();
        tasks.push(tokio::spawn(accept_loop(
            listener,
            Arc::clone(&shared),
            shutdown_rx.clone(),
        )));
        if let Some(tick) = config.tick {
            tasks.push(tokio::spawn(tick_loop(
                Arc::clone(&shared),
                tick,
                shutdown_rx,
            )));
        }
        tracing::info!("FOCAS simulator listening on {addr}");
        Ok(FocasSimulator {
            addr,
            shared,
            shutdown,
            tasks: Mutex::new(tasks),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Change the simulated CNC. Runs under the lock the request handlers use, so keep it short.
    pub fn with_model<R>(&self, f: impl FnOnce(&mut CncModel) -> R) -> R {
        f(&mut self.shared.model())
    }

    /// A copy of the simulated CNC as it is now.
    pub fn model(&self) -> CncModel {
        self.shared.model().clone()
    }

    /// Advance simulated time by hand (needed when `tick` is `None`).
    pub fn advance(&self, dt: Duration) {
        self.shared.model().advance(dt.as_secs_f64());
    }

    /// Requests answered so far (sub-packets, not frames).
    pub fn requests_served(&self) -> u64 {
        self.shared.requests.load(Ordering::Relaxed)
    }

    /// Clients currently connected.
    pub fn active_sessions(&self) -> u64 {
        self.shared.sessions.load(Ordering::Relaxed)
    }

    /// Stop listening and drop every connection.
    pub async fn stop(&self) {
        let _ = self.shutdown.send(true);
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|p| p.into_inner()));
        for t in tasks {
            let _ = t.await;
        }
    }
}

async fn tick_loop(shared: Arc<Shared>, tick: Duration, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = Instant::now();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Instant::now();
                // a stalled runtime must not teleport the axes
                let dt = now.duration_since(last).as_secs_f64().min(1.0);
                last = now;
                shared.model().advance(dt);
            }
            _ = shutdown.changed() => break,
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    // Dropping the set aborts every session when the loop ends.
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    stream.set_nodelay(true).ok();
                    let shared = Arc::clone(&shared);
                    sessions.spawn(async move {
                        shared.sessions.fetch_add(1, Ordering::Relaxed);
                        if let Err(e) = serve(stream, &shared).await {
                            tracing::debug!("FOCAS session {peer} ended: {e}");
                        }
                        shared.sessions.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                Err(e) => tracing::warn!("FOCAS accept failed: {e}"),
            },
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
            _ = shutdown.changed() => break,
        }
    }
}

async fn serve(mut stream: TcpStream, shared: &Shared) -> Result<(), String> {
    loop {
        let frame = read_frame(&mut stream).await.map_err(|e| e.to_string())?;
        let reply = match frame.frame_type {
            FrameType::OpenRequest => {
                if frame.open_channel() != Some(CHANNEL_CONTROL) {
                    tracing::debug!(
                        "open request for channel {:?}: only the control stream is simulated",
                        frame.open_channel()
                    );
                }
                Frame::open_response()
            }
            FrameType::CloseRequest => {
                send(&mut stream, &Frame::close_response()).await?;
                return Ok(());
            }
            FrameType::VarRequest => {
                let subs = decode_var_request(&frame).map_err(|e| e.to_string())?;
                let responses: Vec<SubResponse> = {
                    let mut model = shared.model();
                    subs.iter().map(|s| answer(&mut model, s)).collect()
                };
                shared
                    .requests
                    .fetch_add(subs.len() as u64, Ordering::Relaxed);
                encode_var_response(&responses).map_err(|e| e.to_string())?
            }
            other => return Err(format!("unsupported frame type {:04x}", other.code())),
        };
        send(&mut stream, &reply).await?;
    }
}

/// One write per frame: pyfanuc reads a response with a single `recv`.
async fn send(stream: &mut TcpStream, frame: &Frame) -> Result<(), String> {
    let bytes = frame.encode().map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())
}

/// Answer one sub-packet from the model. Exposed for tests and for hosts that carry the framing
/// themselves.
pub fn answer(model: &mut CncModel, req: &SubRequest) -> SubResponse {
    let cmd = req.command;
    let ok = |payload: Vec<u8>| SubResponse::ok(cmd, payload);
    let fail = |code: i16| SubResponse::failed(cmd, code);
    let a = req.args;

    if let Some(code) = model.refuse.get(&cmd.function) {
        return fail(*code);
    }

    if cmd.device == DEVICE_PMC {
        return if cmd == func::PMC_READ {
            answer_pmc(model, req)
        } else {
            fail(ew::FUNC)
        };
    }

    match cmd {
        func::SYSINFO => ok(encode_sysinfo(&model.sysinfo)),
        func::STATINFO => ok(encode_statinfo(&model.status())),
        func::ALARM => ok(encode_alarm_bits(model.alarm_bits())),
        func::PROGRAM_NUMBER => ok(encode_program_numbers(model.program_numbers())),
        func::SEQUENCE_NUMBER => ok(model.sequence_number().to_be_bytes().to_vec()),
        func::ACTUAL_FEED => ok(encode_scaled(model.feed_rate, 0).to_vec()),
        func::ACTUAL_SPINDLE => ok(encode_scaled(model.spindle_speed, 0).to_vec()),
        func::AXIS_NAMES => {
            let names: Vec<AxisName> = model.axes.iter().map(|x| x.name.clone()).collect();
            ok(encode_axis_names(&names))
        }
        func::POSITION => {
            let Some(kind) = PositionKind::from_code(a[0]) else {
                return fail(ew::NUMBER);
            };
            // Only the all-axes form appears in the reference samples.
            if a[1] != ALL_AXES {
                return fail(ew::NUMBER);
            }
            let values: Vec<Option<f64>> = model
                .axes
                .iter()
                .map(|x| {
                    Some(match kind {
                        PositionKind::Absolute => x.absolute,
                        PositionKind::Machine => x.machine(),
                        PositionKind::Relative => x.relative(),
                        PositionKind::DistanceToGo => x.distance_to_go,
                        PositionKind::Skip => x.skip_position,
                    })
                })
                .collect();
            ok(encode_scaled_list(
                &values,
                POSITION_SLOTS,
                model.position_decimals,
            ))
        }
        func::SERVO_LOAD => {
            if a[0] != 1 {
                return fail(ew::NUMBER);
            }
            let loads: Vec<Option<f64>> = model
                .axes
                .iter()
                .map(|x| Some(x.servo_load.round()))
                .collect();
            ok(encode_scaled_list(&loads, SERVO_SLOTS, 0))
        }
        func::MACRO_READ => {
            let (first, last) = (a[0], a[1]);
            if first < 1 || last < first {
                return fail(ew::NUMBER);
            }
            if last - first >= 1000 {
                return fail(ew::LENGTH);
            }
            let values: Vec<Option<f64>> = (first..=last)
                .map(|n| model.macros.get(&(n as u32)).copied())
                .collect();
            ok(encode_macro_values(&values))
        }
        func::MACRO_WRITE_DOUBLE => {
            if a[0] < 1 {
                return fail(ew::NUMBER);
            }
            if a[4] != 8 || req.extra.len() < 8 {
                return fail(ew::LENGTH);
            }
            let value = f64::from_be_bytes(req.extra[..8].try_into().expect("8 bytes"));
            if !value.is_finite() {
                return fail(ew::DATA);
            }
            model.set_macro(a[0] as u32, value);
            ok(Vec::new())
        }
        func::PARAM => answer_numbered(model, NumberedKind::Param, req),
        func::DIAG => answer_numbered(model, NumberedKind::Diag, req),
        func::ALARM_MESSAGE => {
            let (alarm_type, max, with_text, text_len) = (a[0], a[1], a[2], a[3]);
            if max < 1 || !(1..=256).contains(&text_len) {
                return fail(ew::LENGTH);
            }
            if alarm_type != -1 && matches!(AlarmType::from_code(alarm_type), AlarmType::Unknown(_))
            {
                return fail(ew::ATTRIB);
            }
            let query = AlarmQuery {
                alarm_type,
                max_messages: max.clamp(1, 1000) as u16,
                with_text: with_text > 0,
                text_length: text_len as u16,
            };
            let msgs: Vec<AlarmMessage> = model
                .alarms
                .iter()
                .filter(|al| alarm_type == -1 || al.alarm_type.code() == alarm_type)
                .take(usize::from(query.max_messages))
                .map(|al| AlarmMessage {
                    number: al.number,
                    alarm_type: al.alarm_type,
                    axis: al.axis,
                    text: Some(al.message.clone()),
                })
                .collect();
            ok(encode_alarm_messages(&query, &msgs))
        }
        func::EXEC_BLOCK => {
            if !(1..=1024).contains(&a[0]) {
                return fail(ew::LENGTH);
            }
            let text: String = model
                .current_block_text()
                .chars()
                .take(a[0] as usize)
                .collect();
            ok(encode_exec_block(&ExecBlock {
                block: model.block_index as i32,
                text,
            }))
        }
        func::PROGRAM_LIST => {
            let (start, count) = (a[0], a[1]);
            if start < 0 || count < 1 {
                return fail(ew::NUMBER);
            }
            let entries: Vec<ProgramEntry> = model
                .programs
                .range(start as u32..)
                .take(count.min(PROGRAM_LIST_BATCH) as usize)
                .map(|(n, p)| ProgramEntry {
                    number: *n,
                    size: p.size(),
                    comment: p.comment.clone(),
                })
                .collect();
            ok(encode_program_list(&entries))
        }
        func::PROGRAM_NAME => ok(encode_program_name(&format!(
            "//CNC_MEM/USER/PATH1/O{:04}",
            model.main_program
        ))),
        _ => fail(ew::FUNC),
    }
}

fn answer_numbered(model: &CncModel, kind: NumberedKind, req: &SubRequest) -> SubResponse {
    let cmd = req.command;
    let fail = |code: i16| SubResponse::failed(cmd, code);
    let (first, last, axis) = (req.args[0], req.args[1], req.args[2]);
    if first < 0 || last < first {
        return fail(ew::NUMBER);
    }
    let table = match kind {
        NumberedKind::Param => &model.params,
        NumberedKind::Diag => &model.diags,
    };
    let max_axis = usize::from(model.sysinfo.max_axis);
    let stride = max_axis * 4 + 8;
    // keep the answer within one TCP segment-sized frame, like the samples
    let max_entries = (1400 / stride).max(1);
    if (last - first) as usize >= max_entries {
        return fail(ew::LENGTH);
    }
    let mut entries = Vec::new();
    for number in first..=last {
        let Some(item) = table.get(&(number as u32)) else {
            return fail(ew::NUMBER);
        };
        let values = match axis {
            -1 => item.values.clone(),
            0 => item.values.iter().take(1).copied().collect(),
            k if (k as usize) <= item.values.len() && k > 0 => vec![item.values[k as usize - 1]],
            _ => return fail(ew::NUMBER),
        };
        entries.push(NumberedValues {
            number: number as u32,
            axis: axis as i16,
            data_type: item.data_type,
            values,
        });
    }
    SubResponse::ok(cmd, encode_numbered(&entries, kind, max_axis))
}

fn answer_pmc(model: &CncModel, req: &SubRequest) -> SubResponse {
    let cmd = req.command;
    let fail = |code: i16| SubResponse::failed(cmd, code);
    let (first, last, area, data_type) = (req.args[0], req.args[1], req.args[2], req.args[3]);
    let Some(area) = PmcArea::from_code(area) else {
        return fail(ew::NUMBER);
    };
    let Some(data_type) = PmcDataType::from_code(data_type) else {
        return fail(ew::ATTRIB);
    };
    if first < 0 || last < first {
        return fail(ew::NUMBER);
    }
    let (first, last) = (first as u32, last as u32);
    if last >= pmc_area_size(area) {
        return fail(ew::NUMBER);
    }
    let size = data_type.size() as u32;
    let count = (last - first + 1) / size;
    if count == 0 || count * size > 1024 {
        return fail(ew::LENGTH);
    }
    let values: Vec<u32> = (0..count)
        .map(|i| {
            (0..size).fold(0u32, |acc, b| {
                (acc << 8) | u32::from(model.pmc_byte(area, first + i * size + b))
            })
        })
        .collect();
    SubResponse::ok(cmd, encode_pmc(data_type, &values))
}
