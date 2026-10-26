//! NATS client wrapper around the official `async_nats` crate.
//!
//! The underlying [`async_nats::Client`] is `Clone` + `Send + Sync` (it is an
//! `Arc` internally), so it can be shared across Tauri commands and background
//! tasks. JetStream access is exposed through [`NatsClient::jetstream`].

use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::AsyncReadExt;

use crate::types::*;

/// A connected NATS client (wraps [`async_nats::Client`]).
#[derive(Clone)]
pub struct NatsClient {
    client: async_nats::Client,
}

impl NatsClient {
    /// Connect to a NATS server.
    pub async fn connect(config: &NatsClientConfig) -> Result<Self, String> {
        let server_url = if config.tls.enabled {
            format!("tls://{}:{}", config.host, config.port)
        } else {
            format!("nats://{}:{}", config.host, config.port)
        };

        let mut opts = async_nats::ConnectOptions::new()
            .name(if config.name.trim().is_empty() {
                "nats-explorer".to_string()
            } else {
                config.name.clone()
            })
            .connection_timeout(Duration::from_millis(config.connect_timeout_ms))
            .max_reconnects(2);

        // Resolve effective auth: the new `auth` field wins; fall back to the
        // legacy username/password/token fields when it is `None`.
        let auth = match &config.auth {
            NatsAuth::None => match (&config.username, &config.password, &config.token) {
                (Some(u), Some(p), _) => NatsAuth::UserPass {
                    username: u.clone(),
                    password: p.clone(),
                },
                (_, _, Some(t)) => NatsAuth::Token { token: t.clone() },
                _ => NatsAuth::None,
            },
            other => other.clone(),
        };

        match auth {
            NatsAuth::None => {}
            NatsAuth::UserPass { username, password } => {
                opts = opts.user_and_password(username, password);
            }
            NatsAuth::Token { token } => {
                opts = opts.token(token);
            }
            NatsAuth::CredentialsFile { path } => {
                opts = opts
                    .credentials_file(path)
                    .await
                    .map_err(|e| format!("Credentials file failed: {}", e))?;
            }
            NatsAuth::Credentials { creds } => {
                opts = opts
                    .credentials(&creds)
                    .map_err(|e| format!("Credentials parse failed: {}", e))?;
            }
            NatsAuth::Nkey { seed } => {
                opts = opts.nkey(seed);
            }
            NatsAuth::Jwt {
                user_jwt,
                nkey_seed,
            } => {
                // Rebuild the KeyPair per nonce so the closure is `'static +
                // Send and we don't depend on KeyPair: Clone.
                opts = opts.jwt(user_jwt, move |nonce: Vec<u8>| {
                    let seed = nkey_seed.clone();
                    async move {
                        let kp = nkeys::KeyPair::from_seed(&seed)
                            .map_err(async_nats::AuthError::new)?;
                        kp.sign(&nonce).map_err(async_nats::AuthError::new)
                    }
                });
            }
        }

        if config.tls.enabled {
            opts = opts.require_tls(true);
            // async-nats 0.50 takes CA/cert/key from file paths. Write the
            // PEM bodies to a temp dir so self-signed / mTLS setups work.
            let tls_files = TlsFiles::write(&config.tls)?;
            opts = opts.add_root_certificates(tls_files.ca_path.clone());
            if let (Some(cert), Some(key)) = (tls_files.cert_path.as_ref(), tls_files.key_path.as_ref())
            {
                opts = opts.add_client_certificate(cert.clone(), key.clone());
            }
            // Keep the temp dir alive for the connection's lifetime.
            let _ = tls_files;
        }

        let client = opts
            .connect(&server_url)
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;
        Ok(Self { client })
    }

    /// The raw async-nats client (for advanced use / JetStream context).
    pub fn inner(&self) -> &async_nats::Client {
        &self.client
    }

    /// Publish a raw message (no headers).
    pub async fn publish(&self, subject: &str, payload: &[u8]) -> Result<(), String> {
        self.client
            .publish(subject.to_string(), payload.to_vec().into())
            .await
            .map_err(|e| format!("Publish failed: {}", e))
    }

