//! Shared BACnet/IP confirmed-request transport (ASHRAE 135 clauses 5 and 6).
//!
//! Every confirmed service in this crate (ReadProperty, ReadRange,
//! SubscribeCOV, AcknowledgeAlarm, ...) goes through [`confirmed_request`].
//! It owns the invoke-ID counter and turns every possible reply into either
//! the service data of a ComplexAck, an empty vector for a SimpleAck, or a
//! descriptive error for an Error / Reject / Abort PDU. Segmented ComplexAcks
//! are reassembled and each segment is acknowledged with a SegmentAck
//! (clause 5.4.5.2), so large replies (object lists, log buffers) work.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
use bacnet_rs::network::Npdu;
use bacnet_rs::service::ConfirmedServiceChoice;
use mt_protocol::ProtocolError;

/// Default time to wait for a reply to a confirmed request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

/// Socket read-timeout slice used while waiting for a reply.
const RECV_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Global invoke ID counter (0-255, wrapping), shared by every confirmed request.
static NEXT_INVOKE_ID: AtomicU8 = AtomicU8::new(0);

/// Allocate the next invoke ID.
pub fn next_invoke_id() -> u8 {
    NEXT_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Wrap an APDU in NPDU + BVLC Original-Unicast-NPDU (0x81 0x0A).
pub fn unicast_frame(apdu: &[u8], expecting_reply: bool) -> Vec<u8> {
    let mut npdu = Npdu::new();
    npdu.control.expecting_reply = expecting_reply;
    let mut body = npdu.encode();
    body.extend_from_slice(apdu);

    let len = (body.len() + 4) as u16;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&[0x81, 0x0A]);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    frame
}

/// Extract the APDU from a received BVLC frame. Handles Original-Unicast (0x0A),
/// Original-Broadcast (0x0B) and Forwarded-NPDU (0x04, 6-byte originating
/// address after the BVLC header). Returns `None` for anything that is not a
/// well-formed application-layer frame (BVLL control messages, network-layer
/// messages, truncated frames).
pub fn apdu_of_frame(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() < 4 || frame[0] != 0x81 {
        return None;
    }
    let bvlc_len = ((frame[2] as usize) << 8) | frame[3] as usize;
    if bvlc_len != frame.len() {
        return None;
    }
    let npdu_start = match frame[1] {
        0x0A | 0x0B => 4,
        0x04 => 10,
        _ => return None,
    };
    let npdu_bytes = frame.get(npdu_start..)?;
    let (npdu, npdu_len) = Npdu::decode(npdu_bytes).ok()?;
    if npdu.control.network_message {
        return None;
    }
    let apdu = npdu_bytes.get(npdu_len..)?;
    if apdu.is_empty() {
        None
    } else {
        Some(apdu)
    }
}

/// BVLC frame carrying a SimpleAck for `invoke_id` (used to acknowledge
/// ConfirmedCOVNotification and other confirmed requests we receive).
pub fn simple_ack_frame(invoke_id: u8, service_choice: u8) -> Vec<u8> {
    let apdu = Apdu::SimpleAck {
        invoke_id,
        service_choice,
    }
    .encode();
    unicast_frame(&apdu, false)
}

/// Result of feeding one received APDU to a [`Transaction`].
#[derive(Debug, Default)]
pub struct Handled {
    /// APDU to send back to the device (a SegmentAck), if any.
    pub send: Option<Vec<u8>>,
    /// Final outcome of the transaction, once known.
    pub done: Option<Result<Vec<u8>, ProtocolError>>,
}

/// Receive-side state of one outstanding confirmed request.
pub struct Transaction {
    invoke_id: u8,
    buffer: Vec<u8>,
    segments_seen: bool,
    next_seq: u8,
}

impl Transaction {
    pub fn new(invoke_id: u8) -> Self {
        Self {
            invoke_id,
            buffer: Vec::new(),
            segments_seen: false,
            next_seq: 0,
        }
    }

    fn segment_ack(&self, negative: bool, seq: u8, window: u8) -> Vec<u8> {
        Apdu::SegmentAck {
            negative,
            server: false,
            invoke_id: self.invoke_id,
            sequence_number: seq,
            window_size: window.max(1),
        }
        .encode()
    }

