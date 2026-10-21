//! Hand-rolled embedded NATS server (core protocol).
//!
//! The official NATS server is written in Go and cannot be embedded in Rust,
//! so this module implements a minimal-but-compatible NATS server directly on
//! `tokio`:
//!
//! - NATS text protocol: `INFO`, `CONNECT`, `PING`/`PONG`, `PUB`/`HPUB`,
//!   `SUB`, `UNSUB`, `MSG`/`HMSG` delivery, `-ERR`.
//! - Subject routing with `*` (single token) and `>` (tail) wildcards via a
//!   token trie, plus queue-group dedup (one member per queue group).
//! - Optional user/pass auth.
//! - An in-process mirror: every routed message is forwarded to a
//!   caller-provided callback and recorded in a [`TopicTracker`], so the app
//!   can show live subjects + messages even for remote publishers.
//! - Graceful stop via a `watch` channel; startup failures (port in use)
//!   surface through the `Err` of [`start_server`].
//!
//! Documented limitations: no TLS, no clustering/NKeys/JWT, no WebSockets, no
//! `no_responders` (`$SRV`). JetStream (`$JS.API`) is implemented in
//! [`crate::server_js`] as a minimal in-memory subset.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, watch, mpsc};

use crate::server_js::JetStreamState;
use crate::types::*;

// ---------------------------------------------------------------------------
// Routing primitives
// ---------------------------------------------------------------------------

/// A single subscription registered by a connected client.
#[derive(Debug, Clone)]
struct Subscription {
    subject: String,
    queue: Option<String>,
    sid: String,
}

/// A routed message ready for delivery to a subscriber.
#[derive(Debug, Clone)]
struct Routed {
    subject: String,
    reply: Option<String>,
    headers: Vec<(String, String)>,
    payload: Vec<u8>,
    sid: String,
}

/// A raw publish routed through the server (used by the embedded JetStream
/// handler and the app's server-publish command).
#[derive(Debug, Clone)]
pub struct RoutePayload {
    pub subject: String,
    pub reply: Option<String>,
    pub headers: Vec<(String, String)>,
    pub payload: Vec<u8>,
}

impl Routed {
    /// Render the wire `MSG`/`HMSG` block (header line + payload + CRLF).
    fn render(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let reply = self.reply.as_deref().unwrap_or("");
        // A `__status__` header means "NATS/1.0 <code> <desc>" status message.
        let status = self
            .headers
            .iter()
            .find(|(k, _)| k == "__status__")
            .map(|(_, v)| v.clone());
        if status.is_none() && self.headers.is_empty() {
            let head = format!(
                "MSG {} {} {} {}\r\n",
                self.subject,
                self.sid,
                reply,
                self.payload.len()
            );
            out.extend_from_slice(head.as_bytes());
        } else {
            let mut hdr = String::from("NATS/1.0");
            if let Some(s) = &status {
                hdr.push(' ');
                hdr.push_str(s);
            }
            hdr.push_str("\r\n");
            for (k, v) in &self.headers {
                if k == "__status__" {
                    continue;
                }
                hdr.push_str(k);
                hdr.push_str(": ");
                hdr.push_str(v);
                hdr.push_str("\r\n");
            }
            hdr.push_str("\r\n");
            let hdr_len = hdr.len();
            let total = hdr_len + self.payload.len();
            let head = format!(
                "HMSG {} {} {} {} {}\r\n",
                self.subject,
                self.sid,
                reply,
                hdr_len,
                total
            );
            out.extend_from_slice(head.as_bytes());
            out.extend_from_slice(hdr.as_bytes());
        }
        out.extend_from_slice(&self.payload);
        out.extend_from_slice(b"\r\n");
        out
    }
}

/// Token trie routing table.
///
/// Each node tracks subscriptions matching exactly at this depth, plus
/// children keyed by the next subject token. `*` children match any single
/// token; `>` children match any remaining tokens (stored under the `>` key).
#[derive(Debug, Default)]
struct TrieNode {
    subs: Vec<(Subscription, u64)>,
    children: HashMap<String, TrieNode>,
}

impl TrieNode {
    fn insert(&mut self, tokens: &[String], sub: Subscription, cid: u64) {
        let mut node = self;
        for tok in tokens {
            node = node.children.entry(tok.clone()).or_default();
        }
        node.subs.push((sub, cid));
    }

