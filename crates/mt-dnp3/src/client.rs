use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::application::{
    AsduHeader, FunctionCode, Iin, build_clear_restart_request, build_cold_restart_request,
    build_confirm, build_event_scan_request, build_freeze_request, build_integrity_poll_request,
    build_read_request, build_time_sync_request, build_warm_restart_request,
};
use crate::control::{
    ControlObject, ControlRequest, ControlStatus, Crob, build_control_request, parse_control_objects,
};
use crate::datalink::{ControlByte, Dnp3Frame};
use crate::objects::{Dnp3PointValue, parse_points};
use crate::transport::{TransportHeader, TransportReassembler, TransportSegment, segment_asdu};
use crate::types::{Dnp3Config, Dnp3Point, Dnp3Value};

/// Tracks application layer sequence numbers (0-15, wraps).
pub struct SequenceTracker {
    seq: u8,
}

impl SequenceTracker {
    pub fn new() -> Self {
        Self { seq: 0 }
    }

    pub fn next(&mut self) -> u8 {
        let current = self.seq;
        self.seq = (self.seq + 1) & 0x0F;
        current
    }
}

/// Build a link-layer RESET LINK STATES frame (master -> outstation).
/// Per IEEE 1815-2012 §9.1 the link services RESET LINK STATES (0),
/// TEST LINK STATES (2) and REQUEST LINK STATUS (9) carry no user data.
pub fn build_link_reset_frame(master_addr: u16, outstation_addr: u16) -> Dnp3Frame {
    Dnp3Frame {
        link_control: ControlByte::master_link(ControlByte::RESET_LINK_STATES),
        destination: outstation_addr,
        source: master_addr,
        data: vec![],
    }
}

/// One frame of traffic a client sent or received, for a traffic monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficRecord {
    /// Sent by this master; false for frames from the outstation.
    pub sent: bool,
    /// Link control of the frame (of the last frame of a reassembled fragment).
    pub link: ControlByte,
    /// The application fragment; empty for link-layer service frames (reset, ACK, status).
    pub asdu: Vec<u8>,
    /// Octets on the wire: link headers, transport headers and CRCs included.
    pub octets: usize,
}

/// Where a client reports its [`TrafficRecord`]s.
pub type TrafficTap = tokio::sync::mpsc::UnboundedSender<TrafficRecord>;

/// What an outstation answered to a READ: the combined internal indications and every data
/// point in the response, with flags and event time stamps.
#[derive(Debug, Clone, PartialEq)]
pub struct PollResponse {
    pub iin: Iin,
    /// Points in the order the outstation sent them (events and static values alike).
    pub points: Vec<Dnp3PointValue>,
    /// Why part of a fragment could not be decoded, one entry per fragment that stopped early.
    pub issues: Vec<String>,
}

/// DNP3 master session connected to a single outstation.
///
/// Wraps a raw TCP stream and the DNP3 protocol stack (data-link, transport, application,
/// object parsing). The inner stream is guarded by an async Mutex so that
/// `Dnp3Client` satisfies `Sync` (required by `ProtocolClient`).
pub struct Dnp3Client {
    inner: tokio::sync::Mutex<Dnp3Inner>,
}

struct Dnp3Inner {
    stream: TcpStream,
    master_addr: u16,
    outstation_addr: u16,
    seq: SequenceTracker,
    response_timeout: Duration,
    reassembler: TransportReassembler,
    tap: Option<TrafficTap>,
    /// Size on the wire of the frame `receive_frame` read last.
    last_frame_octets: usize,
    /// Octets of the frames of the fragment being reassembled.
    rx_octets: usize,
    /// Internal indications of the last response.
    last_iin: Iin,
}

const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long connect waits for the outstation to answer RESET LINK STATES. Over TCP many
/// outstations never answer it, so silence is not a failure.
const LINK_RESET_WAIT: Duration = Duration::from_millis(1500);

impl ProtocolClient for Dnp3Client {
    type Config = Dnp3Config;
    type Tag = Dnp3Point;
    type Value = Dnp3Value;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        Self::connect_with_tap(config, None).await
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;

        // Integrity poll: class 1, 2, 3 events then class 0 static data.
        let seq = inner.seq.next();
        let response = inner
            .exchange(build_integrity_poll_request(seq), seq, "integrity poll")
            .await?;
        response.check_iin(false, "integrity poll", ProtocolError::ReadFailed)?;

        let raw = collect_points(response.points());
        Ok(tags
            .iter()
            .map(|tag| Dnp3Value {
                point: tag.clone(),
                value: lookup_polled_value(&raw, tag).unwrap_or(f64::NAN),
            })
            .collect())
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        self.direct_operate(&ControlRequest {
            index: tag.index,
            object: ControlObject::Crob(Crob::pulse(value.value != 0.0)),
        })
        .await
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        // Shut down the TCP stream gracefully.
        inner
            .stream
            .shutdown()
            .await
            .map_err(|e| ProtocolError::Other(format!("Shutdown error: {e}")))?;
        Ok(())
    }
}

/// Polled values keyed by static group and index. A static value always wins over an event for
/// the same point, whatever order the outstation sent them in.
type PointMap = HashMap<(u8, u16), (f64, bool)>;

fn collect_points(points: Vec<Dnp3PointValue>) -> PointMap {
    let mut map = PointMap::new();
    for p in points {
        match map.get(&(p.group, p.index)) {
            Some(&(_, false)) if p.event => {}
            _ => {
                map.insert((p.group, p.index), (p.value, p.event));
            }
        }
    }
    map
}

/// Look up a polled value by group and index, ignoring the requested variation: outstations
/// answer with the variation they are configured for (a point requested as g30v5 may come back
/// as g30v1).
fn lookup_polled_value(raw: &PointMap, point: &Dnp3Point) -> Option<f64> {
    raw.get(&(point.group, point.index)).map(|&(value, _)| value)
}

/// What an outstation sent back for one request: the combined IIN of all fragments and the
/// object bytes of each fragment.
struct Response {
    iin: Iin,
    fragments: Vec<Vec<u8>>,
}

impl Response {
    fn points(&self) -> Vec<Dnp3PointValue> {
        let mut all = Vec::new();
        for fragment in &self.fragments {
            let (points, issue) = parse_points(fragment);
            if let Some(issue) = issue {
                tracing::warn!("Response not fully decoded: {issue}");
            }
            all.extend(points);
        }
        all
    }

