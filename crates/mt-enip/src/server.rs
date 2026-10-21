use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::task::JoinHandle;

use crate::types::*;
use crate::packets::{build_send_unit_data, encode_cip_value, decode_cip_value};

// ── Public types ──

pub struct EnipServerState {
    pub tags: Mutex<BTreeMap<String, CipTagEntry>>,
    pub templates: Vec<CipTemplateDef>,
}

#[derive(Debug, Clone)]
pub struct CipTagEntry {
    pub data_type: CipDataType,
    pub element_count: u16,
    pub value: CipValue,
    pub template_id: Option<u16>,
}

// ── Start the server ──

pub async fn start_enip_server(
    config: EnipServerConfig,
    tags: Vec<CipTagDef>,
    templates: Vec<CipTemplateDef>,
) -> anyhow::Result<(
    Arc<EnipServerState>,
    mpsc::UnboundedReceiver<EnipServerLogEntry>,
    watch::Sender<bool>,
)> {
    let tag_map: BTreeMap<String, CipTagEntry> = tags
        .into_iter()
        .map(|t| {
            (
                t.name.clone(),
                CipTagEntry {
                    data_type: t.data_type,
                    element_count: t.element_count,
                    value: t.initial_value,
                    template_id: t.template_id,
                },
            )
        })
        .collect();

    let state = Arc::new(EnipServerState {
        tags: Mutex::new(tag_map),
        templates,
    });

    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let listener = TcpListener::bind(format!("{}:{}", config.bind_addr, config.port)).await?;

    let state_clone = state.clone();
    let log_tx_clone = log_tx.clone();

    tokio::spawn(async move {
        let mut shutdown = shutdown_rx;
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            let state = state_clone.clone();
                            let log_tx = log_tx_clone.clone();
                            tokio::spawn(handle_connection(stream, addr, state, log_tx));
                        }
                        Err(e) => {
                            let _ = log_tx_clone.send(EnipServerLogEntry {
                                timestamp: now_ms(),
                                service: "accept".into(),
                                tag_name: String::new(),
                                message: format!("Accept error: {e}"),
                            });
                        }
                    }
                }
                _ = shutdown.changed() => {
                    let _ = log_tx_clone.send(EnipServerLogEntry {
                        timestamp: now_ms(),
                        service: "server".into(),
                        tag_name: String::new(),
                        message: "Server shutting down".into(),
                    });
                    break;
                }
            }
        }
    });

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "server".into(),
        tag_name: String::new(),
        message: format!("ENIP server listening on {}:{}", config.bind_addr, config.port),
    });

    Ok((state, log_rx, shutdown_tx))
}

// ── Connection handler ──

async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: std::net::SocketAddr,
    state: Arc<EnipServerState>,
    log_tx: mpsc::UnboundedSender<EnipServerLogEntry>,
) {
    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "connect".into(),
        tag_name: String::new(),
        message: format!("Client connected from {addr}"),
    });

    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    // Shared writer so a spawned cyclic-IO producer can push unsolicited T→O
    // frames on the same socket as request replies.
    let writer: Arc<Mutex<OwnedWriteHalf>> = Arc::new(Mutex::new(writer));
    let producers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut session_handle: u32 = 0;
    let mut header_buf = [0u8; 24];

    loop {
        // Read ENIP header (24 bytes)
        if reader.read_exact(&mut header_buf).await.is_err() {
            break;
        }

        let command = u16::from_le_bytes([header_buf[0], header_buf[1]]);
        let length = u16::from_le_bytes([header_buf[2], header_buf[3]]);
        let recv_session = u32::from_le_bytes([header_buf[4], header_buf[5], header_buf[6], header_buf[7]]);
        let sender_context = u64::from_le_bytes([
            header_buf[12], header_buf[13], header_buf[14], header_buf[15],
            header_buf[16], header_buf[17], header_buf[18], header_buf[19],
        ]);

        // Read body
        let mut body = vec![0u8; length as usize];
        if length > 0 && reader.read_exact(&mut body).await.is_err() {
            break;
        }

        // ForwardOpen / ForwardClose ride inside SendRRData; peek the CIP service.
        let cip_service = if command == 0x006F && body.len() >= 17 {
            body[16]
        } else {
            0
        };

        let mut start_producer: Option<ProducerSpec> = None;
        let reply = match command {
            0x0065 => {
                // RegisterSession
                session_handle = rand_session_handle();
                let mut resp_body = vec![0u8; 4];
                resp_body[0..2].copy_from_slice(&1u16.to_le_bytes()); // protocol version
                resp_body[2..4].copy_from_slice(&0u16.to_le_bytes()); // options
                build_enip_reply(0x0065, session_handle, sender_context, &resp_body)
            }
            0x0066 => {
                // UnregisterSession
                break;
            }
            0x006F if cip_service == 0x54 => {
                // ForwardOpen — accept IO connection; producer starts after reply flush.
                let (r, spec) =
                    handle_forward_open(session_handle, sender_context, &body, &state, &log_tx).await;
                start_producer = spec;
                r
            }
            0x006F if cip_service == 0x4E => {
                // ForwardClose — cancel cyclic producer(s).
                handle_forward_close(session_handle, sender_context, &body, &producers, &log_tx)
                    .await
            }
            0x006F => {
                // SendRRData — CIP request
                handle_rr_data(session_handle, sender_context, &body, &state, &log_tx).await
            }
            0x0070 => {
                // SendUnitData (connected CIP request) — route the embedded
                // message-router request, reply on the same connection id/seq.
                handle_send_unit_data(session_handle, &body, &state, &log_tx).await
            }
            _ => {
                // Unknown command — return error
                build_enip_reply(command, recv_session, sender_context, &[0u8; 0])
            }
        };

        {
            let mut w = writer.lock().await;
            if w.write_all(&reply).await.is_err() {
                break;
            }
        }

        // Start the cyclic T→O producer only after the ForwardOpen reply is on
        // the wire, so the client's RRData read never sees a producer frame.
        if let Some(spec) = start_producer {
            let h = tokio::spawn(cyclic_producer(
                session_handle,
                spec.t_to_o_connection_id,
                spec.rpi_us,
                state.clone(),
                writer.clone(),
                log_tx.clone(),
            ));
            producers.lock().await.push(h);
        }
    }

    // Abort any cyclic producers for this connection.
    let mut handles = producers.lock().await;
    for h in handles.drain(..) {
        h.abort();
    }

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "disconnect".into(),
        tag_name: String::new(),
        message: format!("Client disconnected from {addr}"),
    });
}

