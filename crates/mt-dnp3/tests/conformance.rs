//! Conformance tests against known-good DNP3 byte sequences.
//!
//! Sources:
//! - IEEE 1815-2012 reference frame and the field report behind the v1.2.2 data-link fix
//!   (RESET LINK STATES / link ACK between master 1 and outstation 4).
//! - "DNP3 Quick Reference" (Chipkin Automation Systems, TD-QuickReference-KC-4, 11 Nov 2002),
//!   page 5, "DNP3 Exchange Samples": captured exchanges between a master at address 1024 and an
//!   outstation at address 1: reset link, integrity poll with a response of events and static
//!   data, link and application confirms, clearing the restart IIN, setting the time, null
//!   responses. Those frames carry CRCs from a third-party stack, so decoding them with
//!   `Dnp3Frame::decode` also checks this crate's CRC-16 and block layout independently.
//! - CROB SELECT/OPERATE frames are assembled by hand from the g12v1 object layout in
//!   IEEE 1815-2012 (control code, count, on time, off time, status): they are not captured from a
//!   device.

use std::sync::Arc;

use mt_dnp3::application::{
    AsduHeader, FunctionCode, build_confirm, build_event_scan_request, build_integrity_poll_request,
    build_time_sync_request_at,
};
use mt_dnp3::objects::parse_points;
use mt_dnp3::{
    ControlByte, Dnp3Client, Dnp3Config, Dnp3Frame, Dnp3Point, OutstationConfig, OutstationState,
    crc16, start_outstation_server,
};
use mt_protocol::ProtocolClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Master RESET LINK STATES to outstation 4 from master 1, as built by a
/// conformant stack (control 0xC0, LENGTH 5, IEEE 1815 CRC-16).
const RESET_LINK_STATES: [u8; 10] =
    [0x05, 0x64, 0x05, 0xC0, 0x04, 0x00, 0x01, 0x00, 0x1C, 0x4D];

/// Secondary ACK from outstation 4 to master 1 — the exact bytes a
/// conformant outstation returns to RESET LINK STATES.
const LINK_ACK: [u8; 10] = [0x05, 0x64, 0x05, 0x00, 0x01, 0x00, 0x04, 0x00, 0x95, 0x9E];

// ── Frames from the DNP3 Quick Reference (master 1024, outstation 1) ──

const QR_RESET_LINK: &str = "05 64 05 C0 01 00 00 04 E9 21";
const QR_ACK: &str = "05 64 05 00 00 04 01 00 19 A6";
/// Master ACK of the outstation's confirmed user data (control 0x80: DIR=1, PRM=0, ACK).
const QR_MASTER_ACK: &str = "05 64 05 80 01 00 00 04 53 11";
/// Integrity poll sent as CONFIRMED USER DATA (control 0xF3), transport C0, application C3 01.
const QR_INTEGRITY_POLL: &str =
    "05 64 14 F3 01 00 00 04 0A 3B C0 C3 01 3C 02 06 3C 03 06 3C 04 06 3C 01 06 9A 12";
/// Response: IIN 96 00 (restart, need time, class 1 and 2 events), 4 binary input events,
/// 2 analog input events, 4 binary inputs and 2 analog inputs, in 5 data blocks.
const QR_RESPONSE: &str = "05 64 53 73 00 04 01 00 03 FC \
    C1 E3 81 96 00 02 01 28 01 00 00 00 01 02 01 28 05 24 \
    01 00 01 00 01 02 01 28 01 00 02 00 01 02 01 28 B4 77 \
    01 00 03 00 01 20 02 28 01 00 00 00 01 00 00 20 A5 25 \
    02 28 01 00 01 00 01 00 00 01 01 01 00 00 03 00 2F AC \
    00 1E 02 01 00 00 01 00 01 00 00 01 00 00 16 ED";
/// Application layer CONFIRM of that response (sequence 3).
const QR_APP_CONFIRM: &str = "05 64 08 C4 01 00 00 04 A4 CF C1 C3 00 20 3F";
/// WRITE g80v1 index 7 = 0: clear the DEVICE_RESTART IIN.
const QR_CLEAR_RESTART: &str =
    "05 64 0E C4 01 00 00 04 7D A4 C0 C4 02 50 01 00 07 07 00 64 11";