    fn poll_response(&self) -> PollResponse {
        let mut points = Vec::new();
        let mut issues = Vec::new();
        for fragment in &self.fragments {
            let (found, issue) = parse_points(fragment);
            if let Some(issue) = issue {
                issues.push(issue.to_string());
            }
            points.extend(found);
        }
        PollResponse { iin: self.iin.clone(), points, issues }
    }

    /// Object bytes of the first fragment (control responses are always one fragment).
    fn objects(&self) -> &[u8] {
        self.fragments.first().map(Vec::as_slice).unwrap_or(&[])
    }

    fn check_iin(
        &self,
        strict_objects: bool,
        what: &str,
        error: fn(String) -> ProtocolError,
    ) -> Result<(), ProtocolError> {
        match self.iin.error(strict_objects) {
            Some(problem) => Err(error(format!("DNP3 {what} refused by the outstation: {problem}"))),
            None => Ok(()),
        }
    }
}

impl Dnp3Client {
    /// Connect like [`ProtocolClient::connect`], reporting every frame sent and received on
    /// `tap` from the link reset on.
    pub async fn connect_with_tap(
        config: Dnp3Config,
        tap: Option<TrafficTap>,
    ) -> Result<Self, ProtocolError> {
        let addr = format!("{}:{}", config.host, config.port);
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address '{addr}': {e}")))?;

        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(socket_addr))
            .await
            .map_err(|_| ProtocolError::Timeout("DNP3 connection timeout (5s)".into()))?
            .map_err(|e| ProtocolError::ConnectionFailed(format!("DNP3 connection failed: {e}")))?;

        let mut inner = Dnp3Inner {
            stream,
            master_addr: config.master_address,
            outstation_addr: config.outstation_address,
            seq: SequenceTracker::new(),
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
            reassembler: TransportReassembler::new(),
            tap,
            last_frame_octets: 0,
            rx_octets: 0,
            last_iin: Iin::default(),
        };

        // RESET LINK STATES. An ACK from the outstation is the normal answer. A frame that does
        // not decode is a failed handshake; silence or a NACK just means the outstation does not
        // use link-level resets, so the session goes on without it.
        inner.send_link_reset().await?;
        match tokio::time::timeout(LINK_RESET_WAIT, inner.receive_frame()).await {
            Ok(Ok(ack)) => {
                let octets = inner.last_frame_octets;
                inner.record(false, ack.link_control, Vec::new(), octets);
                if ack.link_control.prm || ack.link_control.function != ControlByte::ACK {
                    tracing::debug!(
                        "Link reset answered with function {:#04x} (prm={}), continuing",
                        ack.link_control.function,
                        ack.link_control.prm
                    );
                } else if ack.destination != config.master_address
                    || ack.source != config.outstation_address
                {
                    return Err(ProtocolError::ConnectionFailed(format!(
                        "Link ACK addressed to {} from {}, expected {} from {}",
                        ack.destination, ack.source, config.master_address, config.outstation_address
                    )));
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => tracing::debug!("No answer to RESET LINK STATES, continuing"),
        }

        Ok(Self {
            inner: tokio::sync::Mutex::new(inner),
        })
    }

    /// Integrity poll (class 1, 2, 3 events, then class 0 static data), returning every point
    /// with its flags and the outstation's internal indications.
    pub async fn integrity_poll(&self) -> Result<PollResponse, ProtocolError> {
        self.poll(build_integrity_poll_request, "integrity poll").await
    }

    /// Event poll of the chosen classes in one READ (g60v2, g60v3, g60v4).
    pub async fn poll_classes(
        &self,
        class1: bool,
        class2: bool,
        class3: bool,
    ) -> Result<PollResponse, ProtocolError> {
        let objects: Vec<(u8, u8)> = [(class1, 2u8), (class2, 3), (class3, 4)]
            .into_iter()
            .filter_map(|(wanted, variation)| wanted.then_some((60, variation)))
            .collect();
        if objects.is_empty() {
            return Err(ProtocolError::InvalidConfig("Poll at least one event class".into()));
        }
        let names: Vec<String> = [(class1, 1), (class2, 2), (class3, 3)]
            .into_iter()
            .filter_map(|(wanted, class)| wanted.then(|| class.to_string()))
            .collect();
        let what = format!("class {} poll", names.join("/"));
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let response = inner.exchange(build_read_request(seq, &objects), seq, &what).await?;
        response.check_iin(false, &what, ProtocolError::ReadFailed)?;
        Ok(response.poll_response())
    }

    async fn poll(&self, build: fn(u8) -> Vec<u8>, what: &str) -> Result<PollResponse, ProtocolError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let response = inner.exchange(build(seq), seq, what).await?;
        response.check_iin(false, what, ProtocolError::ReadFailed)?;
        Ok(response.poll_response())
    }

    /// Clear IIN1.7 DEVICE_RESTART by writing 0 to it (g80v1 index 7), as a master does once it
    /// has seen the outstation restart.
    pub async fn clear_restart(&self) -> Result<(), ProtocolError> {
        self.simple_command(build_clear_restart_request, "restart IIN clear").await
    }

    /// Internal indications of the outstation's last response.
    pub async fn last_iin(&self) -> Iin {
        self.inner.lock().await.last_iin.clone()
    }

    /// Perform a class-based event scan (Class 1/2/3) and return changed points.
    pub async fn scan_events(
        &self,
        class: u8,
        points: &[Dnp3Point],
    ) -> Result<Vec<Dnp3Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;

        let seq = inner.seq.next();
        let what = format!("class {class} event scan");
        let response = inner
            .exchange(build_event_scan_request(seq, class), seq, &what)
            .await?;
        response.check_iin(false, &what, ProtocolError::ReadFailed)?;

