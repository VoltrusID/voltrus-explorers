//! HSMS (SEMI E37) over TCP: framing, control messages, timers and the connection.
//!
//! A frame is a 4-byte big-endian length (counting the header and the text, not itself), a
//! 10-byte header and the message text. The header is `session id u16 | byte 2 | byte 3 |
//! PType | SType | system bytes u32`. For a data message (SType 0) byte 2 is the W-bit and the
//! stream and byte 3 the function; for control messages the two bytes carry a status or reason.
//!
//! [`HsmsConnection`] runs one TCP connection: it answers the control messages itself (Select,
//! Deselect, Linktest, Separate), matches replies to the primary messages sent with
//! [`HsmsConnection::request`], enforces the timers T3, T6, T7 and T8, and hands every primary
//! data message to the owner through an [`Incoming`] channel. T5 is the minimum time between
//! connection attempts, enforced by [`Connector`].

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{Instant, sleep, timeout};

use crate::error::{
    CodecError, RejectReason, Result, S9Error, S9Kind, SecsError, SelectStatus, Timer,
};
use crate::item::Item;
use crate::message::SecsMessage;

/// Port the simulator listens on by default. HSMS itself has no fixed port.
pub const DEFAULT_PORT: u16 = 5000;

/// Session id of control messages that belong to no device (Linktest).
pub const NO_DEVICE: u16 = 0xFFFF;

pub const HEADER_LEN: usize = 10;

/// Timers and limits of one HSMS connection. The `t*_ms` defaults are the E37 defaults:
/// T3 45 s, T5 10 s, T6 5 s, T7 10 s, T8 5 s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HsmsConfig {
    /// Device id (session id) of data messages.
    pub device_id: u16,
    /// T3, reply timeout.
    pub t3_ms: u64,
    /// T5, connect separation timeout.
    pub t5_ms: u64,
    /// T6, control transaction timeout.
    pub t6_ms: u64,
    /// T7, not-selected timeout.
    pub t7_ms: u64,
    /// T8, network inter-character timeout.
    pub t8_ms: u64,
    /// Send a Linktest.req this often while selected; `None` sends none.
    pub linktest_interval_ms: Option<u64>,
    /// TCP connect timeout of an active connection.
    pub connect_timeout_ms: u64,
    /// Largest message (header + text) accepted from the peer.
    pub max_message_len: u32,
}

impl Default for HsmsConfig {
    fn default() -> Self {
        HsmsConfig {
            device_id: 0,
            t3_ms: 45_000,
            t5_ms: 10_000,
            t6_ms: 5_000,
            t7_ms: 10_000,
            t8_ms: 5_000,
            linktest_interval_ms: None,
            connect_timeout_ms: 10_000,
            max_message_len: 16 * 1024 * 1024,
        }
    }
}

impl HsmsConfig {
    pub fn t3(&self) -> Duration {
        Duration::from_millis(self.t3_ms)
    }
    pub fn t5(&self) -> Duration {
        Duration::from_millis(self.t5_ms)
    }
    pub fn t6(&self) -> Duration {
        Duration::from_millis(self.t6_ms)
    }
    pub fn t7(&self) -> Duration {
        Duration::from_millis(self.t7_ms)
    }
    pub fn t8(&self) -> Duration {
        Duration::from_millis(self.t8_ms)
    }
}

/// SType, header byte 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SType {
    Data,
    SelectReq,
    SelectRsp,
    DeselectReq,
    DeselectRsp,
    LinktestReq,
    LinktestRsp,
    RejectReq,
    SeparateReq,
}

impl SType {
    pub fn from_code(code: u8) -> Option<SType> {
        Some(match code {
            0 => SType::Data,
            1 => SType::SelectReq,
            2 => SType::SelectRsp,
            3 => SType::DeselectReq,
            4 => SType::DeselectRsp,
            5 => SType::LinktestReq,
            6 => SType::LinktestRsp,
            7 => SType::RejectReq,
            9 => SType::SeparateReq,
            _ => return None,
        })
    }

    pub fn code(self) -> u8 {
        match self {
            SType::Data => 0,
            SType::SelectReq => 1,
            SType::SelectRsp => 2,
            SType::DeselectReq => 3,
            SType::DeselectRsp => 4,
            SType::LinktestReq => 5,
            SType::LinktestRsp => 6,
            SType::RejectReq => 7,
            SType::SeparateReq => 9,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SType::Data => "Data",
            SType::SelectReq => "Select.req",
            SType::SelectRsp => "Select.rsp",
            SType::DeselectReq => "Deselect.req",
            SType::DeselectRsp => "Deselect.rsp",
            SType::LinktestReq => "Linktest.req",
            SType::LinktestRsp => "Linktest.rsp",
            SType::RejectReq => "Reject.req",
            SType::SeparateReq => "Separate.req",
        }
    }
}

