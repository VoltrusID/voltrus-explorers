//! Cypress EZ-USB FX2(LP) logic analyzers (the "24 MHz 8-channel" boards) running the open
//! fx2lafw firmware.
//!
//! **Untested on hardware.** Everything here is written from the fx2lafw firmware's USB
//! interface (its `command.h` request codes and the FX2's documented firmware-load request) and
//! is tested only against [`mock::MockFx2`], an emulation of the chip. No sigrok/libsigrok code
//! is used or linked.
//!
//! Flow: find a known board by VID:PID; if it does not answer the fx2lafw version request, hold
//! the 8051 in reset, write the firmware image into its RAM with vendor request 0xA0, release the
//! reset, and wait for the board to re-enumerate as fx2lafw; then send `CMD_START` with the clock
//! and divider for the sample rate and stream samples from bulk endpoint 2 (one byte per sample).
//!
//! USB I/O sits behind [`UsbBackend`] / [`UsbLink`], so the driver logic runs in tests without a
//! device; [`nusb_backend`] is the real one.

pub mod firmware;
pub mod mock;
#[cfg(feature = "usb")]
pub mod nusb_backend;

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::engine::SampleStream;
use crate::error::{LogicError, Result};

// fx2lafw vendor requests.
pub const CMD_GET_FW_VERSION: u8 = 0xB0;
pub const CMD_START: u8 = 0xB1;
pub const CMD_GET_REVID_VERSION: u8 = 0xB2;
// CMD_START flags.
pub const FLAG_CLK_CTL2: u8 = 1 << 4;
pub const FLAG_SAMPLE_16BIT: u8 = 1 << 5;
pub const FLAG_CLK_48MHZ: u8 = 1 << 6;
/// The FX2's own firmware load request (handled by the chip, no firmware needed) and its CPU
/// control/status register.
pub const REQ_FIRMWARE_LOAD: u8 = 0xA0;
pub const CPUCS: u16 = 0xE600;
/// Bulk IN endpoint 2.
pub const EP_DATA: u8 = 0x82;
/// The fx2lafw major version this driver speaks.
pub const FW_MAJOR: u8 = 1;
/// The divider fits in 5 delay states of 256 cycles plus one of up to 255 (`sample_delay_h` < 6).
pub const MAX_DELAY: u32 = 6 * 256 - 1;

/// Sample rates offered for fx2lafw boards (8 channels). 24 MHz needs a fast, otherwise idle USB 2.0 bus.
pub const SAMPLE_RATES: [u64; 16] = [
    20_000, 25_000, 50_000, 100_000, 200_000, 250_000, 500_000, 1_000_000, 2_000_000, 3_000_000, 4_000_000, 6_000_000, 8_000_000, 12_000_000, 16_000_000, 24_000_000,
];

/// A board family: USB id before the firmware is loaded and the fx2lafw image for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub firmware: &'static str,
}

/// Boards fx2lafw supports, by the USB id they enumerate with out of the box.
pub const BOARDS: [Board; 9] = [
    Board { vid: 0x0925, pid: 0x3881, name: "Saleae Logic compatible (24 MHz 8 ch)", firmware: "fx2lafw-saleae-logic.fw" },
    Board { vid: 0x04B4, pid: 0x8613, name: "Cypress FX2 board (CY7C68013A, blank EEPROM)", firmware: "fx2lafw-cypress-fx2.fw" },
    Board { vid: 0x1D50, pid: 0x608C, name: "sigrok FX2 8 ch (WeAct and similar)", firmware: "fx2lafw-sigrok-fx2-8ch.fw" },
    Board { vid: 0x1D50, pid: 0x608D, name: "sigrok FX2 16 ch (first 8 channels)", firmware: "fx2lafw-sigrok-fx2-16ch.fw" },
    Board { vid: 0x08A9, pid: 0x0014, name: "CWAV USBee AX compatible", firmware: "fx2lafw-cwav-usbeeax.fw" },
    Board { vid: 0x08A9, pid: 0x0015, name: "CWAV USBee DX compatible (first 8 channels)", firmware: "fx2lafw-cwav-usbeedx.fw" },
    Board { vid: 0x08A9, pid: 0x0009, name: "CWAV USBee SX compatible", firmware: "fx2lafw-cwav-usbeesx.fw" },
    Board { vid: 0x08A9, pid: 0x0005, name: "CWAV USBee ZX compatible", firmware: "fx2lafw-cwav-usbeezx.fw" },
    Board { vid: 0x16D0, pid: 0x0498, name: "Braintechnology USB-LPS", firmware: "fx2lafw-braintechnology-usb-lps.fw" },
];

