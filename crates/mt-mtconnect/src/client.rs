//! HTTP client for an MTConnect agent: probe, current, sample, assets and live streaming.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use url::Url;

use crate::error::{AgentErrorCode, MtConnectError, Result};
use crate::multipart::{MultipartParser, boundary_from_content_type};
use crate::parse::{Document, parse_document, unexpected};
use crate::types::{Asset, AssetList, Devices, Streams};
use crate::util::percent_encode;

fn default_timeout_ms() -> u64 {
    10_000
}

/// Connection settings for an MTConnect agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Agent base URL, e.g. `http://localhost:5050` or `https://demo.mtconnect.org`.
    pub base_url: String,
    /// Restrict requests to one device (`/{device}/current`, ...). Name or uuid.
    #[serde(default)]
    pub device: Option<String>,
    /// Connect and per-request timeout in milliseconds (not applied to streaming reads).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl AgentConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), device: None, timeout_ms: default_timeout_ms() }
    }
}

/// Parameters of a `sample` request.
#[derive(Debug, Clone, Default)]
pub struct SampleQuery {
    /// XPath filter, e.g. `//Axes//Linear`.
    pub path: Option<String>,
    /// First sequence to return (`from`). Defaults to the agent's first sequence.
    pub from: Option<u64>,
    /// Maximum number of observations (`count`). Defaults to the agent's default (100).
    pub count: Option<u32>,
}

/// Options for [`AgentClient::stream_sample_with`].
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Minimum time between documents (streaming) or between polls (fallback).
    pub interval: Duration,
    /// XPath filter.
    pub path: Option<String>,
    /// Start at this sequence. When `None` the stream first fetches `current`, emits it as
    /// [`StreamEvent::Snapshot`] and continues from its `nextSequence`.
    pub from: Option<u64>,
    /// Maximum observations per document.
    pub count: Option<u32>,
    /// Try `multipart` streaming before polling.
    pub prefer_streaming: bool,
    /// A streaming connection that delivers nothing for this long is considered dead.
    pub stall_timeout: Duration,
}

impl StreamOptions {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            path: None,
            from: None,
            count: None,
            prefer_streaming: true,
            stall_timeout: Duration::from_secs(30),
        }
    }
}

/// Why a stream re-synchronised from `current`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum ResyncReason {
    /// The header `instanceId` changed: the agent restarted and sequence numbers restarted.
    /// Device definitions may have changed, so re-`probe()` if you cache them.
    InstanceChanged { previous: u64, current: u64 },
    /// The agent answered `OUT_OF_RANGE`: the requested sequence was no longer in its buffer.
    OutOfRange,
}

/// One item delivered by a [`SampleStream`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "streams", rename_all = "camelCase")]
pub enum StreamEvent {
    /// Initial `current` state (only when `from` was not given).
    Snapshot(Streams),
    /// New observations since the previous event.
    Observations(Streams),
    /// Observations were missed; `snapshot` is the agent's `current` state to replace the local
    /// view with. New observations follow from `snapshot.header.nextSequence`.
    Resync { reason: ResyncReason, snapshot: Streams },
}

/// How a [`SampleStream`] is currently obtaining data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamTransport {
    Connecting,
    /// `multipart` streaming connection to the agent.
    Streaming,
    /// Repeated `sample?from=nextSequence` requests.
    Polling,
}

/// A running live stream. Dropping it stops the background task.
pub struct SampleStream {
    rx: mpsc::Receiver<Result<StreamEvent>>,
    task: JoinHandle<()>,
    transport: Arc<AtomicU8>,
}

impl SampleStream {
    /// Next event. An `Err` item is terminal: the stream ends after it.
    pub async fn next(&mut self) -> Option<Result<StreamEvent>> {
        self.rx.recv().await
    }

    pub fn transport(&self) -> StreamTransport {
        match self.transport.load(Ordering::Relaxed) {
            1 => StreamTransport::Streaming,
            2 => StreamTransport::Polling,
            _ => StreamTransport::Connecting,
        }
    }

    /// Stop the stream.
    pub fn stop(self) {}
}

impl Drop for SampleStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Client for one MTConnect agent.
#[derive(Clone)]
pub struct AgentClient {
    http: reqwest::Client,
    base: Url,
    device: Option<String>,
    timeout: Duration,
}

