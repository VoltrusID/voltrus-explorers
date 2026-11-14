//! An EtherCAT master session for the explorer, on EtherCrab: scan the bus, describe each device (ESC, SII, ports,
//! DC), read and write ESC registers, browse and change the CoE object dictionary (SDO, SDO Info, own mailbox
//! handling), map process data, change AL states, and run the cyclic exchange with working counter and timing
//! statistics.
//!
//! EtherCrab does the init (addresses, SII, mailbox and PDO configuration, DC delays) and the cyclic LRW. AL state
//! changes after that are plain AL control writes polled for the result, per device or for all, so a device that
//! refuses a state shows its AL status code instead of ending the session.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use ethercrab::std::ethercat_now;
use ethercrab::subdevice_group::{PreOp, PreOpPdi};
use ethercrab::{Command, DcSupport, MainDevice, MainDeviceConfig, PduStorage, SubDeviceGroup, Timeouts};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::coe::{self, SdoResponse};
use crate::esc::{self, AlState};
use crate::link::{Tap, VirtualLink};
use crate::sii::{self, Sii};
use crate::sim::{Segment, Template};
use crate::wire;

pub const MAX_DEVICES: usize = 64;
pub const MAX_PDI: usize = 4096;
const MAX_FRAMES: usize = 32;
const PDU_DATA: usize = PduStorage::element_size(1100);

type GroupPreOp = SubDeviceGroup<MAX_DEVICES, MAX_PDI, ethercrab::DefaultLock, PreOp>;
type GroupPdi = SubDeviceGroup<MAX_DEVICES, MAX_PDI, ethercrab::DefaultLock, PreOpPdi>;

enum Group {
    /// After init: devices in PRE-OP, process data not mapped yet.
    PreOp(GroupPreOp),
    /// FMMUs configured: the cyclic exchange can run in any state.
    Pdi(GroupPdi),
    /// Init failed or a transition consumed the group: only raw access is left.
    None,
}