/// The 10-byte HSMS header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub session_id: u16,
    pub byte2: u8,
    pub byte3: u8,
    pub ptype: u8,
    pub stype: u8,
    pub system: u32,
}

impl Header {
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..2].copy_from_slice(&self.session_id.to_be_bytes());
        b[2] = self.byte2;
        b[3] = self.byte3;
        b[4] = self.ptype;
        b[5] = self.stype;
        b[6..10].copy_from_slice(&self.system.to_be_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; HEADER_LEN]) -> Header {
        Header {
            session_id: u16::from_be_bytes([b[0], b[1]]),
            byte2: b[2],
            byte3: b[3],
            ptype: b[4],
            stype: b[5],
            system: u32::from_be_bytes([b[6], b[7], b[8], b[9]]),
        }
    }

    /// A control message header.
    pub fn control(stype: SType, session_id: u16, byte2: u8, byte3: u8, system: u32) -> Header {
        Header {
            session_id,
            byte2,
            byte3,
            ptype: 0,
            stype: stype.code(),
            system,
        }
    }

    pub fn stype_kind(&self) -> Option<SType> {
        SType::from_code(self.stype)
    }
}

/// One HSMS message: header and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: Header,
    pub text: Vec<u8>,
}

impl Frame {
    /// The bytes on the wire, length prefix included.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + HEADER_LEN + self.text.len());
        out.extend_from_slice(&((HEADER_LEN + self.text.len()) as u32).to_be_bytes());
        out.extend_from_slice(&self.header.to_bytes());
        out.extend_from_slice(&self.text);
        out
    }

    /// Decode one frame from a complete buffer (length prefix included).
    pub fn decode(bytes: &[u8]) -> Result<Frame, SecsError> {
        let proto = |m: &str| SecsError::Protocol(m.to_string());
        let len = bytes
            .get(0..4)
            .ok_or_else(|| proto("frame shorter than its length prefix"))?;
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        if len < HEADER_LEN {
            return Err(proto("message length below the 10-byte header"));
        }
        if bytes.len() != 4 + len {
            return Err(proto("frame length does not match its length prefix"));
        }
        let head: [u8; HEADER_LEN] = bytes[4..4 + HEADER_LEN].try_into().expect("10 bytes");
        Ok(Frame {
            header: Header::from_bytes(&head),
            text: bytes[4 + HEADER_LEN..].to_vec(),
        })
    }

    /// A data message frame.
    pub fn data(device_id: u16, message: &SecsMessage, system: u32) -> Result<Frame, CodecError> {
        Ok(Frame {
            header: Header {
                session_id: device_id,
                byte2: message.byte2(),
                byte3: message.function,
                ptype: 0,
                stype: 0,
                system,
            },
            text: message.encode_text()?,
        })
    }
}

/// Fill `buf`. With `idle_ok` the wait for the first byte has no limit (an idle line); every
/// other read must deliver within `t8`, which is how T8 (time between characters) is enforced on
/// a stream socket.
async fn read_full<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut [u8],
    t8: Duration,
    idle_ok: bool,
) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = r.read(&mut buf[filled..]);
        let n = if idle_ok && filled == 0 {
            read.await
        } else {
            timeout(t8, read)
                .await
                .map_err(|_| SecsError::Timeout(Timer::T8))?
        }
        .map_err(|e| SecsError::Io(e.to_string()))?;
        if n == 0 {
            return if idle_ok && filled == 0 {
                Err(SecsError::Closed)
            } else {
                Err(SecsError::Io("connection closed inside a message".into()))
            };
        }
        filled += n;
    }
    Ok(())
}

/// Read one frame; waits indefinitely for its first byte, then applies T8.
pub async fn read_frame<R: AsyncRead + Unpin>(
    r: &mut R,
    t8: Duration,
    max_len: u32,
) -> Result<Frame> {
    let mut len = [0u8; 4];
    read_full(r, &mut len, t8, true).await?;
    let len = u32::from_be_bytes(len);
    if (len as usize) < HEADER_LEN {
        return Err(SecsError::Protocol(format!(
            "message length {len} is below the 10-byte header"
        )));
    }
    if len > max_len {
        return Err(SecsError::Protocol(format!(
            "message length {len} exceeds the limit of {max_len}"
        )));
    }
    let mut head = [0u8; HEADER_LEN];
    read_full(r, &mut head, t8, false).await?;
    let mut text = vec![0u8; len as usize - HEADER_LEN];
    read_full(r, &mut text, t8, false).await?;
    Ok(Frame {
        header: Header::from_bytes(&head),
        text,
    })
}

/// Write one frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> Result<()> {
    w.write_all(&frame.encode())
        .await
        .map_err(|e| SecsError::Io(e.to_string()))
}

