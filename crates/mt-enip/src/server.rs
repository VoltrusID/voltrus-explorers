//! EtherNet/IP adapter used by the Adapter mode and the device simulator.
//!
//! The goal of this module is to behave like a real Logix-style target so the
//! client (and third-party clients) exercise the same wire format they meet on
//! hardware: session checks, CPF framing, Unconnected_Send + route paths,
//! Multiple_Service_Packet, partial/fragmented tag transfers, strict write type
//! checks, Forward_Open/Large_Forward_Open with real status codes, class 3
//! connected messaging over SendUnitData and class 1 implicit I/O over UDP.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::task::JoinHandle;

use crate::cip::*;
use crate::packets::*;
use crate::types::*;

/// Payload budget of a single message-router reply (type + data). Real
/// controllers answer ~500-byte unconnected/connected-explicit packets; bigger
/// transfers come back as status 0x06 and must be continued with the
/// fragmented services.
const REPLY_PAYLOAD_LIMIT: usize = 480;

/// Symbol `software control` attribute value flagging a base (non-alias) tag.
const SOFTWARE_CONTROL_BASE_TAG: u32 = 1 << 26;

// ── Public types ──

pub struct EnipServerState {
    pub tags: Mutex<BTreeMap<String, CipTagEntry>>,
    pub templates: Vec<CipTemplateDef>,
    /// Last O→T implicit I/O payload received over UDP (run/idle header + data).
    pub last_output: Mutex<Option<Vec<u8>>>,
    /// Data of the output assembly ([`OUTPUT_ASSEMBLY`]) as an originator last
    /// set it with Set_Attribute_Single.
    pub output_assembly: Mutex<Vec<u8>>,
    /// Implicit I/O connections keyed by the O→T connection id the target assigned.
    io: Mutex<std::collections::HashMap<u32, IoConn>>,
    io_socket: Option<Arc<UdpSocket>>,
    next_conn_id: AtomicU32,
    tcp_port: u16,
}

impl EnipServerState {
    /// UDP port the implicit-I/O socket is bound to (2222 when free).
    pub fn io_port(&self) -> Option<u16> {
        self.io_socket.as_ref().and_then(|s| s.local_addr().ok()).map(|a| a.port())
    }

    /// TCP port the listener is actually bound to (useful when configured with port 0).
    pub fn tcp_port(&self) -> u16 {
        self.tcp_port
    }

    /// Number of implicit I/O connections currently open on the target.
    pub async fn open_io_connections(&self) -> usize {
        self.io.lock().await.len()
    }
}

struct IoConn {
    last_rx: Instant,
}

#[derive(Debug, Clone)]
pub struct CipTagEntry {
    pub data_type: CipDataType,
    pub element_count: u16,
    pub value: CipValue,
    pub template_id: Option<u16>,
    /// Refuses a client's Write Tag (the simulator drives this tag).
    pub read_only: bool,
}

/// Assign a freshly computed scalar to a tag, preserving array shape.
pub fn assign_scalar(entry: &mut CipTagEntry, value: CipValue) {
    if entry.element_count > 1 && matches!(entry.value, CipValue::Array(_)) {
        entry.value = CipValue::Array(vec![value; entry.element_count as usize]);
    } else {
        entry.value = value;
    }
}

/// Build the (name, entry) pair for a tag definition. A scalar initial value
/// on a multi-element tag is expanded to an array of that value.
pub fn tag_entry_from_def(t: CipTagDef) -> (String, CipTagEntry) {
    let value = if t.element_count > 1 && !matches!(t.initial_value, CipValue::Array(_)) && t.template_id.is_none() {
        CipValue::Array(vec![t.initial_value; t.element_count as usize])
    } else {
        t.initial_value
    };
    (
        t.name,
        CipTagEntry { data_type: t.data_type, element_count: t.element_count.max(1), value, template_id: t.template_id, read_only: t.read_only },
    )
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
    let tag_map: BTreeMap<String, CipTagEntry> = tags.into_iter().map(tag_entry_from_def).collect();

    let listener = TcpListener::bind(format!("{}:{}", config.bind_addr, config.port)).await?;

    // Implicit I/O socket: UDP 2222 on the bind address, falling back to an
    // ephemeral port when 2222 is taken (e.g. another adapter on this host).
    let io_socket = match UdpSocket::bind(format!("{}:{}", config.bind_addr, IO_UDP_PORT)).await {
        Ok(s) => Some(Arc::new(s)),
        Err(_) => UdpSocket::bind(format!("{}:0", config.bind_addr)).await.ok().map(Arc::new),
    };

    let state = Arc::new(EnipServerState {
        tags: Mutex::new(tag_map),
        templates,
        last_output: Mutex::new(None),
        output_assembly: Mutex::new(Vec::new()),
        io: Mutex::new(std::collections::HashMap::new()),
        io_socket,
        next_conn_id: AtomicU32::new(0x2000_0001),
        tcp_port: listener.local_addr()?.port(),
    });

    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    if let Some(sock) = state.io_socket.clone() {
        tokio::spawn(io_receiver(sock, state.clone(), shutdown_rx.clone()));
    }

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
                            tokio::spawn(handle_connection(stream, addr, state, log_tx, shutdown.clone()));
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

fn log(log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>, service: &str, tag: &str, message: String) {
    let _ = log_tx.send(EnipServerLogEntry {
        timestamp: now_ms(),
        service: service.into(),
        tag_name: tag.into(),
        message,
    });
}

// ── Connection handler ──

/// One CIP connection opened with Forward_Open on this TCP session.
struct OpenConn {
    serial: u16,
    vendor: u16,
    originator: u32,
    /// Connection id the originator uses to address the target (assigned here).
    o_to_t: u32,
    /// Connection id the target answers with (chosen by the originator).
    t_to_o: u32,
    class3: bool,
    producer: Option<JoinHandle<()>>,
}

/// Serve one client's encapsulation session until it leaves or the server
/// shuts down (a stopped adapter drops its clients, as a powered-off one does).
async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    state: Arc<EnipServerState>,
    log_tx: mpsc::UnboundedSender<EnipServerLogEntry>,
    mut shutdown: watch::Receiver<bool>,
) {
    log(&log_tx, "connect", "", format!("Client connected from {addr}"));

    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut conns: Vec<OpenConn> = Vec::new();
    let mut session_handle: u32 = 0;
    let mut header_buf = [0u8; ENIP_HEADER_SIZE];

    loop {
        let received = tokio::select! {
            read = reader.read_exact(&mut header_buf) => read.is_ok(),
            _ = shutdown.changed() => false,
        };
        if !received {
            break;
        }

        let command = u16::from_le_bytes([header_buf[0], header_buf[1]]);
        let length = u16::from_le_bytes([header_buf[2], header_buf[3]]);
        let recv_session = u32::from_le_bytes([header_buf[4], header_buf[5], header_buf[6], header_buf[7]]);
        let sender_context = u64::from_le_bytes(header_buf[12..20].try_into().unwrap());

        let mut body = vec![0u8; length as usize];
        if length > 0 && reader.read_exact(&mut body).await.is_err() {
            break;
        }

        let session_ok = session_handle != 0 && recv_session == session_handle;
        let reply: Option<Vec<u8>> = match command {
            CMD_LIST_IDENTITY => {
                let mut r = build_list_identity_response([0, 0, 0, 0], state.tcp_port);
                r[12..20].copy_from_slice(&sender_context.to_le_bytes());
                Some(r)
            }
            CMD_LIST_SERVICES => Some(build_list_services_reply(sender_context)),
            CMD_LIST_INTERFACES => Some(build_enip_frame(CMD_LIST_INTERFACES, 0, sender_context, 0, &0u16.to_le_bytes())),
            CMD_REGISTER_SESSION => {
                let version = body.get(0..2).map(|b| u16::from_le_bytes([b[0], b[1]]));
                if body.len() != 4 {
                    Some(build_enip_frame(command, 0, sender_context, 0x0065, &[]))
                } else if version != Some(1) {
                    // Unsupported protocol revision: answer with our supported version.
                    Some(build_enip_frame(command, 0, sender_context, 0x0069, &[1, 0, 0, 0]))
                } else if session_handle != 0 {
                    // A second RegisterSession on the same TCP connection is rejected.
                    Some(build_enip_frame(command, 0, sender_context, 0x0001, &[1, 0, 0, 0]))
                } else {
                    session_handle = rand_session_handle();
                    Some(build_enip_frame(command, session_handle, sender_context, 0, &[1, 0, 0, 0]))
                }
            }
            CMD_UNREGISTER_SESSION => break,
            CMD_SEND_RR_DATA if !session_ok => {
                Some(build_enip_frame(command, recv_session, sender_context, 0x0064, &[]))
            }
            CMD_SEND_RR_DATA => Some(
                handle_send_rr(session_handle, sender_context, &body, &state, &log_tx, &mut conns, addr).await,
            ),
            CMD_SEND_UNIT_DATA if !session_ok => {
                Some(build_enip_frame(command, recv_session, sender_context, 0x0064, &[]))
            }
            CMD_SEND_UNIT_DATA => handle_send_unit_data(session_handle, &body, &state, &log_tx, &conns).await,
            _ => Some(build_enip_frame(command, recv_session, sender_context, 0x0001, &[])),
        };

        if let Some(reply) = reply {
            if writer.write_all(&reply).await.is_err() {
                break;
            }
        }
    }

    close_all(&mut conns, &state).await;
    let _ = writer.shutdown().await;
    log(&log_tx, "disconnect", "", format!("Client disconnected from {addr}"));
}

async fn close_all(conns: &mut Vec<OpenConn>, state: &Arc<EnipServerState>) {
    for c in conns.drain(..) {
        if let Some(h) = c.producer {
            h.abort();
        }
        state.io.lock().await.remove(&c.o_to_t);
    }
}

/// ListServices reply: one Communications item advertising CIP encapsulation
/// over TCP (bit 5) and class 0/1 over UDP (bit 8) — ODVA Vol 2 §2-4.5.
fn build_list_services_reply(sender_context: u64) -> Vec<u8> {
    let mut item = Vec::with_capacity(20);
    item.extend_from_slice(&1u16.to_le_bytes()); // encapsulation protocol version
    item.extend_from_slice(&0x0120u16.to_le_bytes()); // capability flags
    let mut name = [0u8; 16];
    name[..14].copy_from_slice(b"Communications");
    item.extend_from_slice(&name);
    let body = build_cpf(&[CpfItem::new(0x0100, item)]);
    build_enip_frame(CMD_LIST_SERVICES, 0, sender_context, 0, &body)
}

