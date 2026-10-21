//! Embedded JetStream (`$JS.API`) support for the hand-rolled NATS server.
//!
//! Implements a minimal in-memory JetStream so the official `async_nats`
//! JetStream client works against the embedded server:
//!
//! - Streams: CREATE / INFO / LIST / NAMES / PURGE / DELETE / MSG.GET
//! - Consumers: CREATE / INFO / DELETE
//! - Pull consumers: CONSUMER.MSG.NEXT (batch fetch via reply subject)
//!
//! Storage is entirely in-memory (documented limitation: no persistence, no
//! replicas, no push consumers, no KV/object-store).
//!
//! Wire format follows the real nats-server:
//! - Requests are PUBs to `$JS.API.*` with a reply subject.
//! - Responses are JSON payloads published to the reply subject.
//! - Pull-delivered messages carry `Nats-Stream` / `Nats-Sequence` /
//!   `Nats-Time-Stamp` / `Nats-Subject` headers and a `$JS.ACK.<stream>.<consumer>...`
//!   reply so `message.info()` + `message.ack()` work.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tokio::sync::RwLock;

use crate::server::{NatsServerState, RoutePayload};

/// Reply to a JetStream API request by publishing to the reply subject via
/// `publish_raw` (which routes but does NOT mirror or re-intercept).
async fn js_reply(server: &NatsServerState, reply: String, payload: Vec<u8>) {
    let _ = server
        .publish_raw(RoutePayload {
            subject: reply,
            reply: None,
            headers: vec![],
            payload,
        })
        .await;
}

/// A stored message in a stream.
#[derive(Debug, Clone)]
pub struct StoredMsg {
    pub sequence: u64,
    pub subject: String,
    pub payload: Vec<u8>,
    pub timestamp_ms: i64,
}

/// A durable pull consumer's cursor.
#[derive(Debug, Clone)]
pub struct ConsumerState {
    pub name: String,
    pub stream: String,
    /// Last delivered stream sequence.
    pub next_seq: u64,
    /// Filter subject (empty = all).
    pub filter_subject: String,
}

/// In-memory stream definition.
#[derive(Debug, Clone)]
pub struct StreamState {
    pub name: String,
    pub subjects: Vec<String>,
    pub max_messages: i64,
    pub max_age: Duration,
    pub messages: VecDeque<StoredMsg>,
    pub next_seq: u64,
    pub created_ms: i64,
}

impl StreamState {
    fn matches(&self, subject: &str) -> bool {
        self.subjects.is_empty() || self.subjects.iter().any(|s| nats_subject_match(s, subject))
    }
}

/// Shared JetStream state inside the server.
#[derive(Clone, Default)]
pub struct JetStreamState {
    pub streams: Arc<RwLock<HashMap<String, StreamState>>>,
    pub consumers: Arc<RwLock<HashMap<String, Vec<ConsumerState>>>>,
}