/// State of the HSMS session on one TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HsmsState {
    /// TCP is up, no Select yet (T7 is running).
    NotSelected,
    /// Data messages may flow.
    Selected,
    /// The connection is gone.
    Closed,
}

/// Which way a logged message went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Sent by us.
    Tx,
    /// Received from the peer.
    Rx,
}

/// One entry of the message log stream ([`HsmsConnection::subscribe_log`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unix time in milliseconds.
    pub time_ms: u64,
    pub direction: Direction,
    pub session_id: u16,
    pub system: u32,
    /// `S1F13` for data messages, `Select.req` and so on for control messages.
    pub kind: String,
    /// One-line SML of a data message; the status or reason of a control message.
    pub text: String,
}

impl LogEntry {
    /// One line for a terminal: time, direction, system bytes and the message.
    pub fn line(&self) -> String {
        let arrow = match self.direction {
            Direction::Tx => "->",
            Direction::Rx => "<-",
        };
        let what = if self.text.is_empty() {
            self.kind.clone()
        } else if self.text.starts_with(&self.kind) {
            self.text.clone()
        } else {
            format!("{} {}", self.kind, self.text)
        };
        format!("{} {arrow} [{:#010x}] {what}", self.time_ms, self.system)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A primary data message (or an undecodable one) received from the peer.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub header: Header,
    /// The decoded message, or why its text is not valid SECS-II (answer with S9F7).
    pub message: std::result::Result<SecsMessage, CodecError>,
}

impl Incoming {
    pub fn system(&self) -> u32 {
        self.header.system
    }

    pub fn session_id(&self) -> u16 {
        self.header.session_id
    }

    pub fn stream(&self) -> u8 {
        self.header.byte2 & 0x7f
    }

    pub fn function(&self) -> u8 {
        self.header.byte3
    }

    pub fn w_bit(&self) -> bool {
        self.header.byte2 & 0x80 != 0
    }
}

enum WriteCmd {
    Frame(Vec<u8>),
    Shutdown,
}

type Reply = Result<Frame>;

struct Inner {
    config: HsmsConfig,
    tx: mpsc::UnboundedSender<WriteCmd>,
    pending: Mutex<HashMap<u32, oneshot::Sender<Reply>>>,
    state: watch::Sender<HsmsState>,
    next_system: AtomicU32,
    log: broadcast::Sender<LogEntry>,
    peer: Option<SocketAddr>,
    local: Option<SocketAddr>,
    close_reason: Mutex<Option<SecsError>>,
}

impl Inner {
    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<u32, oneshot::Sender<Reply>>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn state(&self) -> HsmsState {
        *self.state.borrow()
    }

    fn next_system(&self) -> u32 {
        self.next_system.fetch_add(1, Ordering::Relaxed)
    }

    fn complete(&self, system: u32, reply: Reply) -> bool {
        match self.pending().remove(&system) {
            Some(tx) => {
                let _ = tx.send(reply);
                true
            }
            None => false,
        }
    }

    /// Mark the connection closed (once), fail every open transaction and stop the writer.
    fn shutdown(&self, reason: Option<SecsError>) {
        let was = self.state.send_replace(HsmsState::Closed);
        if was == HsmsState::Closed {
            return;
        }
        if let Some(r) = &reason {
            tracing::debug!("HSMS connection closed: {r}");
        }
        *self.close_reason.lock().unwrap_or_else(|p| p.into_inner()) = reason.clone();
        let drained: Vec<_> = self.pending().drain().collect();
        for (_, tx) in drained {
            let _ = tx.send(Err(reason.clone().unwrap_or(SecsError::Closed)));
        }
        let _ = self.tx.send(WriteCmd::Shutdown);
    }

    fn write(&self, frame: &Frame) -> Result<()> {
        if self.state() == HsmsState::Closed {
            return Err(SecsError::NotConnected);
        }
        self.tx
            .send(WriteCmd::Frame(frame.encode()))
            .map_err(|_| SecsError::NotConnected)
    }

    fn log_entry(&self, entry: impl FnOnce() -> LogEntry) {
        if self.log.receiver_count() > 0 {
            let _ = self.log.send(entry());
        }
    }

    fn log_control(&self, direction: Direction, h: &Header) {
        self.log_entry(|| {
            let kind = h
                .stype_kind()
                .map_or_else(|| format!("SType {}", h.stype), |s| s.name().to_string());
            let text = match h.stype_kind() {
                Some(SType::SelectRsp) => format!("status {}", h.byte3),
                Some(SType::DeselectRsp) => format!("status {}", h.byte3),
                Some(SType::RejectReq) => format!("reason {}", h.byte3),
                _ => String::new(),
            };
            LogEntry {
                time_ms: now_ms(),
                direction,
                session_id: h.session_id,
                system: h.system,
                kind,
                text,
            }
        });
    }