fn build_rr_reply(session_handle: u32, sender_context: u64, cip_data: &[u8], extra: &[CpfItem]) -> Vec<u8> {
    let mut items = vec![
        CpfItem::new(CPF_NULL_ADDRESS, Vec::new()),
        CpfItem::new(CPF_UNCONNECTED_DATA, cip_data.to_vec()),
    ];
    items.extend_from_slice(extra);
    build_enip_frame(CMD_SEND_RR_DATA, session_handle, sender_context, 0, &wrap_rr_body(0, &build_cpf(&items)))
}

/// Build a CIP error reply: `service|0x80, 0, status, ext words, ext…`.
fn err_reply(service: u8, status: u8, ext: &[u16]) -> Vec<u8> {
    let mut r = vec![service | 0x80, 0x00, status, ext.len() as u8];
    for e in ext {
        r.extend_from_slice(&e.to_le_bytes());
    }
    r
}

fn ok_reply(service: u8) -> Vec<u8> {
    vec![service | 0x80, 0x00, 0x00, 0x00]
}

// ── SendRRData ──

async fn handle_send_rr(
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
    conns: &mut Vec<OpenConn>,
    peer: SocketAddr,
) -> Vec<u8> {
    let items = match parse_rr_body(body) {
        Ok(i) => i,
        Err(_) => return build_enip_frame(CMD_SEND_RR_DATA, session_handle, sender_context, 0x0003, &[]),
    };
    let data = match items.iter().find(|i| i.type_id == CPF_UNCONNECTED_DATA) {
        Some(d) if d.data.len() >= 2 => d.data.clone(),
        _ => return build_enip_frame(CMD_SEND_RR_DATA, session_handle, sender_context, 0x0003, &[]),
    };

    let service = data[0];
    let targets_cm = {
        let pw = data[1] as usize;
        data.get(2..2 + pw * 2)
            .map(|p| {
                let (c, i, _) = parse_logical_path(p);
                c == Some(CONNECTION_MANAGER_CLASS_ID) && i == Some(1)
            })
            .unwrap_or(false)
    };

    if targets_cm && (service == CIP_FORWARD_OPEN || service == CIP_LARGE_FORWARD_OPEN) {
        let t_to_o_sockaddr = items
            .iter()
            .find(|i| i.type_id == CPF_SOCKADDR_T_TO_O)
            .and_then(|i| parse_sockaddr_item(&i.data));
        let (cip, extra) = handle_forward_open(&data, t_to_o_sockaddr, peer, state, log_tx, conns).await;
        return build_rr_reply(session_handle, sender_context, &cip, &extra);
    }
    if targets_cm && service == CIP_FORWARD_CLOSE {
        let cip = handle_forward_close(&data, state, log_tx, conns).await;
        return build_rr_reply(session_handle, sender_context, &cip, &[]);
    }

    let reply = route_cip(&data, state, log_tx).await;
    build_rr_reply(session_handle, sender_context, &reply, &[])
}

// ── SendUnitData (class 3 connected explicit messaging) ──

async fn handle_send_unit_data(
    session_handle: u32,
    body: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
    conns: &[OpenConn],
) -> Option<Vec<u8>> {
    let (conn_id, seq, cip) = parse_unit_data_body(body).ok()?;
    let conn = match conns.iter().find(|c| c.class3 && c.o_to_t == conn_id) {
        Some(c) => c,
        None => {
            // A target silently drops connected data for unknown connections.
            log(log_tx, "unit_data", "", format!("SendUnitData for unknown connection 0x{conn_id:08X} dropped"));
            return None;
        }
    };
    let reply = route_cip(&cip, state, log_tx).await;
    Some(build_send_unit_data(session_handle, conn.t_to_o, seq, &reply))
}

// ── Forward_Open / Forward_Close ──

/// Parse a connection path: optional port segments (this simulator sits at
/// backplane port 1, slot 0) followed by the target object's logical path.
/// Returns the target class on success, or the extended status to report.
fn connection_target(path: &[u8]) -> Result<Option<u16>, u16> {
    let mut i = 0;
    while i < path.len() && path[i] & 0xE0 == 0x00 {
        let port = path[i] & 0x0F;
        if path[i] & 0x10 != 0 || port != 1 {
            return Err(0x0311); // port not available
        }
        if path.get(i + 1) != Some(&0) {
            return Err(0x0312); // link address not valid
        }
        i += 2;
    }
    let (class, _, _) = parse_logical_path(&path[i..]);
    Ok(class)
}

async fn handle_forward_open(
    cip: &[u8],
    t_to_o_sockaddr: Option<([u8; 4], u16)>,
    peer: SocketAddr,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
    conns: &mut Vec<OpenConn>,
) -> (Vec<u8>, Vec<CpfItem>) {
    let service = cip[0];
    let p = match parse_forward_open_request(cip) {
        Some(p) => p,
        None => return (err_reply(service, 0x13, &[]), Vec::new()),
    };

    let fail = |ext: u16| {
        let mut r = err_reply(service, 0x01, &[ext]);
        r.extend_from_slice(&p.connection_serial.to_le_bytes());
        r.extend_from_slice(&p.vendor_id.to_le_bytes());
        r.extend_from_slice(&p.originator_serial.to_le_bytes());
        r.push(0); // remaining path size
        r.push(0);
        (r, Vec::new())
    };

    if conns
        .iter()
        .any(|c| c.serial == p.connection_serial && c.vendor == p.vendor_id && c.originator == p.originator_serial)
    {
        return fail(0x0100);
    }

    let target_class = match connection_target(&p.connection_path) {
        Ok(c) => c,
        Err(ext) => return fail(ext),
    };
    let class = p.transport_class_trigger & 0x0F;
    let class3 = class == 3;
    let implicit = class == 0 || class == 1;
    match (class3, implicit, target_class) {
        (true, _, Some(0x02)) => {}                // explicit messaging → Message Router
        (_, true, Some(0x04)) => {}                // implicit I/O → Assembly
        (_, _, None) => return fail(0x0315),       // no class segment in the path
        _ => return fail(0x0103),                  // transport class/trigger not supported for this object
    }
    if p.o_to_t_rpi_us == 0 || p.t_to_o_rpi_us == 0 {
        return fail(0x0111);
    }

    // Network connection parameters: size + connection types.
    let (o_size, o_type, t_size, t_type) = if p.large {
        (
            p.o_to_t_params & 0xFFFF,
            (p.o_to_t_params >> 29) & 3,
            p.t_to_o_params & 0xFFFF,
            (p.t_to_o_params >> 29) & 3,
        )
    } else {
        (
            p.o_to_t_params & 0x01FF,
            (p.o_to_t_params >> 13) & 3,
            p.t_to_o_params & 0x01FF,
            (p.t_to_o_params >> 13) & 3,
        )
    };
    if !p.large && p.o_to_t_params > 0xFFFF || !p.large && p.t_to_o_params > 0xFFFF {
        return fail(0x0108);
    }
    if class3 {
        // Explicit connections are bidirectional point-to-point.
        if o_type != 2 || t_type != 2 {
            return fail(0x0108);
        }
    } else if o_type == 0 && t_type == 0 {
        return fail(0x0108);
    }
    if (o_type == 3 || t_type == 3) || (!class3 && t_type != 0 && t_size < 2) {
        return fail(if t_size < 2 { 0x0128 } else { 0x0108 });
    }

    let o_to_t_id = state.next_conn_id.fetch_add(1, Ordering::Relaxed);
    let t_to_o_id = p.t_to_o_connection_id;
    let api = p.t_to_o_rpi_us.clamp(10_000, 3_600_000_000);
    let api_o = p.o_to_t_rpi_us.clamp(10_000, 3_600_000_000);

    let mut extra = Vec::new();
    let mut producer = None;
    if implicit {
        if let Some(sock) = state.io_socket.clone() {
            let port = sock.local_addr().map(|a| a.port()).unwrap_or(IO_UDP_PORT);
            extra.push(CpfItem::new(CPF_SOCKADDR_O_TO_T, sockaddr_item_data([0, 0, 0, 0], port)));
            if t_type != 0 {
                let dest_port = t_to_o_sockaddr.map(|(_, port)| port).unwrap_or(IO_UDP_PORT);
                let dest_ip: IpAddr = peer.ip();
                let dest = SocketAddr::new(dest_ip, dest_port);
                let watchdog = if o_type != 0 {
                    // multiplier code n → ×(4 << n)
                    let factor = 4u64 << (p.timeout_multiplier.min(7) as u32);
                    Some(Duration::from_micros(api_o as u64 * factor).max(Duration::from_millis(200)))
                } else {
                    None
                };
                state.io.lock().await.insert(o_to_t_id, IoConn { last_rx: Instant::now() });
                producer = Some(tokio::spawn(io_producer(
                    sock,
                    dest,
                    state.clone(),
                    o_to_t_id,
                    t_to_o_id,
                    api,
                    (t_size as usize).saturating_sub(2),
                    watchdog,
                    log_tx.clone(),
                )));
            } else {
                state.io.lock().await.insert(o_to_t_id, IoConn { last_rx: Instant::now() });
            }
        } else {
            return fail(0x0113); // out of connections: no UDP socket
        }
    }

    log(
        log_tx,
        "forward_open",
        "",
        format!(
            "{} class {class} serial=0x{:04X} O→T=0x{o_to_t_id:08X} T→O=0x{t_to_o_id:08X} rpi={}us sizes O→T {o_size} / T→O {t_size}",
            if p.large { "Large_Forward_Open" } else { "Forward_Open" },
            p.connection_serial,
            p.t_to_o_rpi_us,
        ),
    );

    let mut r = ok_reply(service);
    r.extend_from_slice(&o_to_t_id.to_le_bytes());
    r.extend_from_slice(&t_to_o_id.to_le_bytes());
    r.extend_from_slice(&p.connection_serial.to_le_bytes());
    r.extend_from_slice(&p.vendor_id.to_le_bytes());
    r.extend_from_slice(&p.originator_serial.to_le_bytes());
    r.extend_from_slice(&api_o.to_le_bytes());
    r.extend_from_slice(&api.to_le_bytes());
    r.push(0); // application reply size (words)
    r.push(0); // reserved

    conns.push(OpenConn {
        serial: p.connection_serial,
        vendor: p.vendor_id,
        originator: p.originator_serial,
        o_to_t: o_to_t_id,
        t_to_o: t_to_o_id,
        class3,
        producer,
    });
    (r, extra)
}