        let raw = collect_points(response.points());
        Ok(points
            .iter()
            .filter_map(|point| {
                lookup_polled_value(&raw, point).map(|value| Dnp3Value { point: point.clone(), value })
            })
            .collect())
    }

    /// Write the current time (Group 50 Var 1) to the outstation.
    pub async fn time_sync(&self) -> Result<(), ProtocolError> {
        self.simple_command(build_time_sync_request, "time sync").await
    }

    /// Send a cold restart command (0x0D) and await the response.
    pub async fn cold_restart(&self) -> Result<(), ProtocolError> {
        self.simple_command(build_cold_restart_request, "cold restart").await
    }

    /// Send a warm restart command (0x0E) and await the response.
    pub async fn warm_restart(&self) -> Result<(), ProtocolError> {
        self.simple_command(build_warm_restart_request, "warm restart").await
    }

    /// Send a FREEZE command to counter inputs and await the response.
    pub async fn freeze_counters(&self) -> Result<(), ProtocolError> {
        self.simple_command(build_freeze_request, "counter freeze").await
    }

    async fn simple_command(
        &self,
        build: fn(u8) -> Vec<u8>,
        what: &str,
    ) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let response = inner.exchange(build(seq), seq, what).await?;
        response.check_iin(true, what, ProtocolError::WriteFailed)
    }

    /// Write an analog output value (Group 41 Var 3 Direct Operate).
    pub async fn write_analog(&self, index: u16, value: f64) -> Result<(), ProtocolError> {
        self.direct_operate(&ControlRequest {
            index,
            object: ControlObject::AnalogOutput(value as f32),
        })
        .await
    }

    /// DIRECT OPERATE (0x05): act on the point in a single request.
    pub async fn direct_operate(&self, request: &ControlRequest) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner
            .control_exchange(FunctionCode::DirectOperate, request)
            .await
    }

    /// SELECT (0x03) followed by OPERATE (0x04) with the same object.
    ///
    /// The connection stays locked for both requests, so no poll can slip between them and the
    /// OPERATE carries the application sequence number right after the SELECT's, as the outstation
    /// requires. If the SELECT is refused or unanswered, OPERATE is never sent. If the OPERATE is
    /// unanswered the result is a timeout and the outcome on the outstation is unknown.
    pub async fn select_before_operate(&self, request: &ControlRequest) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner.control_exchange(FunctionCode::Select, request).await?;
        inner.control_exchange(FunctionCode::Operate, request).await
    }

    /// How long to wait for the outstation's answer to a request (default 5 s).
    pub async fn set_response_timeout(&self, timeout: Duration) {
        self.inner.lock().await.response_timeout = timeout;
    }
}

impl Dnp3Inner {
    /// Send one control request (SELECT, OPERATE or DIRECT OPERATE) and check the outstation's
    /// echo of the control object. A missing echo, a different object or a status other than
    /// SUCCESS is an error, so a command is never reported as done unless the outstation said so.
    async fn control_exchange(
        &mut self,
        function: FunctionCode,
        request: &ControlRequest,
    ) -> Result<(), ProtocolError> {
        let name = match function {
            FunctionCode::Select => "select",
            FunctionCode::Operate => "operate",
            _ => "direct operate",
        };
        let seq = self.seq.next();
        let asdu = build_control_request(function, seq, request)
            .map_err(|e| ProtocolError::InvalidConfig(e.to_string()))?;
        let response = match self.exchange(asdu, seq, name).await {
            Err(e @ ProtocolError::Timeout(_)) => return Err(e),
            Err(e) => return Err(ProtocolError::WriteFailed(format!("DNP3 {name} failed: {e}"))),
            Ok(response) => response,
        };

        let echo = parse_control_objects(response.objects())
            .map_err(|e| ProtocolError::WriteFailed(format!("DNP3 {name} response: {e}")))?;
        let Some(echo) = echo else {
            // No echo: the outstation refused in the IIN, or did not act on it at all.
            response.check_iin(true, name, ProtocolError::WriteFailed)?;
            return Err(ProtocolError::WriteFailed(format!(
                "DNP3 {name} not confirmed: the outstation did not echo the control object"
            )));
        };
        if echo.request != *request {
            return Err(ProtocolError::WriteFailed(format!(
                "DNP3 {name} response does not match the request (point {})",
                request.index
            )));
        }
        if echo.status != ControlStatus::Success {
            return Err(ProtocolError::WriteFailed(format!(
                "DNP3 {name} rejected by the outstation: {}",
                echo.status
            )));
        }
        Ok(())
    }

    /// Send `asdu` and collect the outstation's response to it: every fragment up to FIN,
    /// confirming each one that asks for it. Unsolicited responses and late answers to earlier
    /// requests that arrive meanwhile are confirmed where required and otherwise skipped.
    async fn exchange(
        &mut self,
        asdu: Vec<u8>,
        seq: u8,
        what: &str,
    ) -> Result<Response, ProtocolError> {
        self.send_asdu(&asdu).await?;
        let wait = self.response_timeout;
        tokio::time::timeout(wait, self.receive_response(seq))
            .await
            .map_err(|_| {
                ProtocolError::Timeout(format!(
                    "DNP3 {what} timeout ({}s): no response from the outstation",
                    wait.as_secs_f32()
                ))
            })?
    }

    async fn receive_response(&mut self, seq: u8) -> Result<Response, ProtocolError> {
        let mut fragments = Vec::new();
        let mut iin = Iin::default();
        let mut expected = seq;
        let mut started = false;
        loop {
            let asdu = self.next_asdu().await?;
            let Ok((header, objects)) = AsduHeader::decode_response(&asdu) else {
                continue; // not a response fragment
            };
            match header.function {
                FunctionCode::UnsolicitedResponse => {
                    if header.control.con {
                        self.send_asdu(&build_confirm(header.control.seq, true)).await?;
                    }
                    continue;
                }
                FunctionCode::Response => {}
                _ => continue,
            }
            if header.control.con {
                self.send_asdu(&build_confirm(header.control.seq, false)).await?;
            }
            if !started {
                if !header.control.fir || header.control.seq != seq {
                    continue; // late answer to an earlier request
                }
                started = true;
            } else if header.control.fir || header.control.seq != expected {
                return Err(ProtocolError::ReadFailed(format!(
                    "Response fragment out of sequence: expected {expected}, got {}",
                    header.control.seq
                )));
            }
            iin.merge(&header.iin);
            fragments.push(objects.to_vec());
            if header.control.fin {
                self.last_iin = iin.clone();
                return Ok(Response { iin, fragments });
            }
            expected = (header.control.seq + 1) & 0x0F;
        }
    }

    /// Report a frame to the traffic tap, if there is one.
    fn record(&self, sent: bool, link: ControlByte, asdu: Vec<u8>, octets: usize) {
        if let Some(tap) = &self.tap {
            let _ = tap.send(TrafficRecord { sent, link, asdu, octets });
        }
    }

    async fn send_link_reset(&mut self) -> Result<(), ProtocolError> {
        let frame = build_link_reset_frame(self.master_addr, self.outstation_addr);
        let bytes = frame.encode();
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Send link reset failed: {e}")))?;
        self.record(true, frame.link_control, Vec::new(), bytes.len());
        Ok(())
    }