    fn log_data(
        &self,
        direction: Direction,
        h: &Header,
        msg: std::result::Result<&SecsMessage, &CodecError>,
    ) {
        self.log_entry(|| {
            let (kind, text) = match msg {
                Ok(m) => (m.name(), m.to_sml()),
                Err(e) => (
                    format!("S{}F{}", h.byte2 & 0x7f, h.byte3),
                    format!("undecodable text: {e}"),
                ),
            };
            LogEntry {
                time_ms: now_ms(),
                direction,
                session_id: h.session_id,
                system: h.system,
                kind,
                text,
            }
        });
    }

    fn send_control(&self, header: Header) -> Result<()> {
        self.log_control(Direction::Tx, &header);
        self.write(&Frame {
            header,
            text: Vec::new(),
        })
    }

    /// Run one control transaction. A T6 timeout closes the connection: the peer is not
    /// answering at the HSMS level.
    async fn control(&self, stype: SType, session_id: u16, status: u8) -> Result<Frame> {
        let system = self.next_system();
        let (tx, rx) = oneshot::channel();
        self.pending().insert(system, tx);
        let _guard = PendingGuard {
            inner: self,
            system,
        };
        if self.state() == HsmsState::Closed {
            return Err(SecsError::NotConnected);
        }
        self.send_control(Header::control(stype, session_id, 0, status, system))?;
        match timeout(self.config.t6(), rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(SecsError::Closed),
            Err(_) => {
                self.shutdown(Some(SecsError::Timeout(Timer::T6)));
                Err(SecsError::Timeout(Timer::T6))
            }
        }
    }

    fn reject(&self, rejected: &Header, byte2: u8, reason: RejectReason) {
        let h = Header::control(
            SType::RejectReq,
            rejected.session_id,
            byte2,
            reason.code(),
            rejected.system,
        );
        let _ = self.send_control(h);
    }
}

/// Removes an open transaction when its future is dropped before it finished.
struct PendingGuard<'a> {
    inner: &'a Inner,
    system: u32,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.inner.pending().remove(&self.system);
    }
}

/// Closes the connection when the last handle is dropped.
struct DropGuard(Arc<Inner>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        close_inner(&self.0);
    }
}

fn close_inner(inner: &Inner) {
    if inner.state() == HsmsState::Selected {
        let h = Header::control(SType::SeparateReq, NO_DEVICE, 0, 0, inner.next_system());
        let _ = inner.send_control(h);
    }
    inner.shutdown(None);
}

/// A handle to one HSMS connection that does not keep it open: background tasks use it to send
/// and receive while the owner holds the [`HsmsConnection`].
#[derive(Clone)]
pub struct HsmsLink {
    inner: Arc<Inner>,
}

/// The owner's handle to one HSMS connection. Clones share the connection; it closes (with a
/// Separate.req when selected) when the last clone is dropped or [`HsmsLink::close`] is called.
/// Everything the connection can do is on [`HsmsLink`], which this derefs to.
#[derive(Clone)]
pub struct HsmsConnection {
    link: HsmsLink,
    _guard: Arc<DropGuard>,
}

impl std::ops::Deref for HsmsConnection {
    type Target = HsmsLink;

    fn deref(&self) -> &HsmsLink {
        &self.link
    }
}

/// Receiver of the primary data messages of one connection. It ends when the connection closes.
pub type IncomingRx = mpsc::UnboundedReceiver<Incoming>;

impl HsmsConnection {
    /// Connect (active mode) and perform the Select procedure.
    pub async fn connect(
        addr: impl ToSocketAddrs,
        config: HsmsConfig,
    ) -> Result<(HsmsConnection, IncomingRx)> {
        Self::connect_with_log(addr, config, None).await
    }

    /// [`connect`](HsmsConnection::connect) with an existing log channel.
    pub async fn connect_with_log(
        addr: impl ToSocketAddrs,
        config: HsmsConfig,
        log: Option<broadcast::Sender<LogEntry>>,
    ) -> Result<(HsmsConnection, IncomingRx)> {
        let (conn, rx) = Self::connect_unselected_with_log(addr, config, log).await?;
        conn.select().await?;
        Ok((conn, rx))
    }

    /// Connect (active mode) without selecting; T7 closes the connection if [`select`] does not
    /// follow in time.
    ///
    /// [`select`]: HsmsConnection::select
    pub async fn connect_unselected(
        addr: impl ToSocketAddrs,
        config: HsmsConfig,
    ) -> Result<(HsmsConnection, IncomingRx)> {
        Self::connect_unselected_with_log(addr, config, None).await
    }

    /// [`connect_unselected`](HsmsConnection::connect_unselected) with an existing log channel.
    pub async fn connect_unselected_with_log(
        addr: impl ToSocketAddrs,
        config: HsmsConfig,
        log: Option<broadcast::Sender<LogEntry>>,
    ) -> Result<(HsmsConnection, IncomingRx)> {
        let limit = Duration::from_millis(config.connect_timeout_ms);
        let stream = timeout(limit, TcpStream::connect(addr))
            .await
            .map_err(|_| SecsError::ConnectionFailed("timed out".into()))?
            .map_err(|e| SecsError::ConnectionFailed(e.to_string()))?;
        Ok(Self::start_with_log(stream, config, log))
    }

