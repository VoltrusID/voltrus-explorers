//! Opening a bus: an SLCAN adapter on a serial port, a SocketCAN interface (Linux), or the
//! in-process virtual bus the simulator sits on. Every kind gives the same [`CanHandle`]: a way to
//! send and a stream of [`BusEvent`]s.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::error::CanError;
use crate::frame::{BusStatus, CanFrame, Direction, TimedFrame, now_us};
use crate::slcan::{self, Firmware};

/// Something that happened on the bus.
#[derive(Debug, Clone, PartialEq)]
pub enum BusEvent {
    Frame(TimedFrame),
    /// An error frame, a refused command or lost frames; the bus stays open.
    Error(String),
    /// Several errors that came too fast to show one by one: how many, and the text of the last.
    /// Errors are passed on at most once per 100 ms (once a second while they keep coming).
    Errors { count: u32, latest: String },
    /// The controller's error state and counters changed (passed on at most once per 100 ms).
    Status(BusStatus),
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
        /// Nominal bit rate. Which rates exist depends on the firmware (see [`slcan::Firmware`]).
        bitrate: u32,
        /// CAN FD data bit rate (CANable 2: 2 or 5 Mbit/s).
        #[serde(default)]
        data_bitrate: Option<u32>,
        /// Serial line speed; USB CDC adapters ignore it.
        #[serde(default = "default_serial_baud")]
        serial_baud: u32,
        /// Open the channel without acknowledging or sending (`L` on Lawicel, `M1` on CANable).
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
    } else if bps >= 1000 {
        // 83 333 -> "83.3 kbit/s"
        format!("{:.1} kbit/s", bps as f64 / 1000.0)
    } else {
        format!("{bps} bit/s")
    }
}

pub(crate) type SendReq = (CanFrame, oneshot::Sender<Result<(), CanError>>);

/// Sends frames on an open bus. Cheap to clone; fails with [`CanError::Closed`] once the bus is gone.
#[derive(Clone)]
pub struct CanSender {
    tx: mpsc::Sender<SendReq>,
    /// The connection carries CAN FD frames.
    fd: bool,
}

