//! Opening a bus: an SLCAN adapter on a serial port, a SocketCAN interface (Linux), or the
//! in-process virtual bus the simulator sits on. Every kind gives the same [`CanHandle`]: a way to
//! send and a stream of [`BusEvent`]s.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::error::CanError;
use crate::frame::{CanFrame, Direction, TimedFrame, now_us};
use crate::slcan;

/// Something that happened on the bus.
#[derive(Debug, Clone, PartialEq)]
pub enum BusEvent {
    Frame(TimedFrame),
    /// An error frame, a refused command or lost frames; the bus stays open.
    Error(String),
    /// Information from the adapter (version, status flags).
    Info(String),
    /// The bus closed; `Some` when it was not asked to.
    Closed(Option<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum InterfaceConfig {
    /// SLCAN adapter on a serial port.
    Slcan {
        port: String,
        /// Nominal bit rate (10k to 1M).
        bitrate: u32,
        /// CAN FD data bit rate, for adapters that support it.
        #[serde(default)]
        data_bitrate: Option<u32>,
        /// Serial line speed; USB CDC adapters ignore it.
        #[serde(default = "default_serial_baud")]
        serial_baud: u32,
        /// Open the channel without acknowledging or sending (`L` instead of `O`).
        #[serde(default)]
        listen_only: bool,
    },
    /// A gs_usb adapter (candleLight firmware) driven over USB from user space.
    GsUsb {
        /// Serial number from [`crate::gsusb::list_devices`]; `None` takes the first one.
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        channel: u8,
        bitrate: u32,
        #[serde(default)]
        data_bitrate: Option<u32>,
        #[serde(default)]
        listen_only: bool,
    },
    /// A Linux SocketCAN interface (`can0`, `vcan0`), already configured and up.
    Socketcan { ifname: String },
    /// The in-process virtual bus.
    Virtual,
}

fn default_serial_baud() -> u32 {
    115_200
}

impl InterfaceConfig {
    pub fn describe(&self) -> String {
        match self {
            InterfaceConfig::Slcan { port, bitrate, data_bitrate, listen_only, .. } => {
                let fd = data_bitrate.map(|d| format!(" / {} data", rate_text(d))).unwrap_or_default();
                let lo = if *listen_only { ", listen only" } else { "" };
                format!("SLCAN {port} at {}{fd}{lo}", rate_text(*bitrate))
            }
            InterfaceConfig::GsUsb { device, channel, bitrate, data_bitrate, listen_only } => {
                let fd = data_bitrate.map(|d| format!(" / {} data", rate_text(d))).unwrap_or_default();
                let lo = if *listen_only { ", listen only" } else { "" };
                let dev = device.as_deref().map(|d| format!(" {d}")).unwrap_or_default();
                let ch = if *channel > 0 { format!(" channel {}", channel + 1) } else { String::new() };
                format!("gs_usb{dev}{ch} at {}{fd}{lo}", rate_text(*bitrate))
            }
            InterfaceConfig::Socketcan { ifname } => format!("SocketCAN {ifname}"),
            InterfaceConfig::Virtual => "virtual bus".into(),
        }
    }

    /// Name written into candump logs.
    pub fn log_name(&self) -> String {
        match self {
            InterfaceConfig::Slcan { .. } => "slcan0".into(),
            InterfaceConfig::GsUsb { channel, .. } => format!("can{channel}"),
            InterfaceConfig::Socketcan { ifname } => ifname.clone(),
            InterfaceConfig::Virtual => "vcan0".into(),
        }
    }
}

pub fn rate_text(bps: u32) -> String {
    if bps >= 1_000_000 && bps.is_multiple_of(1_000_000) {
        format!("{} Mbit/s", bps / 1_000_000)
    } else if bps.is_multiple_of(1000) {
        format!("{} kbit/s", bps / 1000)
    } else {
        format!("{bps} bit/s")
    }
}

pub(crate) type SendReq = (CanFrame, oneshot::Sender<Result<(), CanError>>);

/// Sends frames on an open bus. Cheap to clone; fails with [`CanError::Closed`] once the bus is gone.
#[derive(Clone)]
pub struct CanSender {
    tx: mpsc::Sender<SendReq>,
}

impl CanSender {
    pub async fn send(&self, frame: CanFrame) -> Result<(), CanError> {
        frame.validate()?;
        let (done, wait) = oneshot::channel();
        self.tx.send((frame, done)).await.map_err(|_| CanError::Closed)?;
        wait.await.map_err(|_| CanError::Closed)?
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// An open bus.
pub struct CanHandle {
    sender: CanSender,
    /// Everything the bus reports. Take it once with [`CanHandle::take_events`].
    events: Option<mpsc::UnboundedReceiver<BusEvent>>,
    task: JoinHandle<()>,
    stop: Option<oneshot::Sender<()>>,
    pub description: String,
}

impl CanHandle {
    pub fn sender(&self) -> CanSender {
        self.sender.clone()
    }

    pub async fn send(&self, frame: CanFrame) -> Result<(), CanError> {
        self.sender.send(frame).await
    }

    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<BusEvent>> {
        self.events.take()
    }

    /// Closes the bus (an SLCAN adapter gets its close command) and waits for the driver to end.
    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let task = std::mem::replace(&mut self.task, tokio::spawn(async {}));
        if tokio::time::timeout(Duration::from_secs(2), task).await.is_err() {
            // The driver is stuck in I/O; dropping the handle aborts it.
        }
    }
}

impl Drop for CanHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) struct Driver {
    pub(crate) events: mpsc::UnboundedSender<BusEvent>,
    pub(crate) requests: mpsc::Receiver<SendReq>,
    pub(crate) stop: oneshot::Receiver<()>,
}

pub(crate) fn channels(description: String) -> (Driver, impl FnOnce(JoinHandle<()>) -> CanHandle) {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (req_tx, req_rx) = mpsc::channel(256);
    let (stop_tx, stop_rx) = oneshot::channel();
    let driver = Driver { events: ev_tx, requests: req_rx, stop: stop_rx };
    let make = move |task| CanHandle {
        sender: CanSender { tx: req_tx },
        events: Some(ev_rx),
        task,
        stop: Some(stop_tx),
        description,
    };
    (driver, make)
}

/// Opens a bus.
pub async fn open(cfg: &InterfaceConfig, vbus: &VirtualBus) -> Result<CanHandle, CanError> {
    match cfg {
        InterfaceConfig::Virtual => Ok(vbus.attach()),
        InterfaceConfig::Slcan { port, bitrate, data_bitrate, serial_baud, listen_only } => {
            open_slcan(port, *bitrate, *data_bitrate, *serial_baud, *listen_only, cfg.describe()).await
        }
        InterfaceConfig::GsUsb { device, channel, bitrate, data_bitrate, listen_only } => {
            crate::gsusb::open(device.as_deref(), *channel, *bitrate, *data_bitrate, *listen_only, cfg.describe()).await
        }
        #[cfg(target_os = "linux")]
        InterfaceConfig::Socketcan { ifname } => crate::socketcan::open(ifname, cfg.describe()),
        #[cfg(not(target_os = "linux"))]
        InterfaceConfig::Socketcan { .. } => {
            Err(CanError::Unsupported("SocketCAN is only available on Linux. On a Mac, use an SLCAN adapter.".into()))
        }
    }
}

// ---- virtual bus ---------------------------------------------------------------------------------

/// An in-process bus. Every attached node sees the frames the others send.
#[derive(Clone)]
pub struct VirtualBus {
    tx: broadcast::Sender<(u64, CanFrame)>,
    next_node: Arc<AtomicU64>,
}

impl Default for VirtualBus {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualBus {
    pub fn new() -> Self {
        VirtualBus { tx: broadcast::channel(8192).0, next_node: Arc::new(AtomicU64::new(1)) }
    }

    /// Attaches a node. Must be called inside a Tokio runtime.
    pub fn attach(&self) -> CanHandle {
        let node = self.next_node.fetch_add(1, Ordering::Relaxed);
        let (mut d, make) = channels("virtual bus".into());
        let bus = self.tx.clone();
        let mut rx = bus.subscribe();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut d.stop => break,
                    r = rx.recv() => match r {
                        Ok((from, frame)) => {
                            if from != node {
                                let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Rx, frame }));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            let _ = d.events.send(BusEvent::Error(format!("receive queue overflow: {n} frames lost")));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    req = d.requests.recv() => match req {
                        Some((frame, done)) => {
                            let ts = now_us();
                            let _ = bus.send((node, frame.clone()));
                            let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: ts, dir: Direction::Tx, frame }));
                            let _ = done.send(Ok(()));
                        }
                        None => break,
                    },
                }
            }
            let _ = d.events.send(BusEvent::Closed(None));
        });
        make(task)
    }

    /// Nodes currently attached (including the simulator).
    pub fn nodes(&self) -> usize {
        self.tx.receiver_count()
    }
}