    fn remove(&mut self, tokens: &[String], cid: u64, sid: &str) -> Option<Subscription> {
        let mut node = self;
        for tok in tokens {
            node = node.children.get_mut(tok)?;
        }
        let idx = node.subs.iter().position(|(s, c)| *c == cid && s.sid == sid)?;
        Some(node.subs.remove(idx).0)
    }

    fn collect<'a>(&'a self, tokens: &[String], idx: usize, out: &mut Vec<&'a (Subscription, u64)>) {
        if let Some(star) = self.children.get("*") {
            star.collect(tokens, idx + 1, out);
        }
        if let Some(tail) = self.children.get(">") {
            out.extend(tail.subs.iter());
        }
        if idx >= tokens.len() {
            out.extend(self.subs.iter());
            return;
        }
        if let Some(next) = self.children.get(&tokens[idx]) {
            next.collect(tokens, idx + 1, out);
        }
    }
}

fn tokens_of(subject: &str) -> Vec<String> {
    subject.split('.').map(|s| s.to_string()).collect()
}

fn find_matches(root: &TrieNode, subject: &str) -> Vec<(Subscription, u64)> {
    let tokens = tokens_of(subject);
    let mut out: Vec<&(Subscription, u64)> = Vec::new();
    root.collect(&tokens, 0, &mut out);
    out.into_iter().cloned().collect()
}

// ---------------------------------------------------------------------------
// Shared server state
// ---------------------------------------------------------------------------

/// Callback fired for every routed message: (subject, payload, timestamp_ms,
/// headers). Used by the app to emit Tauri events and feed the tracker.
pub type OnMessage =
    Arc<dyn Fn(String, Vec<u8>, i64, Vec<(String, String)>) + Send + Sync + 'static>;

/// Public connection info for introspection.
#[derive(Debug, Clone)]
pub struct ConnInfo {
    pub cid: u64,
    pub name: Option<String>,
    pub connected_at: i64,
    pub subscription_count: usize,
}

/// Cloneable handle to the running embedded server.
#[derive(Clone)]
pub struct NatsServerState {
    pub status: Arc<watch::Sender<ServerStatus>>,
    status_rx: watch::Receiver<ServerStatus>,
    pub config: Arc<RwLock<NatsServerConfig>>,
    pub tracker: TopicTracker,
    pub shutdown_tx: Arc<RwLock<Option<watch::Sender<bool>>>>,
    pub server_id: Arc<String>,
    root: Arc<RwLock<TrieNode>>,
    conns: Arc<RwLock<HashMap<u64, ConnInfo>>>,
    outboxes: Arc<RwLock<HashMap<u64, mpsc::UnboundedSender<Routed>>>>,
    next_cid: Arc<std::sync::atomic::AtomicU64>,
    on_message: Arc<RwLock<Option<OnMessage>>>,
    /// Embedded JetStream state (in-memory subset of `$JS.API`).
    pub jetstream: JetStreamState,
}