    /// Wrap an accepted or connected TCP stream. The session starts not selected.
    pub fn start(stream: TcpStream, config: HsmsConfig) -> (HsmsConnection, IncomingRx) {
        Self::start_with_log(stream, config, None)
    }

    /// Like [`start`](HsmsConnection::start), logging into an existing channel so a log
    /// subscriber sees the very first message of the connection.
    pub fn start_with_log(
        stream: TcpStream,
        config: HsmsConfig,
        log: Option<broadcast::Sender<LogEntry>>,
    ) -> (HsmsConnection, IncomingRx) {
        stream.set_nodelay(true).ok();
        let peer = stream.peer_addr().ok();
        let local = stream.local_addr().ok();
        Self::spawn(stream, config, log, peer, local)
    }

    /// Run HSMS over any byte stream (an in-memory pipe in tests, a TLS stream). The session
    /// starts not selected; [`HsmsConnection::peer_addr`] is `None`.
    pub fn start_io<S>(stream: S, config: HsmsConfig) -> (HsmsConnection, IncomingRx)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        Self::spawn(stream, config, None, None, None)
    }

    fn spawn<S>(
        stream: S,
        config: HsmsConfig,
        log: Option<broadcast::Sender<LogEntry>>,
        peer: Option<SocketAddr>,
        local: Option<SocketAddr>,
    ) -> (HsmsConnection, IncomingRx)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (read, write) = tokio::io::split(stream);
        let (tx, wrx) = mpsc::unbounded_channel();
        let (itx, irx) = mpsc::unbounded_channel();
        let (state, _) = watch::channel(HsmsState::NotSelected);
        let log = log.unwrap_or_else(|| broadcast::channel(512).0);
        let inner = Arc::new(Inner {
            config,
            tx,
            pending: Mutex::new(HashMap::new()),
            state,
            next_system: AtomicU32::new(1),
            log,
            peer,
            local,
            close_reason: Mutex::new(None),
        });
        tokio::spawn(writer_task(write, wrx, Arc::clone(&inner)));
        tokio::spawn(reader_task(read, itx, Arc::clone(&inner)));
        tokio::spawn(t7_task(Arc::clone(&inner)));
        if let Some(ms) = inner.config.linktest_interval_ms {
            tokio::spawn(linktest_task(Arc::clone(&inner), Duration::from_millis(ms)));
        }
        let conn = HsmsConnection {
            _guard: Arc::new(DropGuard(Arc::clone(&inner))),
            link: HsmsLink { inner },
        };
        (conn, irx)
    }

    /// A handle that sends and receives but does not keep the connection open.
    pub fn link(&self) -> HsmsLink {
        self.link.clone()
    }
}

impl HsmsLink {
    /// Whether both handles are the same connection.
    pub fn same_connection(&self, other: &HsmsLink) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn config(&self) -> &HsmsConfig {
        &self.inner.config
    }

    pub fn state(&self) -> HsmsState {
        self.inner.state()
    }

    pub fn is_selected(&self) -> bool {
        self.state() == HsmsState::Selected
    }

    /// Follow state changes.
    pub fn watch_state(&self) -> watch::Receiver<HsmsState> {
        self.inner.state.subscribe()
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.inner.peer
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.inner.local
    }