async fn handle_forward_close(
    cip: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
    conns: &mut Vec<OpenConn>,
) -> Vec<u8> {
    let pw = cip[1] as usize;
    let b = match cip.get(2 + pw * 2..) {
        Some(b) if b.len() >= 10 => b,
        _ => return err_reply(CIP_FORWARD_CLOSE, 0x13, &[]),
    };
    let serial = u16::from_le_bytes([b[2], b[3]]);
    let vendor = u16::from_le_bytes([b[4], b[5]]);
    let originator = u32::from_le_bytes([b[6], b[7], b[8], b[9]]);

    let mut r;
    if let Some(pos) = conns
        .iter()
        .position(|c| c.serial == serial && c.vendor == vendor && c.originator == originator)
    {
        let c = conns.remove(pos);
        if let Some(h) = c.producer {
            h.abort();
        }
        state.io.lock().await.remove(&c.o_to_t);
        log(log_tx, "forward_close", "", format!("Forward_Close serial=0x{serial:04X}"));
        r = ok_reply(CIP_FORWARD_CLOSE);
    } else {
        log(log_tx, "forward_close", "", format!("Forward_Close serial=0x{serial:04X}: connection not found"));
        r = err_reply(CIP_FORWARD_CLOSE, 0x01, &[0x0107]);
    }
    r.extend_from_slice(&serial.to_le_bytes());
    r.extend_from_slice(&vendor.to_le_bytes());
    r.extend_from_slice(&originator.to_le_bytes());
    r.push(0); // application reply size / remaining path size
    r.push(0);
    r
}

// ── Class 1 implicit I/O over UDP ──

/// Receives O→T datagrams and refreshes each connection's watchdog.
async fn io_receiver(sock: Arc<UdpSocket>, state: Arc<EnipServerState>, mut shutdown: watch::Receiver<bool>) {
    let mut buf = vec![0u8; 2048];
    loop {
        tokio::select! {
            r = sock.recv_from(&mut buf) => {
                let Ok((n, _peer)) = r else { continue };
                if let Ok((conn_id, _seq32, _seq16, payload)) = parse_io_packet(&buf[..n]) {
                    let mut io = state.io.lock().await;
                    if let Some(c) = io.get_mut(&conn_id) {
                        c.last_rx = Instant::now();
                        drop(io);
                        *state.last_output.lock().await = Some(payload);
                    }
                }
            }
            _ = shutdown.changed() => break,
        }
    }
}

/// Cyclic T→O producer: one UDP datagram per RPI carrying the current input
/// assembly (tag value image) padded/truncated to the negotiated size. Stops
/// when the O→T watchdog expires, like a real adapter dropping the connection.
#[allow(clippy::too_many_arguments)]
async fn io_producer(
    sock: Arc<UdpSocket>,
    dest: SocketAddr,
    state: Arc<EnipServerState>,
    o_to_t_id: u32,
    t_to_o_id: u32,
    api_us: u32,
    payload_len: usize,
    watchdog: Option<Duration>,
    log_tx: mpsc::UnboundedSender<EnipServerLogEntry>,
) {
    let mut ticker = tokio::time::interval(Duration::from_micros(api_us as u64));
    let mut seq32: u32 = 0;
    let mut seq16: u16 = 0;
    loop {
        ticker.tick().await;
        if let Some(limit) = watchdog {
            let expired = state
                .io
                .lock()
                .await
                .get(&o_to_t_id)
                .map(|c| c.last_rx.elapsed() > limit)
                .unwrap_or(true);
            if expired {
                state.io.lock().await.remove(&o_to_t_id);
                log(&log_tx, "forward_open", "", format!("I/O connection 0x{o_to_t_id:08X} timed out (no O→T data)"));
                break;
            }
        }
        let mut assembly = snapshot_input_assembly(&state).await;
        assembly.resize(payload_len, 0);
        seq32 = seq32.wrapping_add(1);
        seq16 = seq16.wrapping_add(1);
        let frame = build_io_packet(t_to_o_id, seq32, seq16, &assembly);
        if sock.send_to(&frame, dest).await.is_err() {
            break;
        }
    }
}

/// Encode the current tag values as a flat input-assembly byte image.
async fn snapshot_input_assembly(state: &Arc<EnipServerState>) -> Vec<u8> {
    let tags = state.tags.lock().await;
    let mut out = Vec::new();
    for entry in tags.values() {
        out.extend_from_slice(&entry_image(entry).bytes);
    }
    out
}

// ── CIP routing ──

type BoxFut<'a> = Pin<Box<dyn Future<Output = Vec<u8>> + Send + 'a>>;

/// Route a message-router request to its handler and return the bare reply
/// (no ENIP / CPF wrapping). Shared by unconnected, connected and embedded
/// (Unconnected_Send / Multiple_Service_Packet) paths.
fn route_cip<'a>(
    cip: &'a [u8],
    state: &'a Arc<EnipServerState>,
    log_tx: &'a mpsc::UnboundedSender<EnipServerLogEntry>,
) -> BoxFut<'a> {
    Box::pin(route_cip_inner(cip, state, log_tx))
}

/// Split a request path into leading symbolic segments (program scope) and the rest.
fn split_scope(path: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i + 1 < path.len() && path[i] == 0x91 {
        let len = path[i + 1] as usize;
        i += 2 + len + (len % 2);
    }
    let i = i.min(path.len());
    path.split_at(i)
}

async fn route_cip_inner(
    cip: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    if cip.len() < 2 {
        // Too short to carry a service byte — reply with a generic error so the
        // client's parser never sees an empty message-router buffer.
        return err_reply(cip.first().copied().unwrap_or(0), 0x13, &[]);
    }
    let service = cip[0];
    let pw = cip[1] as usize;
    let path = match cip.get(2..2 + pw * 2) {
        Some(p) => p,
        None => return err_reply(service, 0x26, &[]), // path size invalid
    };
    let (scope, rest) = split_scope(path);
    let (class, instance, _attr) = parse_logical_path(rest);

    match class {
        Some(0x06) if instance == Some(1) => match service {
            CIP_UNCONNECTED_SEND => handle_unconnected_send(cip, state, log_tx).await,
            _ => err_reply(service, 0x08, &[]),
        },
        Some(0x02) if instance == Some(1) => match service {
            CIP_MULTIPLE_SERVICE_PACKET => handle_multiple_service(cip, pw, state, log_tx).await,
            _ => err_reply(service, 0x08, &[]),
        },
        Some(0x6C) => match service {
            0x03 => handle_template_get_attributes(cip, instance, state, log_tx).await,
            CIP_READ_TAG => handle_template_read(cip, pw, instance, state, log_tx).await,
            _ => err_reply(service, 0x08, &[]),
        },
        Some(c) if c == SYMBOL_CLASS_ID as u16 => match service {
            CIP_GET_INSTANCE_ATTRIBUTE_LIST => {
                handle_get_instance_attribute_list(cip, pw, scope, instance.unwrap_or(0), state, log_tx).await
            }
            _ => err_reply(service, 0x08, &[]),
        },
        Some(0x01) => handle_identity(service, instance, rest),
        Some(ASSEMBLY_CLASS_ID) => {
            let data = cip.get(2 + pw * 2..).unwrap_or(&[]);
            handle_assembly(service, instance, rest, data, state).await
        }
        Some(0x4F) | Some(0x4E) => handle_energy(service, class.unwrap(), instance, rest),
        Some(_) => err_reply(service, 0x05, &[]),
        None => match service {
            CIP_READ_TAG | CIP_READ_TAG_FRAGMENTED => handle_read_tag(cip, pw, service, state, log_tx).await,
            CIP_WRITE_TAG | CIP_WRITE_TAG_FRAGMENTED => handle_write_tag(cip, pw, service, state, log_tx).await,
            _ => err_reply(service, 0x08, &[]),
        },
    }
}

/// Unconnected_Send: unwrap the embedded message, check the route path, and
/// return the embedded reply as-is (the Connection Manager does not wrap
/// successful replies — cpppo capture `D2 00 00 00 C3 00 …`).
async fn handle_unconnected_send(
    cip: &[u8],
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let (msg, route) = match parse_unconnected_send(cip) {
        Some(v) => v,
        None => return err_reply(CIP_UNCONNECTED_SEND, 0x01, &[0x0205]),
    };
    // The controller sits at backplane port 1, slot 0.
    let mut i = 0;
    while i < route.len() {
        let seg = route[i];
        if seg & 0xE0 != 0 {
            return err_reply(CIP_UNCONNECTED_SEND, 0x01, &[0x0315]);
        }
        let port = seg & 0x0F;
        if seg & 0x10 != 0 || port != 1 {
            return err_reply(CIP_UNCONNECTED_SEND, 0x01, &[0x0311]);
        }
        if route.get(i + 1) != Some(&0) {
            return err_reply(CIP_UNCONNECTED_SEND, 0x01, &[0x0312]);
        }
        i += 2;
    }
    route_cip(msg, state, log_tx).await
}