    /// Publish a message with headers.
    pub async fn publish_with_headers(
        &self,
        subject: &str,
        headers: &[(String, String)],
        payload: &[u8],
    ) -> Result<(), String> {
        let mut h = async_nats::HeaderMap::default();
        for (k, v) in headers {
            h.append(k.as_str(), v.as_str());
        }
        self.client
            .publish_with_headers(subject.to_string(), h, payload.to_vec().into())
            .await
            .map_err(|e| format!("Publish failed: {}", e))
    }

    /// Publish a full UI message (payload may be hex-encoded).
    pub async fn publish_message(&self, msg: &NatsMessage) -> Result<(), String> {
        let payload = msg.payload_bytes();
        if msg.headers.is_empty() {
            self.publish(&msg.subject, &payload).await
        } else {
            self.publish_with_headers(&msg.subject, &msg.headers, &payload).await
        }
    }

    /// Send a request and await the reply (for request/reply patterns).
    pub async fn request(
        &self,
        subject: &str,
        payload: &[u8],
        timeout_ms: u64,
    ) -> Result<NatsMessage, String> {
        let fut = self.client.request(subject.to_string(), payload.to_vec().into());
        let reply = tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
            .await
            .map_err(|_| "Request timed out".to_string())?
            .map_err(|e| format!("Request failed: {}", e))?;
        Ok(msg_from_async(reply))
    }

    /// Subscribe to a subject (with optional queue group) and return a
    /// subscription wrapper that can be drained into `NatsMessage`s.
    pub async fn subscribe(
        &self,
        subject: &str,
        queue: Option<String>,
    ) -> Result<NatsSubscription, String> {
        let sub = if let Some(q) = queue.filter(|q| !q.trim().is_empty()) {
            self.client
                .queue_subscribe(subject.to_string(), q.to_string())
                .await
                .map_err(|e| format!("Subscribe failed: {}", e))?
        } else {
            self.client
                .subscribe(subject.to_string())
                .await
                .map_err(|e| format!("Subscribe failed: {}", e))?
        };
        let sid = format!("{:x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default());
        Ok(NatsSubscription { sid, subscription: sub })
    }
}

/// A live subscription; `next()` returns the next message.
pub struct NatsSubscription {
    sid: String,
    subscription: async_nats::Subscriber,
}

impl NatsSubscription {
    pub fn sid(&self) -> &str {
        &self.sid
    }

    pub async fn next(&mut self) -> Option<NatsMessage> {
        self.subscription.next().await.map(msg_from_async)
    }

    /// Stop receiving further messages.
    pub async fn unsubscribe(&mut self) -> Result<(), String> {
        self.subscription
            .unsubscribe()
            .await
            .map_err(|e| format!("Unsubscribe failed: {}", e))
    }
}

// ---------------------------------------------------------------------------
// JetStream helpers
// ---------------------------------------------------------------------------

impl NatsClient {
    /// A JetStream context bound to this client.
    pub fn jetstream(&self) -> async_nats::jetstream::Context {
        async_nats::jetstream::new(self.client.clone())
    }

    /// List stream names.
    pub async fn js_list_streams(&self) -> Result<Vec<String>, String> {
        let ctx = self.jetstream();
        let mut names = Vec::new();
        let mut it = ctx.stream_names();
        while let Some(name) = it.next().await {
            names.push(name.map_err(|e| e.to_string())?);
        }
        Ok(names)
    }

    /// Get a stream's summary info (or None when it doesn't exist).
    pub async fn js_stream_info(&self, name: &str) -> Result<Option<NatsStreamInfo>, String> {
        use async_nats::jetstream::context::GetStreamErrorKind;
        let ctx = self.jetstream();
        let stream = match ctx.get_stream(name).await {
            Ok(s) => s,
            Err(e) => {
                if matches!(e.kind(), GetStreamErrorKind::JetStream(err) if is_not_found(&err)) {
                    return Ok(None);
                }
                return Err(e.to_string());
            }
        };
        let info = stream.cached_info().clone();
        Ok(Some(stream_info_from(info)))
    }