fn es(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

/// Turns an EtherCrab error into words.
pub fn error_text(e: &ethercrab::error::Error) -> String {
    use ethercrab::error::Error as E;
    match e {
        E::Timeout(_) => "No answer from the network (timeout). Check the cable and that the devices have power.".into(),
        E::WorkingCounter { expected, received } => format!("Working counter {received}, expected {expected}: not every device answered"),
        E::SubDevice(code) => format!("Device error: {code:?}"),
        other => format!("{other}"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkSpec {
    Simulator { template: Template },
    Interface { name: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortView {
    pub kind: &'static str,
    pub link: bool,
    pub loop_closed: bool,
    pub communication: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub position: usize,
    pub address: u16,
    pub alias: u16,
    pub name: String,
    pub description: String,
    pub vendor_id: u32,
    pub vendor_name: Option<&'static str>,
    pub product_code: u32,
    pub revision: u32,
    pub serial: u32,
    pub esc_type: u8,
    pub esc_name: &'static str,
    pub esc_revision: u8,
    pub esc_build: u16,
    pub fmmus: u8,
    pub sync_managers: u8,
    pub ram_kb: u8,
    pub ports: Vec<PortView>,
    pub dc: &'static str,
    pub propagation_delay_ns: u32,
    pub mailbox: Vec<&'static str>,
    pub coe: bool,
    pub sdo_info: bool,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub state: String,
    pub al_code: Option<u16>,
    pub al_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub devices: Vec<DeviceInfo>,
    /// Set when EtherCrab's init failed and the list comes from a raw scan (identity and state only).
    pub init_error: Option<String>,
    pub link: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub position: usize,
    pub state: String,
    pub al_code: Option<u16>,
    pub al_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterView {
    pub addr: u16,
    pub len: u16,
    pub name: &'static str,
    pub hex: String,
    pub value: u64,
    pub decoded: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FmmuView {
    pub index: usize,
    pub logical_start: u32,
    pub length: u16,
    pub logical_bits: (u8, u8),
    pub physical_start: u16,
    pub physical_bit: u8,
    pub kind: &'static str,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmView {
    pub index: usize,
    pub start: u16,
    pub length: u16,
    pub control: u8,
    pub mode: &'static str,
    pub direction: &'static str,
    pub status: u8,
    pub mailbox_full: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistersView {
    pub registers: Vec<RegisterView>,
    pub fmmus: Vec<FmmuView>,
    pub sync_managers: Vec<SmView>,
    /// RX errors per port (invalid frame, physical layer), forwarded errors, lost links.
    pub error_counters: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdoEntryView {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    pub data_type: u16,
    pub type_name: String,
    pub bits: u8,
    /// Bit offset in this device's input or output image.
    pub bit_offset: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdoView {
    pub index: u16,
    pub name: String,
    pub entries: Vec<PdoEntryView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdoMapView {
    pub inputs: Vec<PdoView>,
    pub outputs: Vec<PdoView>,
    /// Where the mapping came from: "CoE" (0x1C12/0x1C13) or "SII".
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectView {
    pub index: u16,
    pub name: String,
    pub object_code: &'static str,
    pub data_type: u16,
    pub type_name: String,
    pub max_sub: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    pub data_type: u16,
    pub type_name: String,
    pub bits: u16,
    pub access: String,
    pub writable: bool,
    pub value: Option<String>,
    pub raw: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleStats {
    pub running: bool,
    pub cycle_ms: f64,
    pub cycles: u64,
    pub wkc: u16,
    pub expected_wkc: u16,
    pub wkc_errors: u64,
    pub timeouts: u64,
    pub min_us: f64,
    pub avg_us: f64,
    pub max_us: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IoImage {
    pub inputs: Vec<u8>,
    pub outputs: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionEvent {
    /// Process data and statistics, about ten times a second while the cyclic exchange runs. AL states are not
    /// here: EtherCrab's state checks ride along only when they fit in the frame, so [`Session::states`] reads them.
    Io { stats: CycleStats, devices: Vec<IoImage> },
    Log { level: &'static str, message: String },
    /// Captured frames (simulator only).
    Frames { frames: Vec<FrameView> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub ts_us: u64,
    pub out: bool,
    pub datagrams: Vec<String>,
    pub bytes: Vec<u8>,
}

pub type EventSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

/// A few EtherCAT vendor IDs (ETG list) for display.
pub fn vendor_name(id: u32) -> Option<&'static str> {
    Some(match id {
        0x0000_0002 => "Beckhoff Automation",
        0x0000_006A => "Kollmorgen",
        0x0000_0083 => "Omron",
        0x0000_009A => "Elmo Motion Control",
        0x0000_01DD => "Delta Electronics",
        0x0000_0539 => "Yaskawa",
        0x0000_066F => "Panasonic",
        0x0010_0000 => "Inovance",
        _ => return None,
    })
}

struct Inner {
    md: Arc<MainDevice<'static>>,
    group: Mutex<Group>,
    /// The output image each device should get every cycle (what the user wrote), by position.
    outputs: StdMutex<Vec<Vec<u8>>>,
    /// The last input and output images seen, by position.
    images: StdMutex<Vec<IoImage>>,
    stats: StdMutex<CycleStats>,
    cycle_ms: StdMutex<f64>,
    cycling: AtomicBool,
    stop: Arc<AtomicBool>,
    mbx_lock: Mutex<()>,
    counters: Vec<AtomicU8>,
    addresses: StdMutex<Vec<u16>>,
    sii_cache: StdMutex<Vec<Option<Sii>>>,
    expected_wkc: StdMutex<u16>,
    emit: EventSink,
    capture: Arc<AtomicUsize>,
}

/// An open EtherCAT network.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
    pub segment: Option<Arc<StdMutex<Segment>>>,
    pub link: String,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn frame_view(out: bool, data: &[u8], t0: Instant) -> FrameView {
    let datagrams = wire::datagrams(data)
        .map(|spans| spans.iter().map(|s| wire::describe(&wire::read_datagram(data, s))).collect())
        .unwrap_or_else(|| vec!["(not an EtherCAT frame)".into()]);
    FrameView { ts_us: t0.elapsed().as_micros() as u64, out, datagrams, bytes: data.to_vec() }
}

impl Session {
    /// Opens the link and scans the bus. On a network interface this needs raw socket access (see
    /// [`crate::link::open_error_hint`]).
    pub async fn open(spec: LinkSpec, emit: EventSink) -> Result<(Session, ScanResult), String> {
        let storage: &'static PduStorage<MAX_FRAMES, PDU_DATA> = Box::leak(Box::new(PduStorage::new()));
        let (tx, rx, pdu_loop) = storage.try_split().map_err(|_| "PDU storage already in use".to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let capture = Arc::new(AtomicUsize::new(0));
        let (segment, link) = match &spec {
            LinkSpec::Simulator { template } => {
                let seg = Arc::new(StdMutex::new(Segment::template(*template)));
                let t0 = Instant::now();
                let cap = capture.clone();
                let sink = emit.clone();
                let buffer: Arc<StdMutex<Vec<FrameView>>> = Arc::new(StdMutex::new(Vec::new()));
                let tap: Tap = Arc::new(move |out, data| {
                    if cap.load(Ordering::Relaxed) == 0 {
                        return;
                    }
                    let left = cap.fetch_sub(1, Ordering::Relaxed);
                    let mut b = buffer.lock().unwrap();
                    b.push(frame_view(out, data, t0));
                    if b.len() >= 50 || left <= 1 {
                        sink(SessionEvent::Frames { frames: std::mem::take(&mut *b) });
                    }
                });
                let vlink = VirtualLink::new(seg.clone(), tx, rx, Some(tap));
                let stop2 = stop.clone();
                std::thread::Builder::new()
                    .name("ethercat-sim-link".into())
                    .spawn(move || futures_lite::future::block_on(futures_lite::future::or(vlink, stopped(stop2))))
                    .map_err(es)?;
                (Some(seg), format!("Simulator: {}", template.label()))
            }
            #[cfg(unix)]
            LinkSpec::Interface { name } => {
                let task = ethercrab::std::tx_rx_task(name, tx, rx).map_err(|e| crate::link::open_error_hint(&e))?;
                let stop2 = stop.clone();
                std::thread::Builder::new()
                    .name("ethercat-link".into())
                    .spawn(move || {
                        futures_lite::future::block_on(futures_lite::future::or(async { let _ = task.await; }, stopped(stop2)))
                    })
                    .map_err(es)?;
                (None, format!("Interface {name}"))
            }
            #[cfg(not(unix))]
            LinkSpec::Interface { .. } => return Err("Network interfaces are supported on macOS and Linux".into()),
        };
        // A real network answers within microseconds, so a lost frame is retried quickly; the simulator shares the
        // computer with everything else and gets patience instead.
        let simulated = segment.is_some();
        let md = Arc::new(MainDevice::new(
            pdu_loop,
            Timeouts {
                wait_loop_delay: Duration::from_millis(2),
                mailbox_response: Duration::from_millis(1000),
                state_transition: Duration::from_secs(5),
                pdu: Duration::from_millis(if simulated { 500 } else { 50 }),
                eeprom: Duration::from_millis(if simulated { 500 } else { 50 }),
                ..Default::default()
            },
            MainDeviceConfig {
                dc_static_sync_iterations: 1000,
                retry_behaviour: if simulated { ethercrab::RetryBehaviour::None } else { ethercrab::RetryBehaviour::Count(3) },
            },
        ));
        let inner = Arc::new(Inner {
            md: md.clone(),
            group: Mutex::new(Group::None),
            outputs: StdMutex::new(Vec::new()),
            images: StdMutex::new(Vec::new()),
            stats: StdMutex::new(CycleStats { cycle_ms: 10.0, ..Default::default() }),
            cycle_ms: StdMutex::new(10.0),
            cycling: AtomicBool::new(false),
            stop,
            mbx_lock: Mutex::new(()),
            counters: (0..MAX_DEVICES).map(|_| AtomicU8::new(0)).collect(),
            addresses: StdMutex::new(Vec::new()),
            sii_cache: StdMutex::new(Vec::new()),
            expected_wkc: StdMutex::new(0),
            emit,
            capture,
        });
        let session = Session { inner, segment, link };
        let scan = session.scan().await;
        match scan {
            Ok(r) => Ok((session, r)),
            Err(e) => {
                session.close();
                Err(e)
            }
        }
    }

    pub fn close(&self) {
        self.inner.cycling.store(false, Ordering::SeqCst);
        self.inner.stop.store(true, Ordering::SeqCst);
    }

    fn log(&self, level: &'static str, message: impl Into<String>) {
        (self.inner.emit)(SessionEvent::Log { level, message: message.into() });
    }

    fn md(&self) -> &MainDevice<'static> {
        &self.inner.md
    }

    fn address(&self, position: usize) -> Result<u16, String> {
        self.inner.addresses.lock().unwrap().get(position).copied().ok_or_else(|| format!("No device at position {position}"))
    }

    pub fn device_count(&self) -> usize {
        self.inner.addresses.lock().unwrap().len()
    }

    async fn scan(&self) -> Result<ScanResult, String> {
        let md = self.inner.md.clone();
        match md.init_single_group::<MAX_DEVICES, MAX_PDI>(ethercat_now).await {
            Ok(group) => {
                let addrs: Vec<u16> = group.iter(&md).map(|s| s.configured_address()).collect();
                let n = addrs.len();
                *self.inner.addresses.lock().unwrap() = addrs;
                *self.inner.sii_cache.lock().unwrap() = vec![None; n];
                *self.inner.outputs.lock().unwrap() = vec![Vec::new(); n];
                *self.inner.images.lock().unwrap() = vec![IoImage { inputs: Vec::new(), outputs: Vec::new() }; n];
                let mut devices = Vec::with_capacity(n);
                for (i, s) in group.iter(&md).enumerate() {
                    let id = s.identity();
                    let mut d = self.esc_info(i, s.configured_address()).await.unwrap_or_else(|_| blank_info(i, s.configured_address()));
                    d.alias = s.alias_address();
                    d.name = s.name().to_string();
                    d.vendor_id = id.vendor_id;
                    d.vendor_name = vendor_name(id.vendor_id);
                    d.product_code = id.product_id;
                    d.revision = id.revision;
                    d.serial = id.serial;
                    d.propagation_delay_ns = s.propagation_delay();
                    d.dc = match s.dc_support() {
                        DcSupport::None => "none",
                        DcSupport::RefOnly => "reference only",
                        DcSupport::Bits64 => "64-bit",
                        DcSupport::Bits32 => "32-bit",
                    };
                    devices.push(d);
                }
                // SII details (description, mailbox protocols) for each device.
                for d in devices.iter_mut() {
                    if let Ok(sii) = self.sii(d.position).await {
                        d.description = sii.display_name();
                        d.mailbox = sii::mailbox_protocol_names(sii.mailbox_protocols);
                        d.coe = sii.mailbox_protocols & sii::MBX_COE != 0;
                        d.sdo_info = d.coe && sii.general.as_ref().is_some_and(|g| g.coe_details & sii::COE_SDO_INFO != 0);
                    }
                }
                *self.inner.group.lock().await = Group::PreOp(group);
                // Map the process image now, while EtherCrab is the only one using the mailboxes: it reads the PDO
                // assignment over CoE with its own mailbox counter, and a device drops a message whose counter
                // repeats the last one it saw. After this, only the explorer's SDO client talks CoE.
                if let Err(e) = self.map_process_data().await {
                    self.log("error", e);
                }
                self.log("info", format!("Found {n} devices; all in PRE-OP"));
                Ok(ScanResult { devices, init_error: None, link: self.link.clone() })
            }
            Err(e) => {
                let msg = error_text(&e);
                self.log("error", format!("Init failed: {msg}. Falling back to a raw scan."));
                let devices = self.raw_scan().await;
                if devices.is_empty() {
                    return Err(format!("No EtherCAT devices answered. {msg}"));
                }
                Ok(ScanResult { devices, init_error: Some(msg), link: self.link.clone() })
            }
        }
    }

    /// Reads ESC information by auto-increment address, without configuring anything: works on a bus EtherCrab
    /// could not initialise (a device with a broken EEPROM, a mailbox it refuses).
    async fn raw_scan(&self) -> Vec<DeviceInfo> {
        let mut out = Vec::new();
        for pos in 0..MAX_DEVICES {
            let adp = 0u16.wrapping_sub(pos as u16);
            let Ok(head) = Command::aprd(adp, esc::TYPE).receive_slice(self.md(), 0x14).await else { break };
            let head = head.to_vec();
            let station = u16::from_le_bytes([head[0x10], head[0x11]]);
            let mut d = blank_info(pos, station);
            fill_esc(&mut d, &head);
            if let Ok(st) = Command::aprd(adp, esc::DL_STATUS).receive_slice(self.md(), 2).await {
                d.ports = ports_view(head[7], u16::from_le_bytes([st[0], st[1]]));
            }
            if let Ok(al) = Command::aprd(adp, esc::AL_STATUS).receive_slice(self.md(), 6).await {
                set_state(&mut d, al[0], u16::from_le_bytes([al[4], al[5]]));
            }
            d.name = format!("Device {pos}");
            out.push(d);
        }
        *self.inner.addresses.lock().unwrap() = out.iter().map(|d| d.address).collect();
        out
    }

    async fn esc_info(&self, position: usize, address: u16) -> Result<DeviceInfo, String> {
        let head = Command::fprd(address, esc::TYPE).receive_slice(self.md(), 0x14).await.map_err(|e| error_text(&e))?.to_vec();
        let mut d = blank_info(position, address);
        fill_esc(&mut d, &head);
        let st = Command::fprd(address, esc::DL_STATUS).receive_slice(self.md(), 2).await.map_err(|e| error_text(&e))?;
        d.ports = ports_view(head[7], u16::from_le_bytes([st[0], st[1]]));
        let al = Command::fprd(address, esc::AL_STATUS).receive_slice(self.md(), 6).await.map_err(|e| error_text(&e))?;
        set_state(&mut d, al[0], u16::from_le_bytes([al[4], al[5]]));
        Ok(d)
    }

    /// The whole SII EEPROM of a device, parsed (cached after the first read).
    pub async fn sii(&self, position: usize) -> Result<Sii, String> {
        if let Some(Some(s)) = self.inner.sii_cache.lock().unwrap().get(position) {
            return Ok(s.clone());
        }
        let img = self.eeprom(position).await?;
        let s = Sii::parse(&img);
        if let Some(slot) = self.inner.sii_cache.lock().unwrap().get_mut(position) {
            *slot = Some(s.clone());
        }
        Ok(s)
    }

    /// Raw EEPROM image: the declared size, read 8 bytes at a time through the SII interface registers.
    pub async fn eeprom(&self, position: usize) -> Result<Vec<u8>, String> {
        let addr = self.address(position)?;
        let head = self.eeprom_read(addr, 0, 128).await?;
        let size = ((u16::from_le_bytes([head[0x7C], head[0x7D]]) as usize + 1) * 128).clamp(128, 64 * 1024);
        let mut img = head;
        if size > img.len() {
            let rest = self.eeprom_read(addr, (img.len() / 2) as u32, size - img.len()).await?;
            img.extend_from_slice(&rest);
        }
        Ok(img)
    }

    async fn eeprom_read(&self, addr: u16, start_word: u32, len: usize) -> Result<Vec<u8>, String> {
        let md = self.md();
        let mut out = Vec::with_capacity(len);
        let mut word = start_word;
        // Take the EEPROM from the PDI if a device application holds it.
        let _ = Command::fpwr(addr, esc::SII_CONFIG).send(md, [0u8]).await;
        while out.len() < len {
            Command::fpwr(addr, esc::SII_ADDRESS).send(md, word.to_le_bytes()).await.map_err(|e| error_text(&e))?;
            Command::fpwr(addr, esc::SII_CONTROL).send(md, 0x0100u16.to_le_bytes()).await.map_err(|e| error_text(&e))?;
            let deadline = Instant::now() + Duration::from_millis(100);
            let status = loop {
                let s = Command::fprd(addr, esc::SII_CONTROL).receive_slice(md, 2).await.map_err(|e| error_text(&e))?;
                let s = u16::from_le_bytes([s[0], s[1]]);
                if s & 0x8000 == 0 {
                    break s;
                }
                if Instant::now() > deadline {
                    return Err("EEPROM busy for more than 100 ms".into());
                }
            };
            if status & 0x2000 != 0 {
                return Err("EEPROM read error (no acknowledge)".into());
            }
            let n = if status & 0x0040 != 0 { 8 } else { 4 };
            let data = Command::fprd(addr, esc::SII_DATA).receive_slice(md, n).await.map_err(|e| error_text(&e))?;
            out.extend_from_slice(&data[..n as usize]);
            word += n as u32 / 2;
        }
        out.truncate(len);
        Ok(out)
    }

    /// The ESC register view of a device.
    pub async fn registers(&self, position: usize) -> Result<RegistersView, String> {
        let addr = self.address(position)?;
        let md = self.md();
        let mut mem = vec![0u8; 0x0A00];
        for (start, len) in [(0x0000u16, 0x0150u16), (0x0300, 0x0020), (0x0400, 0x0050), (0x0500, 0x0010), (0x0600, 0x0100), (0x0800, 0x0080), (0x0900, 0x00B0)] {
            let d = Command::fprd(addr, start).receive_slice(md, len).await.map_err(|e| error_text(&e))?;
            mem[start as usize..start as usize + len as usize].copy_from_slice(&d);
        }
        let le = |a: usize, n: usize| mem[a..a + n].iter().enumerate().fold(0u64, |v, (i, &b)| v | (b as u64) << (8 * i));
        let registers = esc::REGISTERS
            .iter()
            .map(|r| {
                let a = r.addr as usize;
                let n = r.len as usize;
                let value = if n <= 8 { le(a, n) } else { 0 };
                RegisterView {
                    addr: r.addr,
                    len: r.len,
                    name: r.name,
                    hex: mem[a..a + n].iter().rev().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(""),
                    value,
                    decoded: decode_register(r.addr, value, &mem[a..a + n]),
                }
            })
            .collect();
        let fmmus = (0..mem[4].min(16) as usize)
            .map(|i| {
                let b = 0x600 + 16 * i;
                FmmuView {
                    index: i,
                    logical_start: le(b, 4) as u32,
                    length: le(b + 4, 2) as u16,
                    logical_bits: (mem[b + 6], mem[b + 7]),
                    physical_start: le(b + 8, 2) as u16,
                    physical_bit: mem[b + 10],
                    kind: match mem[b + 11] & 3 {
                        1 => "read (inputs)",
                        2 => "write (outputs)",
                        3 => "read/write",
                        _ => "unused",
                    },
                    active: mem[b + 12] & 1 != 0,
                }
            })
            .collect();
        let sync_managers = (0..mem[5].min(16) as usize)
            .map(|i| {
                let b = 0x800 + 8 * i;
                let control = mem[b + 4];
                SmView {
                    index: i,
                    start: le(b, 2) as u16,
                    length: le(b + 2, 2) as u16,
                    control,
                    mode: if control & 3 == 2 { "mailbox" } else { "buffered" },
                    direction: if (control >> 2) & 3 == 1 { "master writes" } else { "master reads" },
                    status: mem[b + 5],
                    mailbox_full: mem[b + 5] & 0x08 != 0,
                    enabled: mem[b + 6] & 1 != 0,
                }
            })
            .collect();
        let mut error_counters = Vec::new();
        for p in 0..4 {
            error_counters.push((format!("Port {p} invalid frames"), mem[0x300 + 2 * p] as u32));
            error_counters.push((format!("Port {p} RX errors"), mem[0x301 + 2 * p] as u32));
        }
        for p in 0..4 {
            error_counters.push((format!("Port {p} forwarded RX errors"), mem[0x308 + p] as u32));
        }
        error_counters.push(("Processing unit errors".into(), mem[0x30C] as u32));
        for p in 0..4 {
            error_counters.push((format!("Port {p} lost links"), mem[0x310 + p] as u32));
        }
        Ok(RegistersView { registers, fmmus, sync_managers, error_counters })
    }

    pub async fn read_register(&self, position: usize, addr: u16, len: u16) -> Result<Vec<u8>, String> {
        let a = self.address(position)?;
        Ok(Command::fprd(a, addr).receive_slice(self.md(), len).await.map_err(|e| error_text(&e))?.to_vec())
    }

    pub async fn write_register(&self, position: usize, addr: u16, data: &[u8]) -> Result<(), String> {
        let a = self.address(position)?;
        Command::fpwr(a, addr).send_receive_slice(self.md(), data).await.map_err(|e| error_text(&e))?;
        self.log("tx", format!("Device {position}: wrote {} bytes at 0x{addr:04X}", data.len()));
        Ok(())
    }

    /// Clears the RX error and lost link counters (any write to 0x0300 clears them all).
    pub async fn reset_error_counters(&self, position: usize) -> Result<(), String> {
        self.write_register(position, esc::RX_ERROR, &[0u8; 0x14]).await
    }

    // ---- AL states -------------------------------------------------------------------------------------------

    async fn read_state(&self, addr: u16) -> Result<(u8, u16), String> {
        let al = Command::fprd(addr, esc::AL_STATUS).receive_slice(self.md(), 6).await.map_err(|e| error_text(&e))?;
        Ok((al[0], u16::from_le_bytes([al[4], al[5]])))
    }

    /// Current state of every device.
    pub async fn states(&self) -> Vec<StateView> {
        let addrs = self.inner.addresses.lock().unwrap().clone();
        let mut out = Vec::with_capacity(addrs.len());
        for (i, a) in addrs.into_iter().enumerate() {
            match self.read_state(a).await {
                Ok((st, code)) => out.push(state_view(i, st, code)),
                // Behind a pulled cable or without power: the read gets no working counter.
                Err(_) => out.push(StateView { position: i, state: "no answer".into(), al_code: None, al_text: None }),
            }
        }
        out
    }

    /// Requests an AL state on some devices (all when `positions` is empty) and waits until each reaches it or
    /// reports an error. Going up, devices pass every state on the way (INIT, PRE-OP, SAFE-OP, OP), since a
    /// device refuses to skip one (0x0011); a device that fails a step stays there with its AL status code.
    pub async fn request_state(&self, target: AlState, positions: Vec<usize>) -> Result<Vec<StateView>, String> {
        let all: Vec<usize> = if positions.is_empty() { (0..self.device_count()).collect() } else { positions };
        let rank = |s: AlState| match s {
            AlState::Init => 1,
            AlState::PreOp => 2,
            AlState::SafeOp => 3,
            AlState::Op => 4,
            _ => 0,
        };
        for rung in [AlState::PreOp, AlState::SafeOp] {
            if rank(rung) >= rank(target) {
                break;
            }
            let states = self.states().await;
            let below: Vec<usize> = all
                .iter()
                .copied()
                .filter(|p| states.get(*p).is_some_and(|v| v.al_code.is_none() && rank(state_of(&v.state)) < rank(rung)))
                .collect();
            if !below.is_empty() {
                self.request_state_once(rung, below).await?;
            }
        }
        self.request_state_once(target, all).await
    }

    /// One AL state request, without intermediate steps. Process data is mapped first when the target needs it,
    /// and the cyclic exchange keeps running meanwhile so watchdogs stay fed.
    async fn request_state_once(&self, target: AlState, positions: Vec<usize>) -> Result<Vec<StateView>, String> {
        if matches!(target, AlState::SafeOp | AlState::Op) {
            self.map_process_data().await?;
            self.start_cycle();
        }
        let all = self.inner.addresses.lock().unwrap().clone();
        let picked: Vec<(usize, u16)> = if positions.is_empty() {
            all.iter().copied().enumerate().collect()
        } else {
            positions.iter().filter_map(|p| all.get(*p).map(|a| (*p, *a))).collect()
        };
        for &(_, a) in &picked {
            let (st, _) = self.read_state(a).await?;
            // Acknowledge a pending error together with the request.
            let ack = if st & 0x10 != 0 { 0x10 } else { 0 };
            Command::fpwr(a, esc::AL_CONTROL)
                .send(self.md(), (target.code() as u16 | ack).to_le_bytes())
                .await
                .map_err(|e| error_text(&e))?;
        }
        let timeout = if target == AlState::Op { Duration::from_secs(10) } else { Duration::from_secs(5) };
        let deadline = Instant::now() + timeout;
        let mut views: Vec<StateView>;
        loop {
            views = Vec::with_capacity(picked.len());
            let mut done = true;
            for &(p, a) in &picked {
                let (st, code) = self.read_state(a).await?;
                let reached = AlState::from_code(st) == target && st & 0x10 == 0;
                let failed = st & 0x10 != 0;
                if !reached && !failed {
                    done = false;
                }
                views.push(state_view(p, st, code));
            }
            if done || Instant::now() > deadline {
                break;
            }
            async_io::Timer::after(Duration::from_millis(10)).await;
        }
        for v in &views {
            if v.state != target.label() || v.al_code.is_some() {
                let why = v.al_text.clone().unwrap_or_else(|| "timed out".into());
                self.log("error", format!("Device {} stays in {}: {why}", v.position, v.state));
            }
        }
        let ok = views.iter().filter(|v| v.state == target.label() && v.al_code.is_none()).count();
        self.log("info", format!("{} of {} devices in {}", ok, views.len(), target.label()));
        if target == AlState::Init || target == AlState::PreOp {
            // Below SAFE-OP there is no process data to exchange.
            if views.len() == self.device_count() && ok == views.len() {
                self.stop_cycle();
            }
        }
        Ok(views)
    }

    /// Configures the FMMUs and the process image (once per scan).
    async fn map_process_data(&self) -> Result<(), String> {
        // EtherCrab reads the PDO assignment over CoE here; no other mailbox exchange may run meanwhile, or one
        // would take the other's answer out of the device's mailbox.
        let _mbx = self.inner.mbx_lock.lock().await;
        let mut g = self.inner.group.lock().await;
        let taken = std::mem::replace(&mut *g, Group::None);
        match taken {
            Group::PreOp(group) => match group.into_pre_op_pdi(self.md()).await {
                Ok(pdi) => {
                    let md = self.md();
                    let mut expected = 0u16;
                    let mut outs = self.inner.outputs.lock().unwrap();
                    for (i, s) in pdi.iter(md).enumerate() {
                        let io = s.io_raw();
                        expected += (!io.inputs().is_empty()) as u16 + 2 * (!io.outputs().is_empty()) as u16;
                        if let Some(o) = outs.get_mut(i) {
                            *o = vec![0u8; io.outputs().len()];
                        }
                    }
                    drop(outs);
                    *self.inner.expected_wkc.lock().unwrap() = expected;
                    *g = Group::Pdi(pdi);
                    self.log("info", "Process data image mapped (FMMUs set); devices stay in PRE-OP");
                    Ok(())
                }
                Err(e) => Err(format!("Mapping process data failed: {} Rescan the bus.", with_period(&error_text(&e)))),
            },
            Group::Pdi(p) => {
                *g = Group::Pdi(p);
                Ok(())
            }
            Group::None => Err("The bus was not initialised; rescan it first".into()),
        }
    }

    pub fn set_cycle_ms(&self, ms: f64) {
        *self.inner.cycle_ms.lock().unwrap() = ms.clamp(1.0, 1000.0);
    }

    pub fn cycling(&self) -> bool {
        self.inner.cycling.load(Ordering::SeqCst)
    }

    pub fn stop_cycle(&self) {
        self.inner.cycling.store(false, Ordering::SeqCst);
    }

    /// Starts the cyclic exchange on its own thread (no-op if running).
    pub fn start_cycle(&self) {
        if self.inner.cycling.swap(true, Ordering::SeqCst) {
            return;
        }
        let inner = self.inner.clone();
        let _ = std::thread::Builder::new().name("ethercat-cycle".into()).spawn(move || {
            futures_lite::future::block_on(cycle_loop(inner));
        });
    }

    /// Sets bits in a device's output image (sent every cycle). `bits` up to 64, little endian.
    pub fn write_output(&self, position: usize, bit_offset: usize, bits: usize, value: u64) -> Result<(), String> {
        let mut outs = self.inner.outputs.lock().unwrap();
        let img = outs.get_mut(position).ok_or("No such device")?;
        if img.is_empty() {
            return Err("This device has no outputs mapped; start process data first".into());
        }
        if bit_offset + bits > img.len() * 8 {
            return Err("Outside the device's output image".into());
        }
        for b in 0..bits {
            let at = bit_offset + b;
            if value >> b & 1 != 0 {
                img[at / 8] |= 1 << (at % 8);
            } else {
                img[at / 8] &= !(1 << (at % 8));
            }
        }
        Ok(())
    }

    pub fn images(&self) -> Vec<IoImage> {
        self.inner.images.lock().unwrap().clone()
    }

    pub fn stats(&self) -> CycleStats {
        self.inner.stats.lock().unwrap().clone()
    }

    /// Captures the next `n` frames (simulator link only); they arrive as [`SessionEvent::Frames`].
    pub fn capture(&self, n: usize) -> Result<(), String> {
        if self.segment.is_none() {
            return Err("Frame capture works with the simulator. On a network interface, use Wireshark (it decodes EtherCAT).".into());
        }
        self.inner.capture.store(n, Ordering::SeqCst);
        Ok(())
    }

    // ---- PDO mapping ----------------------------------------------------------------------------------------

    /// The device's process data layout: from CoE (assignment and mapping objects, with names from SDO Info)
    /// when it has CoE, else from its SII.
    pub async fn pdo_map(&self, position: usize) -> Result<PdoMapView, String> {
        let sii = self.sii(position).await?;
        let coe_ok = sii.mailbox_protocols & sii::MBX_COE != 0;
        if coe_ok {
            if let Ok(v) = self.pdo_map_coe(position, &sii).await {
                return Ok(v);
            }
        }
        let view = |pdos: &[sii::SiiPdo]| {
            let mut bit = 0usize;
            pdos.iter()
                .filter(|p| p.sm != 0xFF)
                .map(|p| PdoView {
                    index: p.index,
                    name: p.name.clone(),
                    entries: p
                        .entries
                        .iter()
                        .map(|e| {
                            let v = PdoEntryView {
                                index: e.index,
                                sub: e.sub,
                                name: e.name.clone(),
                                data_type: e.data_type as u16,
                                type_name: if e.index == 0 { "gap".into() } else { coe::data_type_name(e.data_type as u16) },
                                bits: e.bits,
                                bit_offset: bit,
                            };
                            bit += e.bits as usize;
                            v
                        })
                        .collect(),
                })
                .collect::<Vec<_>>()
        };
        Ok(PdoMapView { inputs: view(&sii.tx_pdos), outputs: view(&sii.rx_pdos), source: "SII" })
    }

    async fn pdo_map_coe(&self, position: usize, sii: &Sii) -> Result<PdoMapView, String> {
        let names = |idx: u16, sub: u8| -> Option<(String, u8)> {
            sii.tx_pdos
                .iter()
                .chain(sii.rx_pdos.iter())
                .flat_map(|p| p.entries.iter())
                .find(|e| e.index == idx && e.sub == sub)
                .map(|e| (e.name.clone(), e.data_type))
        };
        let pdo_name = |idx: u16| sii.tx_pdos.iter().chain(sii.rx_pdos.iter()).find(|p| p.index == idx).map(|p| p.name.clone());
        let mut dirs = Vec::new();
        for assign in [0x1C13u16, 0x1C12] {
            let n = self.sdo_upload(position, assign, 0).await?.first().copied().unwrap_or(0);
            let mut pdos = Vec::new();
            let mut bit = 0usize;
            for i in 1..=n {
                let pdo = u16::from_le_bytes(self.sdo_upload(position, assign, i).await?.get(..2).ok_or("short")?.try_into().unwrap());
                let m = self.sdo_upload(position, pdo, 0).await?.first().copied().unwrap_or(0);
                let mut entries = Vec::new();
                for j in 1..=m {
                    let raw = self.sdo_upload(position, pdo, j).await?;
                    let v = u32::from_le_bytes(raw.get(..4).ok_or("short")?.try_into().unwrap());
                    let (index, sub, bits) = ((v >> 16) as u16, (v >> 8) as u8, v as u8);
                    let (name, dt) = if index == 0 {
                        (String::new(), 0u16)
                    } else if let Some((n, dt)) = names(index, sub) {
                        (n, dt as u16)
                    } else {
                        match self.entry_desc(position, index, sub).await {
                            Ok(e) => (e.name, e.data_type),
                            Err(_) => (format!("0x{index:04X}:{sub:02X}"), 0),
                        }
                    };
                    entries.push(PdoEntryView {
                        index,
                        sub,
                        name,
                        data_type: dt,
                        type_name: if index == 0 { "gap".into() } else if dt == 0 { format!("{bits} bits") } else { coe::data_type_name(dt) },
                        bits,
                        bit_offset: bit,
                    });
                    bit += bits as usize;
                }
                let name = match pdo_name(pdo) {
                    Some(n) => n,
                    None => self.object_desc(position, pdo).await.map(|o| o.name).unwrap_or_default(),
                };
                pdos.push(PdoView { index: pdo, name, entries });
            }
            dirs.push(pdos);
        }
        let outputs = dirs.pop().unwrap_or_default();
        let inputs = dirs.pop().unwrap_or_default();
        Ok(PdoMapView { inputs, outputs, source: "CoE" })
    }

    // ---- mailbox and CoE ------------------------------------------------------------------------------------

    async fn mailbox_layout(&self, addr: u16) -> Result<((u16, u16), (u16, u16), u16), String> {
        let sms = Command::fprd(addr, esc::sm(0)).receive_slice(self.md(), 16).await.map_err(|e| error_text(&e))?;
        let rx = (u16::from_le_bytes([sms[0], sms[1]]), u16::from_le_bytes([sms[2], sms[3]]));
        let tx = (u16::from_le_bytes([sms[8], sms[9]]), u16::from_le_bytes([sms[10], sms[11]]));
        if rx.1 == 0 || tx.1 == 0 || sms[4] & 3 != 2 || sms[12] & 3 != 2 {
            return Err("This device has no mailbox configured (it may not support CoE, or it is in INIT)".into());
        }
        Ok((rx, tx, esc::sm(1) + 5))
    }

    fn next_counter(&self, position: usize) -> u8 {
        let c = &self.inner.counters[position.min(MAX_DEVICES - 1)];
        let mut v = c.load(Ordering::Relaxed);
        v = if v >= 7 { 1 } else { v + 1 };
        c.store(v, Ordering::Relaxed);
        v
    }

    /// Reads one message from the device-to-master mailbox, waiting up to `timeout`.
    async fn mailbox_read(&self, addr: u16, tx: (u16, u16), timeout: Duration) -> Result<(u8, Vec<u8>), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match Command::fprd(addr, tx.0).with_wkc(1).receive_slice(self.md(), tx.1).await {
                Ok(d) => {
                    let h = coe::MbxHeader::parse(&d).ok_or("Mailbox message too short")?;
                    let end = (coe::MBX_HEADER + h.len as usize).min(d.len());
                    return Ok((h.kind, d[coe::MBX_HEADER..end].to_vec()));
                }
                Err(ethercrab::error::Error::WorkingCounter { .. }) => {}
                Err(e) => return Err(error_text(&e)),
            }
            if Instant::now() > deadline {
                return Err("No mailbox answer".into());
            }
            async_io::Timer::after(Duration::from_millis(1)).await;
        }
    }

    /// Sends a CoE message and returns the first answer's body. A repeated mailbox counter makes a device ignore
    /// the message, so a silent device gets it once more with the next counter.
    async fn coe_exchange(&self, position: usize, body: &[u8]) -> Result<Vec<u8>, String> {
        let addr = self.address(position)?;
        let (rx, tx, tx_status) = self.mailbox_layout(addr).await?;
        // Throw away a stale message left in the device-to-master mailbox.
        if let Ok(s) = Command::fprd(addr, tx_status).receive_slice(self.md(), 1).await {
            if s[0] & 0x08 != 0 {
                let _ = Command::fprd(addr, tx.0).ignore_wkc().receive_slice(self.md(), tx.1).await;
            }
        }
        for attempt in 0..2 {
            let mut msg = coe::mailbox(coe::MBX_TYPE_COE, self.next_counter(position), body);
            if msg.len() > rx.1 as usize {
                return Err("Request larger than the device's mailbox".into());
            }
            msg.resize(rx.1 as usize, 0);
            let deadline = Instant::now() + Duration::from_millis(500);
            loop {
                match Command::fpwr(addr, rx.0).with_wkc(1).send_receive_slice(self.md(), &msg[..]).await {
                    Ok(_) => break,
                    Err(ethercrab::error::Error::WorkingCounter { .. }) if Instant::now() < deadline => {
                        async_io::Timer::after(Duration::from_millis(2)).await;
                    }
                    Err(e) => return Err(format!("Mailbox write refused: {}", error_text(&e))),
                }
            }
            let wait = if attempt == 0 { Duration::from_millis(400) } else { Duration::from_millis(1500) };
            match self.mailbox_read(addr, tx, wait).await {
                Ok((coe::MBX_TYPE_COE, b)) => return Ok(b),
                Ok((coe::MBX_TYPE_ERROR, b)) => {
                    let code = u16::from_le_bytes([b.get(2).copied().unwrap_or(0), b.get(3).copied().unwrap_or(0)]);
                    return Err(format!("Mailbox error 0x{code:04X}: {}", coe::mailbox_error_text(code)));
                }
                Ok((kind, _)) => return Err(format!("Unexpected mailbox message type {kind}")),
                Err(e) if attempt == 0 && e == "No mailbox answer" => continue,
                Err(e) => return Err(e),
            }
        }
        Err("No mailbox answer".into())
    }

    /// SDO upload of one entry (expedited, normal or segmented).
    pub async fn sdo_upload(&self, position: usize, index: u16, sub: u8) -> Result<Vec<u8>, String> {
        let _g = self.inner.mbx_lock.lock().await;
        let mut resp = self.coe_exchange(position, &coe::upload_request(index, sub)).await?;
        let mut data = Vec::new();
        let mut toggle = false;
        loop {
            match coe::parse_sdo_response(&resp)? {
                SdoResponse::Upload { data: d, more, .. } => {
                    data.extend_from_slice(&d);
                    if !more {
                        return Ok(data);
                    }
                }
                SdoResponse::UploadSegment { data: d, last, .. } => {
                    data.extend_from_slice(&d);
                    if last {
                        return Ok(data);
                    }
                    toggle = !toggle;
                }
                SdoResponse::Abort { code, .. } => {
                    return Err(format!("SDO abort 0x{code:08X}: {}", coe::abort_text(code)));
                }
                other => return Err(format!("Unexpected SDO answer {other:?}")),
            }
            resp = self.coe_exchange(position, &coe::upload_segment_request(toggle)).await?;
        }
    }

    /// SDO download (expedited up to 4 bytes, else normal plus segments).
    pub async fn sdo_download(&self, position: usize, index: u16, sub: u8, data: &[u8]) -> Result<(), String> {
        let _g = self.inner.mbx_lock.lock().await;
        let addr = self.address(position)?;
        let (rx, _, _) = self.mailbox_layout(addr).await?;
        let (req, mut sent) = coe::download_request(index, sub, data, rx.1 as usize);
        let check = |r: Vec<u8>| -> Result<(), String> {
            match coe::parse_sdo_response(&r)? {
                SdoResponse::Download | SdoResponse::DownloadSegment { .. } => Ok(()),
                SdoResponse::Abort { code, .. } => Err(format!("SDO abort 0x{code:08X}: {}", coe::abort_text(code))),
                other => Err(format!("Unexpected SDO answer {other:?}")),
            }
        };
        check(self.coe_exchange(position, &req).await?)?;
        let room = (rx.1 as usize).saturating_sub(coe::MBX_HEADER + 3).max(7);
        let mut toggle = false;
        while sent < data.len() {
            let end = (sent + room).min(data.len());
            let seg = coe::download_segment_request(toggle, end == data.len(), &data[sent..end]);
            check(self.coe_exchange(position, &seg).await?)?;
            sent = end;
            toggle = !toggle;
        }
        self.log("tx", format!("Device {position}: SDO 0x{index:04X}:{sub:02X} written ({} bytes)", data.len()));
        Ok(())
    }

    async fn info_exchange(&self, position: usize, req: &[u8]) -> Result<Vec<u8>, String> {
        let addr = self.address(position)?;
        let (_, tx, _) = self.mailbox_layout(addr).await?;
        let first = self.coe_exchange(position, req).await?;
        let mut f = coe::parse_info(&first)?;
        if f.opcode == coe::INFO_ERROR {
            let code = u32::from_le_bytes(f.payload.get(..4).ok_or("short")?.try_into().unwrap());
            return Err(format!("SDO Info error 0x{code:08X}: {}", coe::abort_text(code)));
        }
        let mut payload = f.payload.clone();
        while f.incomplete {
            let (_, b) = self.mailbox_read(addr, tx, Duration::from_millis(1000)).await?;
            f = coe::parse_info(&b)?;
            payload.extend_from_slice(&f.payload);
        }
        Ok(payload)
    }

    /// The object dictionary's index list and object descriptions (SDO Information).
    pub async fn object_list(&self, position: usize) -> Result<Vec<ObjectView>, String> {
        let list = {
            let _g = self.inner.mbx_lock.lock().await;
            self.info_exchange(position, &coe::od_list_request(coe::LIST_ALL)).await?
        };
        let indices: Vec<u16> = list.get(2..).unwrap_or(&[]).chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut out = Vec::with_capacity(indices.len());
        for i in indices {
            match self.object_desc(position, i).await {
                Ok(o) => out.push(o),
                Err(_) => out.push(ObjectView {
                    index: i,
                    name: String::new(),
                    object_code: "?",
                    data_type: 0,
                    type_name: String::new(),
                    max_sub: 0,
                }),
            }
        }
        Ok(out)
    }

    pub async fn object_desc(&self, position: usize, index: u16) -> Result<ObjectView, String> {
        let p = {
            let _g = self.inner.mbx_lock.lock().await;
            self.info_exchange(position, &coe::object_desc_request(index)).await?
        };
        let d = coe::parse_object_desc(&p)?;
        Ok(ObjectView {
            index: d.index,
            name: d.name,
            object_code: coe::object_code_name(d.object_code),
            data_type: d.data_type,
            type_name: coe::data_type_name(d.data_type),
            max_sub: d.max_sub,
        })
    }

    async fn entry_desc(&self, position: usize, index: u16, sub: u8) -> Result<coe::EntryDesc, String> {
        let p = {
            let _g = self.inner.mbx_lock.lock().await;
            self.info_exchange(position, &coe::entry_desc_request(index, sub)).await?
        };
        coe::parse_entry_desc(&p)
    }

    /// The entries of one object with descriptions and current values.
    pub async fn object_entries(&self, position: usize, index: u16) -> Result<Vec<EntryView>, String> {
        let obj = self.object_desc(position, index).await?;
        let state = self.read_state(self.address(position)?).await.map(|s| AlState::from_code(s.0)).unwrap_or(AlState::PreOp);
        let mut out = Vec::new();
        let last = if obj.object_code == "VAR" { 0 } else { obj.max_sub };
        for sub in 0..=last {
            let Ok(e) = self.entry_desc(position, index, sub).await else { continue };
            let readable = e.access & coe::ACC_RO != 0;
            let (value, raw, error) = if readable {
                match self.sdo_upload(position, index, sub).await {
                    Ok(b) => (Some(coe::format_value(e.data_type, &b)), Some(hex(&b)), None),
                    Err(err) => (None, None, Some(err)),
                }
            } else {
                (None, None, None)
            };
            let wbit = match state {
                AlState::Op => coe::ACC_WRITE_OP,
                AlState::SafeOp => coe::ACC_WRITE_SAFEOP,
                _ => coe::ACC_WRITE_PREOP,
            };
            out.push(EntryView {
                index,
                sub,
                name: e.name,
                data_type: e.data_type,
                type_name: coe::data_type_name(e.data_type),
                bits: e.bits,
                access: coe::access_text(e.access),
                writable: e.access & wbit != 0,
                value,
                raw,
                error,
            });
        }
        Ok(out)
    }
}

fn with_period(s: &str) -> String {
    if s.ends_with('.') { s.to_string() } else { format!("{s}.") }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

async fn stopped(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::Relaxed) {
        async_io::Timer::after(Duration::from_millis(50)).await;
    }
}

fn blank_info(position: usize, address: u16) -> DeviceInfo {
    DeviceInfo {
        position,
        address,
        alias: 0,
        name: String::new(),
        description: String::new(),
        vendor_id: 0,
        vendor_name: None,
        product_code: 0,
        revision: 0,
        serial: 0,
        esc_type: 0,
        esc_name: "",
        esc_revision: 0,
        esc_build: 0,
        fmmus: 0,
        sync_managers: 0,
        ram_kb: 0,
        ports: Vec::new(),
        dc: "none",
        propagation_delay_ns: 0,
        mailbox: Vec::new(),
        coe: false,
        sdo_info: false,
        input_bytes: 0,
        output_bytes: 0,
        state: "?".into(),
        al_code: None,
        al_text: None,
    }
}

fn fill_esc(d: &mut DeviceInfo, head: &[u8]) {
    d.esc_type = head[0];
    d.esc_name = esc::esc_type_name(head[0]);
    d.esc_revision = head[1];
    d.esc_build = u16::from_le_bytes([head[2], head[3]]);
    d.fmmus = head[4];
    d.sync_managers = head[5];
    d.ram_kb = head[6];
    let features = u16::from_le_bytes([head[8], head[9]]);
    // As EtherCrab reads it: DC SYNC needs the enhanced activation bit, else the clock is a reference only.
    d.dc = match (features & 0x04 != 0, features & 0x0100 != 0, features & 0x08 != 0) {
        (false, _, _) => "none",
        (true, false, _) => "reference only",
        (true, true, true) => "64-bit",
        (true, true, false) => "32-bit",
    };
    d.alias = u16::from_le_bytes([head[0x12], head[0x13]]);
}

fn ports_view(desc: u8, dl: u16) -> Vec<PortView> {
    let st = esc::dl_status_ports(dl);
    (0..4)
        .map(|p| PortView { kind: esc::port_kind(desc, p), link: st[p].link, loop_closed: st[p].loop_closed, communication: st[p].communication })
        .collect()
}

fn state_view(position: usize, st: u8, code: u16) -> StateView {
    let err = st & 0x10 != 0;
    StateView {
        position,
        state: AlState::from_code(st).label().to_string(),
        al_code: err.then_some(code),
        al_text: err.then(|| format!("0x{code:04X} {}", esc::al_status_text(code))),
    }
}

fn state_of(label: &str) -> AlState {
    [AlState::Init, AlState::PreOp, AlState::Boot, AlState::SafeOp, AlState::Op].into_iter().find(|s| s.label() == label).unwrap_or(AlState::Unknown)
}

fn set_state(d: &mut DeviceInfo, st: u8, code: u16) {
    let v = state_view(d.position, st, code);
    d.state = v.state;
    d.al_code = v.al_code;
    d.al_text = v.al_text;
}

fn decode_register(addr: u16, v: u64, raw: &[u8]) -> String {
    match addr {
        esc::TYPE => esc::esc_type_name(v as u8).to_string(),
        esc::PORT_DESC => (0..4).map(|p| format!("P{p} {}", esc::port_kind(v as u8, p))).collect::<Vec<_>>().join(", "),
        esc::FEATURES => {
            let mut f = Vec::new();
            if v & 0x04 != 0 {
                f.push(if v & 0x08 != 0 { "DC 64-bit" } else { "DC 32-bit" });
            }
            if v & 0x10 != 0 {
                f.push("low jitter EBUS");
            }
            if v & 0x0100 != 0 {
                f.push("enhanced DC SYNC activation");
            }
            f.join(", ")
        }
        esc::DL_STATUS => {
            let p = esc::dl_status_ports(v as u16);
            (0..4)
                .map(|i| {
                    format!(
                        "P{i} {}",
                        if p[i].communication {
                            "comm"
                        } else if p[i].link {
                            "link"
                        } else if p[i].loop_closed {
                            "closed"
                        } else {
                            "open"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        esc::AL_CONTROL | esc::AL_STATUS => {
            let s = AlState::from_code(v as u8).label();
            if v & 0x10 != 0 { format!("{s} + error") } else { s.to_string() }
        }
        esc::AL_STATUS_CODE => esc::al_status_text(v as u16).to_string(),
        esc::SII_CONTROL => {
            let mut f = vec![if v & 0x40 != 0 { "8-byte reads" } else { "4-byte reads" }];
            if v & 0x8000 != 0 {
                f.push("busy");
            }
            if v & 0x0800 != 0 {
                f.push("checksum error");
            }
            if v & 0x2000 != 0 {
                f.push("no acknowledge");
            }
            f.join(", ")
        }
        esc::DC_SYSTEM_TIME | esc::DC_RECV_TIME_PU | esc::DC_SYSTEM_OFFSET => format!("{v} ns"),
        esc::DC_SYSTEM_DELAY => format!("{v} ns"),
        esc::DC_SYSTEM_DIFF => {
            let mag = v & 0x7FFF_FFFF;
            format!("{}{mag} ns", if v & 0x8000_0000 != 0 { "-" } else { "+" })
        }
        esc::DC_RECV_TIME => raw
            .chunks_exact(4)
            .enumerate()
            .map(|(p, c)| format!("P{p} {}", u32::from_le_bytes(c.try_into().unwrap())))
            .collect::<Vec<_>>()
            .join(", "),
        esc::WATCHDOG_PD | esc::WATCHDOG_PDI => format!("{v} x divider"),
        esc::WATCHDOG_STATUS => (if v & 1 != 0 { "running" } else { "expired" }).to_string(),
        esc::RX_ERROR => raw
            .chunks_exact(2)
            .enumerate()
            .map(|(p, c)| format!("P{p} {}/{}", c[0], c[1]))
            .collect::<Vec<_>>()
            .join(", "),
        esc::LOST_LINK => raw.iter().enumerate().map(|(p, c)| format!("P{p} {c}")).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

async fn cycle_loop(inner: Arc<Inner>) {
    let mut next = Instant::now();
    // The first snapshot goes out after the first cycle, then ten a second.
    let mut last_emit = Instant::now() - Duration::from_millis(100);
    let mut sum_us = 0f64;
    let mut n_us = 0u64;
    {
        let mut s = inner.stats.lock().unwrap();
        *s = CycleStats { running: true, cycle_ms: *inner.cycle_ms.lock().unwrap(), min_us: f64::MAX, ..Default::default() };
    }
    let mut consecutive_timeouts = 0u32;
    while inner.cycling.load(Ordering::SeqCst) && !inner.stop.load(Ordering::SeqCst) {
        let period = Duration::from_secs_f64(*inner.cycle_ms.lock().unwrap() / 1000.0);
        let t0 = Instant::now();
        let mut result: Option<Result<u16, String>> = None;
        {
            let g = inner.group.lock().await;
            if let Group::Pdi(group) = &*g {
                let md = &*inner.md;
                {
                    let outs = inner.outputs.lock().unwrap();
                    for (i, s) in group.iter(md).enumerate() {
                        if let Some(o) = outs.get(i) {
                            let mut raw = s.outputs_raw_mut();
                            let n = raw.len().min(o.len());
                            raw[..n].copy_from_slice(&o[..n]);
                        }
                    }
                }
                result = Some(match group.tx_rx(md).await {
                    Ok(r) => Ok(r.working_counter),
                    Err(e) => Err(error_text(&e)),
                });
                let mut imgs = inner.images.lock().unwrap();
                for (i, s) in group.iter(md).enumerate() {
                    if let Some(slot) = imgs.get_mut(i) {
                        let io = s.io_raw();
                        *slot = IoImage { inputs: io.inputs().to_vec(), outputs: io.outputs().to_vec() };
                    }
                }
            }
        }
        let us = t0.elapsed().as_secs_f64() * 1e6;
        let expected = *inner.expected_wkc.lock().unwrap();
        {
            let mut s = inner.stats.lock().unwrap();
            s.cycle_ms = period.as_secs_f64() * 1000.0;
            s.expected_wkc = expected;
            match &result {
                Some(Ok(wkc)) => {
                    consecutive_timeouts = 0;
                    s.cycles += 1;
                    s.wkc = *wkc;
                    if *wkc != expected {
                        s.wkc_errors += 1;
                    }
                    s.min_us = s.min_us.min(us);
                    s.max_us = s.max_us.max(us);
                    sum_us += us;
                    n_us += 1;
                    s.avg_us = sum_us / n_us as f64;
                }
                Some(Err(_)) => {
                    s.timeouts += 1;
                    consecutive_timeouts += 1;
                }
                None => {}
            }
        }
        if consecutive_timeouts == 20 {
            (inner.emit)(SessionEvent::Log {
                level: "error",
                message: "20 cycles in a row got no answer: the cable may be unplugged or the devices off".into(),
            });
        }
        if last_emit.elapsed() >= Duration::from_millis(100) {
            last_emit = Instant::now();
            let mut stats = inner.stats.lock().unwrap().clone();
            if stats.min_us == f64::MAX {
                stats.min_us = 0.0;
            }
            (inner.emit)(SessionEvent::Io {
                stats,
                devices: inner.images.lock().unwrap().clone(),
            });
        }
        next += period;
        let now = Instant::now();
        if next < now {
            next = now;
        }
        async_io::Timer::at(next).await;
    }
    inner.stats.lock().unwrap().running = false;
    let mut stats = inner.stats.lock().unwrap().clone();
    if stats.min_us == f64::MAX {
        stats.min_us = 0.0;
    }
    (inner.emit)(SessionEvent::Io { stats, devices: inner.images.lock().unwrap().clone() });
}

/// Decodes a value from a byte image at a bit offset (for process data entries), little endian.
pub fn extract_bits(img: &[u8], bit_offset: usize, bits: usize) -> u64 {
    let mut v = 0u64;
    for b in 0..bits.min(64) {
        let at = bit_offset + b;
        if at / 8 < img.len() && img[at / 8] >> (at % 8) & 1 != 0 {
            v |= 1 << b;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits() {
        let img = [0b1010_0000u8, 0x34, 0x12];
        assert_eq!(extract_bits(&img, 5, 3), 0b101);
        assert_eq!(extract_bits(&img, 8, 16), 0x1234);
    }
}