impl JetStreamState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a published message into every stream whose subjects match.
    /// Returns true if it was stored in at least one stream.
    pub async fn store_message(&self, subject: &str, payload: &[u8], _no_headers: bool) -> bool {
        let now = now_ms();
        let mut streams = self.streams.write().await;
        let mut stored = false;
        for s in streams.values_mut() {
            if !s.matches(subject) {
                continue;
            }
            let msg = StoredMsg {
                sequence: s.next_seq,
                subject: subject.to_string(),
                payload: payload.to_vec(),
                timestamp_ms: now,
            };
            s.next_seq += 1;
            s.messages.push_back(msg);
            // Enforce max_messages (drop oldest).
            if s.max_messages > 0 {
                while s.messages.len() > s.max_messages as usize {
                    s.messages.pop_front();
                }
            }
            // Enforce max_age.
            if !s.max_age.is_zero() {
                while let Some(front) = s.messages.front() {
                    if now - front.timestamp_ms > s.max_age.as_millis() as i64 {
                        s.messages.pop_front();
                    } else {
                        break;
                    }
                }
            }
            stored = true;
        }
        stored
    }

    /// Handle an intercepted `$JS.API.*` request.
    pub async fn handle(
        &self,
        subject: &str,
        payload: &[u8],
        reply: Option<String>,
        server: &NatsServerState,
    ) {
        let Some(reply) = reply else {
            return; // No reply subject — nothing to respond to.
        };
        let rest = subject.strip_prefix("$JS.API.").unwrap_or(subject);
        let parts: Vec<&str> = rest.split('.').collect();

        match parts.first().copied() {
            Some("STREAM") => self.handle_stream(&parts, payload, reply, server).await,
            Some("CONSUMER") => self.handle_consumer(&parts, payload, reply, server).await,
            _ => {
                // Unknown API op → error response.
                let err = json_err(10052, "unknown JetStream API request");
                js_reply(server, reply, err).await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stream ops
    // -----------------------------------------------------------------------

    async fn handle_stream(
        &self,
        parts: &[&str],
        payload: &[u8],
        reply: String,
        server: &NatsServerState,
    ) {
        if parts.len() < 2 {
            return;
        }
        match parts[1] {
            "CREATE" | "UPDATE" if parts.len() >= 3 => {
                let name = parts[2];
                self.stream_create(name, payload, reply, server).await;
            }
            "INFO" if parts.len() >= 3 => {
                self.stream_info(parts[2], reply, server).await;
            }
            "LIST" => {
                self.stream_list(payload, reply, server).await;
            }
            "NAMES" => {
                self.stream_names(payload, reply, server).await;
            }
            "PURGE" if parts.len() >= 3 => {
                self.stream_purge(parts[2], reply, server).await;
            }
            "DELETE" if parts.len() >= 3 => {
                self.stream_delete(parts[2], reply, server).await;
            }
            "MSG" if parts.len() >= 4 && parts[2] == "GET" => {
                self.stream_msg_get(parts[3], payload, reply, server).await;
            }
            _ => {
                let err = json_err(10052, "unknown stream API request");
                js_reply(server, reply, err).await;
            }
        }
    }

    async fn stream_create(
        &self,
        name: &str,
        payload: &[u8],
        reply: String,
        server: &NatsServerState,
    ) {
        // The request body is a stream::Config JSON.
        let cfg: serde_json::Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            Err(_) => {
                let err = json_err(10052, "malformed stream config");
                js_reply(server, reply, err).await;
                return;
            }
        };
        let subjects: Vec<String> = cfg
            .get("subjects")
            .and_then(|s| s.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let max_messages = cfg.get("max_msgs").and_then(|v| v.as_i64()).unwrap_or(-1);
        let max_age = cfg
            .get("max_age")
            .and_then(|v| v.as_i64())
            .map(|ns| Duration::from_nanos(ns as u64))
            .unwrap_or(Duration::ZERO);

        let stream = StreamState {
            name: name.to_string(),
            subjects,
            max_messages,
            max_age,
            messages: VecDeque::new(),
            next_seq: 1,
            created_ms: now_ms(),
        };

        let mut map = self.streams.write().await;
        map.insert(name.to_string(), stream.clone());
        drop(map);

        let info = self.stream_info_json(&stream).await;
        js_reply(server, reply, serde_json::to_vec(&info).unwrap_or_default()).await;
    }

    async fn stream_info(&self, name: &str, reply: String, server: &NatsServerState) {
        let map = self.streams.read().await;
        match map.get(name) {
            Some(s) => {
                let info = self.stream_info_json(s).await;
                drop(map);
                js_reply(server, reply, serde_json::to_vec(&info).unwrap_or_default()).await;
            }
            None => {
                drop(map);
                let err = json_err(10059, "stream not found");
                js_reply(server, reply, err).await;
            }
        }
    }

    async fn stream_list(&self, _payload: &[u8], reply: String, server: &NatsServerState) {
        let map = self.streams.read().await;
        let infos: Vec<serde_json::Value> = {
            let mut v = Vec::new();
            for s in map.values() {
                v.push(self.stream_info_json(s).await);
            }
            v
        };
        drop(map);
        let resp = serde_json::json!({
            "total": infos.len(),
            "offset": 0,
            "limit": infos.len().max(1),
            "streams": infos,
        });
        js_reply(server, reply, serde_json::to_vec(&resp).unwrap_or_default()).await;
    }

    async fn stream_names(&self, _payload: &[u8], reply: String, server: &NatsServerState) {
        let map = self.streams.read().await;
        let names: Vec<String> = map.keys().cloned().collect();
        drop(map);
        let resp = serde_json::json!({
            "total": names.len(),
            "offset": 0,
            "limit": names.len().max(1),
            "streams": names,
        });
        js_reply(server, reply, serde_json::to_vec(&resp).unwrap_or_default()).await;
    }

    async fn stream_purge(&self, name: &str, reply: String, server: &NatsServerState) {
        let mut map = self.streams.write().await;
        match map.get_mut(name) {
            Some(s) => {
                let purged = s.messages.len() as u64;
                s.messages.clear();
                let resp = serde_json::json!({
                    "success": true,
                    "purged": purged,
                    "seqs": {
                        "first_seq": 0,
                        "last_seq": s.next_seq.saturating_sub(1),
                    },
                });
                drop(map);
                js_reply(server, reply, serde_json::to_vec(&resp).unwrap_or_default()).await;
            }
            None => {
                drop(map);
                let err = json_err(10059, "stream not found");
                js_reply(server, reply, err).await;
            }
        }
    }

    async fn stream_delete(&self, name: &str, reply: String, server: &NatsServerState) {
        let mut map = self.streams.write().await;
        if map.remove(name).is_some() {
            drop(map);
            self.consumers.write().await.remove(name);
            let resp = serde_json::json!({ "success": true });
            js_reply(server, reply, serde_json::to_vec(&resp).unwrap_or_default()).await;
        } else {
            drop(map);
            let err = json_err(10059, "stream not found");
            js_reply(server, reply, err).await;
        }
    }

    async fn stream_msg_get(&self, name: &str, payload: &[u8], reply: String, server: &NatsServerState) {
        // The request is a `STREAM.MSG.GET` request with {seq} or {last_by_subj}.
        let req: serde_json::Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            Err(_) => {
                let err = json_err(10052, "malformed MSG.GET request");
                js_reply(server, reply, err).await;
                return;
            }
        };
        let map = self.streams.read().await;
        let Some(stream) = map.get(name) else {
            drop(map);
            let err = json_err(10059, "stream not found");
            js_reply(server, reply, err).await;
            return;
        };
        let msg = if let Some(seq) = req.get("seq").and_then(|v| v.as_u64()) {
            stream.messages.iter().find(|m| m.sequence == seq).cloned()
        } else if let Some(subj) = req.get("last_by_subj").and_then(|v| v.as_str()) {
            stream.messages.iter().rev().find(|m| nats_subject_match(subj, &m.subject)).cloned()
        } else {
            None
        };
        let response = match msg {
            Some(m) => serde_json::json!({
                "message": {
                    "subject": m.subject,
                    "seq": m.sequence,
                    "data": base64::engine::general_purpose::STANDARD.encode(&m.payload),
                    "time": rfc3339(m.timestamp_ms),
                }
            }),
            None => serde_json::json!({
                "error": {
                    "code": 10037,
                    "err_code": 10037,
                    "description": "no message found",
                }
            }),
        };
        drop(map);
        js_reply(server, reply, serde_json::to_vec(&response).unwrap_or_default()).await;
    }

    async fn stream_info_json(&self, s: &StreamState) -> serde_json::Value {
        let msgs = s.messages.len() as u64;
        let bytes: u64 = s.messages.iter().map(|m| m.payload.len() as u64).sum();
        let first = s.messages.front().map(|m| m.sequence).unwrap_or(0);
        let last = s.messages.back().map(|m| m.sequence).unwrap_or(0);
        let first_ts = s.messages.front().map(|m| rfc3339(m.timestamp_ms)).unwrap_or_else(|| rfc3339(0));
        let last_ts = s.messages.back().map(|m| rfc3339(m.timestamp_ms)).unwrap_or_else(|| rfc3339(0));
        let consumers = self.consumers.read().await.get(&s.name).map(|c| c.len()).unwrap_or(0);

        serde_json::json!({
            "type": "io.nats.jetstream.api.v1.stream_info_response",
            "config": {
                "name": s.name,
                "subjects": s.subjects,
                "max_msgs": if s.max_messages > 0 { s.max_messages } else { -1 },
                "max_age": if s.max_age.is_zero() { 0 } else { s.max_age.as_nanos() as i64 },
                "storage": "memory",
                "discard": "old",
                "retention": "limits",
                "num_replicas": 1,
            },
            "created": rfc3339(s.created_ms),
            "state": {
                "messages": msgs,
                "bytes": bytes,
                "first_seq": first,
                "first_ts": first_ts,
                "last_seq": last,
                "last_ts": last_ts,
                "consumer_count": consumers,
            },
        })
    }

    // -----------------------------------------------------------------------
    // Consumer ops
    // -----------------------------------------------------------------------

    async fn handle_consumer(
        &self,
        parts: &[&str],
        payload: &[u8],
        reply: String,
        server: &NatsServerState,
    ) {
        if parts.len() < 3 {
            return;
        }
        match parts[1] {
            "CREATE" => {
                // CONSUMER.CREATE.<stream>.<name> or CONSUMER.CREATE.<stream>
                let stream = parts[2];
                let name = parts.get(3).copied().unwrap_or("default");
                self.consumer_create(stream, name, payload, reply, server).await;
            }
            "INFO" if parts.len() >= 4 => {
                self.consumer_info(parts[2], parts[3], reply, server).await;
            }
            "DELETE" if parts.len() >= 4 => {
                self.consumer_delete(parts[2], parts[3], reply, server).await;
            }
            "MSG" if parts.len() >= 5 && parts[2] == "NEXT" => {
                self.consumer_msg_next(parts[3], parts[4], payload, reply, server).await;
            }
            _ => {
                let err = json_err(10052, "unknown consumer API request");
                js_reply(server, reply, err).await;
            }
        }
    }

    async fn consumer_create(
        &self,
        stream: &str,
        name: &str,
        payload: &[u8],
        reply: String,
        server: &NatsServerState,
    ) {
        let cfg: serde_json::Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            Err(_) => {
                let err = json_err(10052, "malformed consumer config");
                js_reply(server, reply, err).await;
                return;
            }
        };
        let durable = cfg.get("durable_name").and_then(|v| v.as_str())
            .or_else(|| cfg.get("name").and_then(|v| v.as_str()))
            .unwrap_or(name)
            .to_string();
        let filter = cfg.get("filter_subject").and_then(|v| v.as_str()).unwrap_or("").to_string();

        {
            let mut cons = self.consumers.write().await;
            let list = cons.entry(stream.to_string()).or_default();
            if !list.iter().any(|c| c.name == durable) {
                list.push(ConsumerState {
                    name: durable.clone(),
                    stream: stream.to_string(),
                    next_seq: 1,
                    filter_subject: filter.clone(),
                });
            }
        }

        let info = self.consumer_info_json(stream, &durable).await;
        js_reply(server, reply, serde_json::to_vec(&info).unwrap_or_default()).await;
    }

    async fn consumer_info(&self, stream: &str, name: &str, reply: String, server: &NatsServerState) {
        let info = self.consumer_info_json(stream, name).await;
        js_reply(server, reply, serde_json::to_vec(&info).unwrap_or_default()).await;
    }

    async fn consumer_delete(&self, stream: &str, name: &str, reply: String, server: &NatsServerState) {
        let mut cons = self.consumers.write().await;
        let removed = if let Some(list) = cons.get_mut(stream) {
            let before = list.len();
            list.retain(|c| c.name != name);
            before != list.len()
        } else {
            false
        };
        drop(cons);
        let resp = serde_json::json!({ "success": removed });
        js_reply(server, reply, serde_json::to_vec(&resp).unwrap_or_default()).await;
    }

    async fn consumer_msg_next(
        &self,
        stream: &str,
        name: &str,
        payload: &[u8],
        reply: String,
        server: &NatsServerState,
    ) {
        // BatchConfig request: {batch, max_bytes, expires(ns), no_wait, idle_heartbeat}.
        let req: serde_json::Value = serde_json::from_slice(payload).unwrap_or(serde_json::json!({"batch": 1}));
        let batch = req.get("batch").and_then(|v| v.as_u64()).unwrap_or(1).min(1000) as usize;

        let mut cons = self.consumers.write().await;
        let list = cons.entry(stream.to_string()).or_default();
        let Some(consumer) = list.iter_mut().find(|c| c.name == name) else {
            drop(cons);
            let err = serde_json::to_vec(&serde_json::json!({
                "error": { "code": 404, "err_code": 10014, "description": "consumer not found" }
            })).unwrap_or_default();
            js_reply(server, reply, err).await;
            return;
        };

        // Collect up to `batch` matching messages from the stream, starting at the cursor.
        let streams = self.streams.read().await;
        let Some(sdef) = streams.get(stream) else {
            drop(streams);
            drop(cons);
            let err = json_err(10059, "stream not found");
            js_reply(server, reply, err).await;
            return;
        };

        let mut delivered = Vec::new();
        let mut cursor = consumer.next_seq;
        for m in &sdef.messages {
            if delivered.len() >= batch {
                break;
            }
            if m.sequence < cursor {
                continue;
            }
            if !consumer.filter_subject.is_empty() && !nats_subject_match(&consumer.filter_subject, &m.subject) {
                continue;
            }
            delivered.push(m.clone());
            cursor = m.sequence + 1;
        }
        consumer.next_seq = cursor;
        drop(streams);
        drop(cons);

        // Deliver each message to the reply subject with JetStream headers.
        for m in &delivered {
            let headers = vec![
                ("Nats-Stream".to_string(), stream.to_string()),
                ("Nats-Sequence".to_string(), m.sequence.to_string()),
                ("Nats-Time-Stamp".to_string(), rfc3339(m.timestamp_ms)),
                ("Nats-Subject".to_string(), m.subject.clone()),
                ("Nats-Last-Stream".to_string(), stream.to_string()),
                ("Nats-Last-Sequence".to_string(), m.sequence.to_string()),
            ];
            // 7-token ack format: $JS.ACK.<stream>.<consumer>.<delivered>.<sseq>.<cseq>.<nanos>.<pending>
            let nanos = m.timestamp_ms * 1_000_000;
            let ack_reply = format!(
                "$JS.ACK.{}.{}.{}.{}.{}.{}.{}",
                stream, name, m.sequence, m.sequence, m.sequence, nanos, 0
            );
            let _ = server
                .publish_raw(RoutePayload {
                    subject: reply.clone(),
                    reply: Some(ack_reply),
                    headers,
                    payload: m.payload.clone(),
                })
                .await;
        }

        if delivered.is_empty() {
            // No messages — respond with a 404 status (headers-only).
            let _ = server
                .publish_with_status(reply, 404, "No Messages", vec![], vec![])
                .await;
        }
    }

    async fn consumer_info_json(&self, stream: &str, name: &str) -> serde_json::Value {
        let (exists, num_pending) = {
            let cons = self.consumers.read().await;
            let c = cons.get(stream).and_then(|l| l.iter().find(|c| c.name == name));
            match c {
                Some(c) => {
                    let streams = self.streams.read().await;
                    let pending = streams
                        .get(stream)
                        .map(|s| s.messages.iter().filter(|m| m.sequence >= c.next_seq).count() as u64)
                        .unwrap_or(0);
                    (true, pending)
                }
                None => (false, 0),
            }
        };
        if !exists {
            return serde_json::json!({
                "error": {
                    "code": 404,
                    "err_code": 10014,
                    "description": "consumer not found",
                }
            });
        }
        serde_json::json!({
            "type": "io.nats.jetstream.api.v1.consumer_info_response",
            "stream_name": stream,
            "name": name,
            "created": rfc3339(now_ms()),
            "config": {
                "durable_name": name,
                "deliver_policy": "all",
                "ack_policy": "explicit",
                "ack_wait": 30000000000_i64,
                "max_deliver": -1,
                "filter_subject": "",
                "replay_policy": "instant",
                "max_waiting": 512,
                "num_replicas": 1,
            },
            "delivered": { "consumer_seq": 0, "stream_seq": 0 },
            "ack_floor": { "consumer_seq": 0, "stream_seq": 0 },
            "num_ack_pending": 0,
            "num_redelivered": 0,
            "num_waiting": 0,
            "num_pending": num_pending,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Format an epoch-millis timestamp as RFC 3339 (UTC, nanoseconds).
fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) * 1_000_000) as u32;
    let dt = time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .map(|d| d + Duration::from_nanos(nanos as u64));
    match dt {
        Some(d) => format!("{}", d.format(&time::format_description::well_known::Rfc3339).unwrap_or_default()),
        None => String::new(),
    }
}

fn json_err(code: u64, desc: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "error": {
            "code": code,
            "err_code": code,
            "description": desc,
        }
    }))
    .unwrap_or_default()
}

/// NATS subject wildcard match (`*` = one token, `>` = tail).
pub fn nats_subject_match(pattern: &str, subject: &str) -> bool {
    let p: Vec<&str> = pattern.split('.').collect();
    let s: Vec<&str> = subject.split('.').collect();
    let mut pi = 0;
    let mut si = 0;
    while pi < p.len() {
        if p[pi] == ">" {
            return true; // matches the rest
        }
        if si >= s.len() {
            return false;
        }
        if p[pi] != "*" && p[pi] != s[si] {
            return false;
        }
        pi += 1;
        si += 1;
    }
    si == s.len()
}
