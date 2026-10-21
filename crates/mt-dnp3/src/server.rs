//! DNP3 outstation simulator: serves a configurable set of binary, analog and counter points to
//! a master over TCP, speaking the same wire format a field outstation does.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::application::{
    AppControl, FunctionCode, Iin, build_response, build_response_fragment, dnp3_time_bytes,
    split_request,
};
use crate::control::{
    ControlObject, ControlRequest, ControlStatus, build_control_response, parse_control_objects,
};
use crate::datalink::{ControlByte, Dnp3Frame};
use crate::objects::parse_object_blocks;
use crate::transport::{TransportHeader, TransportReassembler, TransportSegment, segment_asdu};
use crate::types::{OutstationConfig, OutstationEvent, OutstationLogEntry};

/// How long the simulated outstation keeps a SELECT valid before the OPERATE.
const DEFAULT_SELECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest application fragment the outstation sends (header included). Responses with more
/// data are split over several fragments, each confirmed by the master before the next goes out.
const MAX_FRAGMENT_SIZE: usize = 2048;

/// How long the outstation waits for the master's CONFIRM of a non-final fragment.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared state backing the outstation server.
pub struct OutstationState {
    /// Analog input values (Group 30).
    pub analog_values: Vec<f64>,
    /// Binary input values (Group 1). true = Online, false = Offline.
    pub binary_values: Vec<bool>,
    /// Counter input values (Group 20).
    pub counter_values: Vec<f64>,
    /// Buffered events with SOE timestamps, ordered by sequence.
    pub events: VecDeque<OutstationEvent>,
    /// Set true once the master sends a time sync (clears need_time).
    pub time_synced: bool,
    /// Event sequence counter (monotonic).
    pub event_seq: u32,
    /// Last analog values (for change detection).
    prev_analog: Vec<f64>,
    /// Last binary values (for change detection).
    prev_binary: Vec<bool>,
    /// Last counter values (for change detection).
    prev_counter: Vec<f64>,
    /// IIN1.7 DEVICE_RESTART: set by a restart, cleared when the master writes it back to 0.
    pub device_restart: bool,
    /// How long a SELECT stays valid for the matching OPERATE.
    pub select_timeout: Duration,
}

impl OutstationState {
    /// Scan the current values against the previous snapshot and buffer any
    /// changed point as an event. Class assignment: binary → 3, counter → 2,
    /// analog → 1.
    pub fn detect_events(&mut self) {
        let now = now_ms();
        let mut changed: Vec<(u8, u8, u16, f64)> = Vec::new();
        for (i, v) in self.analog_values.iter().enumerate() {
            let prev = self.prev_analog.get(i).copied().unwrap_or(*v);
            if (*v - prev).abs() > 1e-9 {
                changed.push((1, 30, i as u16, *v));
            }
        }
        for (i, v) in self.binary_values.iter().enumerate() {
            let prev = self.prev_binary.get(i).copied().unwrap_or(*v);
            if *v != prev {
                changed.push((3, 1, i as u16, if *v { 1.0 } else { 0.0 }));
            }
        }
        for (i, v) in self.counter_values.iter().enumerate() {
            let prev = self.prev_counter.get(i).copied().unwrap_or(*v);
            if (*v - prev).abs() > 1e-9 {
                changed.push((2, 20, i as u16, *v));
            }
        }
        self.prev_analog = self.analog_values.clone();
        self.prev_binary = self.binary_values.clone();
        self.prev_counter = self.counter_values.clone();
        for (class, group, index, value) in changed {
            self.push_event(class, group, index, value, now);
        }
    }

    /// Buffer a single event with a SOE timestamp.
    pub fn push_event(&mut self, class: u8, group: u8, index: u16, value: f64, timestamp: u64) {
        self.event_seq = self.event_seq.wrapping_add(1);
        self.events.push_back(OutstationEvent {
            class,
            group,
            index,
            value,
            timestamp,
            sequence: self.event_seq,
        });
        // Bound the buffer so it can't grow unbounded.
        while self.events.len() > 1024 {
            self.events.pop_front();
        }
    }

    /// Take the buffered events for a given class (1/2/3), clearing them.
    pub fn drain_events(&mut self, class: u8) -> Vec<OutstationEvent> {
        let mut out = Vec::new();
        let mut remaining = VecDeque::new();
        for ev in self.events.drain(..) {
            if ev.class == class {
                out.push(ev);
            } else {
                remaining.push_back(ev);
            }
        }
        self.events = remaining;
        out
    }

    /// True if any events of the given class are buffered.
    pub fn has_events(&self, class: u8) -> bool {
        self.events.iter().any(|e| e.class == class)
    }
}