    /// Why the connection closed (`None` for a clean close or while open).
    pub fn close_reason(&self) -> Option<SecsError> {
        self.inner
            .close_reason
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Every message sent and received, control messages included. Only entries made while a
    /// receiver exists are produced.
    pub fn subscribe_log(&self) -> broadcast::Receiver<LogEntry> {
        self.inner.log.subscribe()
    }

    /// Resolves when the connection is closed.
    pub async fn closed(&self) {
        let mut rx = self.watch_state();
        let _ = rx.wait_for(|s| *s == HsmsState::Closed).await;
    }

    /// Send Separate.req (when selected) and close the TCP connection.
    pub fn close(&self) {
        close_inner(&self.inner);
    }

    async fn control(&self, stype: SType, session_id: u16, status: u8) -> Result<Frame> {
        self.inner.control(stype, session_id, status).await
    }

    /// Select.req (active side). Moves the session to selected on status 0.
    pub async fn select(&self) -> Result<()> {
        let rsp = self
            .control(SType::SelectReq, self.inner.config.device_id, 0)
            .await?;
        // The reader task already moved the session to selected when the Select.rsp arrived, so
        // data the peer sends right behind it is accepted.
        match SelectStatus::from_code(rsp.header.byte3) {
            SelectStatus::Ok => Ok(()),
            other => Err(SecsError::SelectRefused(other)),
        }
    }

    /// Deselect.req: back to not-selected without closing TCP (T7 then runs).
    pub async fn deselect(&self) -> Result<()> {
        let rsp = self
            .control(SType::DeselectReq, self.inner.config.device_id, 0)
            .await?;
        if rsp.header.byte3 != 0 {
            return Err(SecsError::DeselectRefused(rsp.header.byte3));
        }
        Ok(())
    }

    /// Linktest.req/rsp round trip.
    pub async fn linktest(&self) -> Result<()> {
        self.control(SType::LinktestReq, NO_DEVICE, 0)
            .await
            .map(|_| ())
    }

    /// Send a primary message with the W-bit and wait up to T3 for its secondary message.
    ///
    /// An `S{n}F0` answer is [`SecsError::Aborted`]; an S9Fx the peer sends about this message
    /// is [`SecsError::S9`]; a Reject.req is [`SecsError::Rejected`].
    pub async fn request(&self, message: SecsMessage) -> Result<SecsMessage> {
        let inner = &*self.inner;
        if inner.state() != HsmsState::Selected {
            return Err(if inner.state() == HsmsState::Closed {
                SecsError::NotConnected
            } else {
                SecsError::NotSelected
            });
        }
        let mut message = message;
        message.w_bit = true;
        let system = inner.next_system();
        let frame = Frame::data(inner.config.device_id, &message, system)?;
        let (tx, rx) = oneshot::channel();
        inner.pending().insert(system, tx);
        let _guard = PendingGuard { inner, system };
        inner.log_data(Direction::Tx, &frame.header, Ok(&message));
        inner.write(&frame)?;
        let reply = match timeout(inner.config.t3(), rx).await {
            Ok(Ok(reply)) => reply?,
            Ok(Err(_)) => return Err(SecsError::Closed),
            Err(_) => return Err(SecsError::Timeout(Timer::T3)),
        };
        let msg = SecsMessage::from_parts(reply.header.byte2, reply.header.byte3, &reply.text)?;
        if msg.function == 0 {
            return Err(SecsError::Aborted { stream: msg.stream });
        }
        Ok(msg)
    }

    /// Send a message that wants no reply (W-bit clear) and return its system bytes.
    pub fn send(&self, message: SecsMessage) -> Result<u32> {
        let mut message = message;
        message.w_bit = false;
        let system = self.inner.next_system();
        self.send_with_system(&message, system)?;
        Ok(system)
    }

    /// Answer a primary message: same system bytes, no W-bit.
    pub fn reply(&self, to: &Incoming, message: SecsMessage) -> Result<()> {
        let mut message = message;
        message.w_bit = false;
        self.send_with_system(&message, to.system())
    }

    /// Answer with the `S{n}F0` abort.
    pub fn reply_abort(&self, to: &Incoming) -> Result<()> {
        self.reply(to, SecsMessage::notify(to.stream(), 0, None))
    }

    /// Send the S9 error message `kind` about `offending` (its header is the message text).
    pub fn send_s9(&self, kind: S9Kind, offending: &Header) -> Result<u32> {
        self.send(SecsMessage::notify(
            9,
            kind.function(),
            Some(Item::Binary(offending.to_bytes().to_vec())),
        ))
    }

    fn send_with_system(&self, message: &SecsMessage, system: u32) -> Result<()> {
        let inner = &*self.inner;
        if inner.state() != HsmsState::Selected {
            return Err(if inner.state() == HsmsState::Closed {
                SecsError::NotConnected
            } else {
                SecsError::NotSelected
            });
        }
        let frame = Frame::data(inner.config.device_id, message, system)?;
        inner.log_data(Direction::Tx, &frame.header, Ok(message));
        inner.write(&frame)
    }
}

async fn writer_task<W: AsyncWrite + Unpin>(
    mut write: W,
    mut rx: mpsc::UnboundedReceiver<WriteCmd>,
    inner: Arc<Inner>,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            WriteCmd::Frame(bytes) => {
                if let Err(e) = write.write_all(&bytes).await {
                    inner.shutdown(Some(SecsError::Io(e.to_string())));
                    break;
                }
            }
            WriteCmd::Shutdown => break,
        }
    }
    let _ = write.shutdown().await;
}

async fn reader_task<R: AsyncRead + Unpin>(
    mut read: R,
    incoming: mpsc::UnboundedSender<Incoming>,
    inner: Arc<Inner>,
) {
    let mut state = inner.state.subscribe();
    loop {
        let frame = tokio::select! {
            f = read_frame(&mut read, inner.config.t8(), inner.config.max_message_len) => f,
            _ = state.wait_for(|s| *s == HsmsState::Closed) => break,
        };
        match frame {
            Ok(frame) => handle_frame(&inner, &incoming, frame),
            Err(SecsError::Closed) => {
                inner.shutdown(None);
                break;
            }
            Err(e) => {
                inner.shutdown(Some(e));
                break;
            }
        }
    }
}