// ---- SLCAN ---------------------------------------------------------------------------------------

/// A serial port that may be an SLCAN adapter.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub name: String,
    pub description: String,
    /// USB vendor and product ids of known SLCAN adapters.
    pub likely_can: bool,
}

/// Serial ports, known CAN adapters first. On macOS only the `/dev/cu.*` call-out devices are listed.
pub fn list_ports() -> Result<Vec<PortInfo>, CanError> {
    let ports = tokio_serial::available_ports().map_err(|e| CanError::Adapter(e.to_string()))?;
    let mut out: Vec<PortInfo> = ports
        .into_iter()
        .filter(|p| !(cfg!(target_os = "macos") && p.port_name.starts_with("/dev/tty.")))
        .map(|p| {
            let (description, likely_can) = match &p.port_type {
                tokio_serial::SerialPortType::UsbPort(u) => {
                    let known = matches!(
                        (u.vid, u.pid),
                        (0xAD50, 0x60C4) | (0x16D0, 0x117E) | (0x0403, 0xFFA8) | (0x04D8, 0x000A) | (0x1D50, 0x606F)
                    ) || u.product.as_deref().is_some_and(|s| {
                        let s = s.to_ascii_lowercase();
                        s.contains("can") && !s.contains("scan")
                    });
                    let name = u.product.clone().or_else(|| u.manufacturer.clone()).unwrap_or_else(|| "USB serial".into());
                    (format!("{name} ({:04X}:{:04X})", u.vid, u.pid), known)
                }
                tokio_serial::SerialPortType::BluetoothPort => ("Bluetooth".into(), false),
                tokio_serial::SerialPortType::PciPort => ("PCI".into(), false),
                tokio_serial::SerialPortType::Unknown => (String::new(), false),
            };
            PortInfo { name: p.port_name, description, likely_can }
        })
        .collect();
    out.sort_by(|a, b| b.likely_can.cmp(&a.likely_can).then(a.name.cmp(&b.name)));
    Ok(out)
}

