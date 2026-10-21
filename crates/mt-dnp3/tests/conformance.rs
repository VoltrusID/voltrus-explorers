//! Conformance regression tests against the exact wire bytes from the
//! IEEE 1815-2012 reference frame and the field report that exposed the
//! v1.2.2 data-link defect (bad CRC, missing header CRC, user data on
//! link service frames).

use mt_dnp3::{Dnp3Client, Dnp3Config, Dnp3Frame, Dnp3Point, crc16};
use mt_protocol::ProtocolClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Master RESET LINK STATES to outstation 4 from master 1, as built by a
/// conformant stack (control 0xC0, LENGTH 5, IEEE 1815 CRC-16).
const RESET_LINK_STATES: [u8; 10] =
    [0x05, 0x64, 0x05, 0xC0, 0x04, 0x00, 0x01, 0x00, 0x1C, 0x4D];

/// Secondary ACK from outstation 4 to master 1 — the exact bytes a
/// conformant outstation returns to RESET LINK STATES.
const LINK_ACK: [u8; 10] = [0x05, 0x64, 0x05, 0x00, 0x01, 0x00, 0x04, 0x00, 0x95, 0x9E];

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
async fn read_link_frame(
    sock: &mut tokio::net::TcpStream,
) -> Result<Dnp3Frame, std::io::Error> {
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

        // Reply with a minimal RESPONSE ASDU in this stack's layout
        // (function, application control, IIN) prefixed by the transport
        // header.
        let asdu: [u8; 5] = [0xC0, 0x81, 0xC0, 0x00, 0x00];
        let resp = link_frame(0x44, 1, 4, &asdu); // outstation data
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
        variation: 4,
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