impl CanSender {
    pub async fn send(&self, frame: CanFrame) -> Result<(), CanError> {
        frame.validate()?;
        if frame.fd && !self.fd {
            return Err(CanError::Unsupported(
                "this connection is classic CAN, so it cannot send CAN FD frames. Reconnect with a CAN FD data rate (on an adapter that does CAN FD), or send a classic frame.".into(),
            ));
        }
        let (done, wait) = oneshot::channel();
        self.tx.send((frame, done)).await.map_err(|_| CanError::Closed)?;
        wait.await.map_err(|_| CanError::Closed)?
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Whether the connection carries CAN FD frames.
    pub fn fd(&self) -> bool {
        self.fd
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

    /// Closes the bus (an SLCAN adapter gets its close command, a gs_usb channel is reset) and waits
    /// for the driver to end.
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

/// The channels between a driver task and its [`CanHandle`]. `fd`: the connection carries CAN FD.
/// Must be called inside a Tokio runtime: errors and status pass through a task that collapses bursts.
pub(crate) fn channels(description: String, fd: bool) -> (Driver, impl FnOnce(JoinHandle<()>) -> CanHandle) {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    tokio::spawn(relay(ev_rx, out_tx));
    let (req_tx, req_rx) = mpsc::channel(256);
    let (stop_tx, stop_rx) = oneshot::channel();
    let driver = Driver { events: ev_tx, requests: req_rx, stop: stop_rx };
    let make = move |task| CanHandle {
        sender: CanSender { tx: req_tx, fd },
        events: Some(out_rx),
        task,
        stop: Some(stop_tx),
        description,
    };
    (driver, make)
}

/// Shortest gap between two error (or status) events.
const ERROR_WINDOW: Duration = Duration::from_millis(100);
/// The gap while errors keep coming (an adapter alone on the bus reports one per retry).
const STORM_WINDOW: Duration = Duration::from_secs(1);

/// Passes driver events on, collapsing bursts of errors and status changes so a failing bus cannot
/// flood the UI. Frames and everything else pass straight through.
async fn relay(mut inner: mpsc::UnboundedReceiver<BusEvent>, out: mpsc::UnboundedSender<BusEvent>) {
    let mut err_last: Option<Instant> = None;
    let mut err_pending: Option<(u32, String)> = None;
    let mut err_window = ERROR_WINDOW;
    let mut st_last: Option<Instant> = None;
    let mut st_pending: Option<BusStatus> = None;

    fn errors(count: u32, latest: String) -> BusEvent {
        if count == 1 { BusEvent::Error(latest) } else { BusEvent::Errors { count, latest } }
    }

    loop {
        let err_due = err_pending.as_ref().and(err_last).map(|t| t + err_window);
        let st_due = st_pending.as_ref().and(st_last).map(|t| t + ERROR_WINDOW);
        let due = match (err_due, st_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                let now = Instant::now();
                if err_due.is_some_and(|d| d <= now)
                    && let Some((n, latest)) = err_pending.take()
                {
                    err_last = Some(now);
                    err_window = STORM_WINDOW;
                    if out.send(errors(n, latest)).is_err() {
                        return;
                    }
                }
                if st_due.is_some_and(|d| d <= now)
                    && let Some(s) = st_pending.take()
                {
                    st_last = Some(now);
                    if out.send(BusEvent::Status(s)).is_err() {
                        return;
                    }
                }
            }
            ev = inner.recv() => {
                let now = Instant::now();
                let (count, text) = match ev {
                    Some(BusEvent::Error(t)) => (1, t),
                    Some(BusEvent::Errors { count, latest }) => (count, latest),
                    Some(BusEvent::Status(s)) => {
                        if st_last.is_some_and(|t| now < t + ERROR_WINDOW) {
                            st_pending = Some(s);
                        } else {
                            st_last = Some(now);
                            if out.send(BusEvent::Status(s)).is_err() {
                                return;
                            }
                        }
                        continue;
                    }
                    other => {
                        // The bus closed (or the driver ended): flush what is held back first.
                        let end = matches!(other, None | Some(BusEvent::Closed(_)));
                        if end {
                            if let Some((n, latest)) = err_pending.take() {
                                let _ = out.send(errors(n, latest));
                            }
                            if let Some(s) = st_pending.take() {
                                let _ = out.send(BusEvent::Status(s));
                            }
                        }
                        match other {
                            Some(ev) => {
                                if out.send(ev).is_err() {
                                    return;
                                }
                                continue;
                            }
                            None => return,
                        }
                    }
                };
                if let Some((n, latest)) = &mut err_pending {
                    *n = n.saturating_add(count);
                    *latest = text;
                } else if err_last.is_some_and(|t| now < t + err_window) {
                    err_pending = Some((count, text));
                } else {
                    err_window = ERROR_WINDOW;
                    err_last = Some(now);
                    if out.send(errors(count, text)).is_err() {
                        return;
                    }
                }
            }
        }
    }
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
            Err(CanError::Unsupported("SocketCAN is only available on Linux. On a Mac, use an SLCAN or gs_usb adapter.".into()))
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
        let (mut d, make) = channels("virtual bus".into(), true);
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
    let mut serial = tokio_serial::new(port, serial_baud)
        .timeout(Duration::from_millis(100))
        .open_native_async()
        .map_err(|e| CanError::Adapter(format!("cannot open {port}: {e}")))?;
    #[cfg(unix)]
    let _ = serial.set_exclusive(true);
    let (fw, version) = slcan_setup(&mut serial, port, bitrate, data_bitrate, listen_only).await?;
    Ok(spawn_slcan(serial, fw, version, listen_only, data_bitrate.is_some(), description))
}

/// How long the adapter gets to answer `V`.
const VERSION_TIMEOUT: Duration = Duration::from_millis(1000);
/// Whatever the adapter still sends after `C` is read and dropped for at most this long.
const DRAIN_LIMIT: Duration = Duration::from_millis(500);

/// Identifies the firmware and configures and opens the channel. Returns the firmware and its
/// version answer.
async fn slcan_setup<S: AsyncRead + AsyncWrite + Unpin>(
    serial: &mut S,
    port: &str,
    bitrate: u32,
    data_bitrate: Option<u32>,
    listen_only: bool,
) -> Result<(Firmware, String), CanError> {
    // Close the channel in case it was left open. Never a bare `\r` to clear the line: CANable
    // firmware runs its previous command again on an empty line, which could resend the last frame.
    write_cmd(serial, "C\r").await?;
    drain(serial, DRAIN_LIMIT).await;
    write_cmd(serial, "V\r").await?;
    let Some((fw, version)) = read_version(serial, VERSION_TIMEOUT).await? else {
        return Err(CanError::Adapter(format!(
            "no SLCAN adapter answered on {port}. Check that this is the adapter's port. If the adapter runs candleLight firmware (it then shows up as a USB device, not a serial port), choose gs_usb instead."
        )));
    };
    // Settings this firmware does not have are refused before anything is sent.
    let bitrate_cmd = slcan::bitrate_command(fw, bitrate)?;
    let data_cmd = data_bitrate.map(|d| slcan::data_bitrate_command(fw, d)).transpose()?;
    let mut dec = slcan::Decoder::new();
    if fw.acknowledges() {
        expect_ok(command(serial, &mut dec, &bitrate_cmd).await, "the bit rate")?;
        if let Some(cmd) = &data_cmd {
            expect_ok(command(serial, &mut dec, cmd).await, "the CAN FD data rate")?;
        }
        let open_cmd = if listen_only { "L\r" } else { "O\r" };
        expect_ok(command(serial, &mut dec, open_cmd).await, if listen_only { "opening the channel in listen-only mode" } else { "opening the channel" })?;
    } else {
        // CANable firmware acknowledges nothing (a refused command is silently ignored), which is why
        // the settings were checked against its own tables above. Listen-only is its silent mode
        // `M1`; `M0` undoes a silent mode left over from an earlier session.
        write_cmd(serial, &bitrate_cmd).await?;
        if let Some(cmd) = &data_cmd {
            write_cmd(serial, cmd).await?;
        }
        write_cmd(serial, if listen_only { "M1\r" } else { "M0\r" }).await?;
        write_cmd(serial, "O\r").await?;
    }
    Ok((fw, version))
}

async fn write_cmd<S: AsyncWrite + Unpin>(serial: &mut S, cmd: &str) -> Result<(), CanError> {
    serial.write_all(cmd.as_bytes()).await?;
    serial.flush().await?;
    Ok(())
}

/// Reads lines until one is a version answer, or the time is up. Other lines (frames still in
/// flight, a GPS or modem talking on the wrong port) are skipped.
async fn read_version<S: AsyncRead + Unpin>(serial: &mut S, limit: Duration) -> Result<Option<(Firmware, String)>, CanError> {
    let deadline = Instant::now() + limit;
    let mut line: Vec<u8> = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let r = match tokio::time::timeout_at(deadline, serial.read(&mut buf)).await {
            Err(_) => return Ok(None),
            Ok(r) => r,
        };
        match r {
            Ok(0) => return Err(CanError::Adapter("the port closed during setup".into())),
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\r' || b == b'\n' || b == 0x07 {
                        let text = String::from_utf8_lossy(&line).into_owned();
                        line.clear();
                        if let Some(fw) = slcan::parse_version(&text) {
                            return Ok(Some((fw, text.trim().to_string())));
                        }
                    } else if line.len() < 200 {
                        line.push(b);
                    } else {
                        line.clear();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
    }
}

/// What a Lawicel-style adapter still owes an answer for, in the order it answers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Frame,
    StatusPoll,
}

/// Runs an open SLCAN channel.
fn spawn_slcan<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    mut serial: S,
    fw: Firmware,
    version: String,
    listen_only: bool,
    fd: bool,
    description: String,
) -> CanHandle {
    let (mut d, make) = channels(description, fd);
    let _ = d.events.send(BusEvent::Info(format!("{} ({version})", fw.name())));
    let _ = d.events.send(BusEvent::Info(if fw.acknowledges() {
        "SLCAN: a sent frame is shown once the adapter has queued it; the adapter does not report whether another node acknowledged it on the bus.".into()
    } else {
        "SLCAN: a sent frame is shown once it is handed to the adapter; CANable firmware does not report whether the adapter accepted it, or whether another node acknowledged it on the bus.".into()
    }));
    if listen_only && !fw.acknowledges() {
        let _ = d.events.send(BusEvent::Info("listen only: CANable silent mode (M1)".into()));
    }
    let task = tokio::spawn(async move {
        let mut dec = slcan::Decoder::new();
        let mut buf = [0u8; 4096];
        // Lawicel-style adapters answer every frame (`z`/`Z` or BEL) and every `F` in order.
        let mut pending: VecDeque<(Pending, Instant)> = VecDeque::new();
        // Poll the status flags once a second, on adapters that answer `F`.
        let mut poll = fw.acknowledges();
        let mut poll_tick = tokio::time::interval(Duration::from_secs(1));
        poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_flags: Option<u8> = None;
        let reason = loop {
            let now = Instant::now();
            while pending.front().is_some_and(|(_, t)| now.duration_since(*t) > Duration::from_secs(1)) || pending.len() > 64 {
                pending.pop_front();
            }
            tokio::select! {
                _ = &mut d.stop => {
                    let _ = write_cmd(&mut serial, "C\r").await;
                    break None;
                }
                r = serial.read(&mut buf) => match r {
                    Ok(0) => break Some("the adapter closed the port (unplugged?)".to_string()),
                    Ok(n) => {
                        for reply in dec.push(&buf[..n]) {
                            let ev = match reply {
                                Ok(slcan::Reply::Frame(frame)) => BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Rx, frame }),
                                Ok(slcan::Reply::Refused) => match pending.pop_front() {
                                    Some((Pending::StatusPoll, _)) => {
                                        poll = false;
                                        continue;
                                    }
                                    Some((Pending::Frame, _)) => BusEvent::Error(
                                        "the adapter refused a frame (its transmit buffer is full, or the bus is off)".into(),
                                    ),
                                    None => BusEvent::Error("the adapter refused a command".into()),
                                },
                                Ok(slcan::Reply::Status(f)) => {
                                    if pending.front().is_some_and(|(p, _)| *p == Pending::StatusPoll) {
                                        pending.pop_front();
                                    }
                                    let prev = last_flags.replace(f);
                                    if slcan::status_state(f) != prev.map(slcan::status_state).unwrap_or(crate::frame::BusState::ErrorActive) {
                                        let _ = d.events.send(BusEvent::Status(BusStatus { state: slcan::status_state(f), tx_errors: None, rx_errors: None }));
                                    }
                                    // Lawicel clears the flags on every read, so each answer is news.
                                    if f & 0xCB != 0 {
                                        BusEvent::Error(format!("adapter status: {}", slcan::status_text(f)))
                                    } else if prev != Some(f) {
                                        BusEvent::Info(format!("adapter status: {}", slcan::status_text(f)))
                                    } else {
                                        continue;
                                    }
                                }
                                Ok(slcan::Reply::TxAck | slcan::Reply::Ok) => {
                                    if pending.front().is_some_and(|(p, _)| *p == Pending::Frame) {
                                        pending.pop_front();
                                    }
                                    continue;
                                }
                                Ok(slcan::Reply::Info(i)) => BusEvent::Info(format!("adapter: {i}")),
                                Ok(slcan::Reply::Other(o)) => BusEvent::Info(format!("adapter: {o}")),
                                Err(e) => BusEvent::Error(e.to_string()),
                            };
                            let _ = d.events.send(ev);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                    Err(e) => break Some(format!("serial port error: {e} (unplugged?)")),
                },
                _ = poll_tick.tick(), if poll => {
                    if !pending.iter().any(|(p, _)| *p == Pending::StatusPoll) {
                        if let Err(e) = write_cmd(&mut serial, "F\r").await {
                            break Some(format!("serial port error: {e} (unplugged?)"));
                        }
                        pending.push_back((Pending::StatusPoll, Instant::now()));
                    }
                }
                req = d.requests.recv() => match req {
                    Some((frame, done)) => {
                        if listen_only {
                            let _ = done.send(Err(CanError::Unsupported("the channel is open in listen-only mode".into())));
                            continue;
                        }
                        let line = match slcan::encode(&frame) {
                            Ok(l) => l,
                            Err(e) => {
                                let _ = done.send(Err(e));
                                continue;
                            }
                        };
                        match serial.write_all(line.as_bytes()).await {
                            Ok(()) => {
                                // Handed to the adapter; SLCAN has no report of the frame reaching the bus.
                                let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }));
                                let _ = done.send(Ok(()));
                                if fw.acknowledges() {
                                    pending.push_back((Pending::Frame, Instant::now()));
                                }
                            }
                            Err(e) => {
                                let msg = format!("serial port error: {e} (unplugged?)");
                                let _ = done.send(Err(CanError::Io(e)));
                                break Some(msg);
                            }
                        }
                    }
                    None => {
                        let _ = write_cmd(&mut serial, "C\r").await;
                        break None;
                    }
                },
            }
        };
        let _ = d.events.send(BusEvent::Closed(reason));
    });
    make(task)
}