    /// Process one received APDU. PDUs for other invoke IDs, request PDUs and
    /// malformed PDUs are ignored (default `Handled`).
    pub fn handle(&mut self, apdu: &[u8]) -> Handled {
        let Some(&first) = apdu.first() else {
            return Handled::default();
        };
        match first >> 4 {
            // SimpleAck: type, invoke id, service choice.
            2 => {
                if apdu.len() < 3 || apdu[1] != self.invoke_id {
                    return Handled::default();
                }
                Handled {
                    send: None,
                    done: Some(Ok(Vec::new())),
                }
            }
            // ComplexAck.
            3 => self.handle_complex_ack(apdu),
            // Error: type, invoke id, service choice, error-class, error-code
            // (both application-tagged enumerated).
            5 => {
                if apdu.len() < 5 || apdu[1] != self.invoke_id {
                    return Handled::default();
                }
                let parsed = parse_enumerated(&apdu[3..]).and_then(|(class, used)| {
                    parse_enumerated(&apdu[3 + used..]).map(|(code, _)| (class, code))
                });
                let msg = match parsed {
                    Some((class, code)) => format!(
                        "BACnet Error from device: {} / {}",
                        error_class_name(class),
                        error_code_name(code)
                    ),
                    None => "BACnet Error from device (undecodable error codes)".to_string(),
                };
                Handled {
                    send: None,
                    done: Some(Err(ProtocolError::Other(msg))),
                }
            }
            // Reject: type, invoke id, reject reason.
            6 => {
                if apdu.len() < 3 || apdu[1] != self.invoke_id {
                    return Handled::default();
                }
                Handled {
                    send: None,
                    done: Some(Err(ProtocolError::Other(format!(
                        "BACnet Reject from device: {}",
                        reject_reason_name(apdu[2])
                    )))),
                }
            }
            // Abort: type|server flag, invoke id, abort reason.
            7 => {
                if apdu.len() < 3 || apdu[1] != self.invoke_id {
                    return Handled::default();
                }
                Handled {
                    send: None,
                    done: Some(Err(ProtocolError::Other(format!(
                        "BACnet Abort from device: {}",
                        abort_reason_name(apdu[2])
                    )))),
                }
            }
            _ => Handled::default(),
        }
    }

    fn handle_complex_ack(&mut self, apdu: &[u8]) -> Handled {
        let segmented = apdu[0] & 0x08 != 0;
        let more_follows = apdu[0] & 0x04 != 0;
        if apdu.len() < 3 || apdu[1] != self.invoke_id {
            return Handled::default();
        }

        if !segmented {
            // type, invoke id, service choice, data...
            return Handled {
                send: None,
                done: Some(Ok(apdu[3..].to_vec())),
            };
        }

        // Segmented: type, invoke id, sequence number, proposed window size,
        // service choice, data...
        if apdu.len() < 5 {
            return Handled::default();
        }
        let seq = apdu[2];
        let window = apdu[3];

        if seq != self.next_seq {
            // Duplicate of the previous segment: repeat its ack. Anything else is
            // out of order: negative ack carrying the last in-order sequence number.
            let last_good = self.next_seq.wrapping_sub(1);
            let negative = !(self.segments_seen && seq == last_good);
            return Handled {
                send: Some(self.segment_ack(negative, last_good, window)),
                done: None,
            };
        }

        self.segments_seen = true;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.buffer.extend_from_slice(&apdu[5..]);
        let ack = self.segment_ack(false, seq, window);
        if more_follows {
            Handled {
                send: Some(ack),
                done: None,
            }
        } else {
            Handled {
                send: Some(ack),
                done: Some(Ok(std::mem::take(&mut self.buffer))),
            }
        }
    }
}

/// Send a confirmed request and wait for its outcome.
///
/// Returns the service data of a ComplexAck (segments reassembled), an empty
/// vector for a SimpleAck, or an error describing the Error / Reject / Abort
/// PDU the device answered with. The timeout restarts every time a segment of a
/// segmented reply arrives.
pub fn confirmed_request(
    socket: &UdpSocket,
    target_addr: SocketAddr,
    service_choice: ConfirmedServiceChoice,
    service_data: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, ProtocolError> {
    let invoke_id = next_invoke_id();
    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_data,
    }
    .encode();

    socket
        .send_to(&unicast_frame(&apdu, true), target_addr)
        .map_err(|e| ProtocolError::WriteFailed(format!("Send: {e}")))?;

    // Poll in short slices so the deadline is honoured even if the caller's
    // socket has no (or a long) read timeout configured.
    socket
        .set_read_timeout(Some(RECV_POLL_INTERVAL))
        .map_err(|e| ProtocolError::Other(format!("Set timeout: {e}")))?;

    let mut txn = Transaction::new(invoke_id);
    let mut buf = [0u8; 1500];
    let mut deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((len, source)) if source == target_addr => {
                let Some(reply) = apdu_of_frame(&buf[..len]) else {
                    continue;
                };
                let handled = txn.handle(reply);
                if let Some(ack) = handled.send {
                    deadline = Instant::now() + timeout;
                    socket
                        .send_to(&unicast_frame(&ack, false), target_addr)
                        .map_err(|e| ProtocolError::WriteFailed(format!("Send SegmentAck: {e}")))?;
                }
                if let Some(result) = handled.done {
                    return result;
                }
            }
            Ok(_) => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(e) => return Err(ProtocolError::ReadFailed(format!("Recv: {e}"))),
        }
    }

    Err(ProtocolError::Timeout(format!(
        "BACnet confirmed request timeout for {target_addr}"
    )))
}