    /// Create (or update) a stream.
    pub async fn js_create_stream(
        &self,
        name: &str,
        subjects: &[String],
        max_messages: i64,
        max_age_ns: i64,
    ) -> Result<NatsStreamInfo, String> {
        let cfg = async_nats::jetstream::stream::Config {
            name: name.to_string(),
            subjects: subjects.to_vec(),
            max_messages,
            max_age: if max_age_ns <= 0 {
                Duration::ZERO
            } else {
                Duration::from_nanos(max_age_ns as u64)
            },
            ..Default::default()
        };
        let stream = self
            .jetstream()
            .create_stream(cfg)
            .await
            .map_err(|e| format!("Create stream failed: {}", e))?;
        Ok(stream_info_from(stream.cached_info().clone()))
    }

    /// Delete a stream.
    pub async fn js_delete_stream(&self, name: &str) -> Result<(), String> {
        self.jetstream()
            .delete_stream(name)
            .await
            .map_err(|e| format!("Delete stream failed: {}", e))?;
        Ok(())
    }

    /// Purge all messages from a stream.
    pub async fn js_purge_stream(&self, name: &str) -> Result<(), String> {
        let ctx = self.jetstream();
        let stream = ctx
            .get_stream(name)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        stream
            .purge()
            .keep(0)
            .await
            .map_err(|e| format!("Purge stream failed: {}", e))?;
        Ok(())
    }

    /// Fetch the last `limit` stored messages from a stream (in order).
    pub async fn js_stream_messages(
        &self,
        name: &str,
        limit: u64,
    ) -> Result<Vec<NatsStoredMessage>, String> {
        let ctx = self.jetstream();
        let stream = ctx
            .get_stream(name)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let state = stream.cached_info().state.clone();
        let total = state.messages;
        let start = total.saturating_sub(limit).max(1);
        let mut out = Vec::new();
        for seq in start..=total {
            match stream.get_raw_message(seq).await {
                Ok(m) => {
                    out.push(NatsStoredMessage {
                        sequence: m.sequence,
                        subject: m.subject.as_ref().to_string(),
                        payload: encode_payload(&m.payload),
                        timestamp_ms: unix_ms(m.time),
                    });
                }
                Err(_) => break,
            }
        }
        Ok(out)
    }

    /// Create a durable pull consumer on a stream.
    pub async fn js_create_consumer(
        &self,
        stream: &str,
        durable: &str,
    ) -> Result<NatsConsumerInfo, String> {
        let ctx = self.jetstream();
        let stream_handle = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let cfg = async_nats::jetstream::consumer::pull::Config {
            durable_name: Some(durable.to_string()),
            ..Default::default()
        };
        let consumer = stream_handle
            .get_or_create_consumer(durable, cfg)
            .await
            .map_err(|e| format!("Create consumer failed: {}", e))?;
        let info = consumer.cached_info().clone();
        Ok(consumer_info_from(info))
    }

    /// Delete a consumer.
    pub async fn js_delete_consumer(&self, stream: &str, durable: &str) -> Result<(), String> {
        let ctx = self.jetstream();
        let stream_handle = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        stream_handle
            .delete_consumer(durable)
            .await
            .map_err(|e| format!("Delete consumer failed: {}", e))?;
        Ok(())
    }

    /// Get a consumer's info.
    pub async fn js_consumer_info(
        &self,
        stream: &str,
        durable: &str,
    ) -> Result<NatsConsumerInfo, String> {
        let ctx = self.jetstream();
        let s = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let c = s
            .get_consumer::<async_nats::jetstream::consumer::pull::Config>(durable)
            .await
            .map_err(|e| format!("Get consumer failed: {}", e))?;
        let info = c.cached_info().clone();
        Ok(consumer_info_from(info))
    }