impl AgentClient {
    pub fn new(config: AgentConfig) -> Result<Self> {
        let mut base = Url::parse(config.base_url.trim())
            .map_err(|e| MtConnectError::InvalidConfig(format!("base URL '{}': {e}", config.base_url)))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(MtConnectError::InvalidConfig("base URL must be http or https".into()));
        }
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        base.set_query(None);
        base.set_fragment(None);
        let timeout = Duration::from_millis(config.timeout_ms.max(1));
        let http = reqwest::Client::builder()
            .connect_timeout(timeout)
            .build()
            .map_err(|e| MtConnectError::InvalidConfig(e.to_string()))?;
        Ok(Self { http, base, device: config.device.filter(|d| !d.is_empty()), timeout })
    }

    /// Client for `base_url` with default settings.
    pub fn from_url(base_url: &str) -> Result<Self> {
        Self::new(AgentConfig::new(base_url))
    }

    fn url(&self, endpoint: &str, query: &[(&str, String)]) -> Url {
        let mut rel = String::new();
        if let Some(d) = &self.device {
            rel.push_str(&percent_encode(d));
            rel.push('/');
        }
        rel.push_str(endpoint);
        // `join` of a relative reference cannot fail for a valid base.
        let mut url = self.base.join(&rel).unwrap_or_else(|_| self.base.clone());
        if !query.is_empty() {
            let q = query
                .iter()
                .map(|(k, v)| format!("{k}={}", percent_encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            url.set_query(Some(&q));
        }
        url
    }

    fn map_http(e: reqwest::Error) -> MtConnectError {
        if e.is_timeout() {
            MtConnectError::Timeout(e.to_string())
        } else {
            MtConnectError::Http(e.to_string())
        }
    }

    /// GET and parse; `MTConnectError` documents are returned as `Document::Error`.
    async fn fetch_raw(&self, endpoint: &str, query: &[(&str, String)]) -> Result<Document> {
        let resp = self
            .http
            .get(self.url(endpoint, query))
            .timeout(self.timeout)
            .send()
            .await
            .map_err(Self::map_http)?;
        let status = resp.status();
        let body = resp.text().await.map_err(Self::map_http)?;
        match parse_document(&body) {
            Ok(doc) => Ok(doc),
            Err(_) if !status.is_success() => Err(MtConnectError::Status { status: status.as_u16() }),
            Err(e) => Err(e),
        }
    }

    async fn fetch(&self, endpoint: &str, query: &[(&str, String)]) -> Result<Document> {
        match self.fetch_raw(endpoint, query).await? {
            Document::Error(e) => Err(MtConnectError::Agent(Box::new(e))),
            doc => Ok(doc),
        }
    }

    /// `GET /probe`: the device tree.
    pub async fn probe(&self) -> Result<Devices> {
        match self.fetch("probe", &[]).await? {
            Document::Devices(d) => Ok(d),
            other => Err(unexpected("MTConnectDevices", other)),
        }
    }

    /// `GET /current`: the latest observation of every data item, optionally filtered by an
    /// XPath expression (`path`).
    pub async fn current(&self, path_filter: Option<&str>) -> Result<Streams> {
        let mut q = Vec::new();
        if let Some(p) = path_filter.filter(|p| !p.is_empty()) {
            q.push(("path", p.to_string()));
        }
        self.expect_streams("current", &q).await
    }

    /// `GET /sample?from=..&count=..`.
    pub async fn sample(&self, from: u64, count: u32) -> Result<Streams> {
        self.sample_with(&SampleQuery { path: None, from: Some(from), count: Some(count) }).await
    }

    /// `GET /sample` with all parameters.
    pub async fn sample_with(&self, query: &SampleQuery) -> Result<Streams> {
        self.expect_streams("sample", &sample_params(query.path.as_deref(), query.from, query.count))
            .await
    }

    async fn expect_streams(&self, endpoint: &str, q: &[(&str, String)]) -> Result<Streams> {
        match self.fetch(endpoint, q).await? {
            Document::Streams(s) => Ok(s),
            other => Err(unexpected("MTConnectStreams", other)),
        }
    }

    /// `GET /assets`: every asset with its raw XML.
    pub async fn assets(&self) -> Result<AssetList> {
        self.assets_query(&[]).await
    }

    /// `GET /assets?type=..&count=..&removed=..`.
    pub async fn assets_filtered(
        &self,
        asset_type: Option<&str>,
        count: Option<u32>,
        include_removed: bool,
    ) -> Result<AssetList> {
        let mut q = Vec::new();
        if let Some(t) = asset_type {
            q.push(("type", t.to_string()));
        }
        if let Some(c) = count {
            q.push(("count", c.to_string()));
        }
        if include_removed {
            q.push(("removed", "true".to_string()));
        }
        self.assets_query(&q).await
    }

    async fn assets_query(&self, q: &[(&str, String)]) -> Result<AssetList> {
        match self.fetch("assets", q).await? {
            Document::Assets(a) => Ok(a),
            other => Err(unexpected("MTConnectAssets", other)),
        }
    }

    /// `GET /asset/{id}`: one asset including its raw XML.
    pub async fn asset(&self, asset_id: &str) -> Result<Asset> {
        let endpoint = format!("asset/{}", percent_encode(asset_id));
        let doc = match self.fetch(&endpoint, &[]).await? {
            Document::Assets(a) => a,
            other => return Err(unexpected("MTConnectAssets", other)),
        };
        doc.assets
            .into_iter()
            .next()
            .ok_or_else(|| MtConnectError::Xml(format!("asset '{asset_id}' not in response")))
    }

    /// Live observations every `interval`, see [`StreamOptions`]. Must be called inside a Tokio
    /// runtime.
    pub fn stream_sample(&self, interval: Duration) -> SampleStream {
        self.stream_sample_with(StreamOptions::new(interval))
    }

    /// Live observations with full control. Uses the agent's `multipart` streaming
    /// (`sample?interval=`) and falls back to polling `sample?from=nextSequence` when the agent
    /// does not stream. Handles agent restarts (`instanceId` change) and `OUT_OF_RANGE` by
    /// emitting [`StreamEvent::Resync`].
    pub fn stream_sample_with(&self, options: StreamOptions) -> SampleStream {
        let (tx, rx) = mpsc::channel(256);
        let transport = Arc::new(AtomicU8::new(0));
        let task = tokio::spawn(run_stream(self.clone(), options, tx, transport.clone()));
        SampleStream { rx, task, transport }
    }
}

fn sample_params(path: Option<&str>, from: Option<u64>, count: Option<u32>) -> Vec<(&'static str, String)> {
    let mut q = Vec::new();
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        q.push(("path", p.to_string()));
    }
    if let Some(f) = from {
        q.push(("from", f.to_string()));
    }
    if let Some(c) = count {
        q.push(("count", c.to_string()));
    }
    q
}