/// Start the DNP3 outstation TCP server.
pub async fn start_outstation_server(
    config: OutstationConfig,
) -> anyhow::Result<(
    Arc<std::sync::Mutex<OutstationState>>,
    tokio::sync::mpsc::UnboundedReceiver<OutstationLogEntry>,
    tokio::sync::watch::Sender<bool>,
)> {
    let addr = format!("{}:{}", config.bind_addr, config.port);
    let listener = TcpListener::bind(&addr).await?;

    let state = Arc::new(std::sync::Mutex::new(OutstationState {
        analog_values: vec![0.0; config.analog_count as usize],
        binary_values: vec![false; config.binary_count as usize],
        counter_values: vec![0.0; config.counter_count as usize],
        events: VecDeque::new(),
        time_synced: false,
        event_seq: 0,
        prev_analog: vec![0.0; config.analog_count as usize],
        prev_binary: vec![false; config.binary_count as usize],
        prev_counter: vec![0.0; config.counter_count as usize],
        device_restart: false,
        select_timeout: DEFAULT_SELECT_TIMEOUT,
    }));

    let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    let outstation_addr = config.outstation_address;
    let master_addr = config.master_address;
    let state_clone = state.clone();
    let log_tx_clone = log_tx.clone();

    tokio::spawn(async move {
        tracing::info!("DNP3 outstation listening on {addr}");

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer)) => {
                            let _ = log_tx_clone.send(OutstationLogEntry {
                                timestamp: now_ms(),
                                function_code: 0,
                                message: format!("Master connected from {peer}"),
                            });
                            handle_master_connection(
                                stream,
                                state_clone.clone(),
                                log_tx_clone.clone(),
                                outstation_addr,
                                master_addr,
                            ).await;
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {e}");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("Outstation server shutting down");
                        let _ = log_tx_clone.send(OutstationLogEntry {
                            timestamp: now_ms(),
                            function_code: 0,
                            message: "Server stopped".into(),
                        });
                        break;
                    }
                }
            }
        }
    });

    Ok((state, log_rx, shutdown_tx))
}

/// One master connection: link-layer services, transport reassembly and fragment sending.
struct Session {
    stream: TcpStream,
    reassembler: TransportReassembler,
    outstation_addr: u16,
    /// Link address of the master: where every reply is sent. Updated from each frame received.
    peer_addr: u16,
    log_tx: tokio::sync::mpsc::UnboundedSender<OutstationLogEntry>,
}

impl Session {
    fn log(&self, function_code: u8, message: impl Into<String>) {
        let _ = self.log_tx.send(OutstationLogEntry {
            timestamp: now_ms(),
            function_code,
            message: message.into(),
        });
    }

    async fn send_link(&mut self, link_control: ControlByte, data: Vec<u8>) -> std::io::Result<()> {
        let frame = Dnp3Frame {
            link_control,
            destination: self.peer_addr,
            source: self.outstation_addr,
            data,
        };
        self.stream.write_all(&frame.encode()).await
    }

    async fn send_asdu(&mut self, asdu: &[u8]) -> std::io::Result<()> {
        for seg in segment_asdu(asdu) {
            let th = TransportHeader { fir: seg.fir, fin: seg.fin, sequence: seg.sequence };
            let mut data = vec![th.to_byte()];
            data.extend_from_slice(&seg.data);
            self.send_link(ControlByte::outstation_data(), data).await?;
        }
        Ok(())
    }

    /// Read link frames until a request ASDU is complete. Link services from the master are
    /// answered here; frames for another station are ignored (broadcast addresses 0xFFFC-0xFFFF
    /// are accepted).
    async fn next_asdu(&mut self) -> anyhow::Result<Vec<u8>> {
        loop {
            let frame = read_frame(&mut self.stream).await?;
            if frame.destination != self.outstation_addr && frame.destination < 0xFFFC {
                self.log(
                    0,
                    format!(
                        "Ignored frame for station {} (this outstation is {})",
                        frame.destination, self.outstation_addr
                    ),
                );
                continue;
            }
            if !frame.link_control.prm {
                continue; // secondary frame (ACK/NACK): nothing for us
            }
            self.peer_addr = frame.source;
            let reply = match frame.link_control.function {
                ControlByte::RESET_LINK_STATES => {
                    self.log(0, "Link reset received, ACK sent");
                    Some(ControlByte::ACK)
                }
                ControlByte::TEST_LINK_STATES => Some(ControlByte::ACK),
                ControlByte::REQUEST_LINK_STATUS => {
                    self.log(ControlByte::LINK_STATUS, "Link status requested");
                    Some(ControlByte::LINK_STATUS)
                }
                ControlByte::USER_DATA_CONFIRMED => Some(ControlByte::ACK),
                ControlByte::USER_DATA_NO_CONFIRM => None,
                _ => continue,
            };
            if let Some(function) = reply {
                self.send_link(ControlByte::outstation_link(function), vec![]).await?;
            }
            if frame.data.is_empty() {
                continue; // link service request: carries no user data
            }
            let th = TransportHeader::from_byte(frame.data[0]);
            let seg = TransportSegment {
                fir: th.fir,
                fin: th.fin,
                sequence: th.sequence,
                data: frame.data[1..].to_vec(),
            };
            if let Some(asdu) = self.reassembler.process(seg) {
                return Ok(asdu);
            }
        }
    }