pub fn board_for(vid: u16, pid: u16) -> Option<&'static Board> {
    BOARDS.iter().find(|b| b.vid == vid && b.pid == pid)
}

/// The 3-byte `CMD_START` payload (flags, delay high, delay low) for `rate`: the 48 MHz clock
/// when it divides evenly, else 30 MHz; samples are taken every `delay + 1` clocks.
pub fn start_command(rate: u64) -> Result<[u8; 3]> {
    for (base, flag) in [(48_000_000u64, FLAG_CLK_48MHZ), (30_000_000, 0)] {
        if rate > 0 && rate <= base / 2 && base.is_multiple_of(rate) {
            let delay = (base / rate - 1) as u32;
            if delay <= MAX_DELAY {
                return Ok([flag, (delay >> 8) as u8, delay as u8]);
            }
        }
    }
    Err(LogicError::Config(format!("{} is not a sample rate an FX2 board can produce", crate::fmt_rate(rate))))
}

/// The rate a `CMD_START` payload produces (for the emulator and for checks).
pub fn rate_of(cmd: [u8; 3]) -> u64 {
    let base = if cmd[0] & FLAG_CLK_48MHZ != 0 { 48_000_000 } else { 30_000_000 };
    base / ((cmd[1] as u64) << 8 | cmd[2] as u64).saturating_add(1)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDeviceInfo {
    /// Stable for as long as the device stays plugged in (and enumerated the same way).
    pub id: String,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// Bus and port, to find the same board after it re-enumerates.
    pub location: String,
}

impl UsbDeviceInfo {
    /// The board announces itself as fx2lafw (the firmware is already running).
    pub fn runs_fx2lafw(&self) -> bool {
        self.manufacturer.as_deref() == Some("sigrok") || self.product.as_deref().is_some_and(|p| p.contains("fx2lafw"))
    }
}

pub trait UsbBackend: Send + Sync {
    fn list(&self) -> Result<Vec<UsbDeviceInfo>>;
    fn open(&self, id: &str) -> Result<Box<dyn UsbLink>>;
}

/// One opened device with interface 0 claimed.
pub trait UsbLink: Send {
    /// Vendor request to the device, host to device.
    fn control_out(&mut self, request: u8, value: u16, index: u16, data: &[u8]) -> Result<()>;
    /// Vendor request to the device, device to host.
    fn control_in(&mut self, request: u8, value: u16, index: u16, len: u16) -> Result<Vec<u8>>;
    /// Keeps `transfers` bulk IN transfers of `size` bytes queued on `ep`.
    fn start_bulk(&mut self, ep: u8, size: usize, transfers: usize) -> Result<()>;
    /// The next completed bulk transfer, `None` on timeout.
    fn read_bulk(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>>;
}

/// A board found on the bus, for the device list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub info: UsbDeviceInfo,
    pub board: Board,
    pub firmware_loaded: bool,
}

/// Supported boards that are plugged in.
pub fn detect(backend: &dyn UsbBackend) -> Result<Vec<Detected>> {
    Ok(backend
        .list()?
        .into_iter()
        .filter_map(|info| {
            let board = *board_for(info.vid, info.pid)?;
            let firmware_loaded = info.runs_fx2lafw();
            Some(Detected { info, board, firmware_loaded })
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FwVersion {
    pub major: u8,
    pub minor: u8,
    pub revid: Option<u8>,
}

fn probe_version(link: &mut dyn UsbLink) -> Option<FwVersion> {
    let v = link.control_in(CMD_GET_FW_VERSION, 0, 0, 2).ok()?;
    if v.len() != 2 {
        return None;
    }
    let revid = link.control_in(CMD_GET_REVID_VERSION, 0, 0, 1).ok().and_then(|r| r.first().copied());
    Some(FwVersion { major: v[0], minor: v[1], revid })
}

/// Writes a firmware image into the FX2's RAM and starts it.
pub fn upload_firmware(link: &mut dyn UsbLink, image: &[u8]) -> Result<()> {
    firmware::check_image(image)?;
    link.control_out(REQ_FIRMWARE_LOAD, CPUCS, 0, &[1])?;
    for (i, chunk) in image.chunks(4096).enumerate() {
        link.control_out(REQ_FIRMWARE_LOAD, (i * 4096) as u16, 0, chunk)?;
    }
    // Releasing the reset starts the firmware; the board then drops off the bus and comes back.
    // Some hosts report an error for this last request because the device is already gone.
    let _ = link.control_out(REQ_FIRMWARE_LOAD, CPUCS, 0, &[0]);
    Ok(())
}

/// Steps reported while connecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum ConnectStep {
    Opening,
    UploadingFirmware { file: String },
    WaitingForRenumeration,
    Ready { version: String },
}

/// An FX2 board running fx2lafw, ready to capture.
pub struct Fx2Lafw {
    link: Box<dyn UsbLink>,
    pub info: UsbDeviceInfo,
    pub board: Board,
    pub version: FwVersion,
}

pub struct ConnectOptions {
    /// How long the board may take to come back after the firmware upload.
    pub renumerate_timeout: Duration,
    pub poll: Duration,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self { renumerate_timeout: Duration::from_secs(5), poll: Duration::from_millis(100) }
    }
}

impl Fx2Lafw {
    /// Opens the board `id` (from [`detect`]), loading fx2lafw first if it is not running.
    pub fn connect(backend: &dyn UsbBackend, id: &str, fw: &dyn firmware::FirmwareSource, opts: &ConnectOptions, progress: &mut dyn FnMut(ConnectStep)) -> Result<Self> {
        progress(ConnectStep::Opening);
        let devices = backend.list()?;
        let info = devices.into_iter().find(|d| d.id == id).ok_or(LogicError::NoDevice)?;
        let board = *board_for(info.vid, info.pid).ok_or_else(|| LogicError::Usb(format!("{:04x}:{:04x} is not a supported FX2 board", info.vid, info.pid)))?;
        let mut link = backend.open(id)?;
        if let Some(version) = probe_version(link.as_mut()) {
            check_version(version)?;
            progress(ConnectStep::Ready { version: format!("{}.{}", version.major, version.minor) });
            return Ok(Self { link, info, board, version });
        }
        let image = fw.load(board.firmware)?;
        progress(ConnectStep::UploadingFirmware { file: board.firmware.to_string() });
        upload_firmware(link.as_mut(), &image)?;
        drop(link);
        progress(ConnectStep::WaitingForRenumeration);
        let deadline = Instant::now() + opts.renumerate_timeout;
        // The same board comes back with the same VID:PID (or fx2lafw's own) on the same port.
        loop {
            std::thread::sleep(opts.poll);
            let now = backend.list()?;
            let back = now.iter().find(|d| d.location == info.location && d.id != info.id && board_for(d.vid, d.pid).is_some()).or_else(|| {
                now.iter().find(|d| d.runs_fx2lafw() && board_for(d.vid, d.pid).is_some() && d.location == info.location)
            });
            if let Some(d) = back {
                let d = d.clone();
                let mut link = backend.open(&d.id)?;
                let version = probe_version(link.as_mut()).ok_or_else(|| LogicError::Firmware("the board came back after the firmware upload but does not answer as fx2lafw".into()))?;
                check_version(version)?;
                progress(ConnectStep::Ready { version: format!("{}.{}", version.major, version.minor) });
                return Ok(Self { link, info: d, board, version });
            }
            if Instant::now() > deadline {
                return Err(LogicError::Firmware("the board did not come back after the firmware upload; unplug it, plug it in again and retry".into()));
            }
        }
    }

    /// Starts acquisition and returns the sample stream. Memory stays bounded: the driver keeps a
    /// fixed set of transfers queued and hands each one to the capture engine as it completes.
    pub fn start(mut self, rate: u64) -> Result<Fx2Stream> {
        let cmd = start_command(rate)?;
        // About 10 ms per transfer, a multiple of the 512-byte packet, and roughly 300 ms queued.
        let size = (rate as usize / 100).div_ceil(512).clamp(16, 2048) * 512;
        let transfers = ((rate as usize * 3 / 10) / size).clamp(4, 64);
        self.link.start_bulk(EP_DATA, size, transfers)?;
        self.link.control_out(CMD_START, 0, 0, &cmd)?;
        Ok(Fx2Stream { dev: self, rate, first: true })
    }
}

fn check_version(v: FwVersion) -> Result<()> {
    if v.major != FW_MAJOR {
        return Err(LogicError::Firmware(format!("the board runs fx2lafw {}.{}; this app speaks version {FW_MAJOR}.x", v.major, v.minor)));
    }
    Ok(())
}

pub struct Fx2Stream {
    dev: Fx2Lafw,
    rate: u64,
    first: bool,
}

impl SampleStream for Fx2Stream {
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        // The first transfer can take a moment while the GPIF starts.
        let timeout = if self.first { Duration::from_secs(3) } else { Duration::from_secs(1) };
        match self.dev.link.read_bulk(timeout)? {
            Some(d) => {
                self.first = false;
                buf.extend_from_slice(&d);
                Ok(true)
            }
            None => Err(LogicError::Usb(format!(
                "the board stopped sending samples at {}: the USB bus could not keep up (try a lower sample rate, or plug the board straight into the computer)",
                crate::fmt_rate(self.rate)
            ))),
        }
    }

    fn sample_rate(&self) -> u64 {
        self.rate
    }
}

/// Text for the app when USB access is refused.
pub fn permission_help() -> &'static str {
    if cfg!(target_os = "linux") {
        "Permission denied opening the USB device. Install the udev rule from the app's README (60-voltrus-logic.rules), run `sudo udevadm control --reload && sudo udevadm trigger`, then unplug and replug the board."
    } else if cfg!(target_os = "windows") {
        "Windows needs the WinUSB driver for this board. Install it with Zadig (choose the board, then WinUSB) and replug it."
    } else {
        "The USB device could not be opened. Close other logic analyzer apps (PulseView, Saleae Logic) that may hold it and replug the board."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::default_channels;
    use crate::decode::DecoderConfig;
    use crate::engine::{CaptureSettings, run};
    use crate::sim::{self, ScenarioId};
    use crate::trigger::Trigger;
    use mock::MockFx2;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn start_command_for_every_rate() {
        for r in SAMPLE_RATES {
            let c = start_command(r).unwrap();
            assert_eq!(rate_of(c), r, "{r}");
            assert!(c[1] < 6);
        }
        assert_eq!(start_command(24_000_000).unwrap(), [FLAG_CLK_48MHZ, 0, 1]);
        assert_eq!(start_command(1_000_000).unwrap(), [FLAG_CLK_48MHZ, 0, 47]);
        // 20 kHz: 48 MHz / 2400 does not fit the divider, 30 MHz / 1500 does.
        assert_eq!(start_command(20_000).unwrap(), [0, 0x05, 0xDB]);
        assert!(start_command(7_000_000).is_err());
        assert!(start_command(48_000_000).is_err());
        assert!(start_command(10_000).is_err());
    }

    #[test]
    fn connect_uploads_firmware_and_waits_for_renumeration() {
        let mock = MockFx2::new(false);
        let fw = firmware::MemoryFirmware::new(&[("fx2lafw-saleae-logic.fw", mock::fake_image())]);
        let found = detect(&mock).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].firmware_loaded);
        let mut steps = vec![];
        let dev = Fx2Lafw::connect(&mock, &found[0].info.id, &fw, &ConnectOptions { poll: Duration::from_millis(1), ..Default::default() }, &mut |s| steps.push(s)).unwrap();
        assert!(matches!(steps[1], ConnectStep::UploadingFirmware { .. }));
        assert_eq!(steps.last(), Some(&ConnectStep::Ready { version: "1.4".into() }));
        assert_eq!(mock.state().ram_written, mock::fake_image().len());
        assert!(dev.info.runs_fx2lafw());
        assert_ne!(dev.info.id, found[0].info.id);
    }

    #[test]
    fn connect_skips_upload_when_running() {
        let mock = MockFx2::new(true);
        let fw = firmware::MemoryFirmware::new(&[]);
        let found = detect(&mock).unwrap();
        assert!(found[0].firmware_loaded);
        Fx2Lafw::connect(&mock, &found[0].info.id, &fw, &ConnectOptions::default(), &mut |_| {}).unwrap();
        assert_eq!(mock.state().ram_written, 0);
    }

    #[test]
    fn missing_firmware_and_permissions_give_clear_errors() {
        let mock = MockFx2::new(false);
        let fw = firmware::MemoryFirmware::new(&[]);
        let id = detect(&mock).unwrap()[0].info.id.clone();
        let e = Fx2Lafw::connect(&mock, &id, &fw, &ConnectOptions::default(), &mut |_| {}).err().unwrap();
        assert!(e.to_string().contains("fx2lafw-saleae-logic.fw"), "{e}");
        let mock = MockFx2::new(false);
        mock.state().permission_denied = true;
        let e = Fx2Lafw::connect(&mock, &id, &fw, &ConnectOptions::default(), &mut |_| {}).err().unwrap();
        assert!(matches!(e, LogicError::Permission(_)), "{e}");
        let e = Fx2Lafw::connect(&MockFx2::empty(), "nothing", &fw, &ConnectOptions::default(), &mut |_| {}).err().unwrap();
        assert!(matches!(e, LogicError::NoDevice));
    }

    #[test]
    fn board_that_never_comes_back() {
        let mock = MockFx2::new(false);
        mock.state().stay_gone = true;
        let fw = firmware::MemoryFirmware::new(&[("fx2lafw-saleae-logic.fw", mock::fake_image())]);
        let id = detect(&mock).unwrap()[0].info.id.clone();
        let opts = ConnectOptions { renumerate_timeout: Duration::from_millis(30), poll: Duration::from_millis(5) };
        let e = Fx2Lafw::connect(&mock, &id, &fw, &opts, &mut |_| {}).err().unwrap();
        assert!(e.to_string().contains("did not come back"));
    }

    #[test]
    fn streams_samples_into_a_triggered_capture_and_decodes_modbus() {
        let mock = MockFx2::new(true).with_scenario(ScenarioId::ModbusRtu);
        let id = detect(&mock).unwrap()[0].info.id.clone();
        let dev = Fx2Lafw::connect(&mock, &id, &firmware::MemoryFirmware::new(&[]), &ConnectOptions::default(), &mut |_| {}).unwrap();
        let mut stream = dev.start(1_000_000).unwrap();
        assert_eq!(mock.state().started_rate, Some(1_000_000));
        let settings = CaptureSettings { sample_rate: 1_000_000, samples: 600_000, trigger: Trigger::Edge { channel: 2, edge: crate::trigger::Edge::Rising }, pre_trigger: 0.01 };
        let cap = run(&mut stream, &settings, default_channels(8), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(cap.len(), 600_000);
        assert_eq!(cap.trigger, Some(6000));
        let DecoderConfig::ModbusRtu(m) = &sim::info(ScenarioId::ModbusRtu).decoders[0] else { panic!() };
        let r = crate::decode::modbus::decode(&cap, m).unwrap();
        assert!(r.transactions.iter().filter(|t| t.status == crate::decode::modbus::Status::Ok).count() >= 4);
    }

    #[test]
    fn stalled_stream_reports_usb_bandwidth() {
        let mock = MockFx2::new(true);
        mock.state().stall_after = Some(2);
        let id = detect(&mock).unwrap()[0].info.id.clone();
        let dev = Fx2Lafw::connect(&mock, &id, &firmware::MemoryFirmware::new(&[]), &ConnectOptions::default(), &mut |_| {}).unwrap();
        let mut stream = dev.start(24_000_000).unwrap();
        let mut buf = vec![];
        stream.read(&mut buf).unwrap();
        stream.read(&mut buf).unwrap();
        let e = stream.read(&mut buf).unwrap_err();
        assert!(e.to_string().contains("lower sample rate"));
    }
}