fn handle_frame(inner: &Inner, incoming: &mpsc::UnboundedSender<Incoming>, frame: Frame) {
    let h = frame.header;
    if h.ptype != 0 {
        inner.log_control(Direction::Rx, &h);
        inner.reject(&h, h.ptype, RejectReason::PTypeNotSupported);
        return;
    }
    let Some(stype) = h.stype_kind() else {
        inner.log_control(Direction::Rx, &h);
        inner.reject(&h, h.stype, RejectReason::STypeNotSupported);
        return;
    };
    if stype != SType::Data {
        inner.log_control(Direction::Rx, &h);
    }
    match stype {
        SType::Data => handle_data(inner, incoming, frame),
        SType::SelectReq => {
            let status = if inner.state() == HsmsState::Selected {
                SelectStatus::AlreadyActive
            } else {
                inner.state.send_replace(HsmsState::Selected);
                SelectStatus::Ok
            };
            let _ = inner.send_control(Header::control(
                SType::SelectRsp,
                h.session_id,
                0,
                status.code(),
                h.system,
            ));
        }
        SType::DeselectReq => {
            let status = if inner.state() == HsmsState::Selected {
                inner.state.send_replace(HsmsState::NotSelected);
                0
            } else {
                1
            };
            let _ = inner.send_control(Header::control(
                SType::DeselectRsp,
                h.session_id,
                0,
                status,
                h.system,
            ));
        }
        SType::LinktestReq => {
            let _ = inner.send_control(Header::control(
                SType::LinktestRsp,
                NO_DEVICE,
                0,
                0,
                h.system,
            ));
        }
        SType::SelectRsp | SType::DeselectRsp | SType::LinktestRsp => {
            // The state changes here, in the reader, before any data frame behind this one is
            // looked at; the caller waiting on the transaction only sees the result.
            let awaited = inner.pending().contains_key(&h.system);
            if awaited && h.byte3 == 0 {
                match stype {
                    SType::SelectRsp => {
                        inner.state.send_if_modified(|s| {
                            let change = *s == HsmsState::NotSelected;
                            if change {
                                *s = HsmsState::Selected;
                            }
                            change
                        });
                    }
                    SType::DeselectRsp => {
                        inner.state.send_if_modified(|s| {
                            let change = *s == HsmsState::Selected;
                            if change {
                                *s = HsmsState::NotSelected;
                            }
                            change
                        });
                    }
                    _ => {}
                }
            }
            inner.complete(h.system, Ok(frame));
        }
        SType::RejectReq => {
            let reason = RejectReason::from_code(h.byte3);
            inner.complete(
                h.system,
                Err(SecsError::Rejected {
                    reason,
                    code: h.byte3,
                }),
            );
        }
        SType::SeparateReq => inner.shutdown(None),
    }
}

fn handle_data(inner: &Inner, incoming: &mpsc::UnboundedSender<Incoming>, frame: Frame) {
    let h = frame.header;
    let message = SecsMessage::from_parts(h.byte2, h.byte3, &frame.text);
    inner.log_data(Direction::Rx, &h, message.as_ref());
    if inner.state() != HsmsState::Selected {
        inner.reject(&h, 0, RejectReason::EntityNotSelected);
        return;
    }
    let function = h.byte3;
    if function.is_multiple_of(2) {
        // A secondary message: it closes the transaction with the same system bytes.
        if !inner.complete(h.system, Ok(frame)) {
            inner.reject(&h, 0, RejectReason::TransactionNotOpen);
        }
        return;
    }
    // A stream 9 error names the message it is about by that message's header; fail that
    // transaction right away instead of letting it run into T3.
    if let Ok(m) = &message
        && m.stream == 9
        && let Some(kind) = S9Kind::from_function(m.function)
        && let Some(Item::Binary(mhead)) = &m.item
        && let Ok(mhead) = <[u8; HEADER_LEN]>::try_from(mhead.as_slice())
    {
        let err = S9Error { kind, mhead };
        inner.complete(err.system_bytes(), Err(SecsError::S9(err)));
    }
    let _ = incoming.send(Incoming { header: h, message });
}