    /// Send the fragments of one response. A non-final fragment is followed by a wait for the
    /// master's CONFIRM. If the master sends anything else instead, or nothing, the rest of the
    /// response is dropped; a new request that interrupted it is returned to be handled.
    async fn send_fragments(&mut self, fragments: Vec<Vec<u8>>) -> anyhow::Result<Option<Vec<u8>>> {
        let last = fragments.len().saturating_sub(1);
        for (i, fragment) in fragments.iter().enumerate() {
            self.send_asdu(fragment).await?;
            if i == last {
                break;
            }
            let seq = AppControl::from_byte(fragment[0]).seq;
            let waited = tokio::time::timeout(CONFIRM_TIMEOUT, self.next_asdu()).await;
            match waited {
                Ok(Ok(asdu)) => match split_request(&asdu) {
                    Some((c, f, _)) if f == FunctionCode::Confirm as u8 && c.seq == seq => {}
                    _ => return Ok(Some(asdu)),
                },
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    self.log(0, "Master did not confirm a response fragment; response dropped");
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }
}

async fn handle_master_connection(
    stream: TcpStream,
    state: Arc<std::sync::Mutex<OutstationState>>,
    log_tx: tokio::sync::mpsc::UnboundedSender<OutstationLogEntry>,
    outstation_addr: u16,
    master_addr: u16,
) {
    let mut session = Session {
        stream,
        reassembler: TransportReassembler::new(),
        outstation_addr,
        peer_addr: master_addr,
        log_tx,
    };
    let mut pending_select: Option<PendingSelect> = None;
    let mut interrupted: Option<Vec<u8>> = None;

    loop {
        let asdu = match interrupted.take() {
            Some(asdu) => asdu,
            None => match session.next_asdu().await {
                Ok(asdu) => asdu,
                Err(_) => break, // connection closed or error
            },
        };

        let outcome = {
            let mut st = state.lock().unwrap();
            process_request(&mut st, &mut pending_select, &asdu, Instant::now())
        };
        for (function, message) in outcome.log {
            session.log(function, message);
        }
        match session.send_fragments(outcome.fragments).await {
            Ok(next) => interrupted = next,
            Err(e) => {
                tracing::debug!("Failed to send response: {e}");
                break;
            }
        }
    }

    session.log(0, "Master disconnected");
}

async fn read_frame(stream: &mut TcpStream) -> anyhow::Result<Dnp3Frame> {
    let mut header = [0u8; 3];
    stream.read_exact(&mut header).await?;

    if header[0] != 0x05 || header[1] != 0x64 {
        anyhow::bail!("Invalid DNP3 start bytes");
    }

    let length = header[2] as usize;
    if length < 5 {
        anyhow::bail!("Invalid LENGTH {length}");
    }
    let user_len = length - 5;
    let total_frame_size = 10 + user_len + 2 * user_len.div_ceil(16);
    let mut buf = vec![0u8; total_frame_size];
    buf[0..3].copy_from_slice(&header);
    stream.read_exact(&mut buf[3..total_frame_size]).await?;

    Dnp3Frame::decode(&buf)
}

/// What the outstation does with one request: the response fragments to send (none for a
/// request that gets no response) and log lines as (function code, message).
struct Outcome {
    fragments: Vec<Vec<u8>>,
    log: Vec<(u8, String)>,
}

impl Outcome {
    fn none() -> Self {
        Self { fragments: Vec::new(), log: Vec::new() }
    }
}

/// The IIN the outstation reports right now.
fn current_iin(st: &OutstationState) -> Iin {
    Iin {
        need_time: !st.time_synced,
        device_restart: st.device_restart,
        class_1_events: st.has_events(1),
        class_2_events: st.has_events(2),
        class_3_events: st.has_events(3),
        ..Iin::default()
    }
}

/// A null response (no objects) carrying the current IIN plus `extra`.
fn null_response(st: &OutstationState, seq: u8, extra: impl FnOnce(&mut Iin)) -> Vec<Vec<u8>> {
    let mut iin = current_iin(st);
    extra(&mut iin);
    vec![build_response(seq, &iin, &[])]
}

/// Handle one request fragment. Pure apart from `st` and `pending`, so it can be tested without
/// a socket.
fn process_request(
    st: &mut OutstationState,
    pending: &mut Option<PendingSelect>,
    asdu: &[u8],
    now: Instant,
) -> Outcome {
    let Some((control, func, objects)) = split_request(asdu) else {
        return Outcome::none();
    };
    let seq = control.seq;
    let mut log = vec![(func, format!("Received function 0x{func:02X} (seq {seq})"))];

    let fragments = match func {
        0x00 => {
            // CONFIRM of a response fragment: nothing to send back.
            return Outcome { fragments: Vec::new(), log: Vec::new() };
        }
        0x01 => handle_read(st, seq, objects, &mut log),
        0x02 => handle_write(st, seq, objects, &mut log),
        0x03..=0x05 => {
            let function = FunctionCode::from_u8(func).unwrap_or(FunctionCode::DirectOperate);
            if func != 0x03 {
                st.detect_events();
            }
            let reply = handle_control(st, pending, function, objects, seq, now);
            if let Ok(Some(echo)) = parse_control_objects(&reply[4..]) {
                log.push((func, format!("Control 0x{func:02X} -> {}", echo.status)));
            } else {
                log.push((func, format!("Control 0x{func:02X} -> no usable object")));
            }
            vec![reply]
        }
        0x06 => {
            // DIRECT OPERATE, NO ACK: act on it, answer nothing.
            st.detect_events();
            let _ = handle_control(st, pending, FunctionCode::DirectOperate, objects, seq, now);
            Vec::new()
        }
        0x07 | 0x09 => {
            log.push((func, "Counter freeze received".into()));
            null_response(st, seq, |_| {})
        }
        0x08 | 0x0A => {
            log.push((func, "Counter freeze received".into()));
            Vec::new()
        }
        0x0D | 0x0E => {
            let cold = func == 0x0D;
            st.device_restart = true;
            st.events.clear();
            st.event_seq = 0;
            if cold {
                st.time_synced = false;
            }
            log.push((func, format!("{} restart received", if cold { "Cold" } else { "Warm" })));
            vec![build_restart_response(st, seq, cold)]
        }
        0x14 | 0x15 => null_response(st, seq, |_| {}),
        _ => {
            log.push((func, format!("Function 0x{func:02X} not supported")));
            null_response(st, seq, |iin| iin.no_func_code_support = true)
        }
    };
    Outcome { fragments, log }
}

/// A SELECT the outstation accepted and is waiting to see operated.
struct PendingSelect {
    sequence: u8,
    request: ControlRequest,
    at: Instant,
}

/// Handle SELECT (0x03), OPERATE (0x04) and DIRECT OPERATE (0x05) for one control object, given
/// the object bytes of the request. Returns the complete response.
///
/// The response echoes the request object with a control status. An OPERATE succeeds only when it
/// repeats the pending SELECT exactly, carries the next application sequence number and arrives
/// within `state.select_timeout`; otherwise the status is NO_SELECT or TIMEOUT. Every OPERATE
/// consumes the pending select.
fn handle_control(
    st: &mut OutstationState,
    pending: &mut Option<PendingSelect>,
    function: FunctionCode,
    objects: &[u8],
    request_seq: u8,
    now: Instant,
) -> Vec<u8> {
    let request = match parse_control_objects(objects) {
        Ok(Some(echo)) => echo.request,
        // Nothing usable to echo: refuse in the IIN, which a master does not take as confirmation.
        _ => {
            let mut iin = current_iin(st);
            iin.parameter_error = true;
            return build_response(request_seq, &iin, &[]);
        }
    };

    let in_range = match request.object {
        ControlObject::Crob(c) => {
            (request.index as usize) < st.binary_values.len() && matches!(c.code & 0x0F, 1..=4)
        }
        ControlObject::AnalogOutput(_) => (request.index as usize) < st.analog_values.len(),
    };

    let status = match function {
        FunctionCode::Select => {
            *pending = None;
            if in_range {
                *pending = Some(PendingSelect { sequence: request_seq, request, at: now });
                ControlStatus::Success
            } else {
                ControlStatus::NotSupported
            }
        }
        FunctionCode::Operate => match pending.take() {
            None => ControlStatus::NoSelect,
            Some(sel) => {
                if sel.request != request || (sel.sequence + 1) & 0x0F != request_seq {
                    ControlStatus::NoSelect
                } else if now.duration_since(sel.at) > st.select_timeout {
                    ControlStatus::Timeout
                } else if !in_range {
                    ControlStatus::NotSupported
                } else {
                    apply_control(st, &request);
                    ControlStatus::Success
                }
            }
        },
        _ => {
            if in_range {
                apply_control(st, &request);
                ControlStatus::Success
            } else {
                ControlStatus::NotSupported
            }
        }
    };
    build_control_response(request_seq, &current_iin(st), &request, status)
}

/// Apply a validated control object to the simulated point and buffer the resulting event.
fn apply_control(st: &mut OutstationState, request: &ControlRequest) {
    let index = request.index as usize;
    let now = now_ms();
    match request.object {
        ControlObject::Crob(c) => {
            let new_val = c.drives_on();
            if st.binary_values[index] != new_val {
                st.binary_values[index] = new_val;
                st.push_event(3, 1, request.index, if new_val { 1.0 } else { 0.0 }, now);
            }
        }
        ControlObject::AnalogOutput(v) => {
            let value = v as f64;
            st.analog_values[index] = value;
            st.push_event(1, 30, request.index, value, now);
        }
    }
}

/// WRITE: time and date (g50v1) marks the outstation as synchronised; writing 0 to bit 7 of the
/// internal indications (g80v1, index 7) clears DEVICE_RESTART.
fn handle_write(
    st: &mut OutstationState,
    seq: u8,
    objects: &[u8],
    log: &mut Vec<(u8, String)>,
) -> Vec<Vec<u8>> {
    let parsed = parse_object_blocks(objects, true);
    let mut unknown = false;
    for block in &parsed.blocks {
        match (block.group, block.variation) {
            (50, 1) => {
                st.time_synced = true;
                log.push((0x02, "Time sync received (Group 50)".into()));
            }
            (80, 1) => {
                // Packed bits starting at block.start: bit i of the body is index start + i.
                for i in 0..block.count as usize {
                    let index = block.start as usize + i;
                    let value = block.body[i / 8] >> (i % 8) & 1;
                    if index == 7 && value == 0 {
                        st.device_restart = false;
                        log.push((0x02, "Device restart IIN cleared".into()));
                    }
                }
            }
            _ => unknown = true,
        }
    }
    let bad_request = parsed.issue.is_some();
    null_response(st, seq, |iin| {
        iin.object_unknown = unknown;
        iin.parameter_error = bad_request;
    })
}

/// One object block queued for a response, and whether it carries events.
struct Block {
    bytes: Vec<u8>,
    events: bool,
}

/// READ: class 0 (g60v1) serves every static point, classes 1-3 (g60v2-4) drain that class's
/// events, and g1, g20 and g30 serve their static points. Any other object is IIN2.1.
fn handle_read(
    st: &mut OutstationState,
    seq: u8,
    objects: &[u8],
    log: &mut Vec<(u8, String)>,
) -> Vec<Vec<u8>> {
    let parsed = parse_object_blocks(objects, false);
    let (mut binary, mut counter, mut analog) = (false, false, false);
    let mut classes = [false; 3];
    let mut unknown = false;
    for block in &parsed.blocks {
        match (block.group, block.variation) {
            (60, 1) => (binary, counter, analog) = (true, true, true),
            (60, v @ 2..=4) => classes[v as usize - 2] = true,
            (1, _) => binary = true,
            (20, _) => counter = true,
            (30, _) => analog = true,
            _ => unknown = true,
        }
    }
    let bad_request = parsed.issue.is_some();

    let mut blocks = Vec::new();
    for (i, wanted) in classes.iter().enumerate() {
        if *wanted {
            let events = st.drain_events(i as u8 + 1);
            log.push((0x01, format!("Class {} event scan -> {} events", i + 1, events.len())));
            blocks.extend(event_blocks(&events));
        }
    }
    if binary {
        blocks.extend(static_binary_blocks(&st.binary_values));
    }
    if analog {
        blocks.extend(static_analog_blocks(&st.analog_values));
    }
    if counter {
        blocks.extend(static_counter_blocks(&st.counter_values));
    }

    let mut iin = current_iin(st);
    iin.object_unknown = unknown;
    iin.parameter_error = bad_request;
    pack_fragments(seq, &iin, blocks)
}

/// Pack object blocks into response fragments of at most [`MAX_FRAGMENT_SIZE`] octets. Every
/// fragment but the last asks for a CONFIRM, and so does one that carries events, which the
/// master must acknowledge before the outstation may discard them. Sequence numbers count up
/// from the request's.
fn pack_fragments(seq: u8, iin: &Iin, blocks: Vec<Block>) -> Vec<Vec<u8>> {
    let mut groups: Vec<(Vec<u8>, bool)> = vec![(Vec::new(), false)];
    for block in blocks {
        let current = groups.last_mut().expect("at least one group");
        if !current.0.is_empty() && 4 + current.0.len() + block.bytes.len() > MAX_FRAGMENT_SIZE {
            groups.push((Vec::new(), false));
        }
        let current = groups.last_mut().expect("at least one group");
        current.0.extend_from_slice(&block.bytes);
        current.1 |= block.events;
    }
    let total = groups.len();
    groups
        .into_iter()
        .enumerate()
        .map(|(i, (objects, has_events))| {
            let fin = i + 1 == total;
            let control = AppControl {
                fir: i == 0,
                fin,
                con: !fin || has_events,
                uns: false,
                seq: (seq + i as u8) & 0x0F,
            };
            build_response_fragment(control, iin, &objects)
        })
        .collect()
}

/// Static binary inputs as g1v2 (flags: bit 0 ONLINE, bit 7 STATE), qualifier 0x01 (2-octet
/// start-stop), in blocks of up to 200 points.
fn static_binary_blocks(values: &[bool]) -> Vec<Block> {
    ranged_blocks(values.len(), 200, |start, stop| {
        let mut b = vec![1, 2, 0x01];
        b.extend_from_slice(&(start as u16).to_le_bytes());
        b.extend_from_slice(&(stop as u16).to_le_bytes());
        for &v in &values[start..=stop] {
            b.push(0x01 | if v { 0x80 } else { 0 });
        }
        b
    })
}

/// Static analog inputs as g30v5 (flags + float32), qualifier 0x01, up to 100 points per block.
fn static_analog_blocks(values: &[f64]) -> Vec<Block> {
    ranged_blocks(values.len(), 100, |start, stop| {
        let mut b = vec![30, 5, 0x01];
        b.extend_from_slice(&(start as u16).to_le_bytes());
        b.extend_from_slice(&(stop as u16).to_le_bytes());
        for &v in &values[start..=stop] {
            b.push(0x01);
            b.extend_from_slice(&(v as f32).to_le_bytes());
        }
        b
    })
}

/// Static counters as g20v1 (flags + uint32), qualifier 0x01, up to 100 points per block.
fn static_counter_blocks(values: &[f64]) -> Vec<Block> {
    ranged_blocks(values.len(), 100, |start, stop| {
        let mut b = vec![20, 1, 0x01];
        b.extend_from_slice(&(start as u16).to_le_bytes());
        b.extend_from_slice(&(stop as u16).to_le_bytes());
        for &v in &values[start..=stop] {
            b.push(0x01);
            b.extend_from_slice(&(v as u32).to_le_bytes());
        }
        b
    })
}

fn ranged_blocks(len: usize, chunk: usize, mut encode: impl FnMut(usize, usize) -> Vec<u8>) -> Vec<Block> {
    (0..len)
        .step_by(chunk)
        .map(|start| Block { bytes: encode(start, (start + chunk).min(len) - 1), events: false })
        .collect()
}

/// Buffered events as g2v2 (binary, flags + absolute time), g32v7 (analog, float32 with flags
/// and time) and g22v5 (counter, uint32 with flags and time), qualifier 0x28 (2-octet count,
/// 2-octet index before each object).
fn event_blocks(events: &[OutstationEvent]) -> Vec<Block> {
    let mut out = Vec::new();
    // (event point group, object group, object variation, octets per event, events per block)
    for (group, object_group, object_variation, size, chunk) in
        [(1u8, 2u8, 2u8, 9usize, 100usize), (30, 32, 7, 13, 60), (20, 22, 5, 13, 60)]
    {
        let of_group: Vec<&OutstationEvent> = events.iter().filter(|e| e.group == group).collect();
        for part in of_group.chunks(chunk) {
            let mut b = Vec::with_capacity(5 + part.len() * size);
            b.extend_from_slice(&[object_group, object_variation, 0x28]);
            b.extend_from_slice(&(part.len() as u16).to_le_bytes());
            for ev in part {
                b.extend_from_slice(&ev.index.to_le_bytes());
                match group {
                    1 => b.push(0x01 | if ev.value >= 0.5 { 0x80 } else { 0 }),
                    30 => {
                        b.push(0x01);
                        b.extend_from_slice(&(ev.value as f32).to_le_bytes());
                    }
                    _ => {
                        b.push(0x01);
                        b.extend_from_slice(&(ev.value as u32).to_le_bytes());
                    }
                }
                b.extend_from_slice(&dnp3_time_bytes(ev.timestamp));
            }
            out.push(Block { bytes: b, events: true });
        }
    }
    out
}

/// Cold/warm restart response: the time until the outstation is back, as g52v1 (coarse, whole
/// seconds) for a cold restart and g52v2 (fine, milliseconds) for a warm one, each a 2-octet
/// value in a count-1 block. DEVICE_RESTART is set.
fn build_restart_response(st: &OutstationState, seq: u8, cold: bool) -> Vec<u8> {
    let (variation, delay): (u8, u16) = if cold { (1, 10) } else { (2, 1000) };
    let mut objects = vec![52, variation, 0x07, 0x01];
    objects.extend_from_slice(&delay.to_le_bytes());
    build_response(seq, &current_iin(st), &objects)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{AsduHeader, build_request, build_time_sync_request_at, build_integrity_poll_request, build_event_scan_request};
    use crate::control::{Crob, build_control_request, crob_code};
    use crate::objects::parse_points;

    fn state_with(analog: usize, binary: usize, counter: usize) -> OutstationState {
        OutstationState {
            analog_values: vec![0.0; analog],
            binary_values: vec![false; binary],
            counter_values: vec![0.0; counter],
            events: VecDeque::new(),
            time_synced: false,
            event_seq: 0,
            prev_analog: vec![0.0; analog],
            prev_binary: vec![false; binary],
            prev_counter: vec![0.0; counter],
            device_restart: false,
            select_timeout: Duration::from_millis(100),
        }
    }

    fn state() -> OutstationState {
        state_with(4, 4, 2)
    }

    fn latch(index: u16, on: bool) -> ControlRequest {
        ControlRequest {
            index,
            object: ControlObject::Crob(Crob {
                code: if on { crob_code::LATCH_ON } else { crob_code::LATCH_OFF },
                count: 1,
                on_time_ms: 0,
                off_time_ms: 0,
            }),
        }
    }

    /// Run one control request and return the status the outstation answered with.
    fn run(
        st: &mut OutstationState,
        pending: &mut Option<PendingSelect>,
        function: FunctionCode,
        seq: u8,
        req: &ControlRequest,
        now: Instant,
    ) -> ControlStatus {
        let asdu = build_control_request(function, seq, req).unwrap();
        let reply = handle_control(st, pending, function, &asdu[2..], seq, now);
        parse_control_objects(&reply[4..]).unwrap().unwrap().status
    }

    #[test]
    fn select_then_operate_applies_the_command() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        let req = latch(1, true);
        assert_eq!(run(&mut st, &mut pending, FunctionCode::Select, 4, &req, t0), ControlStatus::Success);
        assert!(!st.binary_values[1], "SELECT must not change the point");
        assert_eq!(run(&mut st, &mut pending, FunctionCode::Operate, 5, &req, t0), ControlStatus::Success);
        assert!(st.binary_values[1]);
        assert!(pending.is_none());
    }

    #[test]
    fn operate_without_select_is_refused() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        let status = run(&mut st, &mut pending, FunctionCode::Operate, 1, &latch(0, true), t0);
        assert_eq!(status, ControlStatus::NoSelect);
        assert!(!st.binary_values[0]);
    }