#[derive(Debug, PartialEq)]
enum CmdResult {
    Ok,
    Refused,
    NoAnswer,
}

/// For adapters that acknowledge commands: anything but `\r` is a failure.
fn expect_ok(r: Result<CmdResult, CanError>, what: &str) -> Result<(), CanError> {
    match r? {
        CmdResult::Ok => Ok(()),
        CmdResult::Refused => Err(CanError::Adapter(format!("the adapter refused {what}. Does it support this setting?"))),
        CmdResult::NoAnswer => Err(CanError::Adapter(format!("the adapter did not answer {what}"))),
    }
}

/// Writes a command and waits up to 500 ms for its acknowledgement (`\r`) or refusal (BEL). Frames
/// that arrive meanwhile are dropped (the channel is not open yet).
async fn command<S: AsyncRead + AsyncWrite + Unpin>(serial: &mut S, dec: &mut slcan::Decoder, cmd: &str) -> Result<CmdResult, CanError> {
    write_cmd(serial, cmd).await?;
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 256];
    loop {
        match tokio::time::timeout_at(deadline, serial.read(&mut buf)).await {
            Err(_) => return Ok(CmdResult::NoAnswer),
            Ok(Ok(0)) => return Err(CanError::Adapter("the port closed during setup".into())),
            Ok(Ok(n)) => {
                for r in dec.push(&buf[..n]) {
                    match r {
                        Ok(slcan::Reply::Ok) => return Ok(CmdResult::Ok),
                        Ok(slcan::Reply::Refused) => return Ok(CmdResult::Refused),
                        _ => {}
                    }
                }
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Ok(Err(e)) => return Err(e.into()),
        }
        if Instant::now() >= deadline {
            return Ok(CmdResult::NoAnswer);
        }
    }
}