/// Multiple_Service_Packet: run each embedded request and pack the replies.
/// The packet's own status is 0x1E (embedded service error) when any embedded
/// reply failed, but the replies are still delivered (Vol 1 §A-5.1).
async fn handle_multiple_service(
    cip: &[u8],
    path_words: usize,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let data = &cip[2 + path_words * 2..];
    let parts = match split_multiple_service_data(data) {
        Ok(p) => p,
        Err(_) => return err_reply(CIP_MULTIPLE_SERVICE_PACKET, 0x13, &[]),
    };
    log(log_tx, "multi_service", "", format!("Multiple_Service_Packet with {} services", parts.len()));
    let mut replies = Vec::with_capacity(parts.len());
    let mut any_error = false;
    for p in &parts {
        let r = route_cip(p, state, log_tx).await;
        if r.get(2).copied().unwrap_or(1) != 0 && r.get(2) != Some(&CIP_STATUS_PARTIAL) {
            any_error = true;
        }
        replies.push(r);
    }
    let mut out = vec![
        CIP_MULTIPLE_SERVICE_PACKET | 0x80,
        0x00,
        if any_error { 0x1E } else { 0x00 },
        0x00,
    ];
    out.extend_from_slice(&(replies.len() as u16).to_le_bytes());
    let mut offset = 2 + 2 * replies.len();
    for r in &replies {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        offset += r.len();
    }
    for r in &replies {
        out.extend_from_slice(r);
    }
    out
}

// ── Tag database access ──

/// A tag's wire image: type, element size, element count and raw bytes.
struct Image {
    ty: TagType,
    es: usize,
    total: usize,
    bytes: Vec<u8>,
}

fn entry_image(entry: &CipTagEntry) -> Image {
    if let Some(tid) = entry.template_id {
        let bytes = match &entry.value {
            CipValue::Bytes(b) => b.clone(),
            _ => Vec::new(),
        };
        return Image { ty: TagType::structure(tid), es: bytes.len().max(1), total: 1, bytes };
    }
    let ty = TagType::for_data_type(entry.data_type);
    let es = ty.element_size().unwrap_or(1);
    match encode_tag_value(&ty, &entry.value) {
        Ok((count, bytes)) => Image { ty, es, total: count.max(1) as usize, bytes },
        Err(_) => Image { ty, es, total: 1, bytes: vec![0u8; es] },
    }
}

/// Where a requested tag name lives: map key, first element, and — for UDT
/// members — the (offset, size) slice inside the structure image.
struct Resolved {
    key: String,
    start: usize,
    member: Option<(usize, usize, CipDataType)>,
}

fn resolve_tag(name: &str, tags: &BTreeMap<String, CipTagEntry>, templates: &[CipTemplateDef]) -> Option<Resolved> {
    if tags.contains_key(name) {
        return Some(Resolved { key: name.to_string(), start: 0, member: None });
    }
    if let Some((base, idx)) = split_element_suffix(name) {
        if let Some(e) = tags.get(base) {
            if e.template_id.is_none() && (idx as usize) < entry_image(e).total {
                return Some(Resolved { key: base.to_string(), start: idx as usize, member: None });
            }
        }
    }
    resolve_udt_member(name, tags, templates).map(|(key, off, size, dt)| Resolved {
        key,
        start: 0,
        member: Some((off, size, dt)),
    })
}

/// Image addressed by `r` (a whole tag, or a UDT member slice).
fn resolved_image(entry: &CipTagEntry, r: &Resolved) -> Image {
    let img = entry_image(entry);
    match r.member {
        Some((off, size, dt)) => {
            let end = (off + size).min(img.bytes.len());
            let bytes = img.bytes[off.min(end)..end].to_vec();
            let es = dt.element_size().max(1) as usize;
            Image { ty: TagType::atomic(dt.code()), es, total: (bytes.len() / es).max(1), bytes }
        }
        None => img,
    }
}

fn aligned_limit(es: usize, limit: usize) -> usize {
    let align = if es <= 8 { es.max(1) } else { 4 };
    limit - limit % align
}

// ── CIP Read Tag (0x4C) / Read Tag Fragmented (0x52) ──

async fn handle_read_tag(
    cip: &[u8],
    pw: usize,
    service: u8,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let path = &cip[2..2 + pw * 2];
    let after = &cip[2 + pw * 2..];
    let Some(name) = decode_symbolic_path(path) else {
        return err_reply(service, 0x04, &[0x0000]);
    };
    let need = if service == CIP_READ_TAG_FRAGMENTED { 6 } else { 2 };
    if after.len() < need {
        return err_reply(service, 0x13, &[]);
    }
    let count = u16::from_le_bytes([after[0], after[1]]) as usize;
    let offset = if service == CIP_READ_TAG_FRAGMENTED {
        u32::from_le_bytes([after[2], after[3], after[4], after[5]]) as usize
    } else {
        0
    };

    let tags = state.tags.lock().await;
    let Some(r) = resolve_tag(&name, &tags, &state.templates) else {
        drop(tags);
        log(log_tx, "read", &name, format!("Read tag not found: {name}"));
        return err_reply(service, 0x04, &[0x0000]);
    };
    let entry = &tags[&r.key];
    let img = resolved_image(entry, &r);
    let display = entry.value.display_value();
    drop(tags);

    let first = r.start * img.es;
    let want = count.max(1) * img.es;
    if first + want > img.total * img.es || (img.es == 0) {
        return err_reply(service, 0xFF, &[0x2105]);
    }
    let region = &img.bytes[first..first + want.min(img.bytes.len().saturating_sub(first))];
    if offset > region.len() {
        return err_reply(service, 0xFF, &[0x2105]);
    }
    let type_bytes = img.ty.encode();
    let capacity = aligned_limit(img.es, REPLY_PAYLOAD_LIMIT - type_bytes.len());
    let remaining = &region[offset..];
    let (chunk, partial) = if remaining.len() > capacity {
        (&remaining[..capacity], true)
    } else {
        (remaining, false)
    };

    let mut reply = vec![service | 0x80, 0x00, if partial { CIP_STATUS_PARTIAL } else { 0 }, 0x00];
    reply.extend_from_slice(&type_bytes);
    reply.extend_from_slice(chunk);
    log(
        log_tx,
        "read",
        &name,
        format!("Read {} = {}{}", name, display, if partial { " (partial)" } else { "" }),
    );
    reply
}

// ── CIP Write Tag (0x4D) / Write Tag Fragmented (0x53) ──

async fn handle_write_tag(
    cip: &[u8],
    pw: usize,
    service: u8,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let path = &cip[2..2 + pw * 2];
    let after = &cip[2 + pw * 2..];
    let Some(name) = decode_symbolic_path(path) else {
        return err_reply(service, 0x04, &[0x0000]);
    };
    let Ok((req_ty, used)) = TagType::parse(after) else {
        return err_reply(service, 0x13, &[]);
    };
    let fragmented = service == CIP_WRITE_TAG_FRAGMENTED;
    let header = used + 2 + if fragmented { 4 } else { 0 };
    if after.len() < header {
        return err_reply(service, 0x13, &[]);
    }
    let count = u16::from_le_bytes([after[used], after[used + 1]]) as usize;
    let offset = if fragmented {
        u32::from_le_bytes(after[used + 2..used + 6].try_into().unwrap()) as usize
    } else {
        0
    };
    let data = &after[header..];

    let mut tags = state.tags.lock().await;
    let Some(r) = resolve_tag(&name, &tags, &state.templates) else {
        drop(tags);
        log(log_tx, "write", &name, format!("Write tag not found: {name}"));
        return err_reply(service, 0x04, &[0x0000]);
    };
    let entry = tags.get_mut(&r.key).unwrap();
    if entry.read_only {
        drop(tags);
        log(log_tx, "write", &name, format!("Write refused: {name} is read-only (driven by the simulator)"));
        return err_reply(service, 0x0E, &[]);
    }
    let whole = entry_image(entry);
    let img = resolved_image(entry, &r);

    if req_ty != img.ty {
        return err_reply(service, 0xFF, &[0x2107]);
    }
    if count == 0 || r.start + count > img.total {
        return err_reply(service, 0xFF, &[0x2105]);
    }
    let region_len = count * img.es;
    if fragmented {
        if offset + data.len() > region_len {
            return err_reply(service, 0x15, &[]);
        }
    } else if data.len() < region_len {
        return err_reply(service, 0x13, &[]);
    } else if data.len() > region_len {
        return err_reply(service, 0x15, &[]);
    }

    // Splice the new bytes into the tag image.
    let mut bytes = whole.bytes.clone();
    let base = match r.member {
        Some((off, _, _)) => off,
        None => 0,
    } + r.start * img.es;
    let at = base + offset;
    if at + data.len() > bytes.len() {
        return err_reply(service, 0xFF, &[0x2105]);
    }
    bytes[at..at + data.len()].copy_from_slice(data);

    let previous = entry.value.display_value();
    let new_value = if entry.template_id.is_some() {
        Ok(CipValue::Bytes(bytes))
    } else {
        decode_tag_value(&whole.ty, &bytes, whole.total as u16)
    };
    match new_value {
        Ok(v) => {
            log(
                log_tx,
                "write",
                &name,
                format!("Write {} = {} (was {})", name, v.display_value(), previous),
            );
            entry.value = v;
            ok_reply(service)
        }
        Err(_) => err_reply(service, 0x09, &[]),
    }
}

// ── Symbol class: Get_Instance_Attribute_List (0x55) ──