    /// Fetch `batch` messages from a durable pull consumer, waiting up to
    /// `wait_ms` for the first message.
    pub async fn js_consumer_next(
        &self,
        stream: &str,
        durable: &str,
        batch: u64,
        wait_ms: u64,
    ) -> Result<Vec<NatsStoredMessage>, String> {
        let ctx = self.jetstream();
        let s = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let c = s
            .get_consumer::<async_nats::jetstream::consumer::pull::Config>(durable)
            .await
            .map_err(|e| format!("Get consumer failed: {}", e))?;

        let builder = c.fetch().max_messages(batch as usize);
        let builder = if wait_ms > 0 {
            builder.expires(Duration::from_millis(wait_ms))
        } else {
            builder
        };
        let mut msgs = builder.messages().await.map_err(|e| e.to_string())?;

        let mut out = Vec::new();
        for _ in 0..batch {
            match msgs.next().await {
                Some(Ok(m)) => {
                    let info = m
                        .info()
                        .map_err(|e| format!("Consumer message info failed: {}", e))?;
                    let seq = info.stream_sequence;
                    let subject = m.subject.as_ref().to_string();
                    let ts = unix_ms(info.published);
                    let payload = m.payload.clone();
                    let _ = m.ack().await;
                    out.push(NatsStoredMessage {
                        sequence: seq,
                        subject,
                        payload: encode_payload(&payload),
                        timestamp_ms: ts,
                    });
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// Every stream with its info, in one paged `STREAM.LIST`.
    pub async fn js_streams(&self) -> Result<Vec<NatsStreamInfo>, String> {
        let ctx = self.jetstream();
        let mut streams = ctx.streams();
        let mut out = Vec::new();
        while let Some(info) = streams.next().await {
            out.push(stream_info_from(info.map_err(|e| e.to_string())?));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Every consumer of a stream, with its info (`CONSUMER.LIST`).
    pub async fn js_stream_consumers(&self, stream: &str) -> Result<Vec<NatsConsumerInfo>, String> {
        let ctx = self.jetstream();
        let handle = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let mut consumers = handle.consumers();
        let mut out = Vec::new();
        while let Some(info) = consumers.next().await {
            out.push(consumer_info_from(info.map_err(|e| e.to_string())?));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// The newest `limit` stored messages of a stream, oldest first. Walks
    /// back from the last sequence, so it works after limits or purges
    /// dropped the first messages; deleted sequences are skipped.
    pub async fn js_stream_tail(
        &self,
        stream: &str,
        limit: u64,
    ) -> Result<Vec<NatsStoredMessage>, String> {
        let ctx = self.jetstream();
        let handle = ctx
            .get_stream(stream)
            .await
            .map_err(|e| format!("Get stream failed: {}", e))?;
        let state = handle.cached_info().state.clone();
        if state.messages == 0 || limit == 0 {
            return Ok(Vec::new());
        }
        let first = state.first_sequence.max(1);
        let mut out = Vec::new();
        let mut seq = state.last_sequence;
        while seq >= first && (out.len() as u64) < limit {
            if let Ok(m) = handle.get_raw_message(seq).await {
                out.push(NatsStoredMessage {
                    sequence: m.sequence,
                    subject: m.subject.as_ref().to_string(),
                    payload: encode_payload(&m.payload),
                    timestamp_ms: unix_ms(m.time),
                });
            }
            seq -= 1;
        }
        out.reverse();
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// KV store
// ---------------------------------------------------------------------------

impl NatsClient {
    /// List KV bucket names (stream names with the `KV_` prefix stripped).
    pub async fn kv_list(&self) -> Result<Vec<String>, String> {
        let ctx = self.jetstream();
        let mut out = Vec::new();
        let mut it = ctx.stream_names();
        while let Some(name) = it.next().await {
            let name = name.map_err(|e| format!("List KV buckets failed: {}", e))?;
            if let Some(b) = name.strip_prefix("KV_") {
                out.push(b.to_string());
            }
        }
        Ok(out)
    }

    pub async fn kv_create(&self, bucket: &str, history: i64, max_age_ns: i64) -> Result<(), String> {
        let cfg = async_nats::jetstream::kv::Config {
            bucket: bucket.to_string(),
            history,
            max_age: if max_age_ns > 0 {
                Duration::from_nanos(max_age_ns as u64)
            } else {
                Duration::ZERO
            },
            ..Default::default()
        };
        self.jetstream()
            .create_key_value(cfg)
            .await
            .map_err(|e| format!("Create KV bucket failed: {}", e))?;
        Ok(())
    }

    pub async fn kv_delete_bucket(&self, bucket: &str) -> Result<(), String> {
        self.jetstream()
            .delete_key_value(bucket)
            .await
            .map(|_| ())
            .map_err(|e| format!("Delete KV bucket failed: {}", e))
    }

    pub async fn kv_status(&self, bucket: &str) -> Result<NatsKvBucketInfo, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        let status = store
            .status()
            .await
            .map_err(|e| format!("KV status failed: {}", e))?;
        Ok(NatsKvBucketInfo {
            bucket: status.bucket().to_string(),
            values: status.values(),
            bytes: status.info.state.bytes,
            history: status.history(),
        })
    }

    pub async fn kv_keys(&self, bucket: &str) -> Result<Vec<String>, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        let mut out = Vec::new();
        let mut keys = store
            .keys()
            .await
            .map_err(|e| format!("KV keys failed: {}", e))?;
        while let Some(k) = keys.next().await {
            out.push(k.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub async fn kv_get(&self, bucket: &str, key: &str) -> Result<Option<NatsKvEntry>, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        let entry = store
            .entry(key.to_string())
            .await
            .map_err(|e| format!("KV entry failed: {}", e))?;
        Ok(entry.map(|e| kv_entry_from(bucket, e)))
    }

    pub async fn kv_put(&self, bucket: &str, key: &str, payload: &[u8]) -> Result<u64, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        store
            .put(key.to_string(), bytes::Bytes::copy_from_slice(payload))
            .await
            .map_err(|e| format!("KV put failed: {}", e))
    }

    pub async fn kv_delete_key(&self, bucket: &str, key: &str) -> Result<(), String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        store
            .delete(key.to_string())
            .await
            .map_err(|e| format!("KV delete failed: {}", e))
    }

    pub async fn kv_history(&self, bucket: &str, key: &str) -> Result<Vec<NatsKvEntry>, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        let mut out = Vec::new();
        let mut hist = store
            .history(key.to_string())
            .await
            .map_err(|e| format!("KV history failed: {}", e))?;
        while let Some(e) = hist.next().await {
            out.push(kv_entry_from(bucket, e.map_err(|e| e.to_string())?));
        }
        Ok(out)
    }

    /// Open a KV watch (all keys, or a single key). The caller drives `.next()`
    /// in a spawned task and emits UI events per entry.
    pub async fn kv_open_watch(
        &self,
        bucket: &str,
        key: Option<&str>,
    ) -> Result<async_nats::jetstream::kv::Watch, String> {
        let store = self
            .jetstream()
            .get_key_value(bucket.to_string())
            .await
            .map_err(|e| format!("Open KV bucket failed: {}", e))?;
        let watch = match key {
            Some(k) => store.watch(k.to_string()).await,
            None => store.watch_all().await,
        }
        .map_err(|e| format!("KV watch failed: {}", e))?;
        Ok(watch)
    }
}

// ---------------------------------------------------------------------------
// Object Store
// ---------------------------------------------------------------------------

impl NatsClient {
    /// List Object Store bucket names (streams with the `OBJ_` prefix stripped).
    pub async fn obj_list(&self) -> Result<Vec<String>, String> {
        let ctx = self.jetstream();
        let mut out = Vec::new();
        let mut it = ctx.stream_names();
        while let Some(name) = it.next().await {
            let name = name.map_err(|e| format!("List Object Store buckets failed: {}", e))?;
            if let Some(b) = name.strip_prefix("OBJ_") {
                out.push(b.to_string());
            }
        }
        Ok(out)
    }

    pub async fn obj_create(&self, bucket: &str) -> Result<(), String> {
        let cfg = async_nats::jetstream::object_store::Config {
            bucket: bucket.to_string(),
            ..Default::default()
        };
        self.jetstream()
            .create_object_store(cfg)
            .await
            .map_err(|e| format!("Create Object Store failed: {}", e))?;
        Ok(())
    }

    pub async fn obj_delete_bucket(&self, bucket: &str) -> Result<(), String> {
        self.jetstream()
            .delete_object_store(bucket)
            .await
            .map(|_| ())
            .map_err(|e| format!("Delete Object Store failed: {}", e))
    }

    pub async fn obj_list_objects(&self, bucket: &str) -> Result<Vec<NatsObjectInfo>, String> {
        let store = self.open_object_store_async(bucket).await?;
        let mut out = Vec::new();
        let mut list = store
            .list()
            .await
            .map_err(|e| format!("Object Store list failed: {}", e))?;
        while let Some(info) = list.next().await {
            out.push(obj_info_from(bucket, info.map_err(|e| e.to_string())?));
        }
        Ok(out)
    }

    pub async fn obj_info(&self, bucket: &str, name: &str) -> Result<NatsObjectInfo, String> {
        let store = self.open_object_store_async(bucket).await?;
        let info = store
            .info(name)
            .await
            .map_err(|e| format!("Object info failed: {}", e))?;
        Ok(obj_info_from(bucket, info))
    }

    pub async fn obj_get(&self, bucket: &str, name: &str) -> Result<Vec<u8>, String> {
        let store = self.open_object_store_async(bucket).await?;
        let mut object = store
            .get(name)
            .await
            .map_err(|e| format!("Object get failed: {}", e))?;
        let mut buf = Vec::new();
        object
            .read_to_end(&mut buf)
            .await
            .map_err(|e| format!("Object read failed: {}", e))?;
        Ok(buf)
    }

    pub async fn obj_put(&self, bucket: &str, name: &str, data: &[u8]) -> Result<(), String> {
        let store = self.open_object_store_async(bucket).await?;
        let mut reader = std::io::Cursor::new(data.to_vec());
        store
            .put(name, &mut reader)
            .await
            .map(|_| ())
            .map_err(|e| format!("Object put failed: {}", e))
    }

    pub async fn obj_delete(&self, bucket: &str, name: &str) -> Result<(), String> {
        let store = self.open_object_store_async(bucket).await?;
        store
            .delete(name)
            .await
            .map_err(|e| format!("Object delete failed: {}", e))
    }
}

impl NatsClient {
    pub async fn open_object_store_async(
        &self,
        bucket: &str,
    ) -> Result<async_nats::jetstream::object_store::ObjectStore, String> {
        self.jetstream()
            .get_object_store(bucket.to_string())
            .await
            .map_err(|e| format!("Open Object Store failed: {}", e))
    }
}

fn obj_info_from(bucket: &str, info: async_nats::jetstream::object_store::ObjectInfo) -> NatsObjectInfo {
    NatsObjectInfo {
        bucket: bucket.to_string(),
        name: info.name,
        size: info.size as u64,
        chunks: info.chunks as u64,
        digest: info.digest,
        modified_ms: info.modified.map(unix_ms).unwrap_or(0),
        deleted: info.deleted,
    }
}

pub fn kv_entry_from(bucket: &str, e: async_nats::jetstream::kv::Entry) -> NatsKvEntry {
    NatsKvEntry {
        bucket: bucket.to_string(),
        key: e.key,
        payload: encode_payload(&e.value),
        revision: e.revision,
        operation: format!("{:?}", e.operation).to_lowercase(),
        created_ms: unix_ms(e.created),
    }
}

/// Millis from a `time::OffsetDateTime` (async-nats `DateTime`).
fn unix_ms(dt: time::OffsetDateTime) -> i64 {
    (dt.unix_timestamp_nanos() / 1_000_000) as i64
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn msg_from_async(m: async_nats::Message) -> NatsMessage {
    let headers: Vec<(String, String)> = m
        .headers
        .as_ref()
        .map(|h| {
            h.iter()
                .flat_map(|(k, vals)| {
                    let k = k.to_string();
                    vals.iter().map(move |v| (k.clone(), v.as_str().to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    NatsMessage {
        subject: m.subject.as_ref().to_string(),
        reply: m.reply.as_ref().map(|s| s.as_ref().to_string()),
        headers,
        payload: encode_payload(&m.payload),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        queue: None,
    }
}

fn stream_info_from(info: async_nats::jetstream::stream::Info) -> NatsStreamInfo {
    NatsStreamInfo {
        name: info.config.name.clone(),
        subjects: info.config.subjects.clone(),
        description: info.config.description.clone(),
        messages: info.state.messages,
        bytes: info.state.bytes,
        max_messages: info.config.max_messages,
        max_age_ns: info.config.max_age.as_nanos() as i64,
        storage: format!("{:?}", info.config.storage).to_lowercase(),
        consumers: info.state.consumer_count as u64,
        created: Some(unix_ms(info.created).to_string()),
        first_seq: info.state.first_sequence,
        last_seq: info.state.last_sequence,
    }
}

fn consumer_info_from(info: async_nats::jetstream::consumer::Info) -> NatsConsumerInfo {
    let durable = info.config.durable_name.clone();
    let (deliver_kind, ack_policy) = match &info.config.deliver_subject {
        Some(_) => ("push".to_string(), "all".to_string()),
        None => (
            "pull".to_string(),
            format!("{:?}", info.config.ack_policy).to_lowercase(),
        ),
    };
    NatsConsumerInfo {
        stream: info.stream_name.clone(),
        name: durable.clone().unwrap_or_else(|| info.name.clone()),
        durable: durable.is_some(),
        deliver_kind,
        ack_policy,
        pending: info.num_pending,
        num_ack_pending: info.num_ack_pending as u64,
        num_redelivered: info.num_redelivered as u64,
    }
}

/// Match a JetStream error against the "not found" server error code.
fn is_not_found(e: &async_nats::jetstream::Error) -> bool {
    use async_nats::jetstream::ErrorCode;
    matches!(
        e.kind(),
        ErrorCode::STREAM_NOT_FOUND | ErrorCode::CONSUMER_NOT_FOUND
    )
}

// ---------------------------------------------------------------------------
// TLS helpers (write PEM bodies to temp files; async-nats 0.50 wants paths)
// ---------------------------------------------------------------------------

struct TlsFiles {
    ca_path: std::path::PathBuf,
    cert_path: Option<std::path::PathBuf>,
    key_path: Option<std::path::PathBuf>,
}

impl TlsFiles {
    fn write(tls: &NatsClientTlsConfig) -> Result<Self, String> {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "mt-nats-tls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create TLS temp dir: {}", e))?;

        let ca = tls.ca_pem.as_deref().unwrap_or("");
        let ca_path = dir.join("ca.pem");
        if !ca.trim().is_empty() {
            let mut f = std::fs::File::create(&ca_path).map_err(|e| e.to_string())?;
            f.write_all(ca.trim().as_bytes()).map_err(|e| e.to_string())?;
        } else {
            // Write an empty file so add_root_certificates doesn't error.
            let _ = std::fs::File::create(&ca_path);
        }

        let (cert_path, key_path) = match (tls.client_cert_pem.as_deref(), tls.client_key_pem.as_deref()) {
            (Some(c), Some(k)) if !c.trim().is_empty() && !k.trim().is_empty() => {
                let cp = dir.join("client.crt");
                let kp = dir.join("client.key");
                let mut f = std::fs::File::create(&cp).map_err(|e| e.to_string())?;
                f.write_all(c.trim().as_bytes()).map_err(|e| e.to_string())?;
                drop(f);
                let mut f = std::fs::File::create(&kp).map_err(|e| e.to_string())?;
                f.write_all(k.trim().as_bytes()).map_err(|e| e.to_string())?;
                (Some(cp), Some(kp))
            }
            _ => (None, None),
        };

        Ok(Self { ca_path, cert_path, key_path })
    }
}