// ── Handle SendRRData (contains CIP request) ──

async fn handle_rr_data(
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    // Parse RRData wrapper:
    // [0-3] interface handle (0 for CIP)
    // [4-5] timeout
    // [6-7] item count (2)
    // [8-9] address item type (0x0000 null)
    // [10-11] address length (0)
    // [12-13] data item type (0x00B2 unconnected)
    // [14-15] data length
    // [16+] CIP payload

    if body.len() < 16 {
        return build_rr_error(session_handle, sender_context, "RRData too short");
    }

    let data_len = u16::from_le_bytes([body[14], body[15]]) as usize;
    if body.len() < 16 + data_len || data_len == 0 {
        return build_rr_error(session_handle, sender_context, "Invalid data length");
    }

    let cip_request = &body[16..16 + data_len];
    if cip_request.len() < 2 {
        return build_rr_error(session_handle, sender_context, "CIP request too short");
    }

    let cip_reply = route_cip(cip_request, state, log_tx).await;

    // Wrap CIP reply in RRData
    build_rr_reply(session_handle, sender_context, &cip_reply)
}

// ── CIP Read Tag (0x4C) ──

async fn handle_read_tag(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let tag_name = match parse_tag_from_path(request) {
        Some(name) => name,
        None => {
            return vec![0xCC, 0x00, 0x04, 0x00]; // path error
        }
    };

    let tags = state.tags.lock().await;
    let entry = match tags.get(&tag_name) {
        Some(e) => e,
        None => {
            // Whole-tag miss — try UDT member access (<udt_tag>.<member>), which
            // is how clients read individual struct fields (e.g. DriveInfo.Poles).
            if let Some((dt, bytes)) = resolve_udt_member(&tag_name, &tags, &state.templates) {
                let mut reply = vec![0xCC, 0x00, 0x00, 0x00]; // success
                reply.extend_from_slice(&(dt.code() as u16).to_le_bytes());
                reply.extend_from_slice(&bytes);
                drop(tags);
                let _ = log_tx.send(EnipServerLogEntry {
                    timestamp: now_ms(),
                    service: "read".into(),
                    tag_name: tag_name.clone(),
                    message: format!("Read UDT member {tag_name}"),
                });
                return reply;
            }
            drop(tags);
            let _ = log_tx.send(EnipServerLogEntry {
                timestamp: now_ms(),
                service: "read".into(),
                tag_name: tag_name.clone(),
                message: format!("Read tag not found: {tag_name}"),
            });
            return vec![0xCC, 0x00, 0x04, 0x00]; // path destination unknown
        }
    };

    let mut reply = vec![
        0xCC,           // Read Tag reply
        0x00,           // reserved
        0x00,           // success
        0x00,           // no extended status
    ];
    if let Some(tid) = entry.template_id {
        // Structure (UDT) read: typeSize = 4 → [handle:u16][size:u16][struct bytes].
        // The handle echoes the template id (== the Symbol type word's low bits),
        // size is the struct byte length, and the bytes are the packed member image.
        let size = state
            .templates
            .iter()
            .find(|t| t.id == tid)
            .map(|t| t.structure_size_bytes())
            .unwrap_or(0);
        // Read Tag reply type field for a structure is the CIP STRUCT type code
        // (0x02A0), NOT the template handle — st-ethernet-ip's parser routes on
        // `type === Types.STRUCT` (0x02A0) and otherwise tries atomic decode and
        // throws "Unrecognized Type". Count = struct byte length; bytes = image.
        reply.extend_from_slice(&0x02A0u16.to_le_bytes());
        reply.extend_from_slice(&(size as u16).to_le_bytes());
        if let CipValue::Bytes(b) = &entry.value {
            reply.extend_from_slice(b);
        }
    } else {
        let (type_code, value_data) = encode_cip_value(&entry.value);
        // Atomic read: typeSize = 2 → [type:u16][data] (no element-count field).
        reply.extend_from_slice(&(type_code as u16).to_le_bytes());
        reply.extend_from_slice(&value_data);
    }

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "read".into(),
        tag_name: tag_name.clone(),
        message: format!("Read {} = {}", tag_name, entry.value.display_value()),
    });

    reply
}

// ── CIP Write Tag (0x4D) ──

async fn handle_write_tag(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let tag_name = match parse_tag_from_path(request) {
        Some(name) => name,
        None => {
            return vec![0xCD, 0x00, 0x04, 0x00];
        }
    };

    // Parse write data: after service(1) + path_size(1) + path(N) + type(2) + elements(2) + value
    if request.len() < 4 {
        return vec![0xCD, 0x00, 0x01, 0x00]; // insufficient data
    }

    let path_words = request[1] as usize;
    let path_bytes = path_words * 2;
    let data_offset = 2 + path_bytes;

    if request.len() < data_offset + 4 {
        return vec![0xCD, 0x00, 0x01, 0x00];
    }

    let _type_code = u16::from_le_bytes([request[data_offset], request[data_offset + 1]]);
    let _elements = u16::from_le_bytes([request[data_offset + 2], request[data_offset + 3]]);
    let value_data = &request[data_offset + 4..];

    let value = match decode_cip_value(_type_code as u8, value_data) {
        Ok(v) => v,
        Err(_) => {
            return vec![0xCD, 0x00, 0x01, 0x00];
        }
    };

    {
        let mut tags = state.tags.lock().await;
        if let Some(entry) = tags.get_mut(&tag_name) {
            let _ = log_tx.send(EnipServerLogEntry {
                timestamp: now_ms(),
                service: "write".into(),
                tag_name: tag_name.clone(),
                message: format!("Write {} = {} (was {})", tag_name, value.display_value(), entry.value.display_value()),
            });
            entry.value = value;
        } else {
            drop(tags);
            let _ = log_tx.send(EnipServerLogEntry {
                timestamp: now_ms(),
                service: "write".into(),
                tag_name: tag_name.clone(),
                message: format!("Write tag not found: {tag_name}"),
            });
            return vec![0xCD, 0x00, 0x04, 0x00];
        }
    }

    vec![0xCD, 0x00, 0x00, 0x00] // success
}

// ── CIP GetAttributeAll (0x01) — tag browsing ──

