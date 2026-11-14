//! A simulated Omron CPU: answers FINS over UDP and FINS/TCP on the same port.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::task::{AbortHandle, JoinHandle};

use crate::area::{FinsAddress, FinsArea};
use crate::frame::{Command, FinsHeader, HEADER_LEN, RunMode, encode_response, end, tcp};

/// Words in each simulated area (a CJ2M-like CPU).
pub fn area_size(a: FinsArea) -> u32 {
    match a {
        FinsArea::Cio => 6144,
        FinsArea::Wr => 512,
        FinsArea::Hr => 1536,
        FinsArea::Ar => 960,
        FinsArea::Dm => 32768,
        FinsArea::Em(_) => 32768,
        FinsArea::Tim | FinsArea::Cnt => 4096,
    }
}

/// A0 to A447 are written by the CPU only.
pub const AR_READ_ONLY_END: u16 = 448;

pub struct FinsMemory {
    areas: HashMap<FinsArea, Vec<u16>>,
    /// 0x00 PROGRAM, 0x02 MONITOR, 0x04 RUN.
    pub mode: u8,
    pub model: String,
    pub version: String,
}

impl Default for FinsMemory {
    fn default() -> Self {
        Self { areas: HashMap::new(), mode: 0x04, model: "CJ2M-CPU33".into(), version: "02.10".into() }
    }
}

impl FinsMemory {
    fn area(&mut self, a: FinsArea) -> Result<&mut Vec<u16>, u16> {
        if let FinsArea::Em(b) = a
            && b > 0x18
        {
            return Err(end::NO_AREA_TYPE);
        }
        Ok(self.areas.entry(a).or_insert_with(|| vec![0; area_size(a) as usize]))
    }

    fn check(a: FinsArea, start_word: u32, words: u32) -> Result<(), u16> {
        if start_word >= area_size(a) {
            return Err(end::ADDRESS_RANGE);
        }
        if start_word + words > area_size(a) {
            return Err(end::ADDRESS_RANGE_EXCEEDED);
        }
        Ok(())
    }

    pub fn read_words(&mut self, at: FinsAddress, count: usize) -> Result<Vec<u16>, u16> {
        Self::check(at.area, at.word as u32, count as u32)?;
        let a = self.area(at.area)?;
        Ok(a[at.word as usize..at.word as usize + count].to_vec())
    }

    pub fn write_words(&mut self, at: FinsAddress, values: &[u16]) -> Result<(), u16> {
        Self::check(at.area, at.word as u32, values.len() as u32)?;
        let a = self.area(at.area)?;
        a[at.word as usize..at.word as usize + values.len()].copy_from_slice(values);
        Ok(())
    }

    fn bit_span(at: FinsAddress, count: usize) -> Result<u32, u16> {
        let start = at.word as u32 * 16 + at.bit.unwrap_or(0) as u32;
        let last_word = (start + count.max(1) as u32 - 1) / 16;
        Self::check(at.area, at.word as u32, last_word - at.word as u32 + 1)?;
        Ok(start)
    }

    pub fn read_bits(&mut self, at: FinsAddress, count: usize) -> Result<Vec<bool>, u16> {
        let start = Self::bit_span(at, count)?;
        let a = self.area(at.area)?;
        Ok((start..start + count as u32).map(|b| a[(b / 16) as usize] & (1 << (b % 16)) != 0).collect())
    }

    pub fn write_bits(&mut self, at: FinsAddress, values: &[bool]) -> Result<(), u16> {
        let start = Self::bit_span(at, values.len())?;
        let a = self.area(at.area)?;
        for (i, v) in values.iter().enumerate() {
            let b = start + i as u32;
            let w = &mut a[(b / 16) as usize];
            if *v {
                *w |= 1 << (b % 16);
            } else {
                *w &= !(1 << (b % 16));
            }
        }
        Ok(())
    }

    fn read_only(at: &FinsAddress) -> bool {
        at.area == FinsArea::Ar && at.word < AR_READ_ONLY_END
    }