const QR_NULL_RESPONSE_RESTART_CLEARED: &str = "05 64 0A 44 00 04 01 00 59 5E C2 C4 81 10 00 93 AD";
/// WRITE g50v1 time and date.
const QR_SET_TIME: &str =
    "05 64 12 C4 01 00 00 04 0E 0B C0 C5 02 32 01 07 01 F8 B8 6C AA F0 00 98 98";
const QR_NULL_RESPONSE_TIME_SET: &str = "05 64 0A 44 00 04 01 00 59 5E C3 C5 81 00 00 55 93";

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

fn decode(s: &str) -> Dnp3Frame {
    Dnp3Frame::decode(&hex(s)).expect("vector must decode with valid CRCs")
}

/// Build a conformant link frame with header CRC and 16-octet data blocks.
fn link_frame(control: u8, dst: u16, src: u16, user_data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x05, 0x64, (5 + user_data.len()) as u8, control];
    out.extend_from_slice(&dst.to_le_bytes());
    out.extend_from_slice(&src.to_le_bytes());
    let header_crc = crc16(&out);
    out.extend_from_slice(&header_crc.to_le_bytes());
    for chunk in user_data.chunks(16) {
        let start = out.len();
        out.extend_from_slice(chunk);
        let block_crc = crc16(&out[start..]);
        out.extend_from_slice(&block_crc.to_le_bytes());
    }
    out
}