/// Decode a leading application-tagged Enumerated (tag 9) or Unsigned value
/// into `(value, bytes_consumed)`.
fn parse_enumerated(data: &[u8]) -> Option<(u32, usize)> {
    let tag = *data.first()?;
    if tag & 0x08 != 0 || !matches!(tag >> 4, 2 | 9) {
        return None;
    }
    let len = (tag & 0x07) as usize;
    if !(1..=4).contains(&len) {
        return None;
    }
    let bytes = data.get(1..1 + len)?;
    let v = bytes.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
    Some((v, 1 + len))
}

/// BACnetErrorClass name (ASHRAE 135 clause 18).
pub fn error_class_name(class: u32) -> String {
    match class {
        0 => "device".into(),
        1 => "object".into(),
        2 => "property".into(),
        3 => "resources".into(),
        4 => "security".into(),
        5 => "services".into(),
        6 => "vt".into(),
        7 => "communication".into(),
        n => format!("class {n}"),
    }
}

/// BACnetErrorCode name for the codes a user is likely to meet.
pub fn error_code_name(code: u32) -> String {
    match code {
        0 => "other".into(),
        2 => "configuration-in-progress".into(),
        3 => "device-busy".into(),
        7 => "inconsistent-parameters".into(),
        9 => "invalid-data-type".into(),
        13 => "invalid-parameter-data-type".into(),
        14 => "invalid-time-stamp".into(),
        16 => "missing-required-parameter".into(),
        20 => "no-space-to-write-property".into(),
        25 => "operational-problem".into(),
        27 => "read-access-denied".into(),
        29 => "service-request-denied".into(),
        30 => "timeout".into(),
        31 => "unknown-object".into(),
        32 => "unknown-property".into(),
        36 => "unsupported-object-type".into(),
        37 => "value-out-of-range".into(),
        40 => "write-access-denied".into(),
        42 => "invalid-array-index".into(),
        43 => "cov-subscription-failed".into(),
        44 => "not-cov-property".into(),
        45 => "optional-functionality-not-supported".into(),
        n => format!("code {n}"),
    }
}

/// BACnetRejectReason name (ASHRAE 135 clause 18.8).
pub fn reject_reason_name(reason: u8) -> String {
    match reason {
        0 => "other".into(),
        1 => "buffer-overflow".into(),
        2 => "inconsistent-parameters".into(),
        3 => "invalid-parameter-data-type".into(),
        4 => "invalid-tag".into(),
        5 => "missing-required-parameter".into(),
        6 => "parameter-out-of-range".into(),
        7 => "too-many-arguments".into(),
        8 => "undefined-enumeration".into(),
        9 => "unrecognized-service".into(),
        n => format!("reason {n}"),
    }
}

