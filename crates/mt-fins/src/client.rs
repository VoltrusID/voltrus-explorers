//! FINS client over UDP or TCP.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::area::FinsAddress;
use crate::frame::{Command, FinsHeader, FinsResponse, RunMode, encode_command, end_code_text, tcp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinsConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub transport: Transport,
    /// Destination network (0 = local network).
    #[serde(default)]
    pub dest_network: u8,
    /// Destination node (DA1). 0 = the last octet of the PLC's IP address, the default node of an Omron Ethernet port.
    /// Over TCP the node the PLC reports in the handshake is used instead.
    #[serde(default)]
    pub dest_node: u8,
    /// Destination unit (DA2): 0 is the CPU.
    #[serde(default)]
    pub dest_unit: u8,
    #[serde(default)]
    pub src_network: u8,
    /// Source node (SA1). 0 = the last octet of this computer's IP address on the route to the PLC. Over TCP the node
    /// the PLC assigns in the handshake is used instead.
    #[serde(default)]
    pub src_node: u8,
    #[serde(default)]
    pub src_unit: u8,
    /// Local UDP port (0 = any). Some older Ethernet units answer only to port 9600.
    #[serde(default)]
    pub local_port: u16,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    3000
}

impl FinsConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            transport: Transport::Udp,
            dest_network: 0,
            dest_node: 0,
            dest_unit: 0,
            src_network: 0,
            src_node: 0,
            src_unit: 0,
            local_port: 0,
            timeout_ms: default_timeout(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FinsError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("no reply from the PLC within {0} ms (check the IP, port, transport and node numbers)")]
    Timeout(u64),
    #[error("PLC answered with end code 0x{code:04X}{}", end_code_text(*.code).map(|t| format!(": {t}")).unwrap_or_default())]
    EndCode { code: u16 },
    #[error("FINS/TCP error 0x{0:08X}: {text}", text = tcp::error_text(*.0))]
    TcpError(u32),
    #[error("{0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Tx,
    Rx,
}

pub type TrafficFn = Arc<dyn Fn(Direction, &[u8]) + Send + Sync>;

/// Words per memory area read or write (the frame limit of 2012 bytes allows 999 words; keep a margin).
pub const MAX_WORDS: usize = 990;
pub const MAX_BITS: usize = 990;

/// Controller model and version (0501 Controller Data Read).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ControllerData {
    pub model: String,
    pub version: String,
}

/// Operating status (0601 Controller Status Read).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ControllerStatus {
    /// "Stop", "Run" or "Standby".
    pub status: String,
    /// "PROGRAM", "MONITOR" or "RUN".
    pub mode: String,
    pub fatal_error: u16,
    pub non_fatal_error: u16,
    pub error_message: String,
}

enum Conn {
    Udp(UdpSocket),
    Tcp(TcpStream),
}

struct Session {
    conn: Conn,
    /// SA1 and DA1 in use (from the config, the addresses, or the TCP handshake).
    src_node: u8,
    dest_node: u8,
}

pub struct FinsClient {
    cfg: FinsConfig,
    session: Mutex<Option<Session>>,
    sid: AtomicU8,
    traffic: Option<TrafficFn>,
}

impl FinsClient {
    pub async fn connect(cfg: FinsConfig, traffic: Option<TrafficFn>) -> Result<Self, FinsError> {
        let session = open(&cfg, traffic.as_ref()).await?;
        Ok(Self { cfg, session: Mutex::new(Some(session)), sid: AtomicU8::new(1), traffic })
    }

    pub fn config(&self) -> &FinsConfig {
        &self.cfg
    }

    /// (SA1, DA1) of the open session.
    pub async fn nodes(&self) -> Option<(u8, u8)> {
        self.session.lock().await.as_ref().map(|s| (s.src_node, s.dest_node))
    }

    pub async fn disconnect(&self) {
        if let Some(Session { conn: Conn::Tcp(mut s), .. }) = self.session.lock().await.take() {
            let _ = s.shutdown().await;
        }
    }