// ── Streaming task ─────────────────────────────────────────────────────────

const STREAM_CONNECT_ATTEMPTS: u32 = 3;
const T_STREAMING: u8 = 1;
const T_POLLING: u8 = 2;

struct StreamState {
    next: Option<u64>,
    instance: Option<u64>,
}

enum Flow {
    Continue,
    /// Sequence position changed: restart the streaming request from `state.next`.
    Reconnect,
    Stop,
}

enum StreamOutcome {
    /// The connection ended (or never produced data); `delivered` is true if documents arrived.
    Closed { delivered: bool },
    Unsupported,
    Stopped,
}

struct Ctx<'a> {
    client: &'a AgentClient,
    opts: &'a StreamOptions,
    tx: &'a mpsc::Sender<Result<StreamEvent>>,
}

impl Ctx<'_> {
    async fn send(&self, item: Result<StreamEvent>) -> bool {
        self.tx.send(item).await.is_ok()
    }

    async fn resync(&self, state: &mut StreamState, reason: ResyncReason) -> Flow {
        match self.client.current(self.opts.path.as_deref()).await {
            Ok(snapshot) => {
                state.instance = Some(snapshot.header.instance_id);
                state.next = snapshot.header.next_sequence;
                if self.send(Ok(StreamEvent::Resync { reason, snapshot })).await {
                    Flow::Reconnect
                } else {
                    Flow::Stop
                }
            }
            Err(e) => {
                self.send(Err(e)).await;
                Flow::Stop
            }
        }
    }

    async fn handle(&self, state: &mut StreamState, doc: Document) -> Flow {
        match doc {
            Document::Error(e) => {
                let instance_changed = e.header.instance_id != 0
                    && state.instance.is_some_and(|i| i != e.header.instance_id);
                if instance_changed {
                    let previous = state.instance.unwrap_or(0);
                    self.resync(state, ResyncReason::InstanceChanged { previous, current: e.header.instance_id })
                        .await
                } else if e.has_code(&AgentErrorCode::OutOfRange) {
                    self.resync(state, ResyncReason::OutOfRange).await
                } else {
                    self.send(Err(MtConnectError::Agent(Box::new(e)))).await;
                    Flow::Stop
                }
            }
            Document::Streams(s) => {
                if let Some(previous) = state.instance.filter(|i| *i != s.header.instance_id) {
                    return self
                        .resync(
                            state,
                            ResyncReason::InstanceChanged { previous, current: s.header.instance_id },
                        )
                        .await;
                }
                state.instance = Some(s.header.instance_id);
                if let Some(n) = s.header.next_sequence {
                    state.next = Some(n);
                }
                if !s.is_empty() && !self.send(Ok(StreamEvent::Observations(s))).await {
                    return Flow::Stop;
                }
                Flow::Continue
            }
            other => {
                self.send(Err(unexpected("MTConnectStreams", other))).await;
                Flow::Stop
            }
        }
    }

    async fn stream_once(&self, state: &mut StreamState, transport: &AtomicU8) -> StreamOutcome {
        let mut q = sample_params(self.opts.path.as_deref(), state.next, self.opts.count);
        q.push(("interval", self.opts.interval.as_millis().max(1).to_string()));
        let url = self.client.url("sample", &q);
        let Ok(mut resp) = self.client.http.get(url).send().await else {
            return StreamOutcome::Closed { delivered: false };
        };
        let boundary = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(boundary_from_content_type);
        let Some(boundary) = boundary else {
            // Either an error document (e.g. OUT_OF_RANGE) or an agent that ignores `interval`.
            return match resp.text().await.ok().and_then(|b| parse_document(&b).ok()) {
                Some(doc @ Document::Error(_)) => match self.handle(state, doc).await {
                    Flow::Continue | Flow::Reconnect => StreamOutcome::Closed { delivered: true },
                    Flow::Stop => StreamOutcome::Stopped,
                },
                _ => StreamOutcome::Unsupported,
            };
        };

        transport.store(T_STREAMING, Ordering::Relaxed);
        let mut parser = MultipartParser::new(&boundary);
        let mut delivered = false;
        loop {
            let chunk = match tokio::time::timeout(self.opts.stall_timeout, resp.chunk()).await {
                Ok(Ok(Some(c))) => c,
                _ => return StreamOutcome::Closed { delivered },
            };
            parser.push(&chunk);
            while let Some(part) = parser.next_part() {
                delivered = true;
                let doc = match String::from_utf8(part)
                    .map_err(|e| MtConnectError::Xml(e.to_string()))
                    .and_then(|t| parse_document(&t))
                {
                    Ok(d) => d,
                    Err(e) => {
                        self.send(Err(e)).await;
                        return StreamOutcome::Stopped;
                    }
                };
                match self.handle(state, doc).await {
                    Flow::Continue => {}
                    Flow::Reconnect => return StreamOutcome::Closed { delivered },
                    Flow::Stop => return StreamOutcome::Stopped,
                }
            }
            if parser.is_finished() {
                return StreamOutcome::Closed { delivered };
            }
        }
    }

    /// One polling round. Returns `false` when the stream should end.
    async fn poll_once(&self, state: &mut StreamState) -> bool {
        let q = sample_params(self.opts.path.as_deref(), state.next, self.opts.count);
        let doc = match self.client.fetch_raw("sample", &q).await {
            Ok(d) => d,
            Err(e) => {
                self.send(Err(e)).await;
                return false;
            }
        };
        let more = matches!(&doc, Document::Streams(s)
            if matches!((s.header.last_sequence, s.header.next_sequence),
                (Some(last), Some(next)) if last >= next));
        match self.handle(state, doc).await {
            Flow::Stop => return false,
            Flow::Reconnect => {}
            Flow::Continue if more => return true,
            Flow::Continue => {}
        }
        tokio::time::sleep(self.opts.interval).await;
        true
    }
}