    /// Send a link-layer service frame (no user data) as the secondary: ACK or LINK STATUS.
    async fn send_secondary(&mut self, function: u8) -> Result<(), ProtocolError> {
        let frame = Dnp3Frame {
            link_control: ControlByte { dir: true, prm: false, fcb: false, fcv: false, function },
            destination: self.outstation_addr,
            source: self.master_addr,
            data: vec![],
        };
        let bytes = frame.encode();
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Send link frame failed: {e}")))?;
        self.record(true, frame.link_control, Vec::new(), bytes.len());
        Ok(())
    }

    async fn send_asdu(&mut self, asdu: &[u8]) -> Result<(), ProtocolError> {
        let mut octets = 0;
        for seg in segment_asdu(asdu) {
            let th = TransportHeader {
                fir: seg.fir,
                fin: seg.fin,
                sequence: seg.sequence,
            };
            let mut data = vec![th.to_byte()];
            data.extend_from_slice(&seg.data);

            let frame = Dnp3Frame {
                link_control: ControlByte::master_data(),
                destination: self.outstation_addr,
                source: self.master_addr,
                data,
            };
            let bytes = frame.encode();
            octets += bytes.len();
            self.stream
                .write_all(&bytes)
                .await
                .map_err(|e| ProtocolError::ReadFailed(format!("Send ASDU failed: {e}")))?;
        }
        if self.tap.is_some() {
            self.record(true, ControlByte::master_data(), asdu.to_vec(), octets);
        }
        Ok(())
    }

    async fn receive_frame(&mut self) -> Result<Dnp3Frame, ProtocolError> {
        let mut header = [0u8; 10];
        self.stream
            .read_exact(&mut header[0..3])
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Read frame header failed: {e}")))?;

        if header[0] != 0x05 || header[1] != 0x64 {
            return Err(ProtocolError::ReadFailed("Invalid DNP3 start bytes".into()));
        }

        let length = header[2] as usize;
        if length < 5 {
            return Err(ProtocolError::ReadFailed(format!("Invalid LENGTH {length}")));
        }
        let user_len = length - 5;
        let total_frame_size = 10 + user_len + 2 * user_len.div_ceil(16);
        self.last_frame_octets = total_frame_size;
        let mut buf = vec![0u8; total_frame_size];
        buf[0..3].copy_from_slice(&header[0..3]);
        self.stream
            .read_exact(&mut buf[3..total_frame_size])
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Read frame body failed: {e}")))?;

        Dnp3Frame::decode(&buf).map_err(|e| ProtocolError::ReadFailed(format!("{e}")))
    }

    /// Read link frames until a complete ASDU has been reassembled from transport segments.
    /// Link services the outstation initiates are answered on the way: ACK for RESET LINK STATES,
    /// TEST LINK STATES and confirmed user data, LINK STATUS for REQUEST LINK STATUS. Frames for
    /// other addresses are ignored.
    async fn next_asdu(&mut self) -> Result<Vec<u8>, ProtocolError> {
        loop {
            let frame = self.receive_frame().await?;
            if frame.destination != self.master_addr || frame.source != self.outstation_addr {
                continue;
            }
            if !frame.link_control.prm || frame.data.is_empty() {
                // ACK, NACK, LINK STATUS, or a link service request: no application data.
                self.record(false, frame.link_control, Vec::new(), self.last_frame_octets);
            }
            if !frame.link_control.prm {
                continue; // ACK, NACK, LINK STATUS: nothing for the application
            }
            match frame.link_control.function {
                ControlByte::RESET_LINK_STATES | ControlByte::TEST_LINK_STATES => {
                    self.send_secondary(ControlByte::ACK).await?;
                    continue;
                }
                ControlByte::REQUEST_LINK_STATUS => {
                    self.send_secondary(ControlByte::LINK_STATUS).await?;
                    continue;
                }
                ControlByte::USER_DATA_CONFIRMED => self.send_secondary(ControlByte::ACK).await?,
                ControlByte::USER_DATA_NO_CONFIRM => {}
                _ => continue,
            }
            let Some(&first) = frame.data.first() else { continue };
            let th = TransportHeader::from_byte(first);
            if th.fir {
                self.rx_octets = 0;
            }
            self.rx_octets += self.last_frame_octets;
            let seg = TransportSegment {
                fir: th.fir,
                fin: th.fin,
                sequence: th.sequence,
                data: frame.data[1..].to_vec(),
            };
            if let Some(asdu) = self.reassembler.process(seg) {
                if self.tap.is_some() {
                    self.record(false, frame.link_control, asdu.clone(), self.rx_octets);
                }
                self.rx_octets = 0;
                return Ok(asdu);
            }
        }
    }
}