async fn handle_get_attribute_all(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    // Identity object (class 0x01), instance 1 — what every CIP client reads at
    // connect (st-ethernet-ip "readControllerProps" / RSLinx). GetAttributesAll
    // returns attrs 1..=7 concatenated, matching the standard Identity layout
    // (revision@6, status@8, serial@10, product-name SHORT_STRING@14).
    if let Some((class, instance, _)) = parse_cip_class_instance_attr(request) {
        if class == 0x01 && instance == 1 {
            let mut reply = vec![0x81, 0x00, 0x00, 0x00]; // success
            for attr in 1u16..=7 {
                if let Some(bytes) = identity_attribute(attr) {
                    reply.extend_from_slice(&bytes);
                }
            }
            let _ = log_tx.send(EnipServerLogEntry {
                timestamp: now_ms(),
                service: "identity".into(),
                tag_name: String::new(),
                message: "GetAttributesAll Identity (controller props)".into(),
            });
            return reply;
        }
    }

    // Parse path: service(1) + path_size(1) + class segment + instance segment
    if request.len() < 8 {
        return vec![0x81, 0x00, 0x01, 0x00]; // error
    }

    let path_words = request[1] as usize;
    let path_bytes = path_words * 2;
    if request.len() < 2 + path_bytes {
        return vec![0x81, 0x00, 0x01, 0x00];
    }

    let path = &request[2..2 + path_bytes];

    // Check if it's the Symbol class (0x6B)
    let is_symbol_class = path.windows(2).any(|w| w[0] == 0x20 && w[1] == SYMBOL_CLASS_ID);

    if !is_symbol_class {
        return vec![0x81, 0x00, 0x08, 0x00]; // service not supported
    }

    // Extract instance ID
    let instance_id = parse_instance_id(path);
    let tags = state.tags.lock().await;
    let tag_list: Vec<&CipTagEntry> = tags.values().collect();

    // Instance 0 = class-level, instance 1..N = individual symbols
    let mut reply = vec![0x81, 0x00, 0x00, 0x00]; // success header

    if instance_id == 0 {
        // Class-level GetAttributeAll — return instance count
        reply.extend_from_slice(&(tag_list.len() as u16).to_le_bytes());
    } else {
        let idx = (instance_id - 1) as usize;
        if idx >= tag_list.len() {
            return vec![0x81, 0x00, 0x04, 0x00]; // instance not found
        }

        let tag_keys: Vec<&String> = tags.keys().collect();
        let name = tag_keys[idx];
        let entry = &tag_list[idx];

        // Build symbol instance data: name (CIP STRING) + type word
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u32;
        reply.extend_from_slice(&name_len.to_le_bytes());
        reply.extend_from_slice(name_bytes);
        // Pad to even
        if name_bytes.len() % 2 != 0 {
            reply.push(0x00);
        }
        // Type word: type_code in low byte, element_count in high byte
        let type_word = match entry.template_id {
            // Structure (UDT): bit 15 set + template id in the low bits. Clients
            // read structure = word & 0x8000, templateId = word & 0x0FFF.
            Some(tid) => 0x8000 | (tid as u16),
            // Atomic: type_code in low byte, element_count in high byte.
            None => ((entry.element_count as u16) << 8) | (entry.data_type.code() as u16),
        };
        reply.extend_from_slice(&type_word.to_le_bytes());
    }

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "browse".into(),
        tag_name: String::new(),
        message: format!("Browse instance {instance_id} ({} tags)", tag_list.len()),
    });

    reply
}

// ── CIP GetAttributeSingle (0x0E) — lets the CIP tree read Identity attrs ──

async fn handle_get_attribute_single(
    request: &[u8],
    _log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let (class, instance, attr) = match parse_cip_class_instance_attr(request) {
        Some(v) => v,
        None => return vec![0x8E, 0x00, 0x06, 0x00], // amount of data invalid / path
    };

    // Modelled objects: Identity (0x01), Base Energy (0x4F), Electrical Energy
    // (0x4E) — all instance 1. Others fall through to "service not supported".
    let payload = if class == 0x01 && instance == 1 {
        identity_attribute(attr)
    } else if class == 0x4F && instance == 1 {
        energy_base_attribute(attr)
    } else if class == 0x4E && instance == 1 {
        electrical_energy_attribute(attr)
    } else {
        None
    };

    match payload {
        Some(bytes) => {
            let mut reply = vec![0x8E, 0x00, 0x00, 0x00]; // success
            reply.extend_from_slice(&bytes);
            reply
        }
        None => vec![0x8E, 0x00, 0x08, 0x00], // service not supported / no such attr
    }
}

// ── CIP GetInstanceAttributeList (0x55) — controller tag enumeration ──

async fn handle_get_instance_attribute_list(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    // st-ethernet-ip / RSLinx enumerate controller tags via GetInstanceAttributeList
    // on the Symbol class (0x6B), requesting attrs 1 (name) and 2 (type). Reply
    // data is, per instance: [id:u32][name_len:u16][name][type:u16]. Return every
    // tag in one response (status 0). NEVER reply status 6 ("partial transfer"):
    // the client paginates on 6 and its EPATH segment builder has a null-buffer
    // bug for instance ids > 65535 (and for NaN), which crashes Node-RED.
    if request.len() < 2 {
        return vec![0xD5, 0x00, 0x01, 0x00]; // insufficient data
    }
    let pw = request[1] as usize;
    let path = request.get(2..2 + 2 * pw).unwrap_or(&[]);
    let is_symbol = path.windows(2).any(|w| w[0] == 0x20 && w[1] == SYMBOL_CLASS_ID)
        || path.windows(4).any(|w| w[0] == 0x21 && w[2] == SYMBOL_CLASS_ID && w[3] == 0x00);
    if !is_symbol {
        return vec![0xD5, 0x00, 0x08, 0x00]; // service not supported
    }
    let start = parse_instance_id(path);

    // Scope: a leading "Program:<name>" symbolic segment = program scope;
    // otherwise controller scope. Mirrors AB program-scoped browsing so the
    // client discovers one program object per program instead of one per tag
    // (which made st-ethernet-ip recurse N×N and report 144 tags for 12).
    let scope = leading_program_scope(path);

    // Build (name, type_code) records for this scope. Program objects are named
    // "Program:<prog>"; program-scoped tags are emitted with the "Program:<prog>."
    // prefix stripped — the form clients use to read them back, since the read
    // path re-prepends the program scope.
    let type_word_of = |e: &CipTagEntry| -> u16 {
        e.template_id
            .map(|t| 0x8000 | t as u16)
            .unwrap_or_else(|| e.data_type.code() as u16)
    };
    let mut records: Vec<(String, u16)> = Vec::new();
    {
        let tags = state.tags.lock().await;
        match scope.as_deref() {
            Some(prog) => {
                let prefix = format!("Program:{prog}.");
                for (name, entry) in tags.iter() {
                    if let Some(local) = name.strip_prefix(&prefix) {
                        records.push((local.to_string(), type_word_of(entry)));
                    }
                }
            }
            None => {
                let mut seen_programs = std::collections::BTreeSet::new();
                for (name, entry) in tags.iter() {
                    if let Some(rest) = name.strip_prefix("Program:") {
                        if let Some(prog) = rest.split('.').next() {
                            seen_programs.insert(prog.to_string());
                            continue;
                        }
                    }
                    records.push((name.clone(), type_word_of(entry)));
                }
                for prog in seen_programs {
                    records.push((format!("Program:{prog}"), 0));
                }
            }
        }
    }

    let mut reply = vec![0xD5, 0x00, 0x00, 0x00]; // success header
    let mut id = 0u32;
    let mut count = 0u32;
    for (name, type_code) in &records {
        id += 1;
        if id <= start {
            continue;
        }
        reply.extend_from_slice(&id.to_le_bytes());
        let name_bytes = name.as_bytes();
        reply.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        reply.extend_from_slice(name_bytes);
        reply.extend_from_slice(&type_code.to_le_bytes());
        count += 1;
    }

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "browse".into(),
        tag_name: String::new(),
        message: format!(
            "GetInstanceAttributeList scope={} ({count} records)",
            scope.as_deref().unwrap_or("controller")
        ),
    });

    reply
}