async fn open_slcan(
    port: &str,
    bitrate: u32,
    data_bitrate: Option<u32>,
    serial_baud: u32,
    listen_only: bool,
    description: String,
) -> Result<CanHandle, CanError> {
    use tokio_serial::SerialPortBuilderExt;
    let bitrate_cmd = slcan::bitrate_command(bitrate)?;
    let data_cmd = data_bitrate.map(slcan::data_bitrate_command).transpose()?;
    let mut serial = tokio_serial::new(port, serial_baud)
        .timeout(Duration::from_millis(100))
        .open_native_async()
        .map_err(|e| CanError::Adapter(format!("cannot open {port}: {e}")))?;
    #[cfg(unix)]
    let _ = serial.set_exclusive(true);

    let mut dec = slcan::Decoder::new();
    let mut infos = Vec::new();

    // Clear whatever is half-typed, close the channel in case it was left open, then configure.
    serial.write_all(b"\r\r\r").await?;
    let _ = command(&mut serial, &mut dec, "C\r", &mut infos).await;
    drain(&mut serial).await;
    dec = slcan::Decoder::new();
    let _ = command(&mut serial, &mut dec, "V\r", &mut infos).await;
    expect_ok(command(&mut serial, &mut dec, &bitrate_cmd, &mut infos).await, "the bit rate")?;
    if let Some(cmd) = &data_cmd {
        expect_ok(command(&mut serial, &mut dec, cmd, &mut infos).await, "the CAN FD data rate")?;
    }
    let open_cmd = if listen_only { "L\r" } else { "O\r" };
    expect_ok(command(&mut serial, &mut dec, open_cmd, &mut infos).await, "opening the channel")?;

    let (mut d, make) = channels(description);
    for i in infos {
        let _ = d.events.send(BusEvent::Info(i));
    }
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let reason = loop {
            tokio::select! {
                _ = &mut d.stop => {
                    let _ = serial.write_all(b"C\r").await;
                    let _ = serial.flush().await;
                    break None;
                }
                r = serial.read(&mut buf) => match r {
                    Ok(0) => break Some("the adapter closed the port".to_string()),
                    Ok(n) => {
                        for reply in dec.push(&buf[..n]) {
                            let ev = match reply {
                                Ok(slcan::Reply::Frame(frame)) => BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Rx, frame }),
                                Ok(slcan::Reply::Refused) => BusEvent::Error("the adapter refused a frame or command (transmit buffer full or bus off?)".into()),
                                Ok(slcan::Reply::Status(f)) => BusEvent::Info(format!("status: {}", slcan::status_text(f))),
                                Ok(slcan::Reply::Info(i)) => BusEvent::Info(i),
                                Ok(slcan::Reply::Other(o)) => BusEvent::Info(format!("adapter: {o}")),
                                Ok(slcan::Reply::Ok | slcan::Reply::TxAck) => continue,
                                Err(e) => BusEvent::Error(e.to_string()),
                            };
                            let _ = d.events.send(ev);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                    Err(e) => break Some(format!("serial port error: {e}")),
                },
                req = d.requests.recv() => match req {
                    Some((frame, done)) => {
                        if listen_only {
                            let _ = done.send(Err(CanError::Unsupported("the channel is open in listen-only mode".into())));
                            continue;
                        }
                        let r = match slcan::encode(&frame) {
                            Ok(line) => serial.write_all(line.as_bytes()).await.map_err(CanError::from),
                            Err(e) => Err(e),
                        };
                        if r.is_ok() {
                            let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }));
                        }
                        let failed = r.as_ref().err().map(|e| e.to_string());
                        let _ = done.send(r);
                        if let Some(e) = failed
                            && e.contains("os error") {
                                break Some(format!("serial port error: {e}"));
                            }
                    }
                    None => {
                        let _ = serial.write_all(b"C\r").await;
                        break None;
                    }
                },
            }
        };
        let _ = d.events.send(BusEvent::Closed(reason));
    });
    Ok(make(task))
}