impl NatsServerState {
    pub fn new(config: NatsServerConfig) -> Self {
        let (status_tx, status_rx) = watch::channel(ServerStatus::Stopped);
        Self {
            status: Arc::new(status_tx),
            status_rx,
            config: Arc::new(RwLock::new(config)),
            tracker: TopicTracker::new(),
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_id: Arc::new(format!(
                "MTNATS_{:X}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            )),
            root: Arc::new(RwLock::new(TrieNode::default())),
            conns: Arc::new(RwLock::new(HashMap::new())),
            outboxes: Arc::new(RwLock::new(HashMap::new())),
            next_cid: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            on_message: Arc::new(RwLock::new(None)),
            jetstream: JetStreamState::new(),
        }
    }

    pub async fn current_status(&self) -> ServerStatus {
        self.status_rx.borrow().clone()
    }

    pub async fn get_connections(&self) -> Vec<ConnInfo> {
        self.conns.read().await.values().cloned().collect()
    }

    /// Publish a message through the server's router (used by the app's
    /// "publish from server mode" command, so local subscribers receive it).
    /// Mirrors the message to the tracker + on_message callback.
    pub async fn publish(
        &self,
        subject: String,
        reply: Option<String>,
        headers: Vec<(String, String)>,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        route_publish(self, subject, reply, headers, payload).await;
        Ok(())
    }

    /// Publish a JetStream status message (a headers-only HMSG with
    /// `NATS/1.0 <code> <description>`). Used for "No Messages" pull replies.
    pub async fn publish_with_status(
        &self,
        reply: String,
        code: u16,
        description: &str,
        headers: Vec<(String, String)>,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        let mut all = headers;
        // The status line goes in the first header line, which our HMSG render
        // emits as `NATS/1.0` — extend it with the code/description.
        all.insert(0, ("__status__".to_string(), format!("{} {}", code, description)));
        let routed = Routed {
            subject: reply.clone(),
            reply: None,
            headers: all,
            payload,
            sid: "js-status".to_string(),
        };
        // Deliver to any subscriber of the reply subject (the requesting
        // client's inbox).
        let outboxes = self.outboxes.read().await;
        for tx in outboxes.values() {
            let _ = tx.send(routed.clone());
        }
        Ok(())
    }

    /// Route an outbound message to all matching subscribers (used by the
    /// embedded JetStream handler). Unlike `publish`, this does NOT mirror to
    /// the tracker/callback (internal messages).
    pub async fn publish_raw(&self, payload: RoutePayload) -> Result<(), String> {
        let subject = payload.subject;
        let reply = payload.reply;
        let headers = payload.headers;
        let payload = payload.payload;
        let matches = {
            let r = self.root.read().await;
            find_matches(&r, &subject)
        };
        let mut delivered_q: HashSet<Option<String>> = HashSet::new();
        for (sub, cid) in matches {
            if let Some(q) = &sub.queue {
                if !delivered_q.insert(Some(q.clone())) {
                    continue;
                }
            } else {
                delivered_q.insert(None);
            }
            let routed = Routed {
                subject: subject.clone(),
                reply: reply.clone(),
                headers: headers.clone(),
                payload: payload.clone(),
                sid: sub.sid.clone(),
            };
            let outboxes = self.outboxes.read().await;
            if let Some(tx) = outboxes.get(&cid) {
                let _ = tx.send(routed);
            }
        }
        Ok(())
    }

    /// Access the message callback (read guard). Used by the app to re-emit
    /// messages it publishes itself through [`NatsServerState::publish`].
    pub async fn on_message(&self) -> tokio::sync::RwLockReadGuard<'_, Option<OnMessage>> {
        self.on_message.read().await
    }
}

/// Topic tracker: per-subject counts + a bounded recent-message log (UI).
#[derive(Clone)]
pub struct TopicTracker {
    subjects: Arc<RwLock<HashMap<String, SubjectInfo>>>,
    recent: Arc<RwLock<std::collections::VecDeque<ServerRecentMessage>>>,
}

impl TopicTracker {
    pub fn new() -> Self {
        Self {
            subjects: Arc::new(RwLock::new(HashMap::new())),
            recent: Arc::new(RwLock::new(std::collections::VecDeque::new())),
        }
    }

    pub async fn record_message(
        &self,
        subject: String,
        payload: Vec<u8>,
        headers: Vec<(String, String)>,
    ) {
        let now = now_ms();
        {
            let mut subs = self.subjects.write().await;
            let entry = subs.entry(subject.clone()).or_insert(SubjectInfo {
                subject: subject.clone(),
                message_count: 0,
                last_update: 0,
            });
            entry.message_count += 1;
            entry.last_update = now;
        }
        {
            let mut recent = self.recent.write().await;
            if recent.len() >= 500 {
                recent.pop_front();
            }
            recent.push_back(ServerRecentMessage {
                subject,
                payload: encode_payload(&payload),
                timestamp: now,
                headers,
            });
        }
    }

    pub async fn get_subjects(&self) -> Vec<SubjectInfo> {
        let mut list: Vec<SubjectInfo> = self.subjects.read().await.values().cloned().collect();
        list.sort_by(|a, b| a.subject.cmp(&b.subject));
        list
    }

    pub async fn get_recent(&self, limit: usize) -> Vec<ServerRecentMessage> {
        let recent = self.recent.read().await;
        let mut v: Vec<ServerRecentMessage> = recent.iter().rev().take(limit).cloned().collect();
        v.reverse();
        v
    }

