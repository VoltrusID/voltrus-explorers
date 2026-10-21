//! End-to-end tests of SZL reads, block listing, upload and delete: the client against the
//! simulator, and requests captured from real clients against the simulator.

use mt_protocol::ProtocolClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::capture::*;
use crate::types::*;
use crate::{S7Client, S7Server};

fn cfg(port: u16) -> S7Config {
    S7Config { id: "t".into(), host: "127.0.0.1".into(), port, rack: 0, slot: 0, poll_interval_ms: 1000 }
}

fn tag(address: &str) -> S7Tag {
    S7Tag::parse_address(address).unwrap()
}

/// A simulator with the given tags and a client connected to it.
async fn connect(seed: Vec<(S7Tag, S7Value)>, pdu_size: u16) -> (S7Server, S7Client) {
    let server = S7Server::start_with_pdu_size(0, seed, pdu_size).await.unwrap();
    let client = S7Client::connect(cfg(server.port())).await.unwrap();
    (server, client)
}

// ── SZL: CPU identity, mode, diagnostic buffer ──

#[tokio::test]
async fn cpu_info_mode_and_diagnostic_buffer_from_the_simulator() {
    let (server, client) = connect(vec![], 480).await;

    let cpu = client.read_cpu_info().await.unwrap();
    assert_eq!(cpu.module_type, "S7 Simulator");
    assert_eq!(cpu.module_name, "S7-1500 SIM");
    assert_eq!(cpu.as_name, "SIMULATOR");
    assert_eq!(cpu.serial_number, "SIM-000001");
    assert_eq!(cpu.firmware_version, "V1.0.0");
    assert_eq!(cpu.order_code, "SIM-S7-0000-0000");
    assert_eq!(cpu.copyright, "mt-s7 simulator");

    assert_eq!(client.read_plc_mode().await.unwrap().mode, "RUN");

    let diag = client.read_diag_buffer().await.unwrap();
    assert_eq!(diag.entry_count, 12);
    assert_eq!(diag.entries.len(), 12);
    assert_eq!(diag.entries[0].event_number, 0x4302);
    assert_eq!(diag.entries[0].timestamp, "2024-03-18 20:00:05.120");
    assert_eq!(diag.entries[0].event_text, "Mode transition from STARTUP to RUN");
    assert_eq!((diag.entries[1].priority_class, diag.entries[1].ob_number), (27, 100));
    assert_eq!(diag.entries[11].timestamp, "2024-03-18 15:22:07.120");
    // Newest first.
    let times: Vec<&str> = diag.entries.iter().map(|e| e.timestamp.as_str()).collect();
    assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");

    client.disconnect().await.unwrap();
    server.stop().await;
}

