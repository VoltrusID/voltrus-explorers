//! A simulated MELSEC CPU: answers 3E and 4E frames, binary and ASCII (detected per frame), over TCP and UDP.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::task::{AbortHandle, JoinHandle};

use crate::device::{ALL_DEVICES, McAddress, McDevice};
use crate::frame::{Command, Encoding, Request, Response, end, frame_len, put_bits, put_words};

/// Number of points of each device in the simulated CPU.
pub fn device_size(d: McDevice) -> u32 {
    use McDevice::*;
    match d {
        SM | SD | F | V => 2048,
        X | Y | B | W | L | DX | DY | SB | SW => 8192,
        M | D | R => 32768,
        ZR => 65536,
        TS | TC | TN | SS | SC | SN | CS | CC | CN => 2048,
        Z => 20,
    }
}

/// Device memory. Bit devices are stored packed, 16 per word, so word access to them sees the same bits.
pub struct McMemory {
    areas: HashMap<McDevice, Vec<u16>>,
    pub running: bool,
    pub model: String,
    pub model_code: u16,
}

impl Default for McMemory {
    fn default() -> Self {
        let areas = ALL_DEVICES
            .into_iter()
            .map(|d| {
                let n = device_size(d);
                (d, vec![0u16; if d.is_bit() { n.div_ceil(16) } else { n } as usize])
            })
            .collect();
        Self { areas, running: true, model: "Q03UDVCPU".into(), model_code: 0x0366 }
    }
}

impl McMemory {
    fn check(&self, head: McAddress, count: usize, unit_bits: bool) -> Result<(), u16> {
        if count == 0 {
            return Err(end::POINTS_OUT_OF_RANGE);
        }
        let points = if head.device.is_bit() && !unit_bits { count as u64 * 16 } else { count as u64 };
        if head.number as u64 + points > device_size(head.device) as u64 {
            return Err(end::DEVICE_OUT_OF_RANGE);
        }
        Ok(())
    }

    pub fn get_bit(&self, device: McDevice, n: u32) -> bool {
        let a = &self.areas[&device];
        a.get((n / 16) as usize).is_some_and(|w| w & (1 << (n % 16)) != 0)
    }

    pub fn set_bit(&mut self, device: McDevice, n: u32, v: bool) {
        let a = self.areas.get_mut(&device).expect("every device has an area");
        if let Some(w) = a.get_mut((n / 16) as usize) {
            if v {
                *w |= 1 << (n % 16);
            } else {
                *w &= !(1 << (n % 16));
            }
        }
    }

    /// Words starting at `head`; on a bit device each word is 16 bits from `head.number` on.
    pub fn read_words(&self, head: McAddress, count: usize) -> Result<Vec<u16>, u16> {
        self.check(head, count, false)?;
        if head.device.is_bit() {
            Ok((0..count as u32)
                .map(|i| (0..16).fold(0u16, |w, b| w | ((self.get_bit(head.device, head.number + i * 16 + b) as u16) << b)))
                .collect())
        } else {
            let a = &self.areas[&head.device];
            Ok(a[head.number as usize..head.number as usize + count].to_vec())
        }
    }

    pub fn write_words(&mut self, head: McAddress, values: &[u16]) -> Result<(), u16> {
        self.check(head, values.len(), false)?;
        if head.device.is_bit() {
            for (i, w) in values.iter().enumerate() {
                for b in 0..16 {
                    self.set_bit(head.device, head.number + i as u32 * 16 + b, w & (1 << b) != 0);
                }
            }
        } else {
            let a = self.areas.get_mut(&head.device).expect("every device has an area");
            a[head.number as usize..head.number as usize + values.len()].copy_from_slice(values);
        }
        Ok(())
    }

    pub fn read_bits(&self, head: McAddress, count: usize) -> Result<Vec<bool>, u16> {
        if !head.device.is_bit() {
            return Err(end::REQUEST_CONTENT);
        }
        self.check(head, count, true)?;
        Ok((0..count as u32).map(|i| self.get_bit(head.device, head.number + i)).collect())
    }

    pub fn write_bits(&mut self, head: McAddress, values: &[bool]) -> Result<(), u16> {
        if !head.device.is_bit() {
            return Err(end::REQUEST_CONTENT);
        }
        self.check(head, values.len(), true)?;
        for (i, v) in values.iter().enumerate() {
            self.set_bit(head.device, head.number + i as u32, *v);
        }
        Ok(())
    }

