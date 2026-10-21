use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::application::{AsduHeader, FunctionCode, Iin, QualifierCode};
use crate::datalink::{ControlByte, Dnp3Frame};
use crate::transport::{TransportHeader, TransportReassembler, TransportSegment, segment_asdu};
use crate::types::{OutstationConfig, OutstationEvent, OutstationLogEntry};

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
    /// Whether a cold/warm restart was issued since last integrity poll.
    pub device_restart: bool,
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

async fn handle_master_connection(
    stream: TcpStream,
    state: Arc<std::sync::Mutex<OutstationState>>,
    log_tx: tokio::sync::mpsc::UnboundedSender<OutstationLogEntry>,
    outstation_addr: u16,
    master_addr: u16,
) {
    let mut stream = stream;
    let mut reassembler = TransportReassembler::new();

    loop {
        match read_frame(&mut stream).await {
            Ok(frame) => {
                if frame.link_control.prm {
                    // Primary (master) frame.
                    if frame.data.is_empty() {
                        // Link service request: carries no user data
                        // (IEEE 1815-2012 §9.1). Answer at the link layer.
                        let reply = match frame.link_control.function {
                            ControlByte::RESET_LINK_STATES => {
                                let _ = log_tx.send(OutstationLogEntry {
                                    timestamp: now_ms(),
                                    function_code: 0,
                                    message: "Link reset received, ACK sent".into(),
                                });
                                Some(ControlByte::ACK)
                            }
                            ControlByte::TEST_LINK_STATES => Some(ControlByte::ACK),
                            ControlByte::REQUEST_LINK_STATUS => {
                                let _ = log_tx.send(OutstationLogEntry {
                                    timestamp: now_ms(),
                                    function_code: ControlByte::LINK_STATUS,
                                    message: "Link status requested".into(),
                                });
                                Some(ControlByte::LINK_STATUS)
                            }
                            _ => None,
                        };
                        if let Some(function) = reply {
                            let resp = Dnp3Frame {
                                link_control: ControlByte::outstation_link(function),
                                destination: master_addr,
                                source: outstation_addr,
                                data: vec![],
                            }
                            .encode();
                            if let Err(e) = stream.write_all(&resp).await {
                                tracing::debug!("Failed to send link response: {e}");
                                break;
                            }
                        }
                        continue;
                    }

                    // Application layer — strip transport header and reassemble.
                    let th = TransportHeader::from_byte(frame.data[0]);
                    let seg = TransportSegment {
                        fir: th.fir,
                        fin: th.fin,
                        sequence: th.sequence,
                        data: frame.data[1..].to_vec(),
                    };
                    let Some(asdu) = reassembler.process(seg) else {
                        continue; // waiting for more segments
                    };

                    if asdu.len() < 4 {
                        continue;
                    }

                    let func = asdu[0];
                    let request_seq = asdu[1] & 0x0F;

                    let _ = log_tx.send(OutstationLogEntry {
                        timestamp: now_ms(),
                        function_code: func,
                        message: format!("Received function 0x{:02X} (seq {})", func, request_seq),
                    });

                    let response_asdu = match func {
                        0x01 => {
                            // READ — detect integrity (G60V1) vs class event scan (G60V2/3/4)
                            let group = asdu.get(4).copied().unwrap_or(60);
                            let variation = asdu.get(5).copied().unwrap_or(1);
                            let mut st = state.lock().unwrap();
                            if group == 60 && variation >= 2 && variation <= 4 {
                                let class = variation - 1;
                                let events = st.drain_events(class);
                                drop(st);
                                let _ = log_tx.send(OutstationLogEntry {
                                    timestamp: now_ms(),
                                    function_code: func,
                                    message: format!("Class {} event scan -> {} events", class, events.len()),
                                });
                                Some(build_event_response(request_seq, &events, class))
                            } else {
                                Some(build_read_response(&st, request_seq))
                            }
                        }
                        0x02 => {
                            // WRITE — time sync (Group 50 Var 1)
                            let group = asdu.get(4).copied().unwrap_or(0);
                            if group == 50 {
                                let mut st = state.lock().unwrap();
                                st.time_synced = true;
                                drop(st);
                                let _ = log_tx.send(OutstationLogEntry {
                                    timestamp: now_ms(),
                                    function_code: func,
                                    message: "Time sync received (Group 50)".into(),
                                });
                                Some(build_ack_response(request_seq))
                            } else {
                                Some(build_ack_response(request_seq))
                            }
                        }
                        0x05 => {
                            // DIRECT_OPERATE (binary CROB or analog output)
                            let mut st = state.lock().unwrap();
                            st.detect_events();
                            drop(st);
                            handle_direct_operate(&state, &asdu, request_seq)
                        }
                        0x07 | 0x08 => {
                            // FREEZE / FREEZE_CLEAR on counters
                            let _ = log_tx.send(OutstationLogEntry {
                                timestamp: now_ms(),
                                function_code: func,
                                message: "Counter freeze received".into(),
                            });
                            Some(build_ack_response(request_seq))
                        }
                        0x0D => {
                            // COLD RESTART
                            let mut st = state.lock().unwrap();
                            st.device_restart = true;
                            st.events.clear();
                            st.time_synced = false;
                            st.event_seq = 0;
                            drop(st);
                            let _ = log_tx.send(OutstationLogEntry {
                                timestamp: now_ms(),
                                function_code: func,
                                message: "Cold restart received".into(),
                            });
                            Some(build_restart_response(request_seq, true))
                        }
                        0x0E => {
                            // WARM RESTART
                            let mut st = state.lock().unwrap();
                            st.device_restart = true;
                            st.events.clear();
                            st.event_seq = 0;
                            drop(st);
                            let _ = log_tx.send(OutstationLogEntry {
                                timestamp: now_ms(),
                                function_code: func,
                                message: "Warm restart received".into(),
                            });
                            Some(build_restart_response(request_seq, false))
                        }
                        0x14 | 0x15 => {
                            // ENABLE/DISABLE UNSOLICITED — ACK
                            Some(build_ack_response(request_seq))
                        }
                        _ => None,
                    };

                    if let Some(asdu) = response_asdu {
                        for seg in segment_asdu(&asdu) {
                            let th = TransportHeader {
                                fir: seg.fir,
                                fin: seg.fin,
                                sequence: seg.sequence,
                            };
                            let mut data = vec![th.to_byte()];
                            data.extend_from_slice(&seg.data);
                            let resp_frame = Dnp3Frame {
                                link_control: ControlByte::outstation_data(),
                                destination: master_addr,
                                source: outstation_addr,
                                data,
                            };
                            let bytes = resp_frame.encode();
                            if let Err(e) = stream.write_all(&bytes).await {
                                tracing::debug!("Failed to send response: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // Connection closed or error
                break;
            }
        }
    }

    let _ = log_tx.send(OutstationLogEntry {
        timestamp: now_ms(),
        function_code: 0,
        message: "Master disconnected".into(),
    });
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

fn build_read_response(state: &OutstationState, request_seq: u8) -> Vec<u8> {
    let mut iin = Iin::default();
    if !state.time_synced {
        iin.need_time = true;
    }
    if state.device_restart {
        iin.device_restart = true;
    }
    iin.class_1_events = state.has_events(1);
    iin.class_2_events = state.has_events(2);
    iin.class_3_events = state.has_events(3);

    let header = AsduHeader {
        function: FunctionCode::Response,
        sequence: request_seq,
        iin,
    };

    let mut asdu = header.encode().to_vec();

    // Binary inputs (Group 1 Variation 2) — 1 byte per point
    if !state.binary_values.is_empty() {
        asdu.push(1); // group
        asdu.push(2); // variation
        asdu.push(QualifierCode::StartStop8 as u8); // qualifier
        asdu.push(0); // start
        asdu.push((state.binary_values.len() - 1) as u8); // stop
        for &val in &state.binary_values {
            asdu.push(if val { 0x01 } else { 0x00 });
        }
    }

    // Analog inputs (Group 30 Variation 4) — 4 bytes per point (float32 LE).
    // NOTE: the client parser (`parse_analog_input_float32`) reads v4 as a bare
    // 4-byte float (no flag byte), so the outstation serves it the same way to
    // stay consistent.
    if !state.analog_values.is_empty() {
        asdu.push(30); // group
        asdu.push(4); // variation
        asdu.push(QualifierCode::StartStop8 as u8); // qualifier
        asdu.push(0); // start
        asdu.push((state.analog_values.len() - 1) as u8); // stop
        for &val in &state.analog_values {
            let bits = (val as f32).to_le_bytes();
            asdu.extend_from_slice(&bits);
        }
    }

    // Counter inputs (Group 20 Variation 1) — 4 bytes per point (uint32 LE).
    if !state.counter_values.is_empty() {
        asdu.push(20); // group
        asdu.push(1); // variation
        asdu.push(QualifierCode::StartStop8 as u8); // qualifier
        asdu.push(0); // start
        asdu.push((state.counter_values.len() - 1) as u8); // stop
        for &val in &state.counter_values {
            asdu.extend_from_slice(&(val as u32).to_le_bytes());
        }
    }

    asdu
}

/// Build a response carrying buffered events for a class event scan.
/// Events are serialized as their base object group (G1/G30/G20) so the
/// master-side parser can read them back.
fn build_event_response(request_seq: u8, events: &[OutstationEvent], class: u8) -> Vec<u8> {
    let iin = Iin::default();
    let mut asdu = AsduHeader {
        function: FunctionCode::Response,
        sequence: request_seq,
        iin,
    }
    .encode()
    .to_vec();

    let _ = class;
    if events.is_empty() {
        return asdu;
    }

    // Group by (group, variation)
    let binary: Vec<&OutstationEvent> = events.iter().filter(|e| e.group == 1).collect();
    let analog: Vec<&OutstationEvent> = events.iter().filter(|e| e.group == 30).collect();
    let counter: Vec<&OutstationEvent> = events.iter().filter(|e| e.group == 20).collect();

    // Binary event (Group 1 Var 2, StartStop with index prefix)
    if !binary.is_empty() {
        asdu.push(1);
        asdu.push(2);
        asdu.push(QualifierCode::CountAndPrefix8 as u8);
        asdu.push(binary.len() as u8);
        for ev in &binary {
            asdu.push(ev.index as u8);
            asdu.push(if ev.value >= 0.5 { 0x01 } else { 0x00 });
        }
    }

    // Analog event (Group 30 Var 4, StartStop with index prefix)
    if !analog.is_empty() {
        asdu.push(30);
        asdu.push(4);
        asdu.push(QualifierCode::CountAndPrefix8 as u8);
        asdu.push(analog.len() as u8);
        for ev in &analog {
            asdu.push(ev.index as u8);
            asdu.extend_from_slice(&(ev.value as f32).to_le_bytes());
        }
    }

    // Counter event (Group 20 Var 1, StartStop with index prefix)
    if !counter.is_empty() {
        asdu.push(20);
        asdu.push(1);
        asdu.push(QualifierCode::CountAndPrefix8 as u8);
        asdu.push(counter.len() as u8);
        for ev in &counter {
            asdu.push(ev.index as u8);
            asdu.extend_from_slice(&(ev.value as u32).to_le_bytes());
        }
    }

    asdu
}

/// Build a cold/warm restart response. The ASDU carries the restart time in
/// milliseconds since epoch (8 bytes LE), matching DNP3 restart semantics.
fn build_restart_response(request_seq: u8, cold: bool) -> Vec<u8> {
    let mut iin = Iin::default();
    iin.device_restart = true;
    let mut asdu = AsduHeader {
        function: FunctionCode::Response,
        sequence: request_seq,
        iin,
    }
    .encode()
    .to_vec();

    asdu.push(if cold { 52 } else { 52 }); // Group 52 — Time Delay / restart
    asdu.push(1);
    asdu.push(QualifierCode::Count8 as u8);
    asdu.push(0x01);
    let delay_ms: u32 = if cold { 10_000 } else { 1_000 };
    asdu.extend_from_slice(&delay_ms.to_le_bytes());
    asdu
}

fn handle_direct_operate(
    state: &Arc<std::sync::Mutex<OutstationState>>,
    data: &[u8],
    request_seq: u8,
) -> Option<Vec<u8>> {
    // Parse Group 12 Var 1 (binary CROB) or Group 41 (analog output)
    if data.len() < 8 {
        return Some(build_ack_response(request_seq));
    }

    let group = data[4];
    let _variation = data[5];
    let _qualifier = data[6];
    let count = data.get(7).copied().unwrap_or(0);

    if count == 0 {
        return Some(build_ack_response(request_seq));
    }

    let mut st = state.lock().unwrap();

    if group == 12 {
        // Binary CROB
        let index = data.get(8).copied().unwrap_or(0) as usize;
        let code = data.get(9).copied().unwrap_or(0);
        if index < st.binary_values.len() {
            let new_val = (code & 0x03) == 0x01; // NUL = trip, bit 0 = close
            if st.binary_values[index] != new_val {
                st.binary_values[index] = new_val;
                let now = now_ms();
                st.push_event(3, 1, index as u16, if new_val { 1.0 } else { 0.0 }, now);
            }
        }
    } else if group == 41 {
        // Analog output (Group 41 Var 3: index prefix + float32 + status byte)
        let index = data.get(8).copied().unwrap_or(0) as usize;
        if index < st.analog_values.len() {
            let value = if data.len() >= 13 {
                let raw = u32::from_le_bytes([data[9], data[10], data[11], data[12]]);
                f32::from_bits(raw) as f64
            } else {
                st.analog_values[index]
            };
            st.analog_values[index] = value;
            let now = now_ms();
            st.push_event(1, 30, index as u16, value, now);
        }
    }

    Some(build_ack_response(request_seq))
}

fn build_ack_response(request_seq: u8) -> Vec<u8> {
    let header = AsduHeader {
        function: FunctionCode::Response,
        sequence: request_seq,
        iin: Iin::default(),
    };
    header.encode().to_vec()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