/// Bytes for an Identity object attribute (class 0x01, instance 1).
fn identity_attribute(attr: u16) -> Option<Vec<u8>> {
    let mut b = Vec::new();
    match attr {
        1 => b.extend_from_slice(&1u16.to_le_bytes()),             // Vendor ID (Rockwell)
        2 => b.extend_from_slice(&0x0Eu16.to_le_bytes()),          // Device Type (communications)
        3 => b.extend_from_slice(&0x0050u16.to_le_bytes()),        // Product Code
        4 => {                                                      // Revision 20.11
            b.push(20);
            b.push(11);
        }
        5 => b.extend_from_slice(&0x0020u16.to_le_bytes()),        // Status
        6 => b.extend_from_slice(&0x12345678u32.to_le_bytes()),    // Serial Number
        7 => {                                                      // Product Name (SHORT_STRING)
            let name = b"ENIP-Sim";
            b.push(name.len() as u8);
            b.extend_from_slice(name);
        }
        _ => return None,
    }
    Some(b)
}

/// Base Energy Object (class 0x4F) instance-1 attributes — what node-red's
/// cip-energy "status" command reads (type/mode USINT, power REAL, energy LREAL).
fn energy_base_attribute(attr: u16) -> Option<Vec<u8>> {
    let mut b = Vec::new();
    match attr {
        1 => b.push(1),                                      // Energy Type: Electrical AC
        2 => b.push(0),                                      // Energy Mode: Normal
        3 => b.extend_from_slice(&0u32.to_le_bytes()),       // Mode Transition Time
        6 => b.extend_from_slice(&7500.0f32.to_le_bytes()),  // Nominal Power (W)
        7 => b.extend_from_slice(&7500.0f32.to_le_bytes()),  // Rated Power (W)
        8 => b.extend_from_slice(&7350.0f32.to_le_bytes()),  // Power (W)
        9 => b.extend_from_slice(&1234.5f64.to_le_bytes()),  // Energy (kWh, LREAL)
        10 => b.extend_from_slice(&9000.0f32.to_le_bytes()), // Peak Power (W)
        11 => b.push(0),                                     // Peak Power Reset
        _ => return None,
    }
    Some(b)
}

/// Electrical Energy Object (class 0x4E) instance-1 attributes (all REAL).
fn electrical_energy_attribute(attr: u16) -> Option<Vec<u8>> {
    let v: f32 = match attr {
        1 => 400.0,   // Voltage LL (V)
        2 => 230.0,   // Voltage LN (V)
        3 => 12.4,    // Current (A)
        4 => 50.0,    // Frequency (Hz)
        5 => 0.95,    // Power Factor
        6 => 8000.0,  // Apparent Power (VA)
        7 => 2500.0,  // Reactive Power (VAR)
        8 => 7350.0,  // Active Power (W)
        9 => 1234.5,  // Active Energy (kWh)
        10 => 410.0,  // Reactive Energy (kVARh)
        11 => 8200.0, // Apparent Energy (kVAh)
        12 => 3.0,    // THD Voltage (%)
        13 => 4.0,    // THD Current (%)
        _ => return None,
    };
    Some(v.to_le_bytes().to_vec())
}

/// Parse class/instance/attribute from a CIP request EPath.
/// Recognizes 8-bit segments (0x20,0x24,0x30) and 16-bit segments (0x21,0x25,0x31).
fn parse_cip_class_instance_attr(request: &[u8]) -> Option<(u16, u32, u16)> {
    if request.len() < 2 {
        return None;
    }
    let pw = request[1] as usize;
    let path = request.get(2..2 + 2 * pw)?;
    let mut class = None;
    let mut instance: u32 = 0;
    let mut attr: u16 = 0;
    let mut i = 0;
    while i < path.len() {
        let seg = path[i];
        match seg {
            0x20 if i + 1 < path.len() => {
                class = Some(path[i + 1] as u16);
                i += 2;
            }
            0x21 if i + 3 < path.len() => {
                class = Some(u16::from_le_bytes([path[i + 2], path[i + 3]]));
                i += 4;
            }
            0x24 if i + 1 < path.len() => {
                instance = path[i + 1] as u32;
                i += 2;
            }
            0x25 if i + 2 < path.len() => {
                instance = u16::from_le_bytes([path[i + 1], path[i + 2]]) as u32;
                i += 3;
            }
            0x30 if i + 1 < path.len() => {
                attr = path[i + 1] as u16;
                i += 2;
            }
            0x31 if i + 2 < path.len() => {
                attr = u16::from_le_bytes([path[i + 1], path[i + 2]]);
                i += 3;
            }
            _ => break,
        }
    }
    Some((class?, instance, attr))
}

// ── Implicit IO: ForwardOpen / ForwardClose / SendUnitData + cyclic producer ──

/// Extract the CIP payload from a SendRRData body.
fn cip_payload_of(body: &[u8]) -> Option<&[u8]> {
    if body.len() < 16 {
        return None;
    }
    let data_len = u16::from_le_bytes([body[14], body[15]]) as usize;
    body.get(16..16 + data_len)
}