/// At the S7-300's PDU size the identity list (SZL 0x001C) and the diagnostic buffer need two
/// parts each, as in the capture; the client fetches them with next requests and the result is
/// the same as in one part.
#[tokio::test]
async fn multi_part_szl_answers_give_the_same_result() {
    let (big_server, big) = connect(vec![], 480).await;
    let (small_server, small) = connect(vec![], 240).await;

    assert_eq!(json(&small.read_cpu_info().await.unwrap()), json(&big.read_cpu_info().await.unwrap()));
    assert_eq!(json(&small.read_diag_buffer().await.unwrap()), json(&big.read_diag_buffer().await.unwrap()));
    assert_eq!(small.read_plc_mode().await.unwrap().mode, "RUN");

    for (server, client) in [(big_server, big), (small_server, small)] {
        client.disconnect().await.unwrap();
        server.stop().await;
    }
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

// ── Block list ──

#[tokio::test]
async fn block_list_has_blocks_of_every_type() {
    let seed = vec![
        (tag("DB1.DBD0"), S7Value::DWord(1)),
        (tag("DB2.DBD0"), S7Value::DWord(2)),
        (tag("MW100"), S7Value::Word(5)), // a marker is not a block
    ];
    let (server, client) = connect(seed, 480).await;

    let blocks = client.list_blocks().await.unwrap();
    let listed: Vec<(&str, u16)> = blocks.iter().map(|b| (b.block_type.as_str(), b.block_number)).collect();
    assert_eq!(
        listed,
        [("OB", 1), ("OB", 100), ("FB", 1), ("FC", 1), ("DB", 1), ("DB", 2), ("SDB", 0), ("SFC", 20), ("SFB", 4)]
    );
    let db1 = blocks.iter().find(|b| b.block_type == "DB" && b.block_number == 1).unwrap();
    assert_eq!((db1.language.as_str(), db1.flags), ("DB", 0x22));
    let ob1 = &blocks[0];
    assert_eq!(ob1.language, "FUP");
    assert_eq!(blocks.iter().find(|b| b.block_type == "SDB").unwrap().language, "SDB");

    client.disconnect().await.unwrap();
    server.stop().await;
}

#[tokio::test]
async fn block_list_in_several_parts() {
    let seed: Vec<(S7Tag, S7Value)> = (1..=120).map(|n| (tag(&format!("DB{n}.DBW0")), S7Value::Word(n))).collect();
    // 53 numbers fit one part at 240 bytes: three parts for the DBs.
    let (server, client) = connect(seed, 240).await;

    let blocks = client.list_blocks().await.unwrap();
    let dbs: Vec<u16> = blocks.iter().filter(|b| b.block_type == "DB").map(|b| b.block_number).collect();
    assert_eq!(dbs, (1..=120).collect::<Vec<u16>>());
    assert_eq!(blocks.len(), 120 + 7);

    client.disconnect().await.unwrap();
    server.stop().await;
}

// ── Upload ──

#[tokio::test]
async fn upload_of_a_db_gives_the_block_in_load_memory_layout() {
    let seed = vec![(tag("DB3.DBW0"), S7Value::Word(0x1234)), (tag("DB3.DBB599"), S7Value::Byte(7))];
    // 36 + 600 + 56 = 692 bytes: three parts of 222 at PDU size 240.
    let (server, client) = connect(seed, 240).await;

    let block = client.upload_block(&BlockType::DB, 3).await.unwrap();
    assert_eq!(block.len(), 692);
    assert_eq!(block[5], 0x0A, "sub block type DB");
    assert_eq!(u16::from_be_bytes([block[6], block[7]]), 3);
    assert_eq!(u32::from_be_bytes([block[8], block[9], block[10], block[11]]), 692);
    assert_eq!(u16::from_be_bytes([block[34], block[35]]), 600);
    assert_eq!(block[36..38], [0x12, 0x34]);
    assert_eq!(block[36 + 599], 7);
    assert_eq!(&block[block.len() - 20..block.len() - 12], b"DB3\0\0\0\0\0");

    // The same at a bigger PDU, in fewer parts.
    let (big_server, big) = connect(vec![(tag("DB3.DBW0"), S7Value::Word(0x1234)), (tag("DB3.DBB599"), S7Value::Byte(7))], 960).await;
    assert_eq!(big.upload_block(&BlockType::DB, 3).await.unwrap(), block);

    for (server, client) in [(server, client), (big_server, big)] {
        client.disconnect().await.unwrap();
        server.stop().await;
    }
}

#[tokio::test]
async fn upload_of_other_blocks_and_of_a_missing_one() {
    let (server, client) = connect(vec![(tag("DB1.DBB0"), S7Value::Byte(1))], 480).await;

    let ob = client.upload_block(&BlockType::OB, 1).await.unwrap();
    assert_eq!(ob.len(), 36 + 2 + 56);
    assert_eq!(ob[5], 0x08, "sub block type OB");
    assert_eq!(ob[4], 0x03, "language FUP");
    assert_eq!(ob[36..38], [0x65, 0x00]);
    let sfc = client.upload_block(&BlockType::SFC, 20).await.unwrap();
    assert_eq!(sfc[5], 0x0D);

    let error = client.upload_block(&BlockType::DB, 99).await.unwrap_err().to_string();
    assert!(error.contains("DB99") && error.contains("0xD209"), "{error}");
    let error = client.upload_block(&BlockType::FC, 2).await.unwrap_err().to_string();
    assert!(error.contains("FC2"), "{error}");
    // The connection is still good.
    assert_eq!(client.read_plc_mode().await.unwrap().mode, "RUN");

    client.disconnect().await.unwrap();
    server.stop().await;
}

/// The simulator allows 8 uploads at a time, like a CPU with a few upload slots: a client that
/// did not end its uploads would be refused after 8.
#[tokio::test]
async fn every_upload_is_ended() {
    let (server, client) = connect(vec![], 480).await;
    for _ in 0..20 {
        client.upload_block(&BlockType::OB, 100).await.unwrap();
    }
    client.disconnect().await.unwrap();
    server.stop().await;
}

// ── Delete ──

#[tokio::test]
async fn delete_removes_a_db_and_refuses_the_rest() {
    let seed = vec![
        (tag("DB1.DBW0"), S7Value::Word(1)),
        (tag("DB2.DBW0"), S7Value::Word(2)),
        (tag("MW10"), S7Value::Word(9)),
    ];
    let (server, client) = connect(seed, 480).await;

    client.delete_block(&BlockType::DB, 2).await.unwrap();
    let blocks = client.list_blocks().await.unwrap();
    let dbs: Vec<u16> = blocks.iter().filter(|b| b.block_type == "DB").map(|b| b.block_number).collect();
    assert_eq!(dbs, [1]);
    // The data is gone with it, and what is left is untouched.
    let error = client.read(&[tag("DB2.DBW0")]).await.unwrap_err().to_string();
    assert!(error.contains("does not exist"), "{error}");
    assert_eq!(client.read(&[tag("DB1.DBW0"), tag("MW10")]).await.unwrap(), [S7Value::Word(1), S7Value::Word(9)]);
    let tags: Vec<String> = server.snapshot().await.iter().map(|(t, _)| t.label()).collect();
    assert!(!tags.iter().any(|t| t.starts_with("DB2.")), "{tags:?}");

    // A block that is not there, and the fixed blocks.
    let error = client.delete_block(&BlockType::DB, 2).await.unwrap_err().to_string();
    assert!(error.contains("DB2") && error.contains("0xD209"), "{error}");
    let error = client.delete_block(&BlockType::OB, 1).await.unwrap_err().to_string();
    assert!(error.contains("write-protected"), "{error}");
    let error = client.delete_block(&BlockType::FB, 77).await.unwrap_err().to_string();
    assert!(error.contains("0xD209"), "{error}");
    assert_eq!(client.list_blocks().await.unwrap().len(), 1 + 7);

    client.delete_block(&BlockType::DB, 1).await.unwrap();
    assert_eq!(client.list_blocks().await.unwrap().len(), 7);

    client.disconnect().await.unwrap();
    server.stop().await;
}

// ── Captured requests against the simulator ──

async fn send_tpkt(stream: &mut TcpStream, payload: &[u8]) {
    let mut packet = vec![0x03, 0x00];
    packet.extend_from_slice(&((payload.len() + 4) as u16).to_be_bytes());
    packet.extend_from_slice(payload);
    stream.write_all(&packet).await.unwrap();
}

async fn recv_tpkt(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.unwrap();
    let mut payload = vec![0u8; u16::from_be_bytes([header[2], header[3]]) as usize - 4];
    stream.read_exact(&mut payload).await.unwrap();
    payload
}

/// Send one S7 PDU in a COTP data TPDU and return the S7 PDU of the answer.
async fn raw_exchange(stream: &mut TcpStream, s7: &[u8]) -> Vec<u8> {
    let mut payload = vec![0x02, 0xf0, 0x80];
    payload.extend_from_slice(s7);
    send_tpkt(stream, &payload).await;
    recv_tpkt(stream).await[3..].to_vec()
}

/// Connect with the capture's Connection Request and Setup Communication (240 bytes PDU).
async fn raw_connect(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    send_tpkt(&mut stream, &hex("11 e0 00 00 00 01 00 c1 02 01 00 c2 02 01 02 c0 01 09")).await;
    assert_eq!(recv_tpkt(&mut stream).await[1], 0xd0, "connection confirm");
    let setup = raw_exchange(&mut stream, &hex("32 01 00 00 ff ff 00 08 00 00 f0 00 00 01 00 01 01 e0")).await;
    assert_eq!(setup[12], 0xf0);
    stream
}

/// The requests of `s7comm_reading_plc_status.pcap`, sent as they are. The simulator's answers
/// have the telegram layout of the CPU's: for the unknown SZL 0x0C91 they are the same bytes.
#[tokio::test]
async fn captured_szl_requests_get_answers_in_the_cpus_layout() {
    let server = S7Server::start_with_pdu_size(0, vec![], 240).await.unwrap();
    let mut stream = raw_connect(server.port()).await;

    // Packets 41 and 42: SZL 0x0C91 index 0x7FFF is not there.
    assert_eq!(raw_exchange(&mut stream, &hex(STATUS_41)).await, hex(STATUS_42));

    // Packets 86 and 87: the mode. Same lengths, parameters, data header and SZL header; the CPU's
    // sequence number (index 17) and its record differ.
    let mode = raw_exchange(&mut stream, &hex(STATUS_86)).await;
    let captured = hex(STATUS_87);
    assert_eq!(mode.len(), captured.len());
    assert_eq!(mode[..17], captured[..17]);
    assert_eq!(mode[18..34], captured[18..34]);
    // Event id and the time stamp differ; byte 3 (RUN) and the reserved bytes agree.
    assert_eq!(mode[36..46], captured[36..46]);

    // Packets 58 and 59: module identification, index 0. The simulator has three records.
    let module = raw_exchange(&mut stream, &hex(STATUS_58)).await;
    let captured = hex(STATUS_59);
    assert_eq!(module[18..24], captured[18..24], "parameters, return code and transport size");
    assert_eq!(u16::from_be_bytes([module[24], module[25]]), 8 + 3 * 28, "data length");
    assert_eq!(module[26..32], captured[26..32], "SZL id, index and record length");
    assert_eq!(module[32..34], [0x00, 0x03], "three records");

    server.stop().await;
}

/// `s7comm_program_blocklist_onlineview.pcap`: list all (packets 58 and 59) and list the DBs
/// (packets 61 and 62). With one DB in the image, the DB answer is the captured bytes.
#[tokio::test]
async fn captured_block_list_requests_get_answers_in_the_cpus_layout() {
    let server = S7Server::start_with_pdu_size(0, vec![(tag("DB1.DBB0"), S7Value::Byte(0))], 240).await.unwrap();
    let mut stream = raw_connect(server.port()).await;

    let all = raw_exchange(&mut stream, &hex(BLOCKS_58)).await;
    let captured = hex(BLOCKS_59);
    assert_eq!(all.len(), captured.len());
    assert_eq!(all[..26], captured[..26], "headers, parameters (sequence 0) and data header");
    let codes = |pdu: &[u8]| pdu[26..].chunks(4).map(|item| item[..2].to_vec()).collect::<Vec<_>>();
    assert_eq!(codes(&all), codes(&captured));
    // OB (one in the capture, two here) and DB1 are listed with the same type codes; the DB count
    // is the capture's.
    assert_eq!(all[26..28], captured[26..28]);
    assert_eq!(all[38..42], captured[38..42]);

    assert_eq!(raw_exchange(&mut stream, &hex(BLOCKS_61)).await, hex(BLOCKS_62));

    server.stop().await;
}

/// A user data request the simulator does not offer, a PI service that is not a delete and an
/// upload that was never started get errors, not silence.
#[tokio::test]
async fn unsupported_requests_are_refused() {
    let server = S7Server::start(0, vec![]).await.unwrap();
    let mut stream = raw_connect(server.port()).await;

    // Clock read (function group 7): information function unavailable.
    let clock = crate::framing::build_request(1, 0x07, 0x01, 0, false, &[0x0A, 0, 0, 0]);
    let answer = raw_exchange(&mut stream, &clock).await;
    let error = crate::framing::parse_response(&answer, 0x07, 0x01).err().unwrap();
    assert!(error.contains("0xD402"), "{error}");

    // PLC stop, the PI service `P_PROGRAM` (snap7 `opPlcStop`).
    let mut stop = vec![0x29, 0, 0, 0, 0, 0, 0x09];
    stop.extend_from_slice(b"P_PROGRAM");
    let answer = raw_exchange(&mut stream, &crate::framing::job_pdu(2, &stop, &[])).await;
    assert_eq!(answer[1], 0x03);
    assert_ne!(answer[10..12], [0x00, 0x00]);

    // Upload with an id nobody got.
    let answer = raw_exchange(&mut stream, &crate::blocks::build_upload_request(3, 99)).await;
    assert_eq!(answer[10..12], [0x84, 0x01]);

    server.stop().await;
}