    /// Execute one command; `Ok` holds the response data.
    pub fn execute(&mut self, c: &Command) -> Result<Vec<u8>, u16> {
        match c {
            Command::ReadWords { at, count } => {
                Ok(self.read_words(*at, *count as usize)?.iter().flat_map(|w| w.to_be_bytes()).collect())
            }
            Command::ReadBits { at, count } => Ok(self.read_bits(*at, *count as usize)?.into_iter().map(u8::from).collect()),
            Command::WriteWords { at, values } => {
                if Self::read_only(at) {
                    return Err(end::READ_ONLY);
                }
                self.write_words(*at, values).map(|_| Vec::new())
            }
            Command::WriteBits { at, values } => {
                if Self::read_only(at) {
                    return Err(end::READ_ONLY);
                }
                self.write_bits(*at, values).map(|_| Vec::new())
            }
            Command::Run(mode) => {
                self.mode = match mode {
                    RunMode::Monitor => 0x02,
                    RunMode::Run => 0x04,
                };
                Ok(Vec::new())
            }
            Command::Stop => {
                self.mode = 0x00;
                Ok(Vec::new())
            }
            Command::ControllerData => {
                let mut d = format!("{:<20}", self.model).into_bytes();
                d.extend(format!("{:<20}", self.version).into_bytes());
                d.extend([0u8; 40]); // for system use
                d.extend([0u8; 12]); // area data
                Ok(d)
            }
            Command::ControllerStatus => {
                let mut d = vec![if self.mode == 0 { 0x00 } else { 0x01 }, self.mode];
                d.extend([0u8; 8]); // fatal, non-fatal, messages, FAL/FALS number
                d.extend([0u8; 16]); // error message
                Ok(d)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerEvent {
    pub peer: SocketAddr,
    pub write: bool,
    pub text: String,
}

pub type EventFn = Arc<dyn Fn(ServerEvent) + Send + Sync>;

/// Answer one FINS command frame. `None` for frames that are not commands (no reply).
pub fn handle(mem: &Mutex<FinsMemory>, frame: &[u8]) -> Option<(Vec<u8>, Option<Command>)> {
    let header = FinsHeader::decode(frame)?;
    if header.is_response() || frame.len() < HEADER_LEN + 2 {
        return None;
    }
    let body = &frame[HEADER_LEN..];
    let code = u16::from_be_bytes([body[0], body[1]]);
    let (end_code, data, cmd) = match Command::decode(body) {
        Ok(c) => match mem.lock().expect("simulator memory lock").execute(&c) {
            Ok(d) => (0, d, Some(c)),
            Err(e) => (e, Vec::new(), Some(c)),
        },
        Err(e) => (e, Vec::new(), None),
    };
    // ICF bit 0 set: no response wanted.
    if header.icf & 0x01 != 0 {
        return Some((Vec::new(), cmd));
    }
    Some((encode_response(&header.reply(), code, end_code, &data), cmd))
}

fn describe(c: &Command) -> (bool, String) {
    match c {
        Command::ReadWords { at, count } => (false, format!("read {count} word(s) from {at}")),
        Command::ReadBits { at, count } => (false, format!("read {count} bit(s) from {at}")),
        Command::WriteWords { at, values } => (true, format!("wrote {} word(s) at {at}: {values:?}", values.len())),
        Command::WriteBits { at, values } => (
            true,
            format!("wrote {} bit(s) at {at}: {}", values.len(), values.iter().map(|b| if *b { '1' } else { '0' }).collect::<String>()),
        ),
        Command::Run(RunMode::Run) => (true, "RUN mode".into()),
        Command::Run(RunMode::Monitor) => (true, "MONITOR mode".into()),
        Command::Stop => (true, "PROGRAM mode (stop)".into()),
        Command::ControllerData => (false, "read controller data".into()),
        Command::ControllerStatus => (false, "read controller status".into()),
    }
}

fn notify(ev: &Option<EventFn>, peer: SocketAddr, cmd: Option<&Command>) {
    if let (Some(f), Some(c)) = (ev, cmd) {
        let (write, text) = describe(c);
        f(ServerEvent { peer, write, text });
    }
}

pub struct FinsServer {
    port: u16,
    node: u8,
    memory: Arc<Mutex<FinsMemory>>,
    tasks: Vec<JoinHandle<()>>,
    sessions: Arc<Mutex<Vec<AbortHandle>>>,
}

impl FinsServer {
    /// Listen on `0.0.0.0:port` for FINS/UDP and FINS/TCP. `node` is the simulated PLC's FINS node number, given to
    /// TCP clients in the handshake.
    pub async fn start(port: u16, node: u8, on_event: Option<EventFn>) -> std::io::Result<Self> {
        let tcp_listener = TcpListener::bind(("0.0.0.0", port)).await?;
        let port = tcp_listener.local_addr()?.port();
        let udp = UdpSocket::bind(("0.0.0.0", port)).await?;
        let memory = Arc::new(Mutex::new(FinsMemory::default()));
        let sessions: Arc<Mutex<Vec<AbortHandle>>> = Arc::default();

        let mut tasks = Vec::new();
        let (mem, ev, list) = (memory.clone(), on_event.clone(), sessions.clone());
        tasks.push(tokio::spawn(async move {
            let mut next_client: u8 = 1;
            loop {
                let Ok((mut stream, peer)) = tcp_listener.accept().await else { continue };
                let _ = stream.set_nodelay(true);
                let (mem, ev) = (mem.clone(), ev.clone());
                // Assign client node numbers in turn, skipping the server's own.
                let mut assigned = next_client;
                if assigned == node {
                    assigned = assigned.wrapping_add(1).max(1);
                }
                next_client = assigned.wrapping_add(1).clamp(1, 254);
                let session = tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        loop {
                            match tcp::frame_len(&buf) {
                                Err(_) => {
                                    let _ = stream.write_all(&tcp::envelope(tcp::ERROR, 0x01, &[])).await;
                                    return;
                                }
                                Ok(Some(n)) if buf.len() >= n => {
                                    let env: Vec<u8> = buf.drain(..n).collect();
                                    let (command, _err, data) = tcp::parse(&env);
                                    let reply = match command {
                                        tcp::NODE_REQUEST => {
                                            let asked = data.get(..4).map(|d| u32::from_be_bytes([d[0], d[1], d[2], d[3]])).unwrap_or(0);
                                            let client = if asked == 0 { assigned as u32 } else { asked };
                                            if client == node as u32 {
                                                tcp::envelope(tcp::NODE_RESPONSE, 0x24, &[])
                                            } else if client > 254 {
                                                tcp::envelope(tcp::NODE_RESPONSE, 0x23, &[])
                                            } else {
                                                let mut d = client.to_be_bytes().to_vec();
                                                d.extend((node as u32).to_be_bytes());
                                                tcp::envelope(tcp::NODE_RESPONSE, 0, &d)
                                            }
                                        }
                                        tcp::FRAME => match handle(&mem, data) {
                                            Some((reply, cmd)) => {
                                                notify(&ev, peer, cmd.as_ref());
                                                if reply.is_empty() {
                                                    continue;
                                                }
                                                tcp::envelope(tcp::FRAME, 0, &reply)
                                            }
                                            None => continue,
                                        },
                                        _ => tcp::envelope(tcp::ERROR, 0x03, &[]),
                                    };
                                    if stream.write_all(&reply).await.is_err() {
                                        return;
                                    }
                                }
                                _ => break,
                            }
                        }
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                });
                let mut list = list.lock().expect("session list lock");
                list.retain(|h| !h.is_finished());
                list.push(session.abort_handle());
            }
        }));
        let (mem, ev) = (memory.clone(), on_event);
        tasks.push(tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let Ok((n, peer)) = udp.recv_from(&mut buf).await else { continue };
                if let Some((reply, cmd)) = handle(&mem, &buf[..n]) {
                    notify(&ev, peer, cmd.as_ref());
                    if !reply.is_empty() {
                        let _ = udp.send_to(&reply, peer).await;
                    }
                }
            }
        }));
        Ok(Self { port, node, memory, tasks, sessions })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn node(&self) -> u8 {
        self.node
    }

    pub fn memory(&self) -> Arc<Mutex<FinsMemory>> {
        self.memory.clone()
    }

    pub fn stop(&self) {
        for t in &self.tasks {
            t.abort();
        }
        for s in self.sessions.lock().expect("session list lock").drain(..) {
            s.abort();
        }
    }
}

impl Drop for FinsServer {
    fn drop(&mut self) {
        self.stop();
    }
}