/// T7: a connection that stays not-selected for the whole timer is closed.
async fn t7_task(inner: Arc<Inner>) {
    let mut rx = inner.state.subscribe();
    loop {
        let state = *rx.borrow_and_update();
        match state {
            HsmsState::Closed => break,
            HsmsState::Selected => {
                if rx.changed().await.is_err() {
                    break;
                }
            }
            HsmsState::NotSelected => {
                tokio::select! {
                    _ = sleep(inner.config.t7()) => {
                        inner.shutdown(Some(SecsError::Timeout(Timer::T7)));
                        break;
                    }
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        }
    }
}

async fn linktest_task(inner: Arc<Inner>, every: Duration) {
    let mut state = inner.state.subscribe();
    loop {
        tokio::select! {
            _ = sleep(every) => {}
            _ = state.wait_for(|s| *s == HsmsState::Closed) => break,
        }
        if inner.state() != HsmsState::Selected {
            continue;
        }
        let result = inner.control(SType::LinktestReq, NO_DEVICE, 0).await;
        if result.is_err() {
            break;
        }
    }
}

/// A passive-mode endpoint: listens and hands out one [`HsmsConnection`] per accepted TCP
/// connection. The peer must Select before data flows; T7 closes connections that do not.
pub struct HsmsListener {
    listener: TcpListener,
    config: HsmsConfig,
    log: Option<broadcast::Sender<LogEntry>>,
}

impl HsmsListener {
    pub async fn bind(addr: impl ToSocketAddrs, config: HsmsConfig) -> std::io::Result<Self> {
        Ok(HsmsListener {
            listener: TcpListener::bind(addr).await?,
            config,
            log: None,
        })
    }

    /// Send every accepted connection's log into `log`.
    pub fn with_log(mut self, log: broadcast::Sender<LogEntry>) -> Self {
        self.log = Some(log);
        self
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn accept(&self) -> std::io::Result<(HsmsConnection, IncomingRx, SocketAddr)> {
        let (stream, peer) = self.listener.accept().await?;
        let (conn, rx) =
            HsmsConnection::start_with_log(stream, self.config.clone(), self.log.clone());
        Ok((conn, rx, peer))
    }
}

/// Active-mode connection attempts that honour T5: an attempt waits until T5 has passed since the
/// previous one started.
pub struct Connector {
    config: HsmsConfig,
    last_attempt: Mutex<Option<Instant>>,
    log: Option<broadcast::Sender<LogEntry>>,
}

impl Connector {
    pub fn new(config: HsmsConfig) -> Self {
        Connector {
            config,
            last_attempt: Mutex::new(None),
            log: None,
        }
    }

    /// Send every connection's log into `log`.
    pub fn with_log(mut self, log: broadcast::Sender<LogEntry>) -> Self {
        self.log = Some(log);
        self
    }

    pub fn config(&self) -> &HsmsConfig {
        &self.config
    }

    /// Wait out T5 if needed, then connect and select.
    pub async fn connect(&self, addr: impl ToSocketAddrs) -> Result<(HsmsConnection, IncomingRx)> {
        let wait_until = {
            let mut last = self.last_attempt.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            let start = match *last {
                Some(prev) => (prev + self.config.t5()).max(now),
                None => now,
            };
            *last = Some(start);
            start
        };
        tokio::time::sleep_until(wait_until).await;
        HsmsConnection::connect_with_log(addr, self.config.clone(), self.log.clone()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout() {
        let h = Header {
            session_id: 0x1234,
            byte2: 0x81,
            byte3: 0x0d,
            ptype: 0,
            stype: 0,
            system: 0xdead_beef,
        };
        let b = h.to_bytes();
        assert_eq!(b, [0x12, 0x34, 0x81, 0x0d, 0, 0, 0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(Header::from_bytes(&b), h);
    }

    #[test]
    fn frame_encode_decode() {
        let msg = SecsMessage::request(1, 1, None);
        let f = Frame::data(7, &msg, 9).unwrap();
        let bytes = f.encode();
        assert_eq!(bytes, [0, 0, 0, 10, 0, 7, 0x81, 1, 0, 0, 0, 0, 0, 9]);
        assert_eq!(Frame::decode(&bytes).unwrap(), f);

        let msg = SecsMessage::request(1, 13, Some(Item::empty_list()));
        let bytes = Frame::data(0, &msg, 1).unwrap().encode();
        assert_eq!(&bytes[..4], &[0, 0, 0, 12]);
        assert_eq!(&bytes[14..], &[0x01, 0x00]);

        assert!(Frame::decode(&bytes[..13]).is_err());
        assert!(Frame::decode(&[0, 0, 0, 5, 1, 2, 3, 4, 5]).is_err());
    }

    #[test]
    fn stype_codes() {
        for code in [0u8, 1, 2, 3, 4, 5, 6, 7, 9] {
            assert_eq!(SType::from_code(code).unwrap().code(), code);
        }
        assert_eq!(SType::from_code(8), None);
        assert_eq!(SType::from_code(10), None);
    }

    #[test]
    fn default_timers_are_the_e37_defaults() {
        let c = HsmsConfig::default();
        assert_eq!(
            (c.t3_ms, c.t5_ms, c.t6_ms, c.t7_ms, c.t8_ms),
            (45_000, 10_000, 5_000, 10_000, 5_000)
        );
        assert_eq!(c.device_id, 0);
    }
}
