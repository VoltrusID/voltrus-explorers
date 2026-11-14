//! MC protocol client over TCP or UDP.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::device::{McAddress, McDevice};
use crate::frame::{Command, Encoding, FrameKind, Request, Response, Route, end_code_text, frame_len};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub frame: FrameKind,
    #[serde(default)]
    pub encoding: Encoding,
    #[serde(default)]
    pub route: Route,
    /// Monitoring timer sent in every request, in units of 250 ms.
    #[serde(default = "default_timer")]
    pub monitor_timer: u16,
    /// How long to wait for a reply.
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timer() -> u16 {
    0x0010
}

fn default_timeout() -> u64 {
    3000
}

impl McConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            transport: Transport::Tcp,
            frame: FrameKind::E3,
            encoding: Encoding::Binary,
            route: Route::default(),
            monitor_timer: default_timer(),
            timeout_ms: default_timeout(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum McError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("no reply from the PLC within {0} ms")]
    Timeout(u64),
    #[error("PLC answered with end code 0x{code:04X}{}", end_code_text(*.code).map(|t| format!(": {t}")).unwrap_or_default())]
    EndCode { code: u16 },
    #[error("{0}")]
    Protocol(String),
}

/// Direction of a traffic record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Tx,
    Rx,
}

/// Called with every frame sent and received (for the comm log).
pub type TrafficFn = Arc<dyn Fn(Direction, &[u8]) + Send + Sync>;

enum Conn {
    Tcp(TcpStream),
    Udp(UdpSocket),
}

/// Most points one batch read or write may carry (Q/L CPU limits for 3E/4E frames).
pub const MAX_WORDS: usize = 960;
pub const MAX_BITS: usize = 3584;

pub struct McClient {
    cfg: McConfig,
    conn: Mutex<Option<Conn>>,
    serial: AtomicU16,
    traffic: Option<TrafficFn>,
}

impl McClient {
    /// Open the connection (TCP) or socket (UDP). A UDP "connection" only checks the address resolves.
    pub async fn connect(cfg: McConfig, traffic: Option<TrafficFn>) -> Result<Self, McError> {
        let conn = open(&cfg).await?;
        Ok(Self { cfg, conn: Mutex::new(Some(conn)), serial: AtomicU16::new(1), traffic })
    }

    pub fn config(&self) -> &McConfig {
        &self.cfg
    }

    pub async fn disconnect(&self) {
        if let Some(Conn::Tcp(mut s)) = self.conn.lock().await.take() {
            let _ = s.shutdown().await;
        }
    }

    /// Send one command and return the successful response. A broken TCP connection is reopened on the next call.
    pub async fn request(&self, command: &Command) -> Result<Response, McError> {
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        let req = Request::new(self.cfg.frame, self.cfg.encoding, serial, self.cfg.route, self.cfg.monitor_timer, command);
        let bytes = req.encode();
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            *guard = Some(open(&self.cfg).await?);
        }
        let conn = guard.as_mut().expect("connection was just opened");
        let wait = Duration::from_millis(self.cfg.timeout_ms);
        let result = timeout(wait, exchange(conn, &bytes, &req, self.traffic.as_ref())).await;
        let resp = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                if matches!(e, McError::Io(_)) {
                    *guard = None;
                }
                return Err(e);
            }
            Err(_) => {
                // A late reply would be read as the answer to the next request on TCP: start over.
                if matches!(conn, Conn::Tcp(_)) {
                    *guard = None;
                }
                return Err(McError::Timeout(self.cfg.timeout_ms));
            }
        };
        if resp.end_code != 0 {
            return Err(McError::EndCode { code: resp.end_code });
        }
        Ok(resp)
    }

    /// Batch read in word units. On a bit device each word holds 16 consecutive bits, the lowest number in bit 0.
    pub async fn read_words(&self, head: McAddress, count: usize) -> Result<Vec<u16>, McError> {
        let step = if head.device.is_bit() { 16 } else { 1 };
        let mut out = Vec::with_capacity(count);
        let mut done = 0;
        while done < count {
            let n = (count - done).min(MAX_WORDS);
            let at = McAddress::new(head.device, head.number + (done * step) as u32);
            let resp = self.request(&Command::ReadWords { head: at, count: n as u16 }).await?;
            let words = resp.reader().words(n).map_err(|_| McError::Protocol(format!("expected {n} words in the reply")))?;
            out.extend(words);
            done += n;
        }
        Ok(out)
    }

    pub async fn read_bits(&self, head: McAddress, count: usize) -> Result<Vec<bool>, McError> {
        let mut out = Vec::with_capacity(count);
        let mut done = 0;
        while done < count {
            let n = (count - done).min(MAX_BITS);
            let at = McAddress::new(head.device, head.number + done as u32);
            let resp = self.request(&Command::ReadBits { head: at, count: n as u16 }).await?;
            let bits = resp.reader().bits(n).map_err(|_| McError::Protocol(format!("expected {n} bits in the reply")))?;
            out.extend(bits);
            done += n;
        }
        Ok(out)
    }

    pub async fn write_words(&self, head: McAddress, values: &[u16]) -> Result<(), McError> {
        let step = if head.device.is_bit() { 16 } else { 1 };
        for (i, chunk) in values.chunks(MAX_WORDS).enumerate() {
            let at = McAddress::new(head.device, head.number + (i * MAX_WORDS * step) as u32);
            self.request(&Command::WriteWords { head: at, values: chunk.to_vec() }).await?;
        }
        Ok(())
    }

    pub async fn write_bits(&self, head: McAddress, values: &[bool]) -> Result<(), McError> {
        for (i, chunk) in values.chunks(MAX_BITS).enumerate() {
            let at = McAddress::new(head.device, head.number + (i * MAX_BITS) as u32);
            self.request(&Command::WriteBits { head: at, values: chunk.to_vec() }).await?;
        }
        Ok(())
    }

    /// CPU model name (for example "Q03UDVCPU") and model code.
    pub async fn read_cpu_model(&self) -> Result<(String, u16), McError> {
        let resp = self.request(&Command::ReadCpuModel).await?;
        parse_cpu_model(&resp)
    }

    pub async fn remote_run(&self, force: bool) -> Result<(), McError> {
        self.request(&Command::RemoteRun { force, clear: 0 }).await.map(|_| ())
    }

    pub async fn remote_stop(&self) -> Result<(), McError> {
        self.request(&Command::RemoteStop).await.map(|_| ())
    }
}