async fn handle_get_instance_attribute_list(
    cip: &[u8],
    pw: usize,
    scope_path: &[u8],
    start: u32,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let svc = CIP_GET_INSTANCE_ATTRIBUTE_LIST;
    let after = &cip[2 + pw * 2..];
    if after.len() < 2 {
        return err_reply(svc, 0x13, &[]);
    }
    let n_attrs = u16::from_le_bytes([after[0], after[1]]) as usize;
    if after.len() < 2 + n_attrs * 2 {
        return err_reply(svc, 0x13, &[]);
    }
    let attrs: Vec<u16> = (0..n_attrs)
        .map(|i| u16::from_le_bytes([after[2 + 2 * i], after[3 + 2 * i]]))
        .collect();
    if attrs.iter().any(|a| !matches!(a, 1 | 2 | 3 | 5 | 6 | 7 | 8 | 10)) {
        return err_reply(svc, 0x14, &[]);
    }

    // A leading "Program:<name>" symbolic segment selects program scope.
    let scope = decode_symbolic_path(scope_path).and_then(|s| s.strip_prefix("Program:").map(str::to_string));

    struct Rec {
        name: String,
        type_word: u16,
        es: usize,
        dims: [u32; 3],
    }
    let rec_of = |name: String, e: &CipTagEntry| -> Rec {
        let img = entry_image(e);
        let (type_word, dims) = if e.template_id.is_some() {
            (0x8000 | e.template_id.unwrap(), [0, 0, 0])
        } else if e.data_type == CipDataType::String {
            (0x8000 | STRUCT_HANDLE_LOGIX_STRING, [0, 0, 0])
        } else {
            (e.data_type.code() as u16, [0, 0, 0])
        };
        let (type_word, dims) = if img.total > 1 {
            (type_word | 0x2000, [img.total as u32, 0, 0]) // one array dimension
        } else {
            (type_word, dims)
        };
        Rec { name, type_word, es: img.es, dims }
    };

    let mut records: Vec<Rec> = Vec::new();
    {
        let tags = state.tags.lock().await;
        match scope.as_deref() {
            Some(prog) => {
                let prefix = format!("Program:{prog}.");
                for (name, entry) in tags.iter() {
                    if let Some(local) = name.strip_prefix(&prefix) {
                        records.push(rec_of(local.to_string(), entry));
                    }
                }
            }
            None => {
                let mut programs = std::collections::BTreeSet::new();
                for (name, entry) in tags.iter() {
                    if let Some(rest) = name.strip_prefix("Program:") {
                        if let Some(prog) = rest.split('.').next() {
                            programs.insert(prog.to_string());
                            continue;
                        }
                    }
                    records.push(rec_of(name.clone(), entry));
                }
                for prog in programs {
                    records.push(Rec { name: format!("Program:{prog}"), type_word: 0, es: 0, dims: [0; 3] });
                }
            }
        }
    }

    let mut reply = ok_reply(svc);
    let mut partial = false;
    let mut count = 0;
    for (i, rec) in records.iter().enumerate() {
        let id = (i + 1) as u32;
        if id < start {
            continue; // returns instances >= the requested id
        }
        let mut item = Vec::new();
        item.extend_from_slice(&id.to_le_bytes());
        for a in &attrs {
            match a {
                1 => {
                    item.extend_from_slice(&(rec.name.len() as u16).to_le_bytes());
                    item.extend_from_slice(rec.name.as_bytes());
                }
                2 => item.extend_from_slice(&rec.type_word.to_le_bytes()),
                3 | 5 => item.extend_from_slice(&0u32.to_le_bytes()),
                6 => item.extend_from_slice(&SOFTWARE_CONTROL_BASE_TAG.to_le_bytes()),
                7 => item.extend_from_slice(&(rec.es as u16).to_le_bytes()),
                8 => {
                    for d in rec.dims {
                        item.extend_from_slice(&d.to_le_bytes());
                    }
                }
                10 => item.push(0), // external access: read/write
                _ => unreachable!(),
            }
        }
        if reply.len() - 4 + item.len() > REPLY_PAYLOAD_LIMIT && count > 0 {
            partial = true;
            break;
        }
        reply.extend_from_slice(&item);
        count += 1;
    }
    if partial {
        reply[2] = CIP_STATUS_PARTIAL;
    }

    log(
        log_tx,
        "browse",
        "",
        format!(
            "Get_Instance_Attribute_List scope={} from {start} ({count} records{})",
            scope.as_deref().unwrap_or("controller"),
            if partial { ", partial" } else { "" }
        ),
    );
    reply
}

// ── Identity / energy objects ──

fn handle_identity(service: u8, instance: Option<u32>, rest: &[u8]) -> Vec<u8> {
    if instance != Some(1) {
        return err_reply(service, 0x05, &[]);
    }
    match service {
        // Get_Attributes_All returns attributes 1..=7 (no state byte) — verified
        // against a 1756-L61 capture in cpppo's enip_test (`81 00 00 00 01 00 0E 00 36 00 …`).
        CIP_GET_ATTRIBUTES_ALL => {
            let mut reply = ok_reply(service);
            for attr in 1u16..=7 {
                reply.extend_from_slice(&identity_attribute(attr).unwrap());
            }
            reply
        }
        CIP_GET_ATTRIBUTE_SINGLE => {
            let (_, _, attr) = parse_logical_path(rest);
            match attr.and_then(identity_attribute) {
                Some(bytes) => {
                    let mut reply = ok_reply(service);
                    reply.extend_from_slice(&bytes);
                    reply
                }
                None => err_reply(service, 0x14, &[]),
            }
        }
        _ => err_reply(service, 0x08, &[]),
    }
}

fn handle_energy(service: u8, class: u16, instance: Option<u32>, rest: &[u8]) -> Vec<u8> {
    if instance != Some(1) {
        return err_reply(service, 0x05, &[]);
    }
    if service != CIP_GET_ATTRIBUTE_SINGLE {
        return err_reply(service, 0x08, &[]);
    }
    let (_, _, attr) = parse_logical_path(rest);
    let payload = attr.and_then(|a| if class == 0x4F { energy_base_attribute(a) } else { electrical_energy_attribute(a) });
    match payload {
        Some(bytes) => {
            let mut reply = ok_reply(service);
            reply.extend_from_slice(&bytes);
            reply
        }
        None => err_reply(service, 0x14, &[]),
    }
}