/// A cyclic T→O producer to launch after the ForwardOpen reply is flushed.
struct ProducerSpec {
    t_to_o_connection_id: u32,
    rpi_us: u32,
}

async fn handle_forward_open(
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
    _state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> (Vec<u8>, Option<ProducerSpec>) {
    let cip = match cip_payload_of(body) {
        Some(c) => c,
        None => return (build_rr_error(session_handle, sender_context, "ForwardOpen bad body"), None),
    };
    if cip.len() < 4 {
        return (build_rr_error(session_handle, sender_context, "ForwardOpen too short"), None);
    }
    let pw = cip[1] as usize;
    let base = 2 + 2 * pw + 1; // skip service + path + timeout_ticks
    if cip.len() < base + 16 {
        return (build_rr_error(session_handle, sender_context, "ForwardOpen truncated"), None);
    }
    let o_to_t = u32::from_le_bytes(cip[base..base + 4].try_into().unwrap());
    let t_to_o = u32::from_le_bytes(cip[base + 4..base + 8].try_into().unwrap());
    let conn_serial = u16::from_le_bytes(cip[base + 8..base + 10].try_into().unwrap());
    let vendor = u16::from_le_bytes(cip[base + 10..base + 12].try_into().unwrap());
    let originator = u32::from_le_bytes(cip[base + 12..base + 16].try_into().unwrap());
    // ForwardOpen fixed fields, offsets relative to `base` (= first byte past the
    // 1-byte timeout, i.e. the O→T connection id under spec/explorer framing):
    //   +0 O→T id(4) +4 T→T id(4) +8 conn serial(2) +10 vendor(2) +12 originator(4)
    //   +16 timeout-mult(4) +20 O→T RPI(4) +24 O→T params(2) +26 T→O RPI(4)
    //   +30 T→O params(2) +32 transport(1)  → path size → connection path.
    let rpi_us = if cip.len() >= base + 30 {
        u32::from_le_bytes(cip[base + 26..base + 30].try_into().unwrap())
    } else {
        1_000_000
    };
    // The connection path (port + target class) begins at base+35 under BOTH
    // framings: the explorer uses 1 timeout byte + a 2-byte path size, while
    // st-ethernet-ip uses 2 timeout bytes + a 1-byte path size — the extra
    // leading byte is cancelled by the shorter size field, so the path lands at
    // the same offset. Explicit (class-3) connections target the Message Router
    // (class 0x02); the explorer's cyclic IO connection targets an Assembly (class 0x04).
    // Produce cyclic T→O data ONLY for Assembly-targeted connections — pushing producer
    // frames into any other connection (e.g. st-ethernet-ip's explicit Message Router
    // connection) makes the client parse them as message-router replies (corruption /
    // OOB crash / read-reply mismatches).
    let conn_path = cip.get(base + 35..).unwrap_or(&[]);
    let is_assembly = conn_path.windows(2).any(|w| w[0] == 0x20 && w[1] == 0x04);

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "forward_open".into(),
        tag_name: String::new(),
        message: format!(
            "ForwardOpen conn_serial=0x{conn_serial:04X} t_to_o=0x{t_to_o:08X} rpi={rpi_us}us"
        ),
    });

    // Reply 0xD4 success + assigned connection IDs (echo requested ids).
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(&o_to_t.to_le_bytes());
    payload.extend_from_slice(&t_to_o.to_le_bytes());
    payload.extend_from_slice(&conn_serial.to_le_bytes());
    payload.extend_from_slice(&vendor.to_le_bytes());
    payload.extend_from_slice(&originator.to_le_bytes());
    let mut cip_reply = vec![0xD4, 0x00, 0x00, 0x00];
    cip_reply.extend_from_slice(&payload);
    (
        build_rr_reply(session_handle, sender_context, &cip_reply),
        if is_assembly {
            Some(ProducerSpec { t_to_o_connection_id: t_to_o, rpi_us })
        } else {
            None
        },
    )
}

async fn handle_forward_close(
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
    producers: &Arc<Mutex<Vec<JoinHandle<()>>>>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let aborted = {
        let mut handles = producers.lock().await;
        let n = handles.len();
        for h in handles.drain(..) {
            h.abort();
        }
        n
    };
    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "forward_close".into(),
        tag_name: String::new(),
        message: format!("ForwardClose — aborted {aborted} producer(s)"),
    });
    let cip_reply = vec![0xCE, 0x00, 0x00, 0x00]; // success
    let _ = body;
    build_rr_reply(session_handle, sender_context, &cip_reply)
}

/// Route a CIP message-router request to its handler and return the bare
/// message-router reply (no ENIP / RRData / SendUnitData wrapping). Shared by
/// the unconnected (SendRRData) and connected (SendUnitData) paths.
async fn route_cip(
    cip: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    if cip.len() < 2 {
        // Too short to carry a service byte — reply with a generic error so the
        // client's parser never sees an empty message-router buffer (OOB crash).
        return vec![cip.first().copied().unwrap_or(0) | 0x80, 0x00, 0x01, 0x00];
    }
    let service = cip[0];

    // Template object (class 0x6C): serve member definitions so structure-typed
    // tags decode into named members. st-ethernet-ip fetches attrs 4/5/2/1 via
    // GetAttributeList (0x03), then reads the definition blob via Read Tag (0x4C).
    if parse_target_class(cip) == Some(0x6C) {
        return match service {
            0x03 => handle_template_get_attributes(cip, state, log_tx).await,
            0x4C => handle_template_read(cip, state, log_tx).await,
            _ => vec![service | 0x80, 0x00, 0x08, 0x00],
        };
    }

    match service {
        0x4C | 0x52 => handle_read_tag(cip, state, log_tx).await, // Read Tag (+ Fragmented: extra offset/size data is ignored)
        0x4D => handle_write_tag(cip, state, log_tx).await,
        0x01 => handle_get_attribute_all(cip, state, log_tx).await,
        0x0E => handle_get_attribute_single(cip, log_tx).await,
        0x55 => handle_get_instance_attribute_list(cip, state, log_tx).await,
        _ => vec![
            service | 0x80, // reply service
            0x00,           // reserved
            0x08,           // service not supported
            0x00,           // no extended status
        ],
    }
}

// ── Template object (class 0x6C) — UDT member definitions ──

/// Class id targeted by a CIP request EPath, if it carries an 8/16-bit class
/// segment (0x20/0x21). Symbolic (0x91) tag reads have no class segment → None.
fn parse_target_class(request: &[u8]) -> Option<u16> {
    parse_cip_class_instance_attr(request).map(|(c, _, _)| c)
}