/// Reads and drops whatever arrives until the line is quiet for 50 ms, for at most `limit`.
async fn drain<S: AsyncRead + Unpin>(serial: &mut S, limit: Duration) {
    let end = Instant::now() + limit;
    let mut buf = [0u8; 1024];
    loop {
        let quiet = (Instant::now() + Duration::from_millis(50)).min(end);
        match tokio::time::timeout_at(quiet, serial.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => {}
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {}
            _ => return,
        }
        if Instant::now() >= end {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::BusState;
    use std::sync::Mutex as StdMutex;

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
        assert_eq!(rate_text(83_333), "83.3 kbit/s");
        assert_eq!(rate_text(750_000), "750 kbit/s");
    }

    /// A scripted SLCAN adapter on an in-memory pipe, after the firmware in `ref/` (CANable,
    /// CANable 2) and the Lawicel protocol: it records every line it gets.
    #[derive(Clone, Copy, PartialEq)]
    enum Emu {
        Canable,
        Canable2,
        Lawicel,
        /// Says nothing at all.
        Dead,
        /// Something else talking on the port (a GPS).
        Chatty,
    }

    type Lines = Arc<StdMutex<Vec<String>>>;

    fn emulator(kind: Emu) -> (tokio::io::DuplexStream, Lines) {
        let (host, mut dev) = tokio::io::duplex(4096);
        let lines: Lines = Arc::default();
        let log = lines.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let mut cur = Vec::new();
            let mut open = false;
            let mut chatter = tokio::time::interval(Duration::from_millis(20));
            loop {
                let n = tokio::select! {
                    r = dev.read(&mut buf) => match r { Ok(0) | Err(_) => return, Ok(n) => n },
                    _ = chatter.tick(), if kind == Emu::Chatty => {
                        if dev.write_all(b"$GPGGA,123519,4807.038,N,01131.000,E\r\n").await.is_err() { return; }
                        continue;
                    }
                };
                for &b in &buf[..n] {
                    if b != b'\r' {
                        cur.push(b);
                        continue;
                    }
                    let line = String::from_utf8_lossy(&cur).into_owned();
                    cur.clear();
                    log.lock().unwrap().push(line.clone());
                    let ack = kind == Emu::Lawicel;
                    let c = line.chars().next().unwrap_or(' ');
                    let reply: &[u8] = match (kind, c) {
                        (Emu::Dead | Emu::Chatty, _) => b"",
                        (Emu::Lawicel, 'V') => b"V1013\r",
                        (Emu::Canable, 'V') => b"bd4f1a2 https://github.com/normaldotcom/canable-fw.git\r",
                        (Emu::Canable2, 'V') => b"0aa3d51 https://github.com/normaldotcom/canable2-fw.git\r",
                        (_, 'O') | (Emu::Lawicel, 'L') => {
                            open = true;
                            if ack { b"\r" } else { b"" }
                        }
                        (_, 'C') => {
                            let was = open;
                            open = false;
                            if !ack { b"" } else if was { b"\r" } else { b"\x07" }
                        }
                        (Emu::Lawicel, 'S') if line.len() == 2 && ('0'..='8').contains(&line.chars().nth(1).unwrap()) => b"\r",
                        (Emu::Lawicel, 'F') if open => b"F24\r",
                        (Emu::Lawicel, 't' | 'T') => b"z\r",
                        (Emu::Lawicel, _) => b"\x07",
                        _ => b"",
                    };
                    if !reply.is_empty() && dev.write_all(reply).await.is_err() {
                        return;
                    }
                }
            }
        });
        (host, lines)
    }

    async fn open_emu(kind: Emu, bitrate: u32, data: Option<u32>, listen_only: bool) -> (Result<CanHandle, CanError>, Lines) {
        let (mut host, lines) = emulator(kind);
        let r = match slcan_setup(&mut host, "/dev/test", bitrate, data, listen_only).await {
            Ok((fw, v)) => Ok(spawn_slcan(host, fw, v, listen_only, data.is_some(), "test".into())),
            Err(e) => Err(e),
        };
        (r, lines)
    }

    fn got(lines: &Lines) -> Vec<String> {
        lines.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn canable_setup_and_close() {
        let (h, lines) = open_emu(Emu::Canable, 750_000, None, false).await;
        let mut h = h.unwrap();
        let mut ev = h.take_events().unwrap();
        let Some(BusEvent::Info(i)) = ev.recv().await else { panic!() };
        assert!(i.starts_with("CANable slcan firmware"), "{i}");
        h.send(CanFrame::new(0x123, &[1, 2, 3]).unwrap()).await.unwrap();
        // A classic connection refuses CAN FD frames before they reach the adapter.
        let e = h.send(CanFrame::fd(0x10, false, true, &[0; 12]).unwrap()).await.unwrap_err();
        assert!(e.to_string().contains("classic CAN"), "{e}");
        h.close().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        // S7 is 750k on CANable; no empty line anywhere (it would re-run the last command); M0 undoes
        // a silent mode left from before; C on close.
        assert_eq!(got(&lines), ["C", "V", "S7", "M0", "O", "t1233010203", "C"]);
    }

    #[tokio::test]
    async fn canable_listen_only_is_silent_mode_and_rates_are_checked() {
        let (h, lines) = open_emu(Emu::Canable, 500_000, None, true).await;
        let h = h.unwrap();
        assert!(h.send(CanFrame::new(1, &[]).unwrap()).await.is_err());
        h.close().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(got(&lines), ["C", "V", "S6", "M1", "O", "C"]);

        let (r, lines) = open_emu(Emu::Canable, 800_000, None, false).await;
        let e = r.err().unwrap().to_string();
        assert!(e.contains("750 kbit/s") && e.contains("no 800 kbit/s"), "{e}");
        assert_eq!(got(&lines), ["C", "V"], "nothing is configured after a refusal");
        let (r, _) = open_emu(Emu::Canable, 500_000, Some(2_000_000), false).await;
        assert!(r.err().unwrap().to_string().contains("does not do CAN FD"));
        let (r, _) = open_emu(Emu::Canable, 83_333, None, false).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn canable2_fd_rates() {
        let (h, lines) = open_emu(Emu::Canable2, 83_333, Some(5_000_000), false).await;
        let h = h.unwrap();
        h.send(CanFrame::fd(0x10, false, true, &[0xAA; 12]).unwrap()).await.unwrap();
        h.close().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(got(&lines), ["C", "V", "S9", "Y5", "M0", "O", &format!("b0109{}", "AA".repeat(12)), "C"]);
        let (r, _) = open_emu(Emu::Canable2, 500_000, Some(4_000_000), false).await;
        let e = r.err().unwrap().to_string();
        assert!(e.contains("2 Mbit/s, 5 Mbit/s"), "{e}");
    }

    #[tokio::test]
    async fn lawicel_acknowledges_and_reports_status() {
        let (h, lines) = open_emu(Emu::Lawicel, 800_000, None, true).await;
        let mut h = h.unwrap();
        let mut ev = h.take_events().unwrap();
        // The first status poll answers F24: error warning and error passive.
        let mut status = None;
        let mut flags = None;
        let until = Instant::now() + Duration::from_secs(2);
        while (status.is_none() || flags.is_none()) && Instant::now() < until {
            match tokio::time::timeout(Duration::from_millis(500), ev.recv()).await {
                Ok(Some(BusEvent::Status(s))) => status = Some(s),
                Ok(Some(BusEvent::Info(i))) if i.contains("adapter status") => flags = Some(i),
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert_eq!(status.unwrap().state, BusState::ErrorPassive);
        assert!(flags.unwrap().contains("error warning, error passive"));
        h.close().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let l = got(&lines);
        assert_eq!(&l[..4], ["C", "V", "S7", "L"]);
        assert!(l.contains(&"F".to_string()) && l.last().unwrap() == "C");

        // Refusals are errors on an adapter that acknowledges.
        let (r, _) = open_emu(Emu::Lawicel, 500_000, Some(2_000_000), false).await;
        assert!(r.err().unwrap().to_string().contains("refused the CAN FD data rate"));
        let (r, _) = open_emu(Emu::Lawicel, 750_000, None, false).await;
        assert!(r.err().unwrap().to_string().contains("800 kbit/s"));
    }

    #[tokio::test]
    async fn a_port_without_an_slcan_adapter_fails_clearly() {
        for kind in [Emu::Dead, Emu::Chatty] {
            let t0 = Instant::now();
            let (r, lines) = open_emu(kind, 500_000, None, false).await;
            let e = r.err().expect("no adapter, no connection").to_string();
            assert!(e.contains("no SLCAN adapter answered on /dev/test") && e.contains("gs_usb"), "{e}");
            assert!(t0.elapsed() < Duration::from_millis(1700), "{:?}", t0.elapsed());
            assert_eq!(got(&lines), ["C", "V"]);
        }
    }

    #[tokio::test]
    async fn error_bursts_are_collapsed() {
        let (d, make) = channels("t".into(), false);
        let mut h = make(tokio::spawn(async {}));
        let mut ev = h.take_events().unwrap();
        let status = |tx: u8| BusEvent::Status(BusStatus { state: BusState::ErrorWarning, tx_errors: Some(tx), rx_errors: Some(0) });
        for tx in [100, 104, 108] {
            let _ = d.events.send(status(tx));
        }
        assert_eq!(ev.recv().await, Some(status(100)));
        let t0 = Instant::now();
        assert_eq!(ev.recv().await, Some(status(108)), "the latest status within the window wins");
        assert!(t0.elapsed() >= Duration::from_millis(80));

        for i in 0..500 {
            let _ = d.events.send(BusEvent::Error(format!("e{i}")));
        }
        let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: 0, dir: Direction::Rx, frame: CanFrame::new(1, &[]).unwrap() }));
        assert_eq!(ev.recv().await, Some(BusEvent::Error("e0".into())));
        // Frames are not held back.
        assert!(matches!(ev.recv().await, Some(BusEvent::Frame(_))));
        let t0 = Instant::now();
        assert_eq!(ev.recv().await, Some(BusEvent::Errors { count: 499, latest: "e499".into() }));
        assert!(t0.elapsed() >= Duration::from_millis(80));
        // The storm keeps errors to one a second.
        let _ = d.events.send(BusEvent::Error("again".into()));
        assert!(tokio::time::timeout(Duration::from_millis(500), ev.recv()).await.is_err());
        assert_eq!(ev.recv().await, Some(BusEvent::Error("again".into())));
        let _ = d.events.send(BusEvent::Error("last".into()));
        let _ = d.events.send(BusEvent::Closed(None));
        assert_eq!(ev.recv().await, Some(BusEvent::Error("last".into())));
        assert_eq!(ev.recv().await, Some(BusEvent::Closed(None)));
        drop(d);
        assert_eq!(ev.recv().await, None);
    }
}