    #[test]
    fn operate_that_differs_from_the_select_is_refused() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        run(&mut st, &mut pending, FunctionCode::Select, 0, &latch(0, true), t0);
        let status = run(&mut st, &mut pending, FunctionCode::Operate, 1, &latch(0, false), t0);
        assert_eq!(status, ControlStatus::NoSelect);
        assert!(pending.is_none(), "a failed OPERATE still consumes the select");
    }

    #[test]
    fn operate_with_wrong_sequence_is_refused() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        let req = latch(0, true);
        run(&mut st, &mut pending, FunctionCode::Select, 3, &req, t0);
        assert_eq!(run(&mut st, &mut pending, FunctionCode::Operate, 5, &req, t0), ControlStatus::NoSelect);
        assert!(!st.binary_values[0]);
    }

    #[test]
    fn operate_after_select_timeout_reports_timeout() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        let req = ControlRequest { index: 2, object: ControlObject::AnalogOutput(12.5) };
        run(&mut st, &mut pending, FunctionCode::Select, 15, &req, t0);
        let late = t0 + Duration::from_millis(500);
        assert_eq!(run(&mut st, &mut pending, FunctionCode::Operate, 0, &req, late), ControlStatus::Timeout);
        assert_eq!(st.analog_values[2], 0.0);
    }

    #[test]
    fn operate_within_timeout_sets_analog_and_buffers_event() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        let req = ControlRequest { index: 2, object: ControlObject::AnalogOutput(12.5) };
        run(&mut st, &mut pending, FunctionCode::Select, 15, &req, t0);
        let status = run(&mut st, &mut pending, FunctionCode::Operate, 0, &req, t0 + Duration::from_millis(50));
        assert_eq!(status, ControlStatus::Success);
        assert_eq!(st.analog_values[2], 12.5);
        assert!(st.events.iter().any(|e| e.group == 30 && e.index == 2));
    }

    #[test]
    fn out_of_range_point_is_not_selected() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        assert_eq!(run(&mut st, &mut pending, FunctionCode::Select, 0, &latch(9, true), t0), ControlStatus::NotSupported);
        assert!(pending.is_none());
        assert_eq!(run(&mut st, &mut pending, FunctionCode::DirectOperate, 1, &latch(9, true), t0), ControlStatus::NotSupported);
    }

    #[test]
    fn direct_operate_latch_pulse_and_trip_close_codes() {
        let (mut st, mut pending, t0) = (state(), None, Instant::now());
        assert_eq!(run(&mut st, &mut pending, FunctionCode::DirectOperate, 0, &latch(0, true), t0), ControlStatus::Success);
        assert!(st.binary_values[0]);
        assert_eq!(run(&mut st, &mut pending, FunctionCode::DirectOperate, 1, &latch(0, false), t0), ControlStatus::Success);
        assert!(!st.binary_values[0]);
        let pulse_on = ControlRequest { index: 0, object: ControlObject::Crob(Crob::pulse(true)) };
        run(&mut st, &mut pending, FunctionCode::DirectOperate, 2, &pulse_on, t0);
        assert!(st.binary_values[0]);
        let trip = Crob { code: crob_code::TRIP | crob_code::PULSE_ON, ..Crob::pulse(true) };
        run(&mut st, &mut pending, FunctionCode::DirectOperate, 3, &ControlRequest { index: 0, object: ControlObject::Crob(trip) }, t0);
        assert!(!st.binary_values[0], "TRIP drives the point off");
        let close = Crob { code: crob_code::CLOSE | crob_code::PULSE_ON, ..Crob::pulse(true) };
        run(&mut st, &mut pending, FunctionCode::DirectOperate, 4, &ControlRequest { index: 0, object: ControlObject::Crob(close) }, t0);
        assert!(st.binary_values[0]);
    }

    // ── Reads, writes and fragmentation ──

    fn process(st: &mut OutstationState, asdu: &[u8]) -> Outcome {
        process_request(st, &mut None, asdu, Instant::now())
    }

    fn decode(fragment: &[u8]) -> (AsduHeader, &[u8]) {
        AsduHeader::decode_response(fragment).unwrap()
    }

    #[test]
    fn integrity_poll_serves_static_data_in_standard_objects() {
        let mut st = state();
        st.binary_values[1] = true;
        st.analog_values[2] = 220.5;
        st.counter_values[0] = 42.0;
        let out = process(&mut st, &build_integrity_poll_request(5));
        assert_eq!(out.fragments.len(), 1);
        let (header, objects) = decode(&out.fragments[0]);
        assert_eq!(header.function, FunctionCode::Response);
        assert_eq!(header.sequence(), 5);
        assert!(header.control.fir && header.control.fin && !header.control.con);
        assert!(header.iin.need_time, "not yet time synchronised");
        let (points, issue) = parse_points(objects);
        assert_eq!(issue, None);
        let find = |g: u8, i: u16| points.iter().find(|p| p.group == g && p.index == i).unwrap().value;
        assert_eq!(find(1, 1), 1.0);
        assert_eq!(find(1, 0), 0.0);
        assert_eq!(find(30, 2), 220.5);
        assert_eq!(find(20, 0), 42.0);
        // Flags octets use bit 0 ONLINE.
        assert!(objects.windows(3).any(|w| w == [1, 2, 0x01]));
    }

    #[test]
    fn class_scan_drains_only_that_class_and_marks_confirm() {
        let mut st = state();
        st.push_event(1, 30, 1, 5.5, 1_700_000_000_000);
        st.push_event(3, 1, 2, 1.0, 1_700_000_000_500);
        let out = process(&mut st, &build_event_scan_request(2, 1));
        let (header, objects) = decode(&out.fragments[0]);
        assert!(header.control.con, "a fragment carrying events asks for a CONFIRM");
        assert!(header.iin.class_3_events && !header.iin.class_1_events);
        let (points, _) = parse_points(objects);
        assert_eq!(points.len(), 1);
        assert!(points[0].event && points[0].group == 30 && points[0].index == 1 && points[0].value == 5.5);
        assert!(st.has_events(3) && !st.has_events(1));
        // g32v7 with qualifier 0x28, 2-octet count and index, and a 48-bit time.
        assert_eq!(objects[..5], [32, 7, 0x28, 1, 0]);
        assert_eq!(objects.len(), 5 + 2 + 1 + 4 + 6);
        assert_eq!(objects[5 + 7..], dnp3_time_bytes(1_700_000_000_000));
    }

    #[test]
    fn large_response_is_split_into_confirmed_fragments() {
        let mut st = state_with(1500, 0, 0);
        let out = process(&mut st, &build_integrity_poll_request(14));
        assert!(out.fragments.len() >= 3, "{} fragments", out.fragments.len());
        let mut seen = 0;
        for (i, fragment) in out.fragments.iter().enumerate() {
            assert!(fragment.len() <= MAX_FRAGMENT_SIZE);
            let (header, objects) = decode(fragment);
            let last = i + 1 == out.fragments.len();
            assert_eq!(header.control.fir, i == 0);
            assert_eq!(header.control.fin, last);
            assert_eq!(header.control.con, !last);
            assert_eq!(header.sequence(), (14 + i as u8) & 0x0F);
            seen += parse_points(objects).0.len();
        }
        assert_eq!(seen, 1500);
    }

    #[test]
    fn unsupported_read_object_sets_object_unknown() {
        let mut st = state();
        let out = process(&mut st, &crate::application::build_read_request(0, &[(110, 0)]));
        let (header, objects) = decode(&out.fragments[0]);
        assert!(header.iin.object_unknown);
        assert!(objects.is_empty());
    }

    #[test]
    fn malformed_read_sets_parameter_error() {
        let mut st = state();
        let out = process(&mut st, &build_request(FunctionCode::Read, 0, &[60, 1, 0x57, 1]));
        assert!(decode(&out.fragments[0]).0.iin.parameter_error);
    }

    #[test]
    fn unknown_function_gets_iin2_not_supported() {
        let mut st = state();
        let out = process(&mut st, &[0xC3, 0x3F]);
        let (header, _) = decode(&out.fragments[0]);
        assert!(header.iin.no_func_code_support);
        assert_eq!(header.sequence(), 3);
    }

    #[test]
    fn confirm_gets_no_reply() {
        let mut st = state();
        assert!(process(&mut st, &[0xC3, 0x00]).fragments.is_empty());
    }

    #[test]
    fn time_write_clears_need_time() {
        let mut st = state();
        assert!(current_iin(&st).need_time);
        let out = process(&mut st, &build_time_sync_request_at(1, 1_700_000_000_000));
        assert!(st.time_synced);
        let (header, objects) = decode(&out.fragments[0]);
        assert!(!header.iin.need_time && objects.is_empty());
    }

    #[test]
    fn restart_iin_is_set_by_restart_and_cleared_by_writing_it_back() {
        let mut st = state();
        let out = process(&mut st, &build_request(FunctionCode::ColdRestart, 0, &[]));
        let (header, objects) = decode(&out.fragments[0]);
        assert!(header.iin.device_restart);
        // g52v1 coarse delay, 10 s.
        assert_eq!(objects, [52, 1, 0x07, 1, 10, 0]);
        // IIN1.7 = 0, written as in the DNP3 Quick Reference: g80v1, qualifier 0, index 7..7.
        let clear = build_request(FunctionCode::Write, 1, &[80, 1, 0x00, 7, 7, 0x00]);
        let out = process(&mut st, &clear);
        assert!(!st.device_restart);
        assert!(!decode(&out.fragments[0]).0.iin.device_restart);
    }

    #[test]
    fn warm_restart_reports_fine_delay() {
        let mut st = state();
        let out = process(&mut st, &build_request(FunctionCode::WarmRestart, 0, &[]));
        assert_eq!(decode(&out.fragments[0]).1, [52, 2, 0x07, 1, 0xE8, 0x03]);
    }

    #[test]
    fn freeze_and_unsolicited_control_get_null_responses() {
        let mut st = state();
        for f in [FunctionCode::Freeze, FunctionCode::EnableUnsolicited, FunctionCode::DisableUnsolicited] {
            let out = process(&mut st, &build_request(f, 2, &[]));
            assert_eq!(out.fragments.len(), 1);
            assert!(decode(&out.fragments[0]).1.is_empty());
        }
        assert!(process(&mut st, &build_request(FunctionCode::FreezeNoAck, 2, &[])).fragments.is_empty());
    }

    #[test]
    fn control_without_a_usable_object_is_a_parameter_error() {
        let mut st = state();
        let out = process(&mut st, &build_request(FunctionCode::Select, 0, &[]));
        let (header, objects) = decode(&out.fragments[0]);
        assert!(header.iin.parameter_error && objects.is_empty());
    }

    #[test]
    fn direct_operate_no_ack_acts_silently() {
        let mut st = state();
        let asdu = build_control_request(FunctionCode::DirectOperateNoAck, 0, &latch(2, true)).unwrap();
        assert!(process(&mut st, &asdu).fragments.is_empty());
        assert!(st.binary_values[2]);
    }
}