/// Bytes for an Identity object attribute (class 0x01, instance 1).
fn identity_attribute(attr: u16) -> Option<Vec<u8>> {
    let mut b = Vec::new();
    match attr {
        1 => b.extend_from_slice(&1u16.to_le_bytes()),             // Vendor ID (Rockwell)
        2 => b.extend_from_slice(&0x0Eu16.to_le_bytes()),          // Device Type (PLC)
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
        8 => b.push(3),                                             // State: operational
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

// ── Template object (class 0x6C) — UDT member definitions ──

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
            offset = offset.div_ceil(align) * align;
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

    /// Build the definition blob returned by Read Tag on class 0x6C. Per
    /// member: `[array size or bit position:u16][type:u16][offset:u32]` (8
    /// bytes, matching pycomm3 `_parse_template_data_member_info`); then
    /// `{name};\0{m0}\0{m1}\0…`, zero-padded to a 4-byte multiple.
    fn build_definition_blob(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        for (m, offset, _) in self.member_layout() {
            let info = if m.array_elements > 1 { m.array_elements } else { 0 };
            blob.extend_from_slice(&info.to_le_bytes());
            blob.extend_from_slice(&(m.data_type.code() as u16).to_le_bytes());
            blob.extend_from_slice(&offset.to_le_bytes());
        }
        blob.extend_from_slice(self.name.as_bytes());
        blob.push(0x3B); // ';' ends the template-name field
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

    /// ObjectDefinitionSize in 32-bit words. Clients request
    /// `words*4 - 23` (ODVA/Rockwell) or `words*4 - 21` (pycomm3) bytes, which
    /// must cover the whole blob.
    fn obj_definition_size_words(&self) -> u32 {
        (self.build_definition_blob().len() as u32 + 23).div_ceil(4)
    }

    /// Pack member values (one f64 per member, in order) into the struct byte
    /// image, each written at its aligned offset.
    pub fn pack_instance(&self, values: &[f64]) -> Vec<u8> {
        let mut buf = vec![0u8; self.structure_size_bytes() as usize];
        for ((m, off, _), v) in self.member_layout().iter().zip(values.iter()) {
            let bytes = encode_member(m.data_type, *v);
            let start = *off as usize;
            if start + bytes.len() <= buf.len() {
                buf[start..start + bytes.len()].copy_from_slice(&bytes);
            }
        }
        buf
    }
}

/// Encode one atomic scalar value to little-endian bytes for struct packing.
fn encode_member(dt: CipDataType, v: f64) -> Vec<u8> {
    match dt {
        CipDataType::Bool => vec![if v >= 0.5 { 1 } else { 0 }],
        CipDataType::Sint => vec![(v.round() as i8) as u8],
        CipDataType::Usint => vec![v.round() as u8],
        CipDataType::Int => (v.round() as i16).to_le_bytes().to_vec(),
        CipDataType::Uint => (v.round() as u16).to_le_bytes().to_vec(),
        CipDataType::Dint => (v.round() as i32).to_le_bytes().to_vec(),
        CipDataType::Udint | CipDataType::Dword => (v.round() as u32).to_le_bytes().to_vec(),
        CipDataType::Real => (v as f32).to_le_bytes().to_vec(),
        CipDataType::Lint => (v.round() as i64).to_le_bytes().to_vec(),
        CipDataType::Ulint => (v.round() as u64).to_le_bytes().to_vec(),
        CipDataType::Lreal => v.to_le_bytes().to_vec(),
        CipDataType::String => Vec::new(),
    }
}

/// Resolve `<udt_tag>.<member>` against the tag map + templates. Returns
/// (map key, byte offset, byte size, member type) of the member inside the
/// struct image. Single level only (nested-UDT members return None).
/// Case-insensitive member.
fn resolve_udt_member(
    name: &str,
    tags: &BTreeMap<String, CipTagEntry>,
    templates: &[CipTemplateDef],
) -> Option<(String, usize, usize, CipDataType)> {
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
    let image_len = match &entry.value {
        CipValue::Bytes(b) => b.len(),
        _ => return None,
    };
    for (m, off, size) in tmpl.member_layout() {
        if m.name.eq_ignore_ascii_case(member_path) {
            let (start, len) = (off as usize, size as usize);
            if start + len <= image_len {
                return Some((tag_name.to_string(), start, len, m.data_type));
            }
        }
    }
    None
}

/// GetAttributeList (service 0x03) on a Template instance. The reply is
/// `count u16` then per requested attribute `[id u16][status u16][value]`:
/// attr 1 StructureHandle (UINT), 2 MemberCount (UINT), 4 ObjectDefinitionSize
/// (UDINT, 32-bit words), 5 StructureSize (UDINT, bytes).
async fn handle_template_get_attributes(
    cip: &[u8],
    instance: Option<u32>,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let svc = 0x03;
    let Some(instance) = instance else {
        return err_reply(svc, 0x04, &[]);
    };
    let Some(tmpl) = state.templates.iter().find(|t| t.id as u32 == instance) else {
        return err_reply(svc, 0x05, &[]);
    };
    let pw = cip[1] as usize;
    let after = &cip[2 + pw * 2..];
    if after.len() < 2 {
        return err_reply(svc, 0x13, &[]);
    }
    let n = u16::from_le_bytes([after[0], after[1]]) as usize;
    if after.len() < 2 + n * 2 {
        return err_reply(svc, 0x13, &[]);
    }

    let mut reply = ok_reply(svc);
    reply.extend_from_slice(&(n as u16).to_le_bytes());
    for i in 0..n {
        let id = u16::from_le_bytes([after[2 + 2 * i], after[3 + 2 * i]]);
        reply.extend_from_slice(&id.to_le_bytes());
        match id {
            1 => {
                reply.extend_from_slice(&0u16.to_le_bytes());
                reply.extend_from_slice(&tmpl.id.to_le_bytes());
            }
            2 => {
                reply.extend_from_slice(&0u16.to_le_bytes());
                reply.extend_from_slice(&(tmpl.members.len() as u16).to_le_bytes());
            }
            4 => {
                reply.extend_from_slice(&0u16.to_le_bytes());
                reply.extend_from_slice(&tmpl.obj_definition_size_words().to_le_bytes());
            }
            5 => {
                reply.extend_from_slice(&0u16.to_le_bytes());
                reply.extend_from_slice(&(tmpl.structure_size_bytes() as u32).to_le_bytes());
            }
            _ => reply.extend_from_slice(&0x0014u16.to_le_bytes()), // attribute not supported, no value
        }
    }

    log(
        log_tx,
        "template",
        &tmpl.name,
        format!("Template {} attributes ({} requested)", tmpl.id, n),
    );
    reply
}

/// Read Tag (service 0x4C) on a Template instance: `offset u32 | bytes u16`
/// request, definition-blob slice reply. Status 0x06 when the reply budget
/// truncates the slice.
async fn handle_template_read(
    cip: &[u8],
    pw: usize,
    instance: Option<u32>,
    state: &Arc<EnipServerState>,
    log_tx: &mpsc::UnboundedSender<EnipServerLogEntry>,
) -> Vec<u8> {
    let svc = CIP_READ_TAG;
    let Some(instance) = instance else {
        return err_reply(svc, 0x04, &[]);
    };
    let Some(tmpl) = state.templates.iter().find(|t| t.id as u32 == instance) else {
        return err_reply(svc, 0x05, &[]);
    };
    let after = &cip[2 + pw * 2..];
    if after.len() < 6 {
        return err_reply(svc, 0x13, &[]);
    }
    let offset = u32::from_le_bytes(after[0..4].try_into().unwrap()) as usize;
    let size = u16::from_le_bytes([after[4], after[5]]) as usize;

    let blob = tmpl.build_definition_blob();
    if offset > blob.len() {
        return err_reply(svc, 0xFF, &[0x2105]);
    }
    let end = (offset + size).min(blob.len());
    let want = &blob[offset..end];
    let (chunk, partial) = if want.len() > REPLY_PAYLOAD_LIMIT {
        (&want[..REPLY_PAYLOAD_LIMIT], true)
    } else {
        (want, false)
    };
    let mut reply = vec![svc | 0x80, 0x00, if partial { CIP_STATUS_PARTIAL } else { 0 }, 0x00];
    reply.extend_from_slice(chunk);

    log(
        log_tx,
        "template",
        &tmpl.name,
        format!("Template {} read: {} blob bytes @{offset}", tmpl.id, chunk.len()),
    );
    reply
}

// ── Assembly object (class 0x04) ──

/// Assembly object class id.
pub const ASSEMBLY_CLASS_ID: u16 = 0x04;
/// Input assembly instance: every tag's value image, in tag name order — the
/// same bytes class 1 I/O produces (attribute 3, read-only).
pub const INPUT_ASSEMBLY: u32 = 100;
/// Output assembly instance: data an originator sets (attribute 3).
pub const OUTPUT_ASSEMBLY: u32 = 150;

/// Explicit access to the assemblies: Get_Attribute_Single of data (3) and
/// size (4), Set_Attribute_Single of the output assembly's data.
async fn handle_assembly(
    service: u8,
    instance: Option<u32>,
    rest: &[u8],
    data: &[u8],
    state: &Arc<EnipServerState>,
) -> Vec<u8> {
    let (_, _, attr) = parse_logical_path(rest);
    let input = match instance {
        Some(INPUT_ASSEMBLY) => true,
        Some(OUTPUT_ASSEMBLY) => false,
        _ => return err_reply(service, 0x05, &[]),
    };
    match (service, attr) {
        (CIP_GET_ATTRIBUTE_SINGLE, Some(3 | 4)) => {
            let image = if input {
                snapshot_input_assembly(state).await
            } else {
                state.output_assembly.lock().await.clone()
            };
            let mut reply = ok_reply(service);
            if attr == Some(3) {
                reply.extend_from_slice(&image);
            } else {
                reply.extend_from_slice(&u16::try_from(image.len()).unwrap_or(u16::MAX).to_le_bytes());
            }
            reply
        }
        (CIP_SET_ATTRIBUTE_SINGLE, Some(3)) if input => err_reply(service, 0x0E, &[]),
        (CIP_SET_ATTRIBUTE_SINGLE, Some(3)) => {
            *state.output_assembly.lock().await = data.to_vec();
            ok_reply(service)
        }
        (CIP_SET_ATTRIBUTE_SINGLE, Some(4)) => err_reply(service, 0x0E, &[]),
        (CIP_GET_ATTRIBUTE_SINGLE | CIP_SET_ATTRIBUTE_SINGLE, _) => err_reply(service, 0x14, &[]),
        _ => err_reply(service, 0x08, &[]),
    }
}

// ── UDP ListIdentity (discovery) ──

/// Answer ListIdentity datagrams on `socket` (UDP discovery, normally port
/// 44818) with this target's identity, advertising `ip:tcp_port` as its
/// encapsulation endpoint, until `shutdown` turns true. `on_request` sees each
/// request's sender and the reply sent to it.
pub async fn answer_list_identity(
    socket: UdpSocket,
    ip: [u8; 4],
    tcp_port: u16,
    mut shutdown: watch::Receiver<bool>,
    on_request: impl Fn(SocketAddr, &[u8], &[u8]) + Send,
) {
    let reply = build_list_identity_response(ip, tcp_port);
    let mut buf = vec![0u8; 512];
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => {
                let Ok((n, peer)) = received else { continue };
                if n >= ENIP_HEADER_SIZE && u16::from_le_bytes([buf[0], buf[1]]) == CMD_LIST_IDENTITY {
                    // Echo the request's sender context, as the TCP path does.
                    let mut answer = reply.clone();
                    answer[12..20].copy_from_slice(&buf[12..20]);
                    on_request(peer, &buf[..n], &answer);
                    let _ = socket.send_to(&answer, peer).await;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

// ── Helpers ──

fn rand_session_handle() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u32;
    ts.wrapping_mul(2654435761).max(1) // Knuth multiplicative hash, never 0
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
        assert_eq!(blob[41], 0x3B); // ';' ends the template-name field
        // Both the ODVA (-23) and pycomm3 (-21) read sizes must cover the blob.
        let words = t.obj_definition_size_words();
        assert!(words * 4 - 23 >= blob.len() as u32);
        assert!(words * 4 - 21 >= blob.len() as u32);
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
                value: CipValue::Bytes(bytes.clone()),
                template_id: Some(1),
                read_only: false,
            },
        );
        let templates = vec![t];

        // Poles DINT @ offset 12
        let (key, off, size, dt) = resolve_udt_member("Program:Main.Motor1.DriveInfo.Poles", &tags, &templates)
            .expect("Poles resolves");
        assert_eq!(key, "Program:Main.Motor1.DriveInfo");
        assert_eq!((off, size, dt), (12, 4, CipDataType::Dint));
        assert_eq!(i32::from_le_bytes(bytes[off..off + size].try_into().unwrap()), 4);

        // RatedSpeed REAL @ offset 0
        let (_, off, size, dt) = resolve_udt_member("Program:Main.Motor1.DriveInfo.RatedSpeed", &tags, &templates)
            .expect("RatedSpeed resolves");
        assert_eq!(dt, CipDataType::Real);
        assert!((f32::from_le_bytes(bytes[off..off + size].try_into().unwrap()) - 1450.0).abs() < 0.1);

        // Unknown member + nested member path both miss
        assert!(resolve_udt_member("Program:Main.Motor1.DriveInfo.Nope", &tags, &templates).is_none());
        assert!(resolve_udt_member("Program:Main.Motor1.DriveInfo.Poles.X", &tags, &templates).is_none());
    }

    // ── Protocol behaviour of the simulated target ──

    fn h(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    fn tag(dt: CipDataType, count: u16, v: CipValue) -> CipTagDef {
        CipTagDef { name: String::new(), data_type: dt, element_count: count, initial_value: v, template_id: None, read_only: false }
    }

    struct Fx {
        state: Arc<EnipServerState>,
        tx: mpsc::UnboundedSender<EnipServerLogEntry>,
        _rx: mpsc::UnboundedReceiver<EnipServerLogEntry>,
        _shutdown: watch::Sender<bool>,
    }

    impl Fx {
        async fn new() -> Self {
            let mut tags = vec![
                CipTagDef { name: "Counter".into(), ..tag(CipDataType::Dint, 1, CipValue::Dint(7)) },
                CipTagDef { name: "Speed".into(), ..tag(CipDataType::Real, 1, CipValue::Real(1.5)) },
                CipTagDef { name: "Flag".into(), ..tag(CipDataType::Bool, 1, CipValue::Bool(true)) },
                CipTagDef { name: "Name".into(), ..tag(CipDataType::String, 1, CipValue::String("Hi".into())) },
                CipTagDef {
                    name: "Arr".into(),
                    ..tag(CipDataType::Dint, 300, CipValue::Array((0..300).map(CipValue::Dint).collect()))
                },
                CipTagDef { name: "Program:Main.Motor".into(), ..tag(CipDataType::Real, 1, CipValue::Real(2.5)) },
                CipTagDef { name: "Sensor".into(), read_only: true, ..tag(CipDataType::Real, 1, CipValue::Real(9.5)) },
            ];
            for i in 0..60 {
                tags.push(CipTagDef { name: format!("Bulk{i:03}_with_a_longish_name"), ..tag(CipDataType::Dint, 1, CipValue::Dint(i)) });
            }
            let t = drive_info_template();
            let packed = t.pack_instance(&[1450.0, 12.4, 7.5, 4.0]);
            tags.push(CipTagDef {
                name: "Drive".into(),
                data_type: CipDataType::Dint,
                element_count: 1,
                initial_value: CipValue::Bytes(packed),
                template_id: Some(1),
                read_only: false,
            });
            let (state, rx, shutdown) = start_enip_server(
                EnipServerConfig { port: 0, bind_addr: "127.0.0.1".into() },
                tags,
                vec![t],
            )
            .await
            .unwrap();
            let (tx, _unused) = mpsc::unbounded_channel();
            Fx { state, tx, _rx: rx, _shutdown: shutdown }
        }

        async fn call(&self, req: &[u8]) -> Vec<u8> {
            route_cip(req, &self.state, &self.tx).await
        }
    }

    #[tokio::test]
    async fn reads_return_wire_exact_types() {
        let fx = Fx::new().await;
        assert_eq!(fx.call(&build_read_tag_request("Counter", 1).unwrap()).await, h("cc 00 00 00 c4 00 07 00 00 00"));
        assert_eq!(fx.call(&build_read_tag_request("Speed", 1).unwrap()).await, h("cc 00 00 00 ca 00 00 00 c0 3f"));
        // Logix BOOL read: one byte, 0xFF when set
        assert_eq!(fx.call(&build_read_tag_request("Flag", 1).unwrap()).await, h("cc 00 00 00 c1 00 ff"));
        // STRING is the structure A0 02 CE 0F + 88-byte image
        let r = fx.call(&build_read_tag_request("Name", 1).unwrap()).await;
        assert_eq!(&r[..8], &h("cc 00 00 00 a0 02 ce 0f")[..]);
        assert_eq!(r.len(), 8 + 88);
        assert_eq!(&r[8..14], &h("02 00 00 00 48 69")[..]);
        // program-scoped, split into 0x91 segments
        let r = fx.call(&build_read_tag_request("Program:Main.Motor", 1).unwrap()).await;
        assert_eq!(r, h("cc 00 00 00 ca 00 00 00 20 40"));
    }

    #[tokio::test]
    async fn unknown_tags_and_ranges_use_logix_status_codes() {
        let fx = Fx::new().await;
        // path segment error with one extended status word (Logix: `04 01 00 00`)
        assert_eq!(fx.call(&build_read_tag_request("Nope", 1).unwrap()).await, h("cc 00 04 01 00 00"));
        // reading past the end of an array: general 0xFF, ext 0x2105
        assert_eq!(fx.call(&build_read_tag_request("Arr[299]", 2).unwrap()).await, h("cc 00 ff 01 05 21"));
        // unsupported service on a tag path
        assert_eq!(fx.call(&h("2a 03 91 04 54 65 73 74")).await[..3], h("aa 00 08"));
        // unknown class
        assert_eq!(fx.call(&h("0e 03 20 99 24 01 30 01")).await, h("8e 00 05 00"));
    }

    #[tokio::test]
    async fn array_elements_and_ranges() {
        let fx = Fx::new().await;
        let r = fx.call(&build_read_tag_request("Arr[3]", 2).unwrap()).await;
        assert_eq!(r, h("cc 00 00 00 c4 00 03 00 00 00 04 00 00 00"));
    }

    #[tokio::test]
    async fn large_reads_come_back_partial_and_continue_with_fragmented_service() {
        let fx = Fx::new().await;
        let first = fx.call(&build_read_tag_request("Arr", 300).unwrap()).await;
        assert_eq!(first[2], 0x06, "1200 bytes cannot fit one reply");
        assert!(first.len() <= 4 + 2 + 480);
        let mut data = first[6..].to_vec();
        let mut rounds = 0;
        loop {
            let r = fx.call(&build_read_tag_fragmented_request("Arr", 300, data.len() as u32).unwrap()).await;
            assert_eq!(r[0], 0xD2);
            assert_eq!(&r[4..6], &h("c4 00")[..], "each fragment repeats the type");
            data.extend_from_slice(&r[6..]);
            rounds += 1;
            if r[2] == 0 {
                break;
            }
            assert_eq!(r[2], 0x06);
        }
        assert!(rounds >= 2);
        assert_eq!(data.len(), 1200);
        for (i, c) in data.chunks_exact(4).enumerate() {
            assert_eq!(i32::from_le_bytes(c.try_into().unwrap()), i as i32);
        }
    }

    #[tokio::test]
    async fn writes_are_type_and_size_checked() {
        let fx = Fx::new().await;
        let dint = TagType::atomic(CIP_TYPE_DINT);
        // wrong type → 0xFF / 0x2107 (this is what a real Logix answers)
        let req = build_write_tag_request("Counter", &TagType::atomic(CIP_TYPE_REAL), 1, &1.0f32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 ff 01 07 21"));
        // BOOL tag written with the old 4-byte encoding → too much data
        let req = build_write_tag_request("Flag", &TagType::atomic(CIP_TYPE_BOOL), 1, &[0xFF, 0, 0, 0]).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 15 00"));
        // too little data
        let req = build_write_tag_request("Counter", &dint, 1, &[1, 2]).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 13 00"));
        // unknown tag
        let req = build_write_tag_request("Nope", &dint, 1, &1i32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 04 01 00 00"));
        // success, then read back
        let req = build_write_tag_request("Counter", &dint, 1, &42i32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 00 00"));
        assert_eq!(fx.call(&build_read_tag_request("Counter", 1).unwrap()).await[6..], 42i32.to_le_bytes());
        // BOOL written the Logix way (one byte)
        let req = build_write_tag_request("Flag", &TagType::atomic(CIP_TYPE_BOOL), 1, &[0x00]).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 00 00"));
        assert_eq!(fx.call(&build_read_tag_request("Flag", 1).unwrap()).await, h("cc 00 00 00 c1 00 00"));
        // STRING write
        let (_, image) = encode_tag_value(&TagType::logix_string(), &CipValue::String("Bye".into())).unwrap();
        let req = build_write_tag_request("Name", &TagType::logix_string(), 1, &image).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 00 00"));
        let r = fx.call(&build_read_tag_request("Name", 1).unwrap()).await;
        assert_eq!(&r[8..15], &h("03 00 00 00 42 79 65")[..]);
    }

    #[tokio::test]
    async fn a_read_only_tag_refuses_writes_with_attribute_not_settable() {
        let fx = Fx::new().await;
        let real = TagType::atomic(CIP_TYPE_REAL);
        let req = build_write_tag_request("Sensor", &real, 1, &1.0f32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 0e 00"));
        // The value is untouched and still readable.
        assert_eq!(fx.call(&build_read_tag_request("Sensor", 1).unwrap()).await[6..], 9.5f32.to_le_bytes());
        // A writable tag next to it still writes.
        let req = build_write_tag_request("Program:Main.Motor", &real, 1, &3.0f32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 00 00"));
    }

    #[tokio::test]
    async fn fragmented_write_assembles_the_array() {
        let fx = Fx::new().await;
        let dint = TagType::atomic(CIP_TYPE_DINT);
        let image: Vec<u8> = (0..300i32).flat_map(|i| (i + 1000).to_le_bytes()).collect();
        for (n, chunk) in image.chunks(400).enumerate() {
            let req = build_write_tag_fragmented_request("Arr", &dint, 300, (n * 400) as u32, chunk).unwrap();
            assert_eq!(fx.call(&req).await, h("d3 00 00 00"));
        }
        let r = fx.call(&build_read_tag_request("Arr[299]", 1).unwrap()).await;
        assert_eq!(&r[6..], &1299i32.to_le_bytes());
        let r = fx.call(&build_read_tag_request("Arr[0]", 1).unwrap()).await;
        assert_eq!(&r[6..], &1000i32.to_le_bytes());
    }

    #[tokio::test]
    async fn structure_tags_and_members() {
        let fx = Fx::new().await;
        let r = fx.call(&build_read_tag_request("Drive", 1).unwrap()).await;
        // type field A0 02 <handle = template id>, then the 16-byte image
        assert_eq!(&r[..8], &h("cc 00 00 00 a0 02 01 00")[..]);
        assert_eq!(r.len(), 8 + 16);
        let r = fx.call(&build_read_tag_request("Drive.Poles", 1).unwrap()).await;
        assert_eq!(r, h("cc 00 00 00 c4 00 04 00 00 00"));
        // write a member through the structure image
        let req = build_write_tag_request("Drive.Poles", &TagType::atomic(CIP_TYPE_DINT), 1, &6i32.to_le_bytes()).unwrap();
        assert_eq!(fx.call(&req).await, h("cd 00 00 00"));
        let r = fx.call(&build_read_tag_request("Drive.Poles", 1).unwrap()).await;
        assert_eq!(&r[6..], &6i32.to_le_bytes());
        // whole-structure write must carry the structure handle
        let img = fx.call(&build_read_tag_request("Drive", 1).unwrap()).await[8..].to_vec();
        let bad = build_write_tag_request("Drive", &TagType::structure(0x7777), 1, &img).unwrap();
        assert_eq!(fx.call(&bad).await, h("cd 00 ff 01 07 21"));
        let ok = build_write_tag_request("Drive", &TagType::structure(1), 1, &img).unwrap();
        assert_eq!(fx.call(&ok).await, h("cd 00 00 00"));
    }

    #[tokio::test]
    async fn multiple_service_packet_runs_each_request() {
        let fx = Fx::new().await;
        let reqs = vec![
            build_read_tag_request("Counter", 1).unwrap(),
            build_read_tag_request("Missing", 1).unwrap(),
            build_read_tag_request("Speed", 1).unwrap(),
        ];
        let reply = fx.call(&build_multiple_service_packet(&reqs)).await;
        // one embedded reply failed → 0x1E "embedded service error", replies still present
        assert_eq!(&reply[..4], &h("8a 00 1e 00")[..]);
        let parts = split_multiple_service_data(&reply[4..]).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], h("cc 00 00 00 c4 00 07 00 00 00"));
        assert_eq!(parts[1], h("cc 00 04 01 00 00"));
        assert_eq!(parts[2], h("cc 00 00 00 ca 00 00 00 c0 3f"));
        // all good → status 0
        let ok = fx.call(&build_multiple_service_packet(&[reqs[0].clone(), reqs[2].clone()])).await;
        assert_eq!(&ok[..4], &h("8a 00 00 00")[..]);
    }

    #[tokio::test]
    async fn unconnected_send_routes_to_slot_zero_only() {
        let fx = Fx::new().await;
        let inner = build_read_tag_request("Counter", 1).unwrap();
        // success: the reply is the embedded reply, not wrapped
        let r = fx.call(&build_unconnected_send(&inner, &[0x01, 0x00])).await;
        assert_eq!(r, h("cc 00 00 00 c4 00 07 00 00 00"));
        // malformed embedded message (odd length, declares a path longer than it has):
        // exercises the pad byte before the route and must not crash the target
        let r = fx.call(&build_unconnected_send(&h("0e 02 20 01 24"), &[0x01, 0x00])).await;
        assert_eq!(r[2], 0x26, "path size invalid: {r:02x?}");
        // wrong slot / wrong port → Connection Manager errors with extended status
        assert_eq!(fx.call(&build_unconnected_send(&inner, &[0x01, 0x05])).await, h("d2 00 01 01 12 03"));
        assert_eq!(fx.call(&build_unconnected_send(&inner, &[0x02, 0x00])).await, h("d2 00 01 01 11 03"));
    }

    #[tokio::test]
    async fn identity_object_replies_like_a_controller() {
        let fx = Fx::new().await;
        let gaa = fx.call(&build_generic_cip_request(CIP_GET_ATTRIBUTES_ALL, 1, 1, None, &[])).await;
        assert_eq!(&gaa[..4], &h("81 00 00 00")[..]);
        assert_eq!(gaa.len(), 4 + 2 + 2 + 2 + 2 + 2 + 4 + 1 + 8, "attributes 1-7, no state byte");
        // unknown attribute: 0x14; unknown instance: 0x05; unsupported service: 0x08
        assert_eq!(fx.call(&build_get_attribute_single_request(1, 1, 99)).await, h("8e 00 14 00"));
        assert_eq!(fx.call(&build_get_attribute_single_request(1, 9, 1)).await, h("8e 00 05 00"));
        assert_eq!(fx.call(&build_generic_cip_request(0x2A, 1, 1, None, &[])).await, h("aa 00 08 00"));
    }

    #[tokio::test]
    async fn assemblies_serve_the_tag_image_and_keep_output_data() {
        let fx = Fx::new().await;
        let image = snapshot_input_assembly(&fx.state).await;
        assert!(!image.is_empty());
        let get = fx.call(&build_get_attribute_single_request(ASSEMBLY_CLASS_ID, INPUT_ASSEMBLY, 3)).await;
        assert_eq!(&get[..4], &h("8e 00 00 00")[..]);
        assert_eq!(&get[4..], &image[..]);
        let size = fx.call(&build_get_attribute_single_request(ASSEMBLY_CLASS_ID, INPUT_ASSEMBLY, 4)).await;
        assert_eq!(&size[4..], &(image.len() as u16).to_le_bytes()[..]);
        // The input assembly is produced by the target: not settable.
        let set = build_set_attribute_single_request(ASSEMBLY_CLASS_ID, INPUT_ASSEMBLY, 3, &[1, 2]);
        assert_eq!(fx.call(&set).await, h("90 00 0e 00"));
        // The output assembly keeps what the originator set.
        let set = build_set_attribute_single_request(ASSEMBLY_CLASS_ID, OUTPUT_ASSEMBLY, 3, &[1, 2, 3, 4]);
        assert_eq!(fx.call(&set).await, h("90 00 00 00"));
        let get = fx.call(&build_get_attribute_single_request(ASSEMBLY_CLASS_ID, OUTPUT_ASSEMBLY, 3)).await;
        assert_eq!(get, h("8e 00 00 00 01 02 03 04"));
        // Unknown instance 0x05, unknown attribute 0x14.
        assert_eq!(fx.call(&build_get_attribute_single_request(ASSEMBLY_CLASS_ID, 101, 3)).await, h("8e 00 05 00"));
        assert_eq!(fx.call(&build_get_attribute_single_request(ASSEMBLY_CLASS_ID, INPUT_ASSEMBLY, 9)).await, h("8e 00 14 00"));
    }

    #[tokio::test]
    async fn symbol_browse_paginates_with_status_6_and_instances_are_inclusive() {
        let fx = Fx::new().await;
        let mut start = 0u32;
        let mut all: Vec<(u32, String)> = Vec::new();
        let mut pages = 0;
        loop {
            let mut path = Vec::new();
            push_logical(&mut path, 0x20, 0x6B);
            push_logical(&mut path, 0x24, start);
            let mut req = vec![CIP_GET_INSTANCE_ATTRIBUTE_LIST, (path.len() / 2) as u8];
            req.extend_from_slice(&path);
            req.extend_from_slice(&h("03 00 01 00 02 00 08 00"));
            let r = fx.call(&req).await;
            assert!(r[2] == 0 || r[2] == 6, "status {:02x}", r[2]);
            let (syms, last) = crate::client::parse_symbol_records(&r[4..]).unwrap();
            if let Some(first) = syms.first() {
                assert_eq!(first.instance, start.max(1), "records start AT the requested instance");
            }
            all.extend(syms.into_iter().map(|s| (s.instance, s.name)));
            pages += 1;
            if r[2] == 0 {
                break;
            }
            start = last.unwrap() + 1;
        }
        assert!(pages >= 2, "60+ tags must not fit one reply");
        // contiguous ids, every top-level tag + the program object present exactly once
        for (i, (id, _)) in all.iter().enumerate() {
            assert_eq!(*id as usize, i + 1);
        }
        assert!(all.iter().any(|(_, n)| n == "Program:Main"));
        assert!(all.iter().any(|(_, n)| n == "Arr"));
        assert!(!all.iter().any(|(_, n)| n.contains("Motor")), "program tags only appear in program scope");

        // program scope: leading Program:Main symbolic segment
        let mut path = encode_symbolic_path("Program:Main").unwrap();
        push_logical(&mut path, 0x20, 0x6B);
        push_logical(&mut path, 0x24, 0);
        let mut req = vec![CIP_GET_INSTANCE_ATTRIBUTE_LIST, (path.len() / 2) as u8];
        req.extend_from_slice(&path);
        req.extend_from_slice(&h("02 00 01 00 02 00"));
        let r = fx.call(&req).await;
        assert_eq!(r[2], 0);
        // 02 00 → attributes 1,2 only: id(4) + name(2+5) + type(2)
        assert_eq!(&r[4..], &h("01 00 00 00 05 00 4d 6f 74 6f 72 ca 00")[..]);
        // unsupported attribute → 0x14
        let mut bad = req.clone();
        let n = bad.len();
        bad[n - 4..].copy_from_slice(&h("63 00 02 00"));
        assert_eq!(fx.call(&bad).await[2], 0x14);
    }

    #[tokio::test]
    async fn symbol_types_report_arrays_structs_and_strings() {
        let fx = Fx::new().await;
        let mut path = Vec::new();
        push_logical(&mut path, 0x20, 0x6B);
        push_logical(&mut path, 0x24, 0);
        let mut req = vec![CIP_GET_INSTANCE_ATTRIBUTE_LIST, (path.len() / 2) as u8];
        req.extend_from_slice(&path);
        req.extend_from_slice(&h("03 00 01 00 02 00 08 00"));
        // collect a few pages so every fixture tag is seen
        let mut syms = Vec::new();
        let mut start = 0;
        loop {
            let mut r = req.clone();
            let mut p = Vec::new();
            push_logical(&mut p, 0x20, 0x6B);
            push_logical(&mut p, 0x24, start);
            r.splice(2..2 + path.len(), p);
            let reply = fx.call(&r).await;
            let (s, last) = crate::client::parse_symbol_records(&reply[4..]).unwrap();
            syms.extend(s);
            if reply[2] == 0 {
                break;
            }
            start = last.unwrap() + 1;
        }
        let find = |n: &str| syms.iter().find(|s| s.name == n).unwrap().clone();
        assert_eq!(find("Counter").type_word, 0x00C4);
        assert_eq!(find("Name").type_word, 0x8FCE);
        assert_eq!(find("Drive").type_word, 0x8001);
        let arr = find("Arr");
        assert_eq!(arr.type_word, 0x20C4, "one dimension in bits 13-14, not an element count in the high byte");
        assert_eq!(arr.dims, [300, 0, 0]);
    }

    #[tokio::test]
    async fn template_attributes_and_sliced_definition_reads() {
        let fx = Fx::new().await;
        let req = build_generic_cip_request(0x03, 0x6C, 1, None, &h("04 00 04 00 05 00 02 00 01 00"));
        let r = fx.call(&req).await;
        assert_eq!(&r[..4], &h("83 00 00 00")[..]);
        let attrs = crate::templates::parse_template_attrs(&r[4..]).expect("attrs");
        assert_eq!((attrs.structure_size, attrs.member_count, attrs.handle), (16, 4, 1));
        // pycomm3 reads (words*4 - 21) bytes starting at offset 0; the target clips at the blob end
        let want = attrs.obj_definition_words as usize * 4 - 21;
        let mut data = 0u32.to_le_bytes().to_vec();
        data.extend_from_slice(&(want as u16).to_le_bytes());
        let blob = fx.call(&build_generic_cip_request(CIP_READ_TAG, 0x6C, 1, None, &data)).await;
        assert_eq!(blob[2], 0);
        let (name, members) = crate::templates::parse_template_blob(&blob[4..], attrs.member_count as usize).unwrap();
        assert_eq!(name, "DriveInfo");
        assert_eq!(members.len(), 4);
        assert_eq!(members[3].name, "Poles");
        // a slice from offset 32 returns the name table onwards
        let mut data = 32u32.to_le_bytes().to_vec();
        data.extend_from_slice(&9u16.to_le_bytes());
        let slice = fx.call(&build_generic_cip_request(CIP_READ_TAG, 0x6C, 1, None, &data)).await;
        assert_eq!(&slice[4..], b"DriveInfo");
        // Template without offset/size is malformed
        let r = fx.call(&build_generic_cip_request(CIP_READ_TAG, 0x6C, 1, None, &[])).await;
        assert_eq!(r[2], 0x13);
    }
}
