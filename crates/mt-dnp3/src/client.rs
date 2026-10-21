use std::collections::HashMap;
use std::net::SocketAddr;

use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::application::{
    FunctionCode, build_analog_output_direct_operate, build_cold_restart_request,
    build_direct_operate_binary, build_event_scan_request, build_freeze_request,
    build_integrity_poll_request, build_time_sync_request, build_warm_restart_request,
    parse_response_asdu,
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
}

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
        let mut inner = self.inner.lock().await;

        let on = value.value != 0.0;
        let seq = inner.seq.next();
        let asdu = build_direct_operate_binary(seq, tag.index, on);
        inner.send_asdu(&asdu).await?;

        // Wait for response with timeout.
        match tokio::time::timeout(std::time::Duration::from_secs(5), inner.receive_asdu()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(ProtocolError::WriteFailed(format!(
                "DNP3 direct operate failed: {e}"
            ))),
            Err(_) => Err(ProtocolError::Timeout(
                "DNP3 direct operate timeout (5s)".into(),
            )),
        }
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
    pub async fn write_analog(
        &self,
        index: u16,
        value: f64,
    ) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.seq.next();
        let asdu = build_analog_output_direct_operate(seq, index, value);
        inner.send_asdu(&asdu).await?;

        match tokio::time::timeout(std::time::Duration::from_secs(5), inner.receive_asdu()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(ProtocolError::WriteFailed(format!(
                "DNP3 analog direct operate failed: {e}"
            ))),
            Err(_) => Err(ProtocolError::Timeout(
                "DNP3 analog direct operate timeout (5s)".into(),
            )),
        }
    }
}

impl Dnp3Inner {
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
}