    pub async fn request(&self, command: &Command) -> Result<FinsResponse, FinsError> {
        let mut guard = self.session.lock().await;
        if guard.is_none() {
            *guard = Some(open(&self.cfg, self.traffic.as_ref()).await?);
        }
        let session = guard.as_mut().expect("session was just opened");
        let sid = self.sid.fetch_add(1, Ordering::Relaxed);
        let header = FinsHeader::command(
            self.cfg.dest_network,
            session.dest_node,
            self.cfg.dest_unit,
            self.cfg.src_network,
            session.src_node,
            self.cfg.src_unit,
            sid,
        );
        let frame = encode_command(&header, command).map_err(FinsError::Protocol)?;
        let wait = Duration::from_millis(self.cfg.timeout_ms);
        let result = timeout(wait, exchange(&mut session.conn, &frame, sid, self.traffic.as_ref())).await;
        let resp = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                if matches!(e, FinsError::Io(_) | FinsError::TcpError(_)) {
                    *guard = None;
                }
                return Err(e);
            }
            Err(_) => {
                if matches!(session.conn, Conn::Tcp(_)) {
                    *guard = None;
                }
                return Err(FinsError::Timeout(self.cfg.timeout_ms));
            }
        };
        if resp.command != command.code() {
            return Err(FinsError::Protocol(format!("reply is for command {:04X}, not {:04X}", resp.command, command.code())));
        }
        let code = resp.end_code();
        if code != 0 {
            return Err(FinsError::EndCode { code });
        }
        Ok(resp)
    }

    pub async fn read_words(&self, at: FinsAddress, count: usize) -> Result<Vec<u16>, FinsError> {
        let mut out = Vec::with_capacity(count);
        let mut done = 0;
        while done < count {
            let n = (count - done).min(MAX_WORDS);
            let a = FinsAddress::new(at.area, at.word + done as u16);
            let resp = self.request(&Command::ReadWords { at: a, count: n as u16 }).await?;
            if resp.data.len() < n * 2 {
                return Err(FinsError::Protocol(format!("expected {n} words in the reply, got {} bytes", resp.data.len())));
            }
            out.extend(resp.data[..n * 2].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])));
            done += n;
        }
        Ok(out)
    }

    /// Bits from `at` on (`at.bit` is the first bit, continuing into the next words).
    pub async fn read_bits(&self, at: FinsAddress, count: usize) -> Result<Vec<bool>, FinsError> {
        let mut out = Vec::with_capacity(count);
        let start = at.word as u32 * 16 + at.bit.unwrap_or(0) as u32;
        let mut done = 0;
        while done < count {
            let n = (count - done).min(MAX_BITS);
            let b = start + done as u32;
            let a = FinsAddress { area: at.area, word: (b / 16) as u16, bit: Some((b % 16) as u8) };
            let resp = self.request(&Command::ReadBits { at: a, count: n as u16 }).await?;
            if resp.data.len() < n {
                return Err(FinsError::Protocol(format!("expected {n} bits in the reply")));
            }
            out.extend(resp.data[..n].iter().map(|b| *b & 1 != 0));
            done += n;
        }
        Ok(out)
    }

    pub async fn write_words(&self, at: FinsAddress, values: &[u16]) -> Result<(), FinsError> {
        for (i, chunk) in values.chunks(MAX_WORDS).enumerate() {
            let a = FinsAddress::new(at.area, at.word + (i * MAX_WORDS) as u16);
            self.request(&Command::WriteWords { at: a, values: chunk.to_vec() }).await?;
        }
        Ok(())
    }

    pub async fn write_bits(&self, at: FinsAddress, values: &[bool]) -> Result<(), FinsError> {
        let start = at.word as u32 * 16 + at.bit.unwrap_or(0) as u32;
        for (i, chunk) in values.chunks(MAX_BITS).enumerate() {
            let b = start + (i * MAX_BITS) as u32;
            let a = FinsAddress { area: at.area, word: (b / 16) as u16, bit: Some((b % 16) as u8) };
            self.request(&Command::WriteBits { at: a, values: chunk.to_vec() }).await?;
        }
        Ok(())
    }

    pub async fn controller_data(&self) -> Result<ControllerData, FinsError> {
        let resp = self.request(&Command::ControllerData).await?;
        if resp.data.len() < 40 {
            return Err(FinsError::Protocol("controller data reply is too short".into()));
        }
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        Ok(ControllerData { model: text(&resp.data[..20]), version: text(&resp.data[20..40]) })
    }

    pub async fn controller_status(&self) -> Result<ControllerStatus, FinsError> {
        let resp = self.request(&Command::ControllerStatus).await?;
        parse_status(&resp.data)
    }

    pub async fn run(&self, mode: RunMode) -> Result<(), FinsError> {
        self.request(&Command::Run(mode)).await.map(|_| ())
    }

    pub async fn stop(&self) -> Result<(), FinsError> {
        self.request(&Command::Stop).await.map(|_| ())
    }
}

pub(crate) fn parse_status(d: &[u8]) -> Result<ControllerStatus, FinsError> {
    if d.len() < 2 {
        return Err(FinsError::Protocol("controller status reply is too short".into()));
    }
    let status = match d[0] {
        0x00 => "Stop",
        0x01 => "Run",
        0x80 => "Standby",
        _ => "Unknown",
    };
    let mode = match d[1] {
        0x00 => "PROGRAM",
        0x02 => "MONITOR",
        0x04 => "RUN",
        _ => "Unknown",
    };
    let word = |i: usize| if d.len() >= i + 2 { u16::from_be_bytes([d[i], d[i + 1]]) } else { 0 };
    let error_message = if d.len() >= 26 {
        String::from_utf8_lossy(&d[10..26]).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
    } else {
        String::new()
    };
    Ok(ControllerStatus { status: status.into(), mode: mode.into(), fatal_error: word(2), non_fatal_error: word(4), error_message })
}

