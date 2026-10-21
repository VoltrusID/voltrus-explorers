use std::collections::HashMap;
use std::net::SocketAddr;

use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::application::{
    ControlObject, ControlRequest, ControlStatus, Crob, FunctionCode, build_cold_restart_request,
    build_control_request, build_event_scan_request, build_freeze_request,
    build_integrity_poll_request, build_time_sync_request, build_warm_restart_request,
    parse_control_objects, parse_response_asdu,
};
use crate::datalink::{ControlByte, Dnp3Frame};
use crate::objects::{self, Dnp3PointValue};
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

/// DNP3 master session connected to a single outstation.
///
/// Wraps a raw TCP stream and the DNP3 protocol stack (data-link, application,
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
    response_timeout: std::time::Duration,
}

const DEFAULT_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl ProtocolClient for Dnp3Client {
    type Config = Dnp3Config;
    type Tag = Dnp3Point;
    type Value = Dnp3Value;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let addr = format!("{}:{}", config.host, config.port);
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|e| ProtocolError::InvalidConfig(format!("Invalid address '{addr}': {e}")))?;

        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            TcpStream::connect(socket_addr),
        )
        .await
        .map_err(|_| ProtocolError::Timeout("DNP3 connection timeout (5s)".into()))?
        .map_err(|e| ProtocolError::ConnectionFailed(format!("DNP3 connection failed: {e}")))?;

        let mut inner = Dnp3Inner {
            stream,
            master_addr: config.master_address,
            outstation_addr: config.outstation_address,
            seq: SequenceTracker::new(),
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
        };

        // Send link reset and wait for the outstation's link ACK
        // (secondary function 0, addressed back to the master).
        inner.send_link_reset().await?;
        let ack = inner.receive_frame().await?;
        if ack.link_control.prm || ack.link_control.function != ControlByte::ACK {
            return Err(ProtocolError::ConnectionFailed(format!(
                "Expected link ACK after RESET LINK STATES, got function {:#04x} (prm={})",
                ack.link_control.function, ack.link_control.prm
            )));
        }
        if ack.destination != config.master_address || ack.source != config.outstation_address {
            return Err(ProtocolError::ConnectionFailed(format!(
                "Link ACK addressed to {} from {}, expected {} from {}",
                ack.destination, ack.source, config.master_address, config.outstation_address
            )));
        }

        Ok(Self {
            inner: tokio::sync::Mutex::new(inner),
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;

        // Perform an integrity poll to get all current point values.
        let seq = inner.seq.next();
        let asdu = build_integrity_poll_request(seq);
        inner.send_asdu(&asdu).await?;

        let raw_values = inner.read_response_objects().await?;

        // Map requested tags to their values.
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let value = lookup_polled_value(&raw_values, tag).unwrap_or(f64::NAN);
            results.push(Dnp3Value {
                point: tag.clone(),
                value,
            });
        }

        Ok(results)
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


/// Look up a polled value by group+index, ignoring the requested variation:
/// outstations answer with the variation they actually encode (e.g. an
/// integrity poll returns Group 1 Var 2 for a point requested as Var 1).
fn lookup_polled_value(
    raw: &std::collections::HashMap<(u8, u8, u16), Dnp3PointValue>,
    point: &Dnp3Point,
) -> Option<f64> {
    raw.iter()
        .find(|((group, _, index), _)| *group == point.group && *index == point.index)
        .map(|(_, pv)| pv.value)
}

impl Dnp3Client {
    /// Perform a class-based event scan (Class 1/2/3) and return changed points.
    pub async fn scan_events(
        &self,
        class: u8,
        points: &[Dnp3Point],
    ) -> Result<Vec<Dnp3Value>, ProtocolError> {
        let mut inner = self.inner.lock().await;

        let seq = inner.seq.next();
        let asdu = build_event_scan_request(seq, class);
        inner.send_asdu(&asdu).await?;

        let raw_values = inner.read_response_objects().await?;

        let mut results = Vec::new();
        for point in points {
            if let Some(value) = lookup_polled_value(&raw_values, point) {
                results.push(Dnp3Value {
                    point: point.clone(),
                    value,
                });
            }
        }
        Ok(results)
    }

    /// Send a time synchronization (Group 50 Var 1 WRITE) and await the response.
    pub async fn time_sync(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let asdu = build_time_sync_request(seq);
        inner.send_asdu(&asdu).await?;

        let asdu = inner.receive_asdu().await?;
        let (header, _) = parse_response_asdu(&asdu)
            .map_err(|e| ProtocolError::ReadFailed(format!("ASDU parse error: {e}")))?;
        if header.function != FunctionCode::Response {
            return Err(ProtocolError::ReadFailed(format!(
                "Expected RESPONSE for time sync, got {:?}",
                header.function
            )));
        }
        Ok(())
    }

    /// Send a cold restart command (0x0D) and await the response.
    pub async fn cold_restart(&self) -> Result<(), ProtocolError> {
        self.send_restart(build_cold_restart_request).await
    }

    /// Send a warm restart command (0x0E) and await the response.
    pub async fn warm_restart(&self) -> Result<(), ProtocolError> {
        self.send_restart(build_warm_restart_request).await
    }

    async fn send_restart<F>(&self, build: F) -> Result<(), ProtocolError>
    where
        F: Fn(u8) -> Vec<u8>,
    {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let asdu = build(seq);
        inner.send_asdu(&asdu).await?;

        let asdu = inner.receive_asdu().await?;
        let (header, _) = parse_response_asdu(&asdu)
            .map_err(|e| ProtocolError::ReadFailed(format!("ASDU parse error: {e}")))?;
        if header.function != FunctionCode::Response {
            return Err(ProtocolError::ReadFailed(format!(
                "Expected RESPONSE for restart, got {:?}",
                header.function
            )));
        }
        Ok(())
    }

    /// Send a FREEZE command to counter inputs and await the response.
    pub async fn freeze_counters(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let asdu = build_freeze_request(seq);
        inner.send_asdu(&asdu).await?;

        let asdu = inner.receive_asdu().await?;
        let (header, _) = parse_response_asdu(&asdu)
            .map_err(|e| ProtocolError::ReadFailed(format!("ASDU parse error: {e}")))?;
        if header.function != FunctionCode::Response {
            return Err(ProtocolError::ReadFailed(format!(
                "Expected RESPONSE for freeze, got {:?}",
                header.function
            )));
        }
        Ok(())
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

    /// How long to wait for the outstation's answer to a control request (default 5 s).
    pub async fn set_response_timeout(&self, timeout: std::time::Duration) {
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
        self.send_asdu(&asdu).await?;

        let wait = self.response_timeout;
        let response = tokio::time::timeout(wait, async {
            loop {
                let asdu = self.receive_asdu().await?;
                let (header, _) = parse_response_asdu(&asdu)
                    .map_err(|e| ProtocolError::ReadFailed(format!("ASDU parse error: {e}")))?;
                // Skip unsolicited reports and late answers to earlier requests.
                if header.function == FunctionCode::Response && header.sequence == seq {
                    return Ok::<_, ProtocolError>(asdu);
                }
            }
        })
        .await
        .map_err(|_| {
            ProtocolError::Timeout(format!(
                "DNP3 {name} timeout ({}s): no response from the outstation",
                wait.as_secs_f32()
            ))
        })?
        .map_err(|e| ProtocolError::WriteFailed(format!("DNP3 {name} failed: {e}")))?;

        let echo = parse_control_objects(&response)
            .map_err(|e| ProtocolError::WriteFailed(format!("DNP3 {name} response: {e}")))?
            .ok_or_else(|| {
                ProtocolError::WriteFailed(format!(
                    "DNP3 {name} not confirmed: the outstation did not echo the control object"
                ))
            })?;
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

    async fn send_link_reset(&mut self) -> Result<(), ProtocolError> {
        let frame = build_link_reset_frame(self.master_addr, self.outstation_addr);
        let bytes = frame.encode();
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|e| ProtocolError::ConnectionFailed(format!("Send link reset failed: {e}")))?;
        Ok(())
    }

    async fn send_asdu(&mut self, asdu: &[u8]) -> Result<(), ProtocolError> {
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
            self.stream
                .write_all(&bytes)
                .await
                .map_err(|e| ProtocolError::ReadFailed(format!("Send ASDU failed: {e}")))?;
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
        let mut buf = vec![0u8; total_frame_size];
        buf[0..3].copy_from_slice(&header[0..3]);
        self.stream
            .read_exact(&mut buf[3..total_frame_size])
            .await
            .map_err(|e| ProtocolError::ReadFailed(format!("Read frame body failed: {e}")))?;

        Dnp3Frame::decode(&buf).map_err(|e| ProtocolError::ReadFailed(format!("{e}")))
    }

    /// Read one or more link frames and reassemble the complete ASDU from
    /// transport segments. Returns the raw ASDU bytes.
    async fn receive_asdu(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let mut reassembler = TransportReassembler::new();
        loop {
            let frame = self.receive_frame().await?;
            if frame.data.is_empty() {
                return Err(ProtocolError::ReadFailed("Empty frame data".into()));
            }
            let th = TransportHeader::from_byte(frame.data[0]);
            let seg = TransportSegment {
                fir: th.fir,
                fin: th.fin,
                sequence: th.sequence,
                data: frame.data[1..].to_vec(),
            };
            if let Some(asdu) = reassembler.process(seg) {
                return Ok(asdu);
            }
        }
    }

    async fn read_response_objects(
        &mut self,
    ) -> Result<HashMap<(u8, u8, u16), Dnp3PointValue>, ProtocolError> {
        let asdu = self.receive_asdu().await?;
        let (header, parsed_objects) = parse_response_asdu(&asdu)
            .map_err(|e| ProtocolError::ReadFailed(format!("ASDU parse error: {e}")))?;

        if header.function != FunctionCode::Response {
            return Err(ProtocolError::ReadFailed(format!(
                "Expected RESPONSE, got {:?}",
                header.function
            )));
        }

        let mut results = HashMap::new();
        for obj in parsed_objects {
            let values = objects::parse_objects(
                obj.group,
                obj.variation,
                obj.start_index,
                obj.count as u16,
                &obj.data,
            );
            for v in values {
                results.insert((v.group, v.variation, v.index), v);
            }
        }
        Ok(results)
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
    let mut inner = client.inner.lock().await;

    let seq = inner.seq.next();
    let asdu = build_event_scan_request(seq, class);
    inner.send_asdu(&asdu).await?;

    let raw_values = inner.read_response_objects().await?;

    let mut results = Vec::with_capacity(points.len());
    for point in points {
        if let Some(value) = lookup_polled_value(&raw_values, point) {
            results.push(Dnp3Value {
                point: point.clone(),
                value,
            });
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── Select-before-operate against a scripted outstation ──

    use crate::application::{build_control_response, crob_code};
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    type Received = Arc<Mutex<Vec<Vec<u8>>>>;

    async fn read_asdu(sock: &mut TcpStream) -> Option<Vec<u8>> {
        let mut header = [0u8; 3];
        sock.read_exact(&mut header).await.ok()?;
        let user_len = header[2] as usize - 5;
        let total = 10 + user_len + 2 * user_len.div_ceil(16);
        let mut buf = vec![0u8; total];
        buf[..3].copy_from_slice(&header);
        sock.read_exact(&mut buf[3..]).await.ok()?;
        let frame = Dnp3Frame::decode(&buf).ok()?;
        Some(frame.data)
    }

    async fn write_asdu(sock: &mut TcpStream, asdu: &[u8]) {
        for seg in segment_asdu(asdu) {
            let th = TransportHeader { fir: seg.fir, fin: seg.fin, sequence: seg.sequence };
            let mut data = vec![th.to_byte()];
            data.extend_from_slice(&seg.data);
            let frame = Dnp3Frame {
                link_control: ControlByte::outstation_data(),
                destination: 1,
                source: 10,
                data,
            };
            sock.write_all(&frame.encode()).await.unwrap();
        }
    }

    /// Start an outstation that ACKs the link reset, records every ASDU it receives and answers
    /// each with whatever `handler` returns (nothing = stay silent).
    async fn scripted_outstation<F>(mut handler: F) -> (Dnp3Client, Received)
    where
        F: FnMut(&[u8]) -> Vec<Vec<u8>> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received: Received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Link reset: answer with a secondary ACK.
            let mut reset = [0u8; 10];
            sock.read_exact(&mut reset).await.unwrap();
            let ack = Dnp3Frame {
                link_control: ControlByte::outstation_link(ControlByte::ACK),
                destination: 1,
                source: 10,
                data: vec![],
            };
            sock.write_all(&ack.encode()).await.unwrap();
            while let Some(data) = read_asdu(&mut sock).await {
                let asdu = data[1..].to_vec(); // strip the transport header
                log.lock().push(asdu.clone());
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
        client.set_response_timeout(std::time::Duration::from_millis(300)).await;
        (client, received)
    }

    /// Echo the control object of a request back with `status`.
    fn echo(asdu: &[u8], status: ControlStatus) -> Vec<Vec<u8>> {
        let req = parse_control_objects(asdu).unwrap().unwrap().request;
        vec![build_control_response(asdu[1] & 0x0F, &req, status)]
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
        let (client, received) = scripted_outstation(|asdu| echo(asdu, ControlStatus::Success)).await;
        client.select_before_operate(&latch_on(3)).await.unwrap();
        client.select_before_operate(&analog(2, 41.5)).await.unwrap();

        let got = received.lock();
        let functions: Vec<u8> = got.iter().map(|a| a[0]).collect();
        assert_eq!(functions, vec![0x03, 0x04, 0x03, 0x04]);
        // OPERATE repeats the SELECT object byte for byte.
        assert_eq!(got[0][4..], got[1][4..]);
        assert_eq!(got[2][4..], got[3][4..]);
        // CROB: g12v1, qualifier 0x17, count 1, index 3, latch on.
        assert_eq!(&got[0][4..10], &[12, 1, 0x17, 1, 3, crob_code::LATCH_ON]);
        // Analog output: g41v3 index 2.
        assert_eq!(&got[2][4..9], &[41, 3, 0x17, 1, 2]);
        assert_eq!(&got[2][9..13], &41.5f32.to_le_bytes());
    }

    #[tokio::test]
    async fn operate_sequence_follows_select_sequence_across_wrap() {
        let (client, received) = scripted_outstation(|asdu| echo(asdu, ControlStatus::Success)).await;
        for _ in 0..9 {
            client.select_before_operate(&latch_on(0)).await.unwrap();
        }
        let got = received.lock();
        assert_eq!(got.len(), 18);
        for pair in got.chunks(2) {
            let select_seq = pair[0][1] & 0x0F;
            let operate_seq = pair[1][1] & 0x0F;
            assert_eq!(operate_seq, (select_seq + 1) & 0x0F);
        }
        // 18 requests wrap the 4-bit counter.
        assert!(got.iter().any(|a| a[1] & 0x0F == 0) && got.iter().any(|a| a[1] & 0x0F == 15));
    }

    #[tokio::test]
    async fn refused_select_never_sends_operate() {
        let (client, received) =
            scripted_outstation(|asdu| echo(asdu, ControlStatus::NotSupported)).await;
        let err = client.select_before_operate(&latch_on(1)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::WriteFailed(_)), "{err:?}");
        assert!(err.to_string().contains("select rejected"), "{err}");
        assert!(err.to_string().contains("NOT_SUPPORTED"), "{err}");
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn unanswered_select_times_out_without_operate() {
        let (client, received) = scripted_outstation(|_| Vec::new()).await;
        let started = std::time::Instant::now();
        let err = client.select_before_operate(&latch_on(1)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)), "{err:?}");
        assert!(err.to_string().contains("select"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn unanswered_operate_times_out_after_select_succeeds() {
        let (client, received) = scripted_outstation(|asdu| {
            if asdu[0] == 0x03 { echo(asdu, ControlStatus::Success) } else { Vec::new() }
        })
        .await;
        let err = client.select_before_operate(&analog(0, 1.0)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Timeout(_)), "{err:?}");
        assert!(err.to_string().contains("operate"), "{err}");
        assert_eq!(received.lock().len(), 2);
    }

    #[tokio::test]
    async fn operate_reports_expired_select_status() {
        let (client, _) = scripted_outstation(|asdu| {
            let status = if asdu[0] == 0x03 { ControlStatus::Success } else { ControlStatus::Timeout };
            echo(asdu, status)
        })
        .await;
        let err = client.select_before_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("operate rejected"), "{err}");
        assert!(err.to_string().contains("TIMEOUT"), "{err}");
    }

    #[tokio::test]
    async fn echo_of_a_different_object_is_rejected() {
        let (client, received) = scripted_outstation(|asdu| {
            let mut other = parse_control_objects(asdu).unwrap().unwrap().request;
            other.index += 1;
            vec![build_control_response(asdu[1] & 0x0F, &other, ControlStatus::Success)]
        })
        .await;
        let err = client.select_before_operate(&latch_on(4)).await.unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn response_without_echo_is_not_a_confirmation() {
        let (client, received) = scripted_outstation(|asdu| {
            let header = crate::application::AsduHeader {
                function: FunctionCode::Response,
                sequence: asdu[1] & 0x0F,
                iin: crate::application::Iin::default(),
            };
            vec![header.encode().to_vec()]
        })
        .await;
        let err = client.select_before_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("did not echo"), "{err}");
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn stale_and_unsolicited_responses_are_skipped() {
        let (client, _) = scripted_outstation(|asdu| {
            let req = parse_control_objects(asdu).unwrap().unwrap().request;
            let seq = asdu[1] & 0x0F;
            // A late answer to some earlier request, then the real one.
            vec![
                build_control_response((seq + 7) & 0x0F, &req, ControlStatus::HardwareError),
                build_control_response(seq, &req, ControlStatus::Success),
            ]
        })
        .await;
        client.select_before_operate(&latch_on(0)).await.unwrap();
    }

    #[tokio::test]
    async fn indices_above_255_and_bad_crob_are_rejected_before_sending() {
        let (client, received) = scripted_outstation(|asdu| echo(asdu, ControlStatus::Success)).await;
        let err = client.select_before_operate(&latch_on(300)).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
        let mut bad = latch_on(0);
        bad.object = ControlObject::Crob(Crob { code: 0x09, count: 1, on_time_ms: 0, off_time_ms: 0 });
        let err = client.select_before_operate(&bad).await.unwrap_err();
        assert!(matches!(err, ProtocolError::InvalidConfig(_)), "{err:?}");
        assert!(received.lock().is_empty());
    }

    #[tokio::test]
    async fn direct_operate_checks_status_too() {
        let (client, received) =
            scripted_outstation(|asdu| echo(asdu, ControlStatus::Local)).await;
        let err = client.direct_operate(&latch_on(0)).await.unwrap_err();
        assert!(err.to_string().contains("LOCAL"), "{err}");
        assert_eq!(received.lock()[0][0], 0x05);
    }
}