impl CipTemplateDef {
    /// (member, byte offset, byte size) per member, naturally aligned to the
    /// member's element size (4B DINT/REAL, 2B INT, 1B BOOL/SINT).
    fn member_layout(&self) -> Vec<(&CipTemplateMember, u32, u16)> {
        let mut offset = 0u32;
        let mut out = Vec::with_capacity(self.members.len());
        for m in &self.members {
            let size = m.data_type.element_size().max(1) as u32;
            let bytes = size * m.array_elements.max(1) as u32;
            let align = size.max(1);
            offset = (offset + align - 1) / align * align;
            out.push((m, offset, bytes as u16));
            offset += bytes;
        }
        out
    }

    /// Packed byte length of one struct instance.
    fn structure_size_bytes(&self) -> u16 {
        let total: u32 = self
            .member_layout()
            .last()
            .map(|(_, off, bytes)| *off + *bytes as u32)
            .unwrap_or(0);
        total as u16
    }

    /// Build the definition blob returned by Read Tag on class 0x6C. The
    /// message-router reply data IS this blob (st-ethernet-ip parses it from
    /// offset 0), so its first 4 bytes double as the Read Tag type+count fields.
    /// Per member: [info:u16][type:u16][offset:u32] (8B); then
    /// `[struct];\0[m0]\0[m1]\0…`, zero-padded to a 4-byte multiple.
    fn build_definition_blob(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        for (m, offset, _) in self.member_layout() {
            blob.extend_from_slice(&0u16.to_le_bytes()); // info: atomic scalar
            blob.extend_from_slice(&(m.data_type.code() as u16).to_le_bytes());
            blob.extend_from_slice(&offset.to_le_bytes());
        }
        blob.extend_from_slice(self.name.as_bytes());
        blob.push(0x3B); // ';' ends struct-name capture
        blob.push(0x00);
        for m in &self.members {
            blob.extend_from_slice(m.name.as_bytes());
            blob.push(0x00);
        }
        while blob.len() % 4 != 0 {
            blob.push(0x00);
        }
        blob
    }

    /// ObjDefinitionSize (template size in 32-bit words). reqSize the client
    /// reads = objDefSize*4 - 16 = blob length.
    fn obj_definition_size_words(&self) -> u32 {
        (self.build_definition_blob().len() as u32 + 16) / 4
    }

    /// Pack member values (one f64 per member, in order) into the struct byte
    /// image, each written at its aligned offset.
    pub fn pack_instance(&self, values: &[f64]) -> Vec<u8> {
        let mut buf = vec![0u8; self.structure_size_bytes() as usize];
        for ((m, off, _), v) in self.member_layout().iter().zip(values.iter()) {
            let bytes = encode_atomic(m.data_type, *v);
            let start = *off as usize;
            if start + bytes.len() <= buf.len() {
                buf[start..start + bytes.len()].copy_from_slice(&bytes);
            }
        }
        buf
    }
}

/// Encode one atomic scalar value to little-endian bytes for struct packing.
fn encode_atomic(dt: CipDataType, v: f64) -> Vec<u8> {
    match dt {
        CipDataType::Bool => vec![if v >= 0.5 { 1 } else { 0 }],
        CipDataType::Sint => vec![(v.round() as i8) as u8],
        CipDataType::Int => (v.round() as i16).to_le_bytes().to_vec(),
        CipDataType::Dint => (v.round() as i32).to_le_bytes().to_vec(),
        CipDataType::Real => (v as f32).to_le_bytes().to_vec(),
        CipDataType::Lint => (v.round() as i64).to_le_bytes().to_vec(),
        _ => Vec::new(),
    }
}

/// Resolve `<udt_tag>.<member>` against the tag map + templates and return the
/// member's atomic type + packed bytes sliced from the struct image. Used so
/// clients can read individual struct fields (e.g. DriveInfo.Poles). Single
/// level only (nested-UDT members return None for now). Case-insensitive member.
fn resolve_udt_member(
    name: &str,
    tags: &BTreeMap<String, CipTagEntry>,
    templates: &[CipTemplateDef],
) -> Option<(CipDataType, Vec<u8>)> {
    // Longest UDT tag name that prefixes `name` (most specific owner).
    let mut best: Option<(&str, &str)> = None;
    for (tag_name, entry) in tags.iter() {
        if entry.template_id.is_none() {
            continue;
        }
        if let Some(rest) = name.strip_prefix(&format!("{tag_name}.")) {
            match best {
                Some((bn, _)) if bn.len() >= tag_name.len() => {}
                _ => best = Some((tag_name.as_str(), rest)),
            }
        }
    }
    let (tag_name, member_path) = best?;
    if member_path.contains('.') {
        return None; // nested-UDT member — not supported
    }
    let entry = tags.get(tag_name)?;
    let tid = entry.template_id?;
    let tmpl = templates.iter().find(|t| t.id == tid)?;
    let bytes = match &entry.value {
        CipValue::Bytes(b) => b.as_slice(),
        _ => return None,
    };
    for (m, off, size) in tmpl.member_layout() {
        if m.name.eq_ignore_ascii_case(member_path) {
            let start = off as usize;
            let end = start + size as usize;
            if end <= bytes.len() {
                return Some((m.data_type, bytes[start..end].to_vec()));
            }
        }
    }
    None
}

/// GetAttributeList (service 0x03) on a Template object instance — attrs 4, 5,
/// 2, 1. st-ethernet-ip's `_parseReadTemplateAttributes` reads the four values
/// at message-router data offsets 6 / 14 / 22 / 28 (relative to the data that
/// begins AFTER the 4-byte router header). In the full reply buffer those land
/// at 10 / 18 / 26 / 32:
///   ObjDefinitionSize(u32@10)  StructureSize(u32@18)
///   MemberCount(u16@26)        StructureHandle(u16@32)
async fn handle_template_get_attributes(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let (_, instance, _) = match parse_cip_class_instance_attr(request) {
        Some(v) => v,
        None => return vec![0x83, 0x00, 0x06, 0x00],
    };
    let tmpl = match state.templates.iter().find(|t| t.id as u32 == instance) {
        Some(t) => t,
        None => return vec![0x83, 0x00, 0x04, 0x00],
    };

    let obj_def = tmpl.obj_definition_size_words();
    let struct_size = tmpl.structure_size_bytes() as u32;
    let member_count = tmpl.members.len() as u16;
    let handle = tmpl.id;

    let mut data = vec![0u8; 34];
    data[0] = 0x83; // GetAttributeList reply | reserved | status 0 | ext-len 0
    data[4..6].copy_from_slice(&4u16.to_le_bytes()); // attribute count
    data[10..14].copy_from_slice(&obj_def.to_le_bytes()); // attr 4
    data[18..22].copy_from_slice(&struct_size.to_le_bytes()); // attr 5
    data[26..28].copy_from_slice(&member_count.to_le_bytes()); // attr 2
    data[32..34].copy_from_slice(&handle.to_le_bytes()); // attr 1

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "template".into(),
        tag_name: tmpl.name.clone(),
        message: format!(
            "Template {} attrs: objDef={} structSize={} members={} handle={}",
            tmpl.id, obj_def, struct_size, member_count, handle
        ),
    });

    data
}