/// Read one complete link frame of the given LENGTH from the socket.
async fn read_link_frame(sock: &mut TcpStream) -> Result<Dnp3Frame, std::io::Error> {
    let mut header = [0u8; 3];
    sock.read_exact(&mut header).await?;
    assert_eq!(&header[0..2], &[0x05, 0x64], "invalid start octets");
    let length = header[2] as usize;
    assert!(length >= 5, "LENGTH {length} below minimum");
    let user_len = length - 5;
    let total = 10 + user_len + 2 * user_len.div_ceil(16);
    let mut frame = vec![0u8; total];
    frame[0..3].copy_from_slice(&header);
    sock.read_exact(&mut frame[3..]).await?;
    Dnp3Frame::decode(&frame)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

// ── Static checks on the vectors ──

#[test]
fn link_frames_match_the_quick_reference_bytes() {
    // Master 1024 -> outstation 1.
    let reset = Dnp3Frame {
        link_control: ControlByte::master_link(ControlByte::RESET_LINK_STATES),
        destination: 1,
        source: 1024,
        data: vec![],
    };
    assert_eq!(reset.encode(), hex(QR_RESET_LINK));

    let ack = Dnp3Frame {
        link_control: ControlByte::outstation_link(ControlByte::ACK),
        destination: 1024,
        source: 1,
        data: vec![],
    };
    assert_eq!(ack.encode(), hex(QR_ACK));

    // The master's ACK of confirmed user data: DIR=1, PRM=0, function ACK (0x80).
    let master_ack = Dnp3Frame {
        link_control: ControlByte { dir: true, prm: false, fcb: false, fcv: false, function: ControlByte::ACK },
        destination: 1,
        source: 1024,
        data: vec![],
    };
    assert_eq!(master_ack.encode(), hex(QR_MASTER_ACK));
}

#[test]
fn integrity_poll_matches_the_quick_reference_request() {
    let frame = decode(QR_INTEGRITY_POLL);
    // CONFIRMED USER DATA with FCB and FCV set, as that sample master sent it.
    assert_eq!(frame.link_control.to_byte(), 0xF3);
    assert_eq!((frame.destination, frame.source), (1, 1024));
    // Transport C0, then the application fragment: control C3 (FIR FIN seq 3), READ, and four
    // object headers g60v2, g60v3, g60v4, g60v1 with qualifier 06.
    assert_eq!(frame.data[0], 0xC0);
    assert_eq!(frame.data[1..], build_integrity_poll_request(3)[..]);
    // Same payload, our link control, re-encodes to the same frame modulo control octet and CRC.
    assert_eq!(frame.encode(), hex(QR_INTEGRITY_POLL));
}

#[test]
fn class_poll_request_is_one_header_with_qualifier_06() {
    // The same object header the sample integrity poll uses for class 1 (3C 02 06).
    assert_eq!(build_event_scan_request(0, 1), [0xC0, 0x01, 0x3C, 0x02, 0x06]);
}

#[test]
fn response_vector_decodes_and_every_object_is_parsed() {
    let frame = decode(QR_RESPONSE);
    assert_eq!(frame.link_control.to_byte(), 0x73); // confirmed user data from the outstation
    assert_eq!((frame.destination, frame.source), (1024, 1));
    assert_eq!(frame.data.len(), 78);

    // Transport C1: FIR, FIN, sequence 1.
    assert_eq!(frame.data[0], 0xC1);
    let (header, objects) = AsduHeader::decode_response(&frame.data[1..]).unwrap();
    assert_eq!(header.function, FunctionCode::Response);
    assert_eq!(header.sequence(), 3);
    assert!(header.control.fir && header.control.fin && header.control.con && !header.control.uns);
    // IIN 96 00: class 1 events, class 2 events, need time, device restart.
    assert!(header.iin.class_1_events && header.iin.class_2_events && !header.iin.class_3_events);
    assert!(header.iin.need_time && header.iin.device_restart);
    assert!(!header.iin.device_trouble && !header.iin.local_control && !header.iin.all_stations);

    let (points, issue) = parse_points(objects);
    assert_eq!(issue, None, "every object in the response must be sized correctly");
    let events: Vec<_> = points.iter().filter(|p| p.event).collect();
    let statics: Vec<_> = points.iter().filter(|p| !p.event).collect();
    // g2v1 binary events at 0..3 and g32v2 analog events at 0..1.
    assert_eq!(events.iter().filter(|p| p.group == 1).map(|p| p.index).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(events.iter().filter(|p| p.group == 30).map(|p| p.index).collect::<Vec<_>>(), [0, 1]);
    // g1v1 packed binary inputs 0..3 and g30v2 analog inputs 0..1.
    assert_eq!(statics.iter().filter(|p| p.group == 1).map(|p| p.index).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(statics.iter().filter(|p| p.group == 30).map(|p| p.index).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(points.len(), 12);
}

#[test]
fn application_confirm_matches_the_quick_reference() {
    let frame = decode(QR_APP_CONFIRM);
    assert_eq!(frame.link_control.to_byte(), 0xC4);
    // Transport C1, application control C3 (FIR FIN, sequence 3, no CON/UNS), function CONFIRM.
    assert_eq!(frame.data, [0xC1, 0xC3, 0x00]);
    assert_eq!(build_confirm(3, false), [0xC3, 0x00]);
}

#[test]
fn write_and_null_response_vectors() {
    let clear = decode(QR_CLEAR_RESTART);
    assert_eq!(clear.data, [0xC0, 0xC4, 0x02, 0x50, 0x01, 0x00, 0x07, 0x07, 0x00]);

    let set_time = decode(QR_SET_TIME);
    // g50v1 (0x32), qualifier 07, count 1, 48-bit little-endian milliseconds.
    assert_eq!(set_time.data[1..], build_time_sync_request_at(5, 0x00F0_AA6C_B8F8)[..]);

    for (vector, seq, need_time) in [(QR_NULL_RESPONSE_RESTART_CLEARED, 4, true), (QR_NULL_RESPONSE_TIME_SET, 5, false)] {
        let frame = decode(vector);
        let (header, objects) = AsduHeader::decode_response(&frame.data[1..]).unwrap();
        assert_eq!(header.function, FunctionCode::Response);
        assert_eq!(header.sequence(), seq);
        assert_eq!(header.iin.need_time, need_time);
        assert!(!header.iin.device_restart);
        assert!(objects.is_empty());
    }
}

// ── The master against the Quick Reference exchange ──

#[tokio::test]
async fn master_replays_the_quick_reference_exchange() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();

        // Reset link states and its ACK, byte for byte.
        let mut reset = [0u8; 10];
        sock.read_exact(&mut reset).await.unwrap();
        assert_eq!(reset.to_vec(), hex(QR_RESET_LINK));
        sock.write_all(&hex(QR_ACK)).await.unwrap();

        // Three polls get an empty response so the master's application sequence reaches 3,
        // the sequence the captured response answers.
        for seq in 0..3u8 {
            let request = read_link_frame(&mut sock).await.unwrap();
            assert_eq!(request.link_control.to_byte(), 0xC4);
            assert_eq!(request.data[1..], build_integrity_poll_request(seq)[..]);
            let null = [0xC0, 0xC0 | seq, 0x81, 0x00, 0x00];
            sock.write_all(&link_frame(0x44, 1024, 1, &null)).await.unwrap();
        }

        // The fourth poll is answered with the captured response (CONFIRMED USER DATA, CON set).
        let request = read_link_frame(&mut sock).await.unwrap();
        assert_eq!(request.data[1..], build_integrity_poll_request(3)[..]);
        sock.write_all(&hex(QR_RESPONSE)).await.unwrap();

        // The master must ACK the confirmed user data at the link layer...
        let link_ack = read_link_frame(&mut sock).await.unwrap();
        assert_eq!(link_ack.encode(), hex(QR_MASTER_ACK));
        // ...and CONFIRM the response (CON was set) at the application layer.
        let confirm = read_link_frame(&mut sock).await.unwrap();
        assert_eq!(confirm.link_control.to_byte(), 0xC4);
        assert_eq!(confirm.data[1..], [0xC3, 0x00]);
    });

    let client = Dnp3Client::connect(Dnp3Config {
        id: "quick-reference".into(),
        host: "127.0.0.1".into(),
        port,
        master_address: 1024,
        outstation_address: 1,
        poll_interval_ms: 100,
    })
    .await
    .expect("connect must succeed");
    let points = [
        Dnp3Point { name: "bi0".into(), group: 1, variation: 1, index: 0, scale: 1.0 },
        Dnp3Point { name: "ai1".into(), group: 30, variation: 2, index: 1, scale: 1.0 },
        Dnp3Point { name: "ai7".into(), group: 30, variation: 2, index: 7, scale: 1.0 },
    ];
    for _ in 0..3 {
        client.read(&points).await.unwrap();
    }
    let values = client.read(&points).await.unwrap();
    assert_eq!(values[0].value, 0.0);
    assert_eq!(values[1].value, 0.0);
    assert!(values[2].value.is_nan(), "index 7 is not in the response");
    server.await.unwrap();
}

// ── The simulated outstation against raw frames ──

async fn raw_outstation(port: u16, outstation: u16, analog: u16, binary: u16) -> Arc<std::sync::Mutex<OutstationState>> {
    let (state, _log_rx, _shutdown) = start_outstation_server(OutstationConfig {
        port,
        bind_addr: "127.0.0.1".into(),
        master_address: 1024,
        outstation_address: outstation,
        analog_count: analog,
        binary_count: binary,
        counter_count: 2,
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    state
}

async fn send_raw(sock: &mut TcpStream, frame_hex: &str) {
    sock.write_all(&hex(frame_hex)).await.unwrap();
}

/// Send `apdu` (after a transport header of 0xC0) as unconfirmed user data and return the
/// response fragment (application bytes, transport header removed).
async fn request(sock: &mut TcpStream, apdu: &[u8]) -> Vec<u8> {
    let mut data = vec![0xC0];
    data.extend_from_slice(apdu);
    sock.write_all(&link_frame(0xC4, 1, 1024, &data)).await.unwrap();
    let frame = read_link_frame(sock).await.unwrap();
    assert_eq!(frame.link_control.to_byte(), 0x44, "unconfirmed user data from the outstation");
    assert_eq!((frame.destination, frame.source), (1024, 1));
    frame.data[1..].to_vec()
}

#[tokio::test]
async fn simulator_answers_the_quick_reference_exchange() {
    let state = raw_outstation(20040, 1, 4, 4).await;
    let mut sock = TcpStream::connect("127.0.0.1:20040").await.unwrap();

    // Reset link states: the outstation answers with the sample's ACK.
    send_raw(&mut sock, QR_RESET_LINK).await;
    let ack = read_link_frame(&mut sock).await.unwrap();
    assert_eq!(ack.encode(), hex(QR_ACK));

    // Integrity poll as in the sample (confirmed user data): ACK, then the response.
    send_raw(&mut sock, QR_INTEGRITY_POLL).await;
    let link_ack = read_link_frame(&mut sock).await.unwrap();
    assert_eq!(link_ack.link_control.to_byte(), 0x00);
    let response = read_link_frame(&mut sock).await.unwrap();
    let (header, objects) = AsduHeader::decode_response(&response.data[1..]).unwrap();
    assert_eq!(header.sequence(), 3);
    assert!(header.iin.need_time, "a new outstation has not been given the time");
    assert_eq!(parse_points(objects).1, None);

    // Clear the restart IIN and set the time with the sample frames.
    state.lock().unwrap().device_restart = true;
    send_raw(&mut sock, QR_CLEAR_RESTART).await;
    let null = read_link_frame(&mut sock).await.unwrap();
    let (header, _) = AsduHeader::decode_response(&null.data[1..]).unwrap();
    assert_eq!(header.sequence(), 4);
    assert!(!header.iin.device_restart && header.iin.need_time);
    assert!(!state.lock().unwrap().device_restart);

    send_raw(&mut sock, QR_SET_TIME).await;
    let null = read_link_frame(&mut sock).await.unwrap();
    let (header, _) = AsduHeader::decode_response(&null.data[1..]).unwrap();
    assert_eq!(header.sequence(), 5);
    assert!(!header.iin.need_time);
    assert!(state.lock().unwrap().time_synced);
}

#[tokio::test]
async fn simulator_ignores_frames_for_other_stations() {
    raw_outstation(20041, 1, 1, 1).await;
    let mut sock = TcpStream::connect("127.0.0.1:20041").await.unwrap();
    // Same request addressed to station 9, then to station 1: only the second is answered.
    let mut for_other = hex(QR_RESET_LINK);
    for_other[4] = 9;
    let crc = crc16(&for_other[..8]).to_le_bytes();
    for_other[8..10].copy_from_slice(&crc);
    sock.write_all(&for_other).await.unwrap();
    send_raw(&mut sock, QR_RESET_LINK).await;
    let ack = read_link_frame(&mut sock).await.unwrap();
    assert_eq!(ack.encode(), hex(QR_ACK));
}

#[tokio::test]
async fn simulator_runs_a_hand_assembled_crob_select_before_operate() {
    let state = raw_outstation(20042, 1, 1, 4).await;
    let mut sock = TcpStream::connect("127.0.0.1:20042").await.unwrap();

    // g12v1, qualifier 17 (1-octet count and index), count 1, index 0; control code 41 (CLOSE +
    // PULSE_ON), CROB count 1, on 100 ms, off 100 ms, status 0.
    let crob = "0C 01 17 01 00 41 01 64 00 00 00 64 00 00 00 00";
    let select = [&[0xC0, 0x03][..], &hex(crob)].concat();
    let operate = [&[0xC1, 0x04][..], &hex(crob)].concat();

    let reply = request(&mut sock, &select).await;
    let (header, objects) = AsduHeader::decode_response(&reply).unwrap();
    assert_eq!(header.sequence(), 0);
    // The object is echoed with status SUCCESS (0) in its last octet.
    assert_eq!(objects, hex(crob));
    assert!(!state.lock().unwrap().binary_values[0], "SELECT alone must not operate");

    let reply = request(&mut sock, &operate).await;
    let (_, objects) = AsduHeader::decode_response(&reply).unwrap();
    assert_eq!(objects, hex(crob));
    assert!(state.lock().unwrap().binary_values[0]);

    // OPERATE without a SELECT is refused with NO_SELECT (2) and changes nothing.
    state.lock().unwrap().binary_values[0] = false;
    let reply = request(&mut sock, &[&[0xC5, 0x04][..], &hex(crob)].concat()).await;
    let (_, objects) = AsduHeader::decode_response(&reply).unwrap();
    assert_eq!(objects.last(), Some(&2));
    assert!(!state.lock().unwrap().binary_values[0]);

    // An OPERATE whose sequence is not SELECT+1 is refused too.
    request(&mut sock, &[&[0xC6, 0x03][..], &hex(crob)].concat()).await;
    let reply = request(&mut sock, &[&[0xC9, 0x04][..], &hex(crob)].concat()).await;
    let (_, objects) = AsduHeader::decode_response(&reply).unwrap();
    assert_eq!(objects.last(), Some(&2));
}

#[tokio::test]
async fn simulator_refuses_unknown_function_codes_in_the_iin() {
    raw_outstation(20043, 1, 1, 1).await;
    let mut sock = TcpStream::connect("127.0.0.1:20043").await.unwrap();
    let reply = request(&mut sock, &[0xC2, 0x1E]).await; // function 0x1E: not implemented
    let (header, _) = AsduHeader::decode_response(&reply).unwrap();
    assert_eq!(header.sequence(), 2);
    assert!(header.iin.no_func_code_support);
}

// ── Handshake ──

#[tokio::test]
async fn master_handshake_uses_ieee_reference_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();

        // The first transmission must be exactly the conformant RESET
        // LINK STATES frame: LENGTH 5, control 0xC0, no user data.
        let mut reset = [0u8; 10];
        sock.read_exact(&mut reset).await.unwrap();
        assert_eq!(reset, RESET_LINK_STATES);

        // Answer with the exact ACK bytes a conformant outstation sends.
        sock.write_all(&LINK_ACK).await.unwrap();

        // The master's first application frame must be a conformant
        // user-data frame (valid header CRC, valid block CRCs).
        let poll = read_link_frame(&mut sock).await.unwrap();
        assert_eq!(poll.link_control.to_byte(), 0xC4); // master data, SEND/NO CONFIRM
        assert!(!poll.data.is_empty(), "integrity poll carries user data");
        assert_eq!(poll.data[1..], build_integrity_poll_request(0)[..]);

        // Reply with a minimal null RESPONSE: transport C0, application control C0 (FIR, FIN,
        // sequence 0), function 0x81, IIN1 and IIN2 clear.
        let user_data: [u8; 5] = [0xC0, 0xC0, 0x81, 0x00, 0x00];
        let resp = link_frame(0x44, 1, 4, &user_data); // outstation data
        sock.write_all(&resp).await.unwrap();
    });

    let config = Dnp3Config {
        id: "conformance".into(),
        host: addr.ip().to_string(),
        port: addr.port(),
        master_address: 1,
        outstation_address: 4,
        poll_interval_ms: 100,
    };
    let client = Dnp3Client::connect(config).await.expect("connect must succeed");
    let points = [Dnp3Point {
        name: "a0".into(),
        group: 30,
        variation: 5,
        index: 0,
        scale: 1.0,
    }];
    let values = client.read(&points).await.expect("integrity poll must succeed");
    // Minimal response carries no objects: requested points read as NaN.
    assert_eq!(values.len(), 1);
    assert!(values[0].value.is_nan());

    server.await.unwrap();
}

#[tokio::test]
async fn connect_fails_clearly_when_outstation_sends_garbage() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut reset = [0u8; 10];
        sock.read_exact(&mut reset).await.unwrap();
        // Reply with a frame whose header CRC is wrong: a conformant
        // receiver must discard it instead of hanging or accepting.
        let mut bad_ack = LINK_ACK;
        bad_ack[8] ^= 0xFF;
        sock.write_all(&bad_ack).await.unwrap();
    });

    let config = Dnp3Config {
        id: "conformance".into(),
        host: addr.ip().to_string(),
        port: addr.port(),
        master_address: 1,
        outstation_address: 4,
        poll_interval_ms: 100,
    };
    let result = Dnp3Client::connect(config).await;
    assert!(result.is_err(), "bad header CRC must fail the handshake");

    server.await.unwrap();
}