async fn run_stream(
    client: AgentClient,
    opts: StreamOptions,
    tx: mpsc::Sender<Result<StreamEvent>>,
    transport: Arc<AtomicU8>,
) {
    let ctx = Ctx { client: &client, opts: &opts, tx: &tx };
    let mut state = StreamState { next: opts.from, instance: None };

    if opts.from.is_none() {
        match client.current(opts.path.as_deref()).await {
            Ok(snapshot) => {
                state.instance = Some(snapshot.header.instance_id);
                state.next = snapshot.header.next_sequence;
                if !ctx.send(Ok(StreamEvent::Snapshot(snapshot))).await {
                    return;
                }
            }
            Err(e) => {
                ctx.send(Err(e)).await;
                return;
            }
        }
    }

    let mut streaming = opts.prefer_streaming;
    let mut failures = 0u32;
    loop {
        if tx.is_closed() {
            return;
        }
        if streaming {
            match ctx.stream_once(&mut state, &transport).await {
                StreamOutcome::Stopped => return,
                StreamOutcome::Unsupported => streaming = false,
                StreamOutcome::Closed { delivered } => {
                    failures = if delivered { 0 } else { failures + 1 };
                    if failures >= STREAM_CONNECT_ATTEMPTS {
                        streaming = false;
                    } else {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
            if !streaming {
                transport.store(T_POLLING, Ordering::Relaxed);
            }
        } else {
            transport.store(T_POLLING, Ordering::Relaxed);
            if !ctx.poll_once(&mut state).await {
                return;
            }
        }
    }
}