/// Read Tag (service 0x4C) on a Template object instance — returns the member
/// definition blob. Reply is `[0xCC,0,0,0] + blob`; the blob's first 4 bytes
/// (member 0 info + type) serve as the Read Tag type+count fields, exactly what
/// st-ethernet-ip's `_parseReadTemplate` reads from offset 0.
async fn handle_template_read(
    request: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let (_, instance, _) = match parse_cip_class_instance_attr(request) {
        Some(v) => v,
        None => return vec![0xCC, 0x00, 0x06, 0x00],
    };
    let tmpl = match state.templates.iter().find(|t| t.id as u32 == instance) {
        Some(t) => t,
        None => return vec![0xCC, 0x00, 0x04, 0x00],
    };

    let blob = tmpl.build_definition_blob();
    let mut reply = vec![0xCC, 0x00, 0x00, 0x00]; // Read Tag reply + success
    reply.extend_from_slice(&blob);

    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: "template".into(),
        tag_name: tmpl.name.clone(),
        message: format!("Template {} read: {} blob bytes", tmpl.id, blob.len()),
    });

    reply
}

/// Split an inbound SendUnitData body into (connection_id, sequence, cip).
/// Item-typed walk so it tolerates address/data item ordering.
fn parse_sud_body(body: &[u8]) -> Option<(u32, u16, &[u8])> {
    if body.len() < 8 {
        return None;
    }
    let mut pos = 8usize; // skip interface(4) + timeout(2) + item count(2)
    let mut conn_id = 0u32;
    let mut seq = 0u16;
    let mut cip: &[u8] = &[];
    for _ in 0..2 {
        let header = body.get(pos..pos + 4)?;
        let type_id = u16::from_le_bytes([header[0], header[1]]);
        let len = u16::from_le_bytes([header[2], header[3]]) as usize;
        pos += 4;
        let item = body.get(pos..pos + len)?;
        pos += len;
        match type_id {
            0x00A1 if item.len() >= 4 => {
                conn_id = u32::from_le_bytes([item[0], item[1], item[2], item[3]]);
            }
            0x00B1 if item.len() >= 2 => {
                seq = u16::from_le_bytes([item[0], item[1]]);
                cip = &item[2..];
            }
            _ => {}
        }
    }
    Some((conn_id, seq, cip))
}

/// Handle an inbound connected CIP request (SendUnitData). Routes the embedded
/// message-router request and replies on the same connection id / sequence.
/// Replying with an empty data item crashes st-ethernet-ip's message-router
/// parser (readUInt8 on a zero-length buffer), so the reply is always ≥4 bytes.
async fn handle_send_unit_data(
    session_handle: u32,
    body: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let (conn_id, seq, cip) = parse_sud_body(body).unwrap_or((0, 0, &[]));
    let cip_reply = route_cip(cip, state, log_tx).await;
    build_send_unit_data(session_handle, conn_id, seq, &cip_reply)
}

/// Cyclic T→O producer: writes a SendUnitData frame encoding the current input
/// assembly (snapshot of tag values) at the negotiated RPI.
async fn cyclic_producer(
    session_handle: u32,
    t_to_o_connection_id: u32,
    rpi_us: u32,
    state: Arc<EnipServerState>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    _log_tx: mpsc::UnboundedSender<EnipServerLogEntry>,
) {
    let period_ms = (rpi_us / 1000).clamp(20, 10_000) as u64;
    let mut ticker = tokio::time::interval(Duration::from_millis(period_ms));
    let mut seq: u16 = 0;
    loop {
        ticker.tick().await;
        let assembly = snapshot_input_assembly(&state).await;
        let frame = build_send_unit_data(session_handle, t_to_o_connection_id, seq, &assembly);
        seq = seq.wrapping_add(1);
        let mut w = writer.lock().await;
        if w.write_all(&frame).await.is_err() {
            break;
        }
    }
}

/// Encode the current tag values as a flat input-assembly byte image.
async fn snapshot_input_assembly(state: &Arc<EnipServerState>) -> Vec<u8> {
    let tags = state.tags.lock().await;
    let mut out = Vec::new();
    for entry in tags.values() {
        let (_type_code, data) = encode_cip_value(&entry.value);
        out.extend_from_slice(&data);
    }
    out
}

// ── Helpers ──