fn last_octet(ip: &std::net::IpAddr) -> u8 {
    match ip {
        std::net::IpAddr::V4(v4) => v4.octets()[3],
        std::net::IpAddr::V6(v6) => v6.octets()[15],
    }
}

async fn open(cfg: &FinsConfig, traffic: Option<&TrafficFn>) -> Result<Session, FinsError> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let wait = Duration::from_millis(cfg.timeout_ms);
    match cfg.transport {
        Transport::Udp => {
            let bind = if cfg.host.contains(':') { format!("[::]:{}", cfg.local_port) } else { format!("0.0.0.0:{}", cfg.local_port) };
            let s = UdpSocket::bind(bind).await?;
            s.connect(&addr).await?;
            let peer = s.peer_addr()?;
            let local = s.local_addr()?;
            let dest_node = if cfg.dest_node != 0 { cfg.dest_node } else { last_octet(&peer.ip()) };
            let src_node = if cfg.src_node != 0 { cfg.src_node } else { last_octet(&local.ip()) };
            Ok(Session { conn: Conn::Udp(s), src_node, dest_node })
        }
        Transport::Tcp => {
            let mut s = timeout(wait, TcpStream::connect(&addr)).await.map_err(|_| FinsError::Timeout(cfg.timeout_ms))??;
            s.set_nodelay(true)?;
            // Node address handshake: ask for our configured node, or 0 to have one assigned.
            let hello = tcp::envelope(tcp::NODE_REQUEST, 0, &(cfg.src_node as u32).to_be_bytes());
            if let Some(t) = traffic {
                t(Direction::Tx, &hello);
            }
            s.write_all(&hello).await?;
            let reply = timeout(wait, read_envelope(&mut s)).await.map_err(|_| FinsError::Timeout(cfg.timeout_ms))??;
            if let Some(t) = traffic {
                t(Direction::Rx, &reply);
            }
            let (command, error, data) = tcp::parse(&reply);
            if error != 0 {
                return Err(FinsError::TcpError(error));
            }
            if command != tcp::NODE_RESPONSE || data.len() < 8 {
                return Err(FinsError::Protocol("unexpected FINS/TCP handshake reply".into()));
            }
            let client = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u8;
            let server = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as u8;
            Ok(Session { conn: Conn::Tcp(s), src_node: client, dest_node: server })
        }
    }
}

async fn read_envelope(s: &mut TcpStream) -> Result<Vec<u8>, FinsError> {
    let mut buf = Vec::with_capacity(64);
    let mut chunk = [0u8; 4096];
    loop {
        match tcp::frame_len(&buf) {
            Err(_) => return Err(FinsError::Protocol("the reply is not a FINS/TCP frame".into())),
            Ok(Some(n)) if buf.len() >= n => {
                // One request is outstanding at a time, so anything after this envelope is not expected.
                buf.truncate(n);
                return Ok(buf);
            }
            _ => {}
        }
        let n = s.read(&mut chunk).await?;
        if n == 0 {
            return Err(FinsError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the PLC closed the connection")));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn exchange(conn: &mut Conn, frame: &[u8], sid: u8, traffic: Option<&TrafficFn>) -> Result<FinsResponse, FinsError> {
    match conn {
        Conn::Udp(s) => {
            if let Some(t) = traffic {
                t(Direction::Tx, frame);
            }
            s.send(frame).await?;
            let mut buf = vec![0u8; 4096];
            loop {
                let n = s.recv(&mut buf).await?;
                if let Some(t) = traffic {
                    t(Direction::Rx, &buf[..n]);
                }
                let resp = FinsResponse::decode(&buf[..n]).map_err(FinsError::Protocol)?;
                // A late reply to an earlier request that timed out: keep waiting for ours.
                if resp.header.sid != sid {
                    continue;
                }
                return Ok(resp);
            }
        }
        Conn::Tcp(s) => {
            let env = tcp::envelope(tcp::FRAME, 0, frame);
            if let Some(t) = traffic {
                t(Direction::Tx, &env);
            }
            s.write_all(&env).await?;
            loop {
                let reply = read_envelope(s).await?;
                if let Some(t) = traffic {
                    t(Direction::Rx, &reply);
                }
                let (command, error, data) = tcp::parse(&reply);
                if command == tcp::ERROR || error != 0 {
                    return Err(FinsError::TcpError(error));
                }
                if command != tcp::FRAME {
                    continue;
                }
                let resp = FinsResponse::decode(data).map_err(FinsError::Protocol)?;
                if resp.header.sid != sid {
                    continue;
                }
                return Ok(resp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reply_is_decoded() {
        let mut d = vec![0x01, 0x04, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00];
        d.extend_from_slice(b"BATTERY LOW     ");
        let s = parse_status(&d).unwrap();
        assert_eq!((s.status.as_str(), s.mode.as_str(), s.non_fatal_error, s.error_message.as_str()), ("Run", "RUN", 0x40, "BATTERY LOW"));
    }
}