/// Perform a single integrity poll of the specified points without keeping a
/// persistent client. Useful for one-shot reads.
pub async fn read_points(
    config: &Dnp3Config,
    points: &[Dnp3Point],
) -> Result<Vec<Dnp3Value>, ProtocolError> {
    let client = Dnp3Client::connect(config.clone()).await?;
    let values = client.read(points).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single binary control output without keeping a persistent client.
pub async fn write_point(
    config: &Dnp3Config,
    point: &Dnp3Point,
    value: Dnp3Value,
) -> Result<(), ProtocolError> {
    let client = Dnp3Client::connect(config.clone()).await?;
    client.write(point, value).await?;
    client.disconnect().await?;
    Ok(())
}

/// Perform an event scan (Class 1/2/3) on an existing connection.
pub async fn event_scan(
    client: &Dnp3Client,
    class: u8,
    points: &[Dnp3Point],
) -> Result<Vec<Dnp3Value>, ProtocolError> {
    client.scan_events(class, points).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{AppControl, build_response, build_response_fragment};
    use crate::control::{build_control_response, crob_code};
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn test_session_sequence_increment() {
        let mut seq = SequenceTracker::new();
        assert_eq!(seq.next(), 0);
        assert_eq!(seq.next(), 1);
        assert_eq!(seq.next(), 2);
    }

    #[test]
    fn test_session_sequence_wraps() {
        let mut seq = SequenceTracker::new();
        for _ in 0..15 {
            seq.next();
        }
        assert_eq!(seq.next(), 15);
        assert_eq!(seq.next(), 0);
    }

    #[test]
    fn test_build_link_reset_frame() {
        let frame = build_link_reset_frame(1, 10);
        assert_eq!(frame.link_control, ControlByte::master_link(ControlByte::RESET_LINK_STATES));
        assert_eq!(frame.destination, 10);
        assert_eq!(frame.source, 1);
        assert!(frame.data.is_empty());
        // Encodes to exactly 10 octets: header + header CRC, no user data.
        assert_eq!(frame.encode().len(), 10);
    }

    // ── A scripted outstation to drive the master over TCP ──

    /// What the scripted outstation saw: the application fragments it received.
    #[derive(Default)]
    struct Seen {
        asdus: Vec<Vec<u8>>,
    }

    type Log = Arc<Mutex<Seen>>;

    async fn read_frame(sock: &mut TcpStream) -> Option<Dnp3Frame> {
        let mut header = [0u8; 3];
        sock.read_exact(&mut header).await.ok()?;
        let user_len = header[2] as usize - 5;
        let total = 10 + user_len + 2 * user_len.div_ceil(16);
        let mut buf = vec![0u8; total];
        buf[..3].copy_from_slice(&header);
        sock.read_exact(&mut buf[3..]).await.ok()?;
        Dnp3Frame::decode(&buf).ok()
    }

    async fn write_frame(sock: &mut TcpStream, control: ControlByte, asdu: &[u8]) {
        let mut data = Vec::new();
        if !asdu.is_empty() {
            for seg in segment_asdu(asdu) {
                let th = TransportHeader { fir: seg.fir, fin: seg.fin, sequence: seg.sequence };
                data.push(th.to_byte());
                data.extend_from_slice(&seg.data);
            }
        }
        // One frame per call: tests only send ASDUs that fit one segment.
        let frame = Dnp3Frame { link_control: control, destination: 1, source: 10, data };
        sock.write_all(&frame.encode()).await.unwrap();
    }

    async fn write_asdu(sock: &mut TcpStream, asdu: &[u8]) {
        write_frame(sock, ControlByte::outstation_data(), asdu).await;
    }

    /// Start an outstation that ACKs the link reset (unless `ack_reset` is false), records what
    /// it receives and answers each application fragment with whatever `handler` returns.
    async fn scripted<F>(ack_reset: bool, mut handler: F) -> (Dnp3Client, Log)
    where
        F: FnMut(&[u8]) -> Vec<Vec<u8>> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log: Log = Arc::new(Mutex::new(Seen::default()));
        let seen = log.clone();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut reset = [0u8; 10];
            sock.read_exact(&mut reset).await.unwrap();
            if ack_reset {
                write_frame(&mut sock, ControlByte::outstation_link(ControlByte::ACK), &[]).await;
            }
            while let Some(frame) = read_frame(&mut sock).await {
                if frame.data.is_empty() {
                    continue; // link-only frame (ACK)
                }
                let asdu = frame.data[1..].to_vec(); // strip the transport header
                seen.lock().asdus.push(asdu.clone());
                if asdu[1] == 0x00 {
                    continue; // CONFIRM: nothing to answer
                }
                for reply in handler(&asdu) {
                    write_asdu(&mut sock, &reply).await;
                }
            }
        });
        let client = Dnp3Client::connect(Dnp3Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port,
            master_address: 1,
            outstation_address: 10,
            poll_interval_ms: 1000,
        })
        .await
        .unwrap();
        client.set_response_timeout(Duration::from_millis(300)).await;
        (client, log)
    }

    fn seq_of(asdu: &[u8]) -> u8 {
        asdu[0] & 0x0F
    }

    fn point(group: u8, index: u16) -> Dnp3Point {
        Dnp3Point { name: format!("g{group}-{index}"), group, variation: 0, index, scale: 1.0 }
    }

    /// Objects: g1v2 index 0..1 (on, off), g30v5 index 0 = 12.5, g20v1 index 3 = 77.
    fn static_objects() -> Vec<u8> {
        let mut o = vec![1, 2, 0x00, 0, 1, 0x81, 0x01, 30, 5, 0x00, 0, 0, 0x01];
        o.extend_from_slice(&12.5f32.to_le_bytes());
        o.extend_from_slice(&[20, 1, 0x00, 3, 3, 0x01]);
        o.extend_from_slice(&77u32.to_le_bytes());
        o
    }

    // ── Reads ──

    #[tokio::test]
    async fn integrity_poll_request_and_decoded_values() {
        let (client, log) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::default(), &static_objects())]
        })
        .await;
        let values = client
            .read(&[point(1, 0), point(1, 1), point(30, 0), point(20, 3), point(30, 9)])
            .await
            .unwrap();
        assert_eq!(values[0].value, 1.0);
        assert_eq!(values[1].value, 0.0);
        assert_eq!(values[2].value, 12.5);
        assert_eq!(values[3].value, 77.0);
        assert!(values[4].value.is_nan(), "a point the outstation did not send reads as NaN");
        // Two-octet request header, then g60v2, v3, v4, v1 with qualifier 0x06.
        let seen = log.lock();
        assert_eq!(seen.asdus[0], [0xC0, 0x01, 60, 2, 6, 60, 3, 6, 60, 4, 6, 60, 1, 6]);
    }

    #[tokio::test]
    async fn static_value_wins_over_event_in_any_order() {
        // An older event for g30 index 0 arrives after the static value.
        let (client, _) = scripted(true, |asdu| {
            let mut objects = static_objects();
            objects.extend_from_slice(&[32, 5, 0x17, 1, 0, 0x01]);
            objects.extend_from_slice(&99.0f32.to_le_bytes());
            vec![build_response(seq_of(asdu), &Iin::default(), &objects)]
        })
        .await;
        let values = client.read(&[point(30, 0)]).await.unwrap();
        assert_eq!(values[0].value, 12.5);
    }

    #[tokio::test]
    async fn event_scan_maps_event_objects_to_points() {
        let (client, log) = scripted(true, |asdu| {
            let mut objects = vec![32, 5, 0x17, 1, 4, 0x01];
            objects.extend_from_slice(&3.25f32.to_le_bytes());
            vec![build_response(seq_of(asdu), &Iin::default(), &objects)]
        })
        .await;
        let values = client.scan_events(2, &[point(30, 4), point(30, 5)]).await.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].value, 3.25);
        assert_eq!(log.lock().asdus[0], [0xC0, 0x01, 60, 3, 6]);
    }

    #[tokio::test]
    async fn multi_fragment_response_is_confirmed_and_joined() {
        let (client, log) = scripted(true, |asdu| {
            let seq = seq_of(asdu);
            let first = AppControl { fir: true, fin: false, con: true, uns: false, seq };
            let second = AppControl { fir: false, fin: true, con: false, uns: false, seq: (seq + 1) & 0x0F };
            let part1 = vec![1, 2, 0x00, 0, 0, 0x81];
            let mut part2 = vec![30, 5, 0x00, 0, 0, 0x01];
            part2.extend_from_slice(&8.5f32.to_le_bytes());
            vec![
                build_response_fragment(first, &Iin::default(), &part1),
                build_response_fragment(second, &Iin::default(), &part2),
            ]
        })
        .await;
        let values = client.read(&[point(1, 0), point(30, 0)]).await.unwrap();
        assert_eq!(values[0].value, 1.0);
        assert_eq!(values[1].value, 8.5);
        // The first fragment had CON set, so the master confirmed it with the same sequence number.
        let asdus = asdus_after(&log, 2).await;
        assert_eq!(asdus.len(), 2);
        assert_eq!(asdus[1], [0xC0, 0x00]);
    }

    /// The fragments the scripted outstation has received once at least `n` have arrived. The
    /// outstation runs in its own task, so a confirm sent just before `read` returns may still
    /// be in flight.
    async fn asdus_after(log: &Log, n: usize) -> Vec<Vec<u8>> {
        for _ in 0..100 {
            {
                let seen = log.lock();
                if seen.asdus.len() >= n {
                    return seen.asdus.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        log.lock().asdus.clone()
    }

    #[tokio::test]
    async fn out_of_sequence_fragment_is_an_error() {
        let (client, _) = scripted(true, |asdu| {
            let seq = seq_of(asdu);
            let first = AppControl { fir: true, fin: false, con: false, uns: false, seq };
            let wrong = AppControl { fir: false, fin: true, con: false, uns: false, seq: (seq + 5) & 0x0F };
            vec![
                build_response_fragment(first, &Iin::default(), &[]),
                build_response_fragment(wrong, &Iin::default(), &[]),
            ]
        })
        .await;
        let err = client.read(&[point(1, 0)]).await.unwrap_err();
        assert!(err.to_string().contains("out of sequence"), "{err}");
    }

    #[tokio::test]
    async fn unsolicited_response_is_confirmed_and_skipped() {
        let (client, log) = scripted(true, |asdu| {
            // An unsolicited null response with CON and UNS (sequence 9) before the real answer.
            let uns = AppControl { fir: true, fin: true, con: true, uns: true, seq: 9 };
            vec![
                {
                    let mut f = build_response_fragment(uns, &Iin::from_bytes(0x80, 0), &[]);
                    f[1] = FunctionCode::UnsolicitedResponse as u8;
                    f
                },
                build_response(seq_of(asdu), &Iin::default(), &static_objects()),
            ]
        })
        .await;
        let values = client.read(&[point(30, 0)]).await.unwrap();
        assert_eq!(values[0].value, 12.5);
        // CONFIRM for an unsolicited response echoes its sequence with the UNS bit set.
        assert_eq!(asdus_after(&log, 2).await[1], [0xD9, 0x00]);
    }

    #[tokio::test]
    async fn late_answer_to_an_earlier_request_is_skipped() {
        let (client, _) = scripted(true, |asdu| {
            let seq = seq_of(asdu);
            vec![
                build_response((seq + 7) & 0x0F, &Iin::default(), &[]),
                build_response(seq, &Iin::default(), &static_objects()),
            ]
        })
        .await;
        assert_eq!(client.read(&[point(30, 0)]).await.unwrap()[0].value, 12.5);
    }

    #[tokio::test]
    async fn silent_outstation_times_out_instead_of_hanging() {
        let (client, _) = scripted(true, |_| Vec::new()).await;
        let started = std::time::Instant::now();
        let err = client.read(&[point(1, 0)]).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn iin2_errors_fail_the_request() {
        let (client, _) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::from_bytes(0, 0x01), &[])]
        })
        .await;
        let err = client.read(&[point(1, 0)]).await.unwrap_err();
        assert!(err.to_string().contains("function code not supported"), "{err}");
    }

    #[tokio::test]
    async fn unknown_object_iin_is_tolerated_when_data_came_back() {
        let (client, _) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::from_bytes(0, 0x02), &static_objects())]
        })
        .await;
        assert_eq!(client.read(&[point(30, 0)]).await.unwrap()[0].value, 12.5);
    }

    // ── Commands ──

    #[tokio::test]
    async fn time_sync_writes_six_octet_time() {
        let (client, log) = scripted(true, |asdu| vec![build_response(seq_of(asdu), &Iin::default(), &[])]).await;
        client.time_sync().await.unwrap();
        let seen = log.lock();
        let req = &seen.asdus[0];
        assert_eq!(req.len(), 2 + 4 + 6);
        assert_eq!(req[..6], [0xC0, 0x02, 50, 1, 0x07, 0x01]);
    }

    #[tokio::test]
    async fn restart_and_freeze_requests() {
        let (client, log) = scripted(true, |asdu| vec![build_response(seq_of(asdu), &Iin::default(), &[])]).await;
        client.cold_restart().await.unwrap();
        client.warm_restart().await.unwrap();
        client.freeze_counters().await.unwrap();
        let seen = log.lock();
        assert_eq!(seen.asdus[0], [0xC0, 0x0D]);
        assert_eq!(seen.asdus[1], [0xC1, 0x0E]);
        assert_eq!(seen.asdus[2], [0xC2, 0x07, 20, 0, 0x06]);
    }

    #[tokio::test]
    async fn command_refused_through_iin2() {
        let (client, _) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::from_bytes(0, 0x01), &[])]
        })
        .await;
        let err = client.cold_restart().await.unwrap_err();
        assert!(matches!(err, ProtocolError::WriteFailed(_)), "{err:?}");
        assert!(err.to_string().contains("cold restart refused"), "{err}");
    }

    // ── Link layer ──

    #[tokio::test]
    async fn connect_proceeds_when_outstation_ignores_the_link_reset() {
        let (client, _) = scripted(false, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::default(), &static_objects())]
        })
        .await;
        assert_eq!(client.read(&[point(30, 0)]).await.unwrap()[0].value, 12.5);
    }

    #[tokio::test]
    async fn confirmed_user_data_from_outstation_gets_a_link_ack() {
        // Reproduces the exchange in the DNP3 Quick Reference: the outstation sends its response
        // as CONFIRMED USER DATA and the master answers with a link ACK (control 0x80).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acks = Arc::new(Mutex::new(Vec::<u8>::new()));
        let acks_seen = acks.clone();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            read_frame(&mut sock).await.unwrap(); // reset
            write_frame(&mut sock, ControlByte::outstation_link(ControlByte::ACK), &[]).await;
            let request = read_frame(&mut sock).await.unwrap();
            let seq = request.data[1] & 0x0F;
            let response = build_response(seq, &Iin::default(), &static_objects());
            let confirmed = ControlByte {
                dir: false,
                prm: true,
                fcb: true,
                fcv: true,
                function: ControlByte::USER_DATA_CONFIRMED,
            };
            write_frame(&mut sock, confirmed, &response).await;
            let ack = read_frame(&mut sock).await.unwrap();
            acks_seen.lock().push(ack.link_control.to_byte());
        });
        let client = Dnp3Client::connect(Dnp3Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port,
            master_address: 1,
            outstation_address: 10,
            poll_interval_ms: 1000,
        })
        .await
        .unwrap();
        assert_eq!(client.read(&[point(30, 0)]).await.unwrap()[0].value, 12.5);
        server.await.unwrap();
        assert_eq!(*acks.lock(), [0x80]);
    }

    #[tokio::test]
    async fn link_services_initiated_by_the_outstation_are_answered() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let replies = Arc::new(Mutex::new(Vec::<u8>::new()));
        let replies_seen = replies.clone();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            read_frame(&mut sock).await.unwrap();
            write_frame(&mut sock, ControlByte::outstation_link(ControlByte::ACK), &[]).await;
            let request = read_frame(&mut sock).await.unwrap();
            let seq = request.data[1] & 0x0F;
            // Outstation-initiated RESET LINK STATES (0x40), TEST LINK STATES (0x52) and
            // REQUEST LINK STATUS (0x49), each before the response.
            for function in [ControlByte::RESET_LINK_STATES, ControlByte::TEST_LINK_STATES, ControlByte::REQUEST_LINK_STATUS] {
                let control = ControlByte { dir: false, prm: true, fcb: false, fcv: false, function };
                write_frame(&mut sock, control, &[]).await;
                let answer = read_frame(&mut sock).await.unwrap().link_control.to_byte();
                replies_seen.lock().push(answer);
            }
            write_asdu(&mut sock, &build_response(seq, &Iin::default(), &static_objects())).await;
        });
        let client = Dnp3Client::connect(Dnp3Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port,
            master_address: 1,
            outstation_address: 10,
            poll_interval_ms: 1000,
        })
        .await
        .unwrap();
        assert_eq!(client.read(&[point(30, 0)]).await.unwrap()[0].value, 12.5);
        server.await.unwrap();
        // ACK, ACK, LINK STATUS, all as secondary frames from the master (DIR=1, PRM=0).
        assert_eq!(*replies.lock(), [0x80, 0x80, 0x8B]);
    }

    // ── Select-before-operate ──

    /// Echo the control object of a request back with `status`.
    fn echo(asdu: &[u8], status: ControlStatus) -> Vec<Vec<u8>> {
        let req = parse_control_objects(&asdu[2..]).unwrap().unwrap().request;
        vec![build_control_response(seq_of(asdu), &Iin::default(), &req, status)]
    }

    fn latch_on(index: u16) -> ControlRequest {
        ControlRequest {
            index,
            object: ControlObject::Crob(Crob {
                code: crob_code::LATCH_ON,
                count: 1,
                on_time_ms: 500,
                off_time_ms: 250,
            }),
        }
    }

    fn analog(index: u16, value: f32) -> ControlRequest {
        ControlRequest { index, object: ControlObject::AnalogOutput(value) }
    }

    #[tokio::test]
    async fn sbo_sends_select_then_operate_with_same_object() {
        let (client, log) = scripted(true, |asdu| echo(asdu, ControlStatus::Success)).await;
        client.select_before_operate(&latch_on(3)).await.unwrap();
        client.select_before_operate(&analog(2, 41.5)).await.unwrap();

        let seen = log.lock();
        let got = &seen.asdus;
        let functions: Vec<u8> = got.iter().map(|a| a[1]).collect();
        assert_eq!(functions, vec![0x03, 0x04, 0x03, 0x04]);
        // OPERATE repeats the SELECT object byte for byte.
        assert_eq!(got[0][2..], got[1][2..]);
        assert_eq!(got[2][2..], got[3][2..]);
        // CROB: g12v1, qualifier 0x17, count 1, index 3, latch on.
        assert_eq!(&got[0][2..8], &[12, 1, 0x17, 1, 3, crob_code::LATCH_ON]);
        // Analog output: g41v3 index 2.
        assert_eq!(&got[2][2..7], &[41, 3, 0x17, 1, 2]);
        assert_eq!(&got[2][7..11], &41.5f32.to_le_bytes());
    }

    #[tokio::test]
    async fn operate_sequence_follows_select_sequence_across_wrap() {
        let (client, log) = scripted(true, |asdu| echo(asdu, ControlStatus::Success)).await;
        for _ in 0..9 {
            client.select_before_operate(&latch_on(0)).await.unwrap();
        }
        let seen = log.lock();
        let got = &seen.asdus;
        assert_eq!(got.len(), 18);
        for pair in got.chunks(2) {
            let select_seq = pair[0][0] & 0x0F;
            let operate_seq = pair[1][0] & 0x0F;
            assert_eq!(operate_seq, (select_seq + 1) & 0x0F);
        }
        // 18 requests wrap the 4-bit counter.
        assert!(got.iter().any(|a| a[0] & 0x0F == 0) && got.iter().any(|a| a[0] & 0x0F == 15));
    }

    #[tokio::test]
    async fn refused_select_never_sends_operate() {
        let (client, log) = scripted(true, |asdu| echo(asdu, ControlStatus::NotSupported)).await;
        let err = client.select_before_operate(&latch_on(1)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::WriteFailed(_)), "{err:?}");
        assert!(err.to_string().contains("select rejected"), "{err}");
        assert!(err.to_string().contains("NOT_SUPPORTED"), "{err}");
        assert_eq!(log.lock().asdus.len(), 1);
    }

    #[tokio::test]
    async fn unanswered_select_times_out_without_operate() {
        let (client, log) = scripted(true, |_| Vec::new()).await;
        let started = std::time::Instant::now();
        let err = client.select_before_operate(&latch_on(1)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)), "{err:?}");
        assert!(err.to_string().contains("select"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(log.lock().asdus.len(), 1);
    }

    #[tokio::test]
    async fn unanswered_operate_times_out_after_select_succeeds() {
        let (client, log) = scripted(true, |asdu| {
            if asdu[1] == 0x03 { echo(asdu, ControlStatus::Success) } else { Vec::new() }
        })
        .await;
        let err = client.select_before_operate(&analog(0, 1.0)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)), "{err:?}");
        assert!(err.to_string().contains("operate"), "{err}");
        assert_eq!(log.lock().asdus.len(), 2);
    }

    #[tokio::test]
    async fn operate_reports_expired_select_status() {
        let (client, _) = scripted(true, |asdu| {
            let status = if asdu[1] == 0x03 { ControlStatus::Success } else { ControlStatus::Timeout };
            echo(asdu, status)
        })
        .await;
        let err = client.select_before_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("operate rejected"), "{err}");
        assert!(err.to_string().contains("TIMEOUT"), "{err}");
    }

    #[tokio::test]
    async fn echo_of_a_different_object_is_rejected() {
        let (client, log) = scripted(true, |asdu| {
            let mut other = parse_control_objects(&asdu[2..]).unwrap().unwrap().request;
            other.index += 1;
            vec![build_control_response(seq_of(asdu), &Iin::default(), &other, ControlStatus::Success)]
        })
        .await;
        let err = client.select_before_operate(&latch_on(4)).await.unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
        assert_eq!(log.lock().asdus.len(), 1);
    }

    #[tokio::test]
    async fn response_without_echo_is_not_a_confirmation() {
        let (client, log) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::default(), &[])]
        })
        .await;
        let err = client.select_before_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("did not echo"), "{err}");
        assert_eq!(log.lock().asdus.len(), 1);
    }

    #[tokio::test]
    async fn iin2_refusal_without_echo_is_reported() {
        let (client, _) = scripted(true, |asdu| {
            vec![build_response(seq_of(asdu), &Iin::from_bytes(0, 0x04), &[])]
        })
        .await;
        let err = client.select_before_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("parameter error"), "{err}");
    }

    #[tokio::test]
    async fn stale_responses_are_skipped_during_control() {
        let (client, _) = scripted(true, |asdu| {
            let req = parse_control_objects(&asdu[2..]).unwrap().unwrap().request;
            let seq = seq_of(asdu);
            vec![
                build_control_response((seq + 7) & 0x0F, &Iin::default(), &req, ControlStatus::HardwareError),
                build_control_response(seq, &Iin::default(), &req, ControlStatus::Success),
            ]
        })
        .await;
        client.select_before_operate(&latch_on(0)).await.unwrap();
    }

    #[tokio::test]
    async fn indices_above_255_and_bad_crob_are_rejected_before_sending() {
        let (client, log) = scripted(true, |asdu| echo(asdu, ControlStatus::Success)).await;
        let err = client.select_before_operate(&latch_on(300)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
        let mut bad = latch_on(0);
        bad.object = ControlObject::Crob(Crob { code: 0x09, count: 1, on_time_ms: 0, off_time_ms: 0 });
        let err = client.select_before_operate(&bad).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
        assert!(log.lock().asdus.is_empty());
    }

    #[tokio::test]
    async fn direct_operate_checks_status_too() {
        let (client, log) = scripted(true, |asdu| echo(asdu, ControlStatus::Local)).await;
        let err = client.direct_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("LOCAL"), "{err}");
        assert_eq!(log.lock().asdus[0][1], 0x05);
    }

    #[tokio::test]
    async fn polls_report_flags_times_iin_and_every_frame_to_the_tap() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            read_frame(&mut sock).await.unwrap(); // reset
            write_frame(&mut sock, ControlByte::outstation_link(ControlByte::ACK), &[]).await;
            while let Some(request) = read_frame(&mut sock).await {
                let asdu = &request.data[1..];
                // g30v1 index 2: COMM_LOST without ONLINE; g32v3 event index 4 at time 0x0102030405.
                let mut objects = vec![30, 1, 0x00, 2, 2, 0x04];
                objects.extend_from_slice(&(-5i32).to_le_bytes());
                objects.extend_from_slice(&[32, 3, 0x17, 1, 4, 0x01]);
                objects.extend_from_slice(&7i32.to_le_bytes());
                objects.extend_from_slice(&[0x05, 0x04, 0x03, 0x02, 0x01, 0x00]);
                // IIN1.4 NEED_TIME and IIN1.2 class 2 events pending.
                write_asdu(&mut sock, &build_response(seq_of(asdu), &Iin::from_bytes(0x14, 0), &objects)).await;
            }
        });
        let (tap, mut records) = tokio::sync::mpsc::unbounded_channel();
        let config = Dnp3Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port,
            master_address: 1,
            outstation_address: 10,
            poll_interval_ms: 1000,
        };
        let client = Dnp3Client::connect_with_tap(config, Some(tap)).await.unwrap();
        let polled = client.integrity_poll().await.unwrap();
        assert!(polled.iin.need_time && polled.iin.class_2_events);
        assert!(polled.issues.is_empty());
        let analog = polled.points.iter().find(|p| !p.event).unwrap();
        assert_eq!((analog.index, analog.value, analog.flags), (2, -5.0, Some(0x04)));
        let event = polled.points.iter().find(|p| p.event).unwrap();
        assert_eq!((event.group, event.index, event.value), (30, 4, 7.0));
        assert_eq!(event.time, Some(0x01_0203_0405));
        assert!(client.last_iin().await.need_time);

        let events = client.poll_classes(false, true, false).await.unwrap();
        assert_eq!(events.points.len(), 2);
        assert!(client.poll_classes(false, false, false).await.is_err(), "no class asked for");

        let mut seen = Vec::new();
        while let Ok(record) = records.try_recv() {
            seen.push(record);
        }
        // Reset out, ACK in, then READ out and RESPONSE in twice.
        assert_eq!(seen.len(), 6, "{seen:?}");
        assert!(seen[0].sent && seen[0].asdu.is_empty() && seen[0].octets == 10);
        assert!(!seen[1].sent && seen[1].asdu.is_empty());
        assert!(seen[2].sent && seen[2].asdu[1] == 0x01);
        assert!(!seen[3].sent && seen[3].asdu[1] == 0x81);
        assert_eq!(seen[4].asdu, [0xC1, 0x01, 60, 3, 0x06]);
        // The response travelled in one frame: header, data blocks with CRCs.
        let user = 1 + seen[3].asdu.len();
        assert_eq!(seen[3].octets, 10 + user + 2 * user.div_ceil(16));
    }
}