    pub async fn clear(&self) {
        self.subjects.write().await.clear();
        self.recent.write().await.clear();
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------------

/// Start the embedded NATS server as a background task.
///
/// `on_message` is invoked for every routed message. Returns `Ok` once the
/// listener is bound; a bind failure (e.g. port in use) is returned as `Err`.
pub async fn start_server(
    state: NatsServerState,
    on_message: Option<OnMessage>,
) -> Result<(), String> {
    let status = state.current_status().await;
    if status == ServerStatus::Running || status == ServerStatus::Starting {
        return Err("Server is already running".to_string());
    }

    let _ = state.status.send(ServerStatus::Starting);
    let port = state.config.read().await.port;
    let max_conns = state.config.read().await.max_connections as usize;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    {
        let mut slot = state.shutdown_tx.write().await;
        *slot = Some(shutdown_tx);
    }

    let listener = TcpListener::bind(("0.0.0.0", port)).await.map_err(|e| {
        let _ = state.status.send(ServerStatus::Error(e.to_string()));
        format!("Failed to bind port {}: {}", port, e)
    })?;
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    if actual_port != port {
        let _ = state.status.send(ServerStatus::Error(format!(
            "Requested port {} but bound to {}",
            port, actual_port
        )));
        let _ = state.shutdown_tx.write().await.take();
        return Err("Port mismatch on bind".to_string());
    }

    let _ = state.status.send(ServerStatus::Running);

    // Store the callback so route_publish can invoke it.
    *state.on_message.write().await = on_message;

    let state2 = state.clone();
    tokio::spawn(async move {
        let mut accept = 0usize;
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, _)) => {
                            if accept >= max_conns {
                                let _ = stream.writable().await;
                                let _ = stream.try_write(b"-ERR 'maximum connections exceeded'\r\n");
                                drop(stream);
                                continue;
                            }
                            accept += 1;
                            let cid = state2.next_cid.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let st = state2.clone();
                            tokio::spawn(async move {
                                handle_client(stream, cid, st).await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        let _ = state2.status.send(ServerStatus::Stopped);
    });

    Ok(())
}

/// Stop the embedded NATS server and clear its tracker.
pub async fn stop_server(state: &NatsServerState) -> Result<(), String> {
    let status = state.current_status().await;
    if status == ServerStatus::Stopped {
        return Err("Server is not running".to_string());
    }
    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(true);
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    state.conns.write().await.clear();
    state.outboxes.write().await.clear();
    state.tracker.clear().await;
    let _ = state.status.send(ServerStatus::Stopped);
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-connection handling
// ---------------------------------------------------------------------------

/// What the command loop should do after parsing one line.
enum CmdOutcome {
    Continue,
    WritePong,
    ReadPayload {
        subject: String,
        reply: Option<String>,
        total: usize,
        hdr_len: usize,
    },
    Fatal(String),
}

struct ClientConn {
    cid: u64,
    subs: HashMap<String, Subscription>,
    authenticated: bool,
    name: Option<String>,
}

async fn handle_client(stream: TcpStream, cid: u64, state: NatsServerState) {
    let (read_half, write_half) = stream.into_split();
    let mut writer = tokio::io::BufWriter::new(write_half);
    let info = serde_json::json!({
        "server_id": *state.server_id,
        "server_name": "mt-nats",
        "version": "2.10.0",
        "proto": 1,
        "go": "go1.22",
        "host": "0.0.0.0",
        "port": state.config.read().await.port,
        "max_payload": 1_048_576,
        "client_id": cid,
        "auth_required": auth_required(&state).await,
        "headers": true,
    });
    if writer
        .write_all(format!("INFO {}\r\n", info).as_bytes())
        .await
        .is_err()
    {
        return;
    }
    if writer.flush().await.is_err() {
        return;
    }

    // Outbox: routing writes for this connection come through this channel so
    // the single writer is only ever touched by this task.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Routed>();
    state.outboxes.write().await.insert(cid, out_tx);

    let mut reader = BufReader::new(read_half);
    let mut conn = ClientConn {
        cid,
        subs: HashMap::new(),
        authenticated: !auth_required(&state).await,
        name: None,
    };

    loop {
        tokio::select! {
            line_res = read_line(&mut reader) => {
                match line_res {
                    Ok(Some(line)) => {
                        let outcome = handle_command(&mut conn, &line, &state).await;
                        match outcome {
                            CmdOutcome::Continue => {}
                            CmdOutcome::WritePong => {
                                if writer.write_all(b"PONG\r\n").await.is_err() { break; }
                                let _ = writer.flush().await;
                            }
                            CmdOutcome::ReadPayload { subject, reply, total, hdr_len } => {
                                let block = match read_block(&mut reader, total).await {
                                    Ok(b) => b,
                                    Err(e) => {
                                        let _ = writer.write_all(format!("-ERR '{}'\r\n", e).as_bytes()).await;
                                        break;
                                    }
                                };
                                let (headers, payload) = split_block(block, hdr_len);
                                route_publish(&state, subject, reply, headers, payload).await;
                            }
                            CmdOutcome::Fatal(err) => {
                                let _ = writer.write_all(format!("-ERR '{}'\r\n", err).as_bytes()).await;
                                let _ = writer.flush().await;
                                break;
                            }
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            msg = out_rx.recv() => {
                match msg {
                    Some(routed) => {
                        if writer.write_all(&routed.render()).await.is_err() { break; }
                        let _ = writer.flush().await;
                    }
                    None => break,
                }
            }
        }
    }

    // Cleanup: remove outbox + conn info, drop this connection's subs.
    state.outboxes.write().await.remove(&cid);
    state.conns.write().await.remove(&cid);
    let subs: Vec<(String, String)> = conn
        .subs
        .iter()
        .map(|(sid, s)| (s.subject.clone(), sid.clone()))
        .collect();
    let mut root_w = state.root.write().await;
    for (subject, sid) in subs {
        root_w.remove(&tokens_of(&subject), cid, &sid);
    }
}

/// Read one control line (terminated by `\n`, trimmed of `\r`).
async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf).await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Read exactly `n` payload bytes plus the trailing CRLF.
async fn read_block<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    n: usize,
) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("payload read failed: {}", e))?;
    let mut crlf = [0u8; 2];
    reader
        .read_exact(&mut crlf)
        .await
        .map_err(|e| format!("payload terminator read failed: {}", e))?;
    Ok(buf)
}

/// Split an HPUB block into (headers, payload).
fn split_block(block: Vec<u8>, hdr_len: usize) -> (Vec<(String, String)>, Vec<u8>) {
    if hdr_len == 0 {
        return (Vec::new(), block);
    }
    let hdr_bytes = &block[..hdr_len.min(block.len())];
    let payload = block[hdr_len.min(block.len())..].to_vec();
    let mut headers = Vec::new();
    let text = String::from_utf8_lossy(hdr_bytes);
    for line in text.lines().skip(1) {
        // First line is "NATS/1.0"; subsequent lines are "K: V".
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (headers, payload)
}

/// Handle one control line and decide what the loop should do next.
async fn handle_command(conn: &mut ClientConn, line: &str, state: &NatsServerState) -> CmdOutcome {
    if line.is_empty() {
        return CmdOutcome::Continue;
    }

    if line.starts_with("PING") {
        return CmdOutcome::WritePong;
    }
    if line.starts_with("PONG") {
        return CmdOutcome::Continue;
    }

    if line.starts_with("CONNECT") {
        let json = &line["CONNECT".len()..];
        let v: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return CmdOutcome::Fatal("malformed CONNECT".to_string()),
        };
        if !conn.authenticated {
            if let Err(e) = authenticate(state, &v).await {
                return CmdOutcome::Fatal(e);
            }
            conn.authenticated = true;
        }
        if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
            conn.name = Some(n.to_string());
        }
        state.conns.write().await.insert(
            conn.cid,
            ConnInfo {
                cid: conn.cid,
                name: conn.name.clone(),
                connected_at: now_ms(),
                subscription_count: 0,
            },
        );
        return CmdOutcome::Continue;
    }

    if line.starts_with("SUB") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || parts.len() > 4 {
            return CmdOutcome::Fatal("malformed SUB".to_string());
        }
        if !conn.authenticated {
            return CmdOutcome::Fatal("authorization violation".to_string());
        }
        let subject = parts[1].to_string();
        let (queue, sid) = if parts.len() == 3 {
            (None, parts[2].to_string())
        } else {
            (Some(parts[2].to_string()), parts[3].to_string())
        };
        let sub = Subscription { subject: subject.clone(), queue, sid: sid.clone() };
        state.root.write().await.insert(&tokens_of(&subject), sub.clone(), conn.cid);
        conn.subs.insert(sid.clone(), sub);
        if let Some(info) = state.conns.write().await.get_mut(&conn.cid) {
            info.subscription_count = conn.subs.len();
        }
        return CmdOutcome::Continue;
    }

    if line.starts_with("UNSUB") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 || parts.len() > 3 {
            return CmdOutcome::Fatal("malformed UNSUB".to_string());
        }
        let sid = parts[1].to_string();
        if let Some(s) = conn.subs.get(&sid) {
            state.root.write().await.remove(&tokens_of(&s.subject), conn.cid, &sid);
        }
        conn.subs.remove(&sid);
        if let Some(info) = state.conns.write().await.get_mut(&conn.cid) {
            info.subscription_count = conn.subs.len();
        }
        return CmdOutcome::Continue;
    }

    if line.starts_with("PUB ") {
        return parse_pub_header(line, false, conn);
    }
    if line.starts_with("HPUB ") {
        return parse_pub_header(line, true, conn);
    }

    let first = line.split_whitespace().next().unwrap_or("");
    CmdOutcome::Fatal(format!("unknown protocol operation: {}", first))
}

/// Parse `PUB <subject> [reply] <total>` or `HPUB <subject> [reply] <hdr_len> <total>`.
fn parse_pub_header(line: &str, is_h: bool, conn: &ClientConn) -> CmdOutcome {
    if !conn.authenticated {
        return CmdOutcome::Fatal("authorization violation".to_string());
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if is_h {
        if parts.len() < 4 {
            return CmdOutcome::Fatal("malformed HPUB".to_string());
        }
        let subject = parts[1].to_string();
        let reply = if parts.len() == 5 { Some(parts[2].to_string()) } else { None };
        let hdr_len: usize = match parts[parts.len() - 2].parse() {
            Ok(v) => v,
            Err(_) => return CmdOutcome::Fatal("bad HPUB hdr size".to_string()),
        };
        let total: usize = match parts[parts.len() - 1].parse() {
            Ok(v) => v,
            Err(_) => return CmdOutcome::Fatal("bad HPUB size".to_string()),
        };
        CmdOutcome::ReadPayload { subject, reply, total, hdr_len }
    } else {
        if parts.len() < 3 || parts.len() > 4 {
            return CmdOutcome::Fatal("malformed PUB".to_string());
        }
        let subject = parts[1].to_string();
        let reply = if parts.len() == 4 { Some(parts[2].to_string()) } else { None };
        let total: usize = match parts[parts.len() - 1].parse() {
            Ok(v) => v,
            Err(_) => return CmdOutcome::Fatal("bad PUB size".to_string()),
        };
        CmdOutcome::ReadPayload { subject, reply, total, hdr_len: 0 }
    }
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// Route a published message to all matching subscribers and mirror it.
async fn route_publish(
    state: &NatsServerState,
    subject: String,
    reply: Option<String>,
    headers: Vec<(String, String)>,
    payload: Vec<u8>,
) {
    // Intercept JetStream API requests (handled in-memory, no subscribers).
    if subject == "$JS.API" || subject.starts_with("$JS.API.") {
        let js = state.jetstream.clone();
        js.handle(&subject, &payload, reply.clone(), state).await;
        // Still mirror so the UI shows JetStream traffic.
        state
            .tracker
            .record_message(subject.clone(), payload.clone(), headers.clone())
            .await;
        return;
    }

    // Record published messages into matching JetStream streams (in-memory
    // storage). This happens before fan-out so consumers see them.
    state
        .jetstream
        .store_message(&subject, &payload, headers.is_empty())
        .await;

    let matches = {
        let r = state.root.read().await;
        find_matches(&r, &subject)
    };

    // Fan out with queue-group dedup: one member per queue group gets a copy.
    let mut delivered_q: HashSet<Option<String>> = HashSet::new();
    for (sub, cid) in matches {
        if let Some(q) = &sub.queue {
            if !delivered_q.insert(Some(q.clone())) {
                continue;
            }
        } else {
            delivered_q.insert(None);
        }
        let routed = Routed {
            subject: subject.clone(),
            reply: reply.clone(),
            headers: headers.clone(),
            payload: payload.clone(),
            sid: sub.sid.clone(),
        };
        let outboxes = state.outboxes.read().await;
        if let Some(tx) = outboxes.get(&cid) {
            let _ = tx.send(routed);
        }
    }

    // Mirror to tracker + callback (shows publishes even without subscribers).
    state
        .tracker
        .record_message(subject.clone(), payload.clone(), headers.clone())
        .await;
    if let Some(cb) = state.on_message.read().await.as_ref() {
        cb(subject, payload, now_ms(), headers);
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

async fn auth_required(state: &NatsServerState) -> bool {
    !state.config.read().await.auth.users.is_empty()
}

async fn authenticate(state: &NatsServerState, connect: &serde_json::Value) -> Result<(), String> {
    let cfg = state.config.read().await;
    if cfg.auth.users.is_empty() {
        return Ok(());
    }
    let user = connect.get("user").and_then(|v| v.as_str()).unwrap_or("");
    let pass = connect.get("pass").and_then(|v| v.as_str()).unwrap_or("");
    if cfg.auth.users.iter().any(|u| u.username == user && u.password == pass) {
        Ok(())
    } else {
        Err("authorization violation".to_string())
    }
}