fn parse_tag_from_path(request: &[u8]) -> Option<String> {
    if request.len() < 3 {
        return None;
    }
    let path_words = request[1] as usize;
    let path_bytes = path_words * 2;
    if request.len() < 2 + path_bytes {
        return None;
    }
    let path = &request[2..2 + path_bytes];

    // Walk every ANSI-symbolic segment (0x91) in the path and join with '.'.
    // Clients split dotted names ("Program:Main.Motor1.Speed") into separate
    // symbolic segments (Program:Main / Motor1 / Speed); joining reconstructs the
    // full tag name as stored in the map. Each 0x91 segment is padded to even.
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 1 < path.len() {
        if path[i] != 0x91 {
            break; // non-symbolic segment (element/bit) — stop collecting parts
        }
        let name_len = path[i + 1] as usize;
        let seg = path.get(i + 2..i + 2 + name_len)?;
        parts.push(std::str::from_utf8(seg).ok()?.to_string());
        i += 2 + name_len;
        if i % 2 != 0 {
            i += 1; // symbolic segment pad byte
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("."))
}

/// If the EPath contains a "Program:<name>" ANSI-symbolic segment, return
/// <name> — the requested program scope. None = controller scope.
fn leading_program_scope(path: &[u8]) -> Option<String> {
    let mut i = 0;
    while i + 1 < path.len() {
        if path[i] == 0x91 {
            let name_len = path[i + 1] as usize;
            if let Some(seg) = path.get(i + 2..i + 2 + name_len) {
                if let Ok(s) = std::str::from_utf8(seg) {
                    if let Some(prog) = s.strip_prefix("Program:") {
                        return Some(prog.to_string());
                    }
                }
                i += 2 + name_len;
                if i % 2 != 0 {
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    None
}

fn parse_instance_id(path: &[u8]) -> u32 {
    // Look for instance segment: 0x24 (8-bit) or 0x25 (16-bit)
    let mut i = 0;
    while i + 1 < path.len() {
        if path[i] == 0x24 && i + 1 < path.len() {
            return path[i + 1] as u32;
        }
        if path[i] == 0x25 && i + 2 < path.len() {
            return u16::from_le_bytes([path[i + 1], path[i + 2]]) as u32;
        }
        i += 1;
    }
    0
}

fn build_enip_reply(command: u16, session_handle: u32, sender_context: u64, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ENIP_HEADER_SIZE + body.len());
    buf.extend_from_slice(&command.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(&session_handle.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // status
    buf.extend_from_slice(&sender_context.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // options
    buf.extend_from_slice(body);
    buf
}

fn build_rr_reply(session_handle: u32, sender_context: u64, cip_data: &[u8]) -> Vec<u8> {
    let mut rr_body = Vec::with_capacity(16 + cip_data.len());
    rr_body.extend_from_slice(&0u32.to_le_bytes()); // interface handle
    rr_body.extend_from_slice(&0u16.to_le_bytes()); // timeout
    rr_body.extend_from_slice(&2u16.to_le_bytes()); // item count
    rr_body.extend_from_slice(&0x0000u16.to_le_bytes()); // null address item
    rr_body.extend_from_slice(&0u16.to_le_bytes()); // address length
    rr_body.extend_from_slice(&0x00B2u16.to_le_bytes()); // unconnected data item
    rr_body.extend_from_slice(&(cip_data.len() as u16).to_le_bytes());
    rr_body.extend_from_slice(cip_data);
    build_enip_reply(CMD_SEND_RR_DATA, session_handle, sender_context, &rr_body)
}

fn build_rr_error(session_handle: u32, sender_context: u64, _msg: &str) -> Vec<u8> {
    let cip_error = vec![0x80, 0x00, 0x01, 0x00]; // generic error
    build_rr_reply(session_handle, sender_context, &cip_error)
}

fn rand_session_handle() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u32;
    ts.wrapping_mul(2654435761) // Knuth multiplicative hash
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive_info_template() -> CipTemplateDef {
        CipTemplateDef {
            id: 1,
            name: "DriveInfo".into(),
            members: vec![
                CipTemplateMember { name: "RatedSpeed".into(), data_type: CipDataType::Real, array_elements: 1 },
                CipTemplateMember { name: "RatedCurrent".into(), data_type: CipDataType::Real, array_elements: 1 },
                CipTemplateMember { name: "RatedPower".into(), data_type: CipDataType::Real, array_elements: 1 },
                CipTemplateMember { name: "Poles".into(), data_type: CipDataType::Dint, array_elements: 1 },
            ],
        }
    }

    #[test]
    fn udt_template_layout_and_blob() {
        let t = drive_info_template();
        let offsets: Vec<u32> = t.member_layout().iter().map(|(_, o, _)| *o).collect();
        assert_eq!(offsets, vec![0, 4, 8, 12], "members 4-byte aligned");
        assert_eq!(t.structure_size_bytes(), 16);

        let blob = t.build_definition_blob();
        assert_eq!(blob.len() % 4, 0, "blob padded to 4-byte multiple");
        assert_eq!(blob.len(), 84);
        // member 0 record: [info:u16][type:u16][offset:u32] → REAL @ offset 0
        assert_eq!(u16::from_le_bytes([blob[2], blob[3]]), 0x00CA);
        assert_eq!(u32::from_le_bytes(blob[4..8].try_into().unwrap()), 0);
        // member 1 offset @ blob[12..16] = 4
        assert_eq!(u32::from_le_bytes(blob[12..16].try_into().unwrap()), 4);
        // struct name "DriveInfo" begins right after the 32 bytes of member records
        assert_eq!(&blob[32..41], b"DriveInfo".as_slice());
        assert_eq!(blob[41], 0x3B); // ';' ends struct-name capture
        // obj definition size (words) → client reqSize = objDef*4 - 16 = blob length
        assert_eq!(t.obj_definition_size_words(), 25);
        assert_eq!(t.obj_definition_size_words() * 4 - 16, blob.len() as u32);
    }

    #[test]
    fn udt_template_pack_instance() {
        let t = drive_info_template();
        let buf = t.pack_instance(&[1450.0, 12.4, 7.5, 4.0]);
        assert_eq!(buf.len(), 16);
        let speed = f32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert!((speed - 1450.0).abs() < 0.1, "RatedSpeed REAL @0");
        let poles = i32::from_le_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(poles, 4, "Poles DINT @12");
    }

    #[test]
    fn udt_member_resolves_from_struct_image() {
        let t = drive_info_template();
        let bytes = t.pack_instance(&[1450.0, 12.4, 7.5, 4.0]);
        let mut tags = BTreeMap::new();
        tags.insert(
            "Program:Main.Motor1.DriveInfo".into(),
            CipTagEntry {
                data_type: CipDataType::Dint,
                element_count: 1,
                value: CipValue::Bytes(bytes),
                template_id: Some(1),
            },
        );
        let templates = vec![t];

        // Poles DINT @ offset 12
        let (dt, slice) = resolve_udt_member("Program:Main.Motor1.DriveInfo.Poles", &tags, &templates)
            .expect("Poles resolves");
        assert_eq!(dt, CipDataType::Dint);
        assert_eq!(i32::from_le_bytes(slice[0..4].try_into().unwrap()), 4);

        // RatedSpeed REAL @ offset 0
        let (dt, slice) =
            resolve_udt_member("Program:Main.Motor1.DriveInfo.RatedSpeed", &tags, &templates)
                .expect("RatedSpeed resolves");
        assert_eq!(dt, CipDataType::Real);
        assert!((f32::from_le_bytes(slice[0..4].try_into().unwrap()) - 1450.0).abs() < 0.1);

        // Unknown member + nested member path both miss
        assert!(resolve_udt_member("Program:Main.Motor1.DriveInfo.Nope", &tags, &templates).is_none());
        assert!(resolve_udt_member("Program:Main.Motor1.DriveInfo.Poles.X", &tags, &templates).is_none());
    }
}