/// BACnetAbortReason name (ASHRAE 135 clause 18.9).
pub fn abort_reason_name(reason: u8) -> String {
    match reason {
        0 => "other".into(),
        1 => "buffer-overflow".into(),
        2 => "invalid-apdu-in-this-state".into(),
        3 => "preempted-by-higher-priority-task".into(),
        4 => "segmentation-not-supported".into(),
        5 => "security-error".into(),
        6 => "insufficient-security".into(),
        7 => "window-size-out-of-range".into(),
        8 => "application-exceeded-reply-time".into(),
        9 => "out-of-resources".into(),
        10 => "tsm-timeout".into(),
        11 => "apdu-too-long".into(),
        n => format!("reason {n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_text(h: Handled) -> String {
        h.done.expect("done").expect_err("error").to_string()
    }

    #[test]
    fn simple_ack_completes_with_empty_data() {
        // 0x20 SimpleAck, invoke 7, service choice 15 (writeProperty).
        let mut t = Transaction::new(7);
        let h = t.handle(&[0x20, 0x07, 0x0F]);
        assert_eq!(h.done.unwrap().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn complex_ack_returns_service_data() {
        // 0x30 ComplexAck (unsegmented), invoke 9, service choice 12 (readProperty),
        // then service data: [0] object-id (AI,1), [1] property 85.
        let mut t = Transaction::new(9);
        let h = t.handle(&[0x30, 0x09, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]);
        assert_eq!(
            h.done.unwrap().unwrap(),
            vec![0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]
        );
    }

    #[test]
    fn other_invoke_ids_are_ignored() {
        let mut t = Transaction::new(9);
        let h = t.handle(&[0x20, 0x08, 0x0F]);
        assert!(h.done.is_none() && h.send.is_none());
    }

    #[test]
    fn error_pdu_names_class_and_code() {
        // 0x50 Error, invoke 4, service choice 15 (writeProperty),
        // 0x91 0x02 = Enumerated 2 (error-class property),
        // 0x91 0x28 = Enumerated 40 (error-code write-access-denied).
        let mut t = Transaction::new(4);
        let text = err_text(t.handle(&[0x50, 0x04, 0x0F, 0x91, 0x02, 0x91, 0x28]));
        assert!(text.contains("property"), "{text}");
        assert!(text.contains("write-access-denied"), "{text}");
    }

    #[test]
    fn error_pdu_with_unknown_service_choice_still_surfaces() {
        // Service choice 0x7F is not a ConfirmedServiceChoice bacnet-rs knows.
        let mut t = Transaction::new(1);
        let text = err_text(t.handle(&[0x50, 0x01, 0x7F, 0x91, 0x01, 0x91, 0x1F]));
        assert!(text.contains("unknown-object"), "{text}");
    }

    #[test]
    fn reject_pdu_names_reason() {
        // 0x60 Reject, invoke 2, reason 9 = unrecognized-service.
        let mut t = Transaction::new(2);
        let text = err_text(t.handle(&[0x60, 0x02, 0x09]));
        assert!(text.contains("unrecognized-service"), "{text}");
    }

    #[test]
    fn abort_pdu_names_reason() {
        // 0x71 Abort (server=1), invoke 3, reason 4 = segmentation-not-supported.
        let mut t = Transaction::new(3);
        let text = err_text(t.handle(&[0x71, 0x03, 0x04]));
        assert!(text.contains("segmentation-not-supported"), "{text}");
    }

    #[test]
    fn segmented_complex_ack_is_reassembled_and_acked() {
        let mut t = Transaction::new(5);
        // 0x3C ComplexAck, segmented + more-follows; invoke 5; sequence 0;
        // proposed window 4; service choice 26 (readRange); data AA BB.
        let h = t.handle(&[0x3C, 0x05, 0x00, 0x04, 0x1A, 0xAA, 0xBB]);
        assert!(h.done.is_none());
        // SegmentAck: 0x40 (positive, from client), invoke 5, sequence 0, window 4.
        assert_eq!(h.send.unwrap(), vec![0x40, 0x05, 0x00, 0x04]);

        // 0x38 segmented, no more-follows; sequence 1; data CC.
        let h = t.handle(&[0x38, 0x05, 0x01, 0x04, 0x1A, 0xCC]);
        assert_eq!(h.send.unwrap(), vec![0x40, 0x05, 0x01, 0x04]);
        assert_eq!(h.done.unwrap().unwrap(), vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn out_of_order_segment_gets_negative_ack() {
        let mut t = Transaction::new(5);
        t.handle(&[0x3C, 0x05, 0x00, 0x04, 0x1A, 0xAA]);
        // Sequence 2 arrives while 1 is expected: NAK (0x42) for last good seq 0.
        let h = t.handle(&[0x3C, 0x05, 0x02, 0x04, 0x1A, 0xEE]);
        assert!(h.done.is_none());
        assert_eq!(h.send.unwrap(), vec![0x42, 0x05, 0x00, 0x04]);
    }

    #[test]
    fn frame_helpers_round_trip() {
        // BVLC 0x81 0x0A length 8, NPDU 01 04 (expecting reply), APDU 20 01 0F.
        let frame = unicast_frame(&[0x20, 0x01, 0x0F], true);
        assert_eq!(frame, vec![0x81, 0x0A, 0x00, 0x09, 0x01, 0x04, 0x20, 0x01, 0x0F]);
        assert_eq!(apdu_of_frame(&frame).unwrap(), &[0x20, 0x01, 0x0F]);

        // Forwarded-NPDU: 81 04 len, 6-byte origin (192.168.1.5:47808), NPDU 01 00, APDU 10 08.
        let fwd = [
            0x81, 0x04, 0x00, 0x0E, 192, 168, 1, 5, 0xBA, 0xC0, 0x01, 0x00, 0x10, 0x08,
        ];
        assert_eq!(apdu_of_frame(&fwd).unwrap(), &[0x10, 0x08]);
    }
}