pub(crate) fn parse_cpu_model(resp: &Response) -> Result<(String, u16), McError> {
    if resp.data.len() < 16 {
        return Err(McError::Protocol("CPU model reply is too short".into()));
    }
    let name = String::from_utf8_lossy(&resp.data[..16]).trim().to_string();
    let mut r = crate::frame::Reader { buf: &resp.data[16..], pos: 0, enc: resp.encoding };
    let code = r.u16().unwrap_or(0);
    Ok((name, code))
}

/// What a device holds, in words (for hints and logs).
pub fn describe(device: McDevice) -> &'static str {
    use McDevice::*;
    match device {
        X => "input",
        Y => "output",
        M => "internal relay",
        L => "latch relay",
        F => "annunciator",
        V => "edge relay",
        B => "link relay",
        D => "data register",
        W => "link register",
        R => "file register",
        ZR => "file register (serial)",
        SM => "special relay",
        SD => "special register",
        TS => "timer contact",
        TC => "timer coil",
        TN => "timer current value",
        SS => "retentive timer contact",
        SC => "retentive timer coil",
        SN => "retentive timer current value",
        CS => "counter contact",
        CC => "counter coil",
        CN => "counter current value",
        SB => "link special relay",
        SW => "link special register",
        DX => "direct input",
        DY => "direct output",
        Z => "index register",
    }
}

async fn open(cfg: &McConfig) -> Result<Conn, McError> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let wait = Duration::from_millis(cfg.timeout_ms);
    match cfg.transport {
        Transport::Tcp => {
            let s = timeout(wait, TcpStream::connect(&addr)).await.map_err(|_| McError::Timeout(cfg.timeout_ms))??;
            s.set_nodelay(true)?;
            Ok(Conn::Tcp(s))
        }
        Transport::Udp => {
            let bind = if cfg.host.contains(':') { "[::]:0" } else { "0.0.0.0:0" };
            let s = UdpSocket::bind(bind).await?;
            s.connect(&addr).await?;
            Ok(Conn::Udp(s))
        }
    }
}

async fn exchange(conn: &mut Conn, bytes: &[u8], req: &Request, traffic: Option<&TrafficFn>) -> Result<Response, McError> {
    if let Some(t) = traffic {
        t(Direction::Tx, bytes);
    }
    match conn {
        Conn::Tcp(s) => {
            s.write_all(bytes).await?;
            let mut buf = Vec::with_capacity(64);
            let mut chunk = [0u8; 4096];
            loop {
                match frame_len(&buf) {
                    Err(_) => return Err(McError::Protocol(format!("the reply is not an MC protocol frame: {}", preview(&buf)))),
                    Ok(Some(n)) if buf.len() >= n => {
                        let frame = buf[..n].to_vec();
                        if let Some(t) = traffic {
                            t(Direction::Rx, &frame);
                        }
                        let resp = Response::decode(&frame).map_err(McError::Protocol)?;
                        check_matches(req, &resp)?;
                        return Ok(resp);
                    }
                    _ => {}
                }
                let n = s.read(&mut chunk).await?;
                if n == 0 {
                    return Err(McError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the PLC closed the connection")));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
        Conn::Udp(s) => {
            s.send(bytes).await?;
            let mut buf = vec![0u8; 8192];
            loop {
                let n = s.recv(&mut buf).await?;
                let frame = &buf[..n];
                if let Some(t) = traffic {
                    t(Direction::Rx, frame);
                }
                let resp = Response::decode(frame).map_err(McError::Protocol)?;
                // A stale 4E reply to an earlier (timed out) request: wait for ours.
                if req.kind == FrameKind::E4 && resp.serial != req.serial {
                    continue;
                }
                check_matches(req, &resp)?;
                return Ok(resp);
            }
        }
    }
}

fn check_matches(req: &Request, resp: &Response) -> Result<(), McError> {
    if resp.kind != req.kind || resp.encoding != req.encoding {
        return Err(McError::Protocol("the reply's frame type or code does not match the request".into()));
    }
    if req.kind == FrameKind::E4 && resp.serial != req.serial {
        return Err(McError::Protocol(format!("reply serial {} does not match request serial {}", resp.serial, req.serial)));
    }
    Ok(())
}

fn preview(buf: &[u8]) -> String {
    buf.iter().take(16).map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