#[derive(Debug, PartialEq)]
enum CmdResult {
    Ok,
    Refused,
    NoAnswer,
}

fn expect_ok(r: Result<CmdResult, CanError>, what: &str) -> Result<(), CanError> {
    match r? {
        CmdResult::Ok | CmdResult::NoAnswer => Ok(()),
        CmdResult::Refused => Err(CanError::Adapter(format!(
            "the adapter refused {what}. Is it an SLCAN adapter, and does it support this setting?"
        ))),
    }
}

/// Writes a command and waits up to 500 ms for its acknowledgement. Frames that arrive meanwhile
/// are dropped (the channel is not open yet).
async fn command<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    serial: &mut S,
    dec: &mut slcan::Decoder,
    cmd: &str,
    infos: &mut Vec<String>,
) -> Result<CmdResult, CanError> {
    serial.write_all(cmd.as_bytes()).await?;
    serial.flush().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 256];
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return Ok(CmdResult::NoAnswer);
        }
        match tokio::time::timeout(left, serial.read(&mut buf)).await {
            Err(_) => return Ok(CmdResult::NoAnswer),
            Ok(Ok(0)) => return Err(CanError::Adapter("the port closed during setup".into())),
            Ok(Ok(n)) => {
                for r in dec.push(&buf[..n]) {
                    match r {
                        Ok(slcan::Reply::Ok) => return Ok(CmdResult::Ok),
                        Ok(slcan::Reply::Refused) => return Ok(CmdResult::Refused),
                        Ok(slcan::Reply::Info(i)) => {
                            infos.push(format!("adapter version {i}"));
                            return Ok(CmdResult::Ok);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

async fn drain<S: tokio::io::AsyncRead + Unpin>(serial: &mut S) {
    let mut buf = [0u8; 1024];
    while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), serial.read(&mut buf)).await {
        if n == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn virtual_bus_delivers_to_other_nodes() {
        let bus = VirtualBus::new();
        let mut a = bus.attach();
        let mut b = bus.attach();
        let mut ea = a.take_events().unwrap();
        let mut eb = b.take_events().unwrap();
        a.send(CanFrame::new(0x123, &[1]).unwrap()).await.unwrap();
        let BusEvent::Frame(tx) = ea.recv().await.unwrap() else { panic!() };
        assert_eq!(tx.dir, Direction::Tx);
        let BusEvent::Frame(rx) = eb.recv().await.unwrap() else { panic!() };
        assert_eq!(rx.dir, Direction::Rx);
        assert_eq!(rx.frame.id, 0x123);
        // The sender does not receive its own frame back.
        assert!(tokio::time::timeout(Duration::from_millis(50), ea.recv()).await.is_err());
        assert!(a.send(CanFrame { id: 0x900, ..CanFrame::new(1, &[]).unwrap() }).await.is_err());
        let sender = b.sender();
        b.close().await;
        assert!(matches!(eb.recv().await, Some(BusEvent::Closed(None))));
        assert!(sender.send(CanFrame::new(1, &[]).unwrap()).await.is_err());
    }

    #[test]
    fn config_serde() {
        let c: InterfaceConfig =
            serde_json::from_str(r#"{"kind":"slcan","port":"/dev/cu.usbmodem1","bitrate":500000,"listenOnly":true}"#).unwrap();
        assert_eq!(c.describe(), "SLCAN /dev/cu.usbmodem1 at 500 kbit/s, listen only");
        let v: InterfaceConfig = serde_json::from_str(r#"{"kind":"virtual"}"#).unwrap();
        assert_eq!(v.log_name(), "vcan0");
        let g: InterfaceConfig =
            serde_json::from_str(r#"{"kind":"gsusb","device":"0039001A","channel":1,"bitrate":250000,"dataBitrate":null,"listenOnly":false}"#).unwrap();
        assert_eq!(g.describe(), "gs_usb 0039001A channel 2 at 250 kbit/s");
        assert_eq!(g.log_name(), "can1");
        let s: InterfaceConfig = serde_json::from_str(r#"{"kind":"socketcan","ifname":"can0"}"#).unwrap();
        assert_eq!(s.describe(), "SocketCAN can0");
    }

    /// The SLCAN setup handshake against a scripted adapter on an in-memory pipe.
    #[tokio::test]
    async fn slcan_command_handshake() {
        let (mut host, mut dev) = tokio::io::duplex(1024);
        let adapter = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let mut got = String::new();
            for reply in ["\r", "\x07"] {
                let n = dev.read(&mut buf).await.unwrap();
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
                dev.write_all(reply.as_bytes()).await.unwrap();
            }
            got
        });
        let mut dec = slcan::Decoder::new();
        let mut infos = Vec::new();
        assert_eq!(command(&mut host, &mut dec, "S6\r", &mut infos).await.unwrap(), CmdResult::Ok);
        assert_eq!(command(&mut host, &mut dec, "Y9\r", &mut infos).await.unwrap(), CmdResult::Refused);
        assert_eq!(adapter.await.unwrap(), "S6\rY9\r");
        assert!(expect_ok(Ok(CmdResult::Refused), "x").is_err());
    }
}