    /// Execute one command; `Ok` holds the response data in the frame's encoding.
    pub fn execute(&mut self, c: &Command, enc: Encoding) -> Result<Vec<u8>, u16> {
        let mut out = Vec::new();
        match c {
            Command::ReadWords { head, count } => put_words(&mut out, enc, &self.read_words(*head, *count as usize)?),
            Command::ReadBits { head, count } => put_bits(&mut out, enc, &self.read_bits(*head, *count as usize)?),
            Command::WriteWords { head, values } => self.write_words(*head, values)?,
            Command::WriteBits { head, values } => self.write_bits(*head, values)?,
            Command::ReadCpuModel => {
                out.extend_from_slice(format!("{:<16}", self.model).as_bytes());
                match enc {
                    Encoding::Binary => out.extend_from_slice(&self.model_code.to_le_bytes()),
                    Encoding::Ascii => out.extend_from_slice(format!("{:04X}", self.model_code).as_bytes()),
                }
            }
            Command::RemoteRun { .. } => self.running = true,
            Command::RemoteStop => self.running = false,
        }
        Ok(out)
    }
}

/// What a client did to the simulator, for its log.
#[derive(Debug, Clone)]
pub struct ServerEvent {
    pub peer: SocketAddr,
    pub write: bool,
    pub text: String,
}

pub type EventFn = Arc<dyn Fn(ServerEvent) + Send + Sync>;

/// Answer one request frame. Returns `None` for bytes that are not an MC request (no reply is sent).
pub fn handle(mem: &Mutex<McMemory>, frame: &[u8]) -> Option<(Vec<u8>, Option<Command>)> {
    let req = Request::decode(frame).ok()?;
    let parsed = Command::decode(req.command, req.subcommand, &req.body, req.encoding);
    let (code, data, cmd) = match parsed {
        Ok(c) => {
            let result = mem.lock().expect("simulator memory lock").execute(&c, req.encoding);
            match result {
                Ok(d) => (0, d, Some(c)),
                Err(code) => (code, Vec::new(), Some(c)),
            }
        }
        Err(code) => (code, Vec::new(), None),
    };
    Some((Response::to(&req, code, data).encode(), cmd))
}

fn describe(c: &Command) -> (bool, String) {
    match c {
        Command::ReadWords { head, count } => (false, format!("read {count} word(s) from {head}")),
        Command::ReadBits { head, count } => (false, format!("read {count} bit(s) from {head}")),
        Command::WriteWords { head, values } => (true, format!("wrote {} word(s) at {head}: {:?}", values.len(), values)),
        Command::WriteBits { head, values } => (
            true,
            format!("wrote {} bit(s) at {head}: {}", values.len(), values.iter().map(|b| if *b { '1' } else { '0' }).collect::<String>()),
        ),
        Command::ReadCpuModel => (false, "read the CPU model".into()),
        Command::RemoteRun { .. } => (true, "remote RUN".into()),
        Command::RemoteStop => (true, "remote STOP".into()),
    }
}

/// The simulator: one TCP listener and one UDP socket on the same port.
pub struct McServer {
    port: u16,
    memory: Arc<Mutex<McMemory>>,
    tasks: Vec<JoinHandle<()>>,
    sessions: Arc<Mutex<Vec<AbortHandle>>>,
}

impl McServer {
    /// Listen on `0.0.0.0:port` (TCP and UDP). Port 0 picks a free port (the TCP one, then UDP binds the same number).
    pub async fn start(port: u16, on_event: Option<EventFn>) -> std::io::Result<Self> {
        let tcp = TcpListener::bind(("0.0.0.0", port)).await?;
        let port = tcp.local_addr()?.port();
        let udp = UdpSocket::bind(("0.0.0.0", port)).await?;
        let memory = Arc::new(Mutex::new(McMemory::default()));

        let sessions: Arc<Mutex<Vec<AbortHandle>>> = Arc::default();
        let mut tasks = Vec::new();
        let (mem, ev, list) = (memory.clone(), on_event.clone(), sessions.clone());
        tasks.push(tokio::spawn(async move {
            loop {
                let Ok((mut stream, peer)) = tcp.accept().await else { continue };
                let _ = stream.set_nodelay(true);
                let (mem, ev) = (mem.clone(), ev.clone());
                let session = tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        // Answer every complete frame in the buffer.
                        loop {
                            match frame_len(&buf) {
                                Err(_) => return,
                                Ok(Some(n)) if buf.len() >= n => {
                                    let frame: Vec<u8> = buf.drain(..n).collect();
                                    if let Some((reply, cmd)) = handle(&mem, &frame) {
                                        notify(&ev, peer, cmd.as_ref());
                                        if stream.write_all(&reply).await.is_err() {
                                            return;
                                        }
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
            let mut buf = vec![0u8; 8192];
            loop {
                let Ok((n, peer)) = udp.recv_from(&mut buf).await else { continue };
                if let Some((reply, cmd)) = handle(&mem, &buf[..n]) {
                    notify(&ev, peer, cmd.as_ref());
                    let _ = udp.send_to(&reply, peer).await;
                }
            }
        }));
        Ok(Self { port, memory, tasks, sessions })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn memory(&self) -> Arc<Mutex<McMemory>> {
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

impl Drop for McServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn notify(ev: &Option<EventFn>, peer: SocketAddr, cmd: Option<&Command>) {
    if let (Some(f), Some(c)) = (ev, cmd) {
        let (write, text) = describe(c);
        f(ServerEvent { peer, write, text });
    }
}
