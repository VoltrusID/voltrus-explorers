//! gs_usb adapters, driven from user space over USB (no kernel or vendor driver): candleLight
//! firmware on CANable, CANable 2, candleLight, CANtact, Geschwister Schneider USB/CAN and the many
//! STM32 boards that run it. The same protocol Linux's `gs_usb` driver speaks, so it works on macOS
//! (IOKit), Linux (usbfs, after detaching the kernel driver) and Windows (WinUSB).
//!
//! Control requests set up the channel (host format, bit timing, mode); CAN frames travel over one
//! bulk IN and one bulk OUT endpoint as `gs_host_frame` structs (12-byte header, then 8 or 64 data
//! bytes). A frame sent by the host comes back on IN with its echo id once it is on the bus.

use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, ControlIn, ControlOut, ControlType, Direction as UsbDir, In, Out, Recipient};
use serde::Serialize;

use crate::bus::{BusEvent, CanHandle};
use crate::error::CanError;
use crate::frame::{CanFrame, Direction, TimedFrame, dlc_to_len, now_us};

/// USB ids of gs_usb devices (the Linux driver's table).
pub const DEVICE_IDS: [(u16, u16); 4] = [(0x1D50, 0x606F), (0x1209, 0x2323), (0x1CD2, 0x606F), (0x16D0, 0x10B8)];

const REQ_HOST_FORMAT: u8 = 0;
const REQ_BITTIMING: u8 = 1;
const REQ_MODE: u8 = 2;
const REQ_BT_CONST: u8 = 4;
const REQ_DEVICE_CONFIG: u8 = 5;
const REQ_DATA_BITTIMING: u8 = 10;
const REQ_BT_CONST_EXT: u8 = 11;

const MODE_RESET: u32 = 0;
const MODE_START: u32 = 1;

pub const FEATURE_LISTEN_ONLY: u32 = 1 << 0;
pub const FEATURE_FD: u32 = 1 << 8;
pub const FEATURE_BT_CONST_EXT: u32 = 1 << 9;
const MODE_FLAG_LISTEN_ONLY: u32 = 1 << 0;
const MODE_FLAG_FD: u32 = 1 << 8;
const MODE_FLAG_BERR_REPORTING: u32 = 1 << 12;
pub const FEATURE_BERR_REPORTING: u32 = 1 << 12;

const FRAME_FLAG_OVERFLOW: u8 = 1 << 0;
const FRAME_FLAG_FD: u8 = 1 << 1;
const FRAME_FLAG_BRS: u8 = 1 << 2;

const CAN_EFF_FLAG: u32 = 0x8000_0000;
const CAN_RTR_FLAG: u32 = 0x4000_0000;
const CAN_ERR_FLAG: u32 = 0x2000_0000;

const HEADER: usize = 12;
/// Echo id of a received frame (anything else is the echo of one we sent).
const RX_ECHO_ID: u32 = 0xFFFF_FFFF;
const TIMEOUT: Duration = Duration::from_millis(1000);
/// Frames sent but not yet echoed; the firmware has a small transmit queue.
const MAX_IN_FLIGHT: usize = 10;

/// Timing limits a channel reports (`gs_device_bt_const`, and the data phase of `_ext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingConst {
    pub tseg1_min: u32,
    pub tseg1_max: u32,
    pub tseg2_min: u32,
    pub tseg2_max: u32,
    pub sjw_max: u32,
    pub brp_min: u32,
    pub brp_max: u32,
    pub brp_inc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtConst {
    pub feature: u32,
    pub fclk: u32,
    pub nominal: TimingConst,
    /// Data phase limits, on adapters with CAN FD.
    pub data: Option<TimingConst>,
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn timing_const(v: &[u32]) -> TimingConst {
    TimingConst { tseg1_min: v[0], tseg1_max: v[1], tseg2_min: v[2], tseg2_max: v[3], sjw_max: v[4], brp_min: v[5], brp_max: v[6], brp_inc: v[7].max(1) }
}

/// Parses `BT_CONST` (40 bytes) or `BT_CONST_EXT` (72 bytes).
pub fn parse_bt_const(b: &[u8]) -> Result<BtConst, CanError> {
    let v = u32s(b);
    if v.len() < 10 {
        return Err(CanError::Adapter(format!("short timing constants from the adapter ({} bytes)", b.len())));
    }
    let data = if v.len() >= 18 { Some(timing_const(&v[10..18])) } else { None };
    Ok(BtConst { feature: v[0], fclk: v[1], nominal: timing_const(&v[2..10]), data })
}

/// One `gs_device_bittiming`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitTiming {
    pub prop_seg: u32,
    pub phase_seg1: u32,
    pub phase_seg2: u32,
    pub sjw: u32,
    pub brp: u32,
}

impl BitTiming {
    pub fn bytes(&self) -> [u8; 20] {
        let mut o = [0u8; 20];
        for (i, v) in [self.prop_seg, self.phase_seg1, self.phase_seg2, self.sjw, self.brp].iter().enumerate() {
            o[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        o
    }

    /// Time quanta per bit.
    pub fn tq(&self) -> u32 {
        1 + self.prop_seg + self.phase_seg1 + self.phase_seg2
    }

    pub fn sample_point(&self) -> f64 {
        (1 + self.prop_seg + self.phase_seg1) as f64 / self.tq() as f64
    }
}

/// Finds an exact bit timing for `bitrate` from the adapter's clock, as close as it can to the
/// CiA 301 sample point (87.5 % up to 800 kbit/s, 80 % at 1 Mbit/s, 75 % above for the data phase).
pub fn calc_bittiming(fclk: u32, bitrate: u32, c: &TimingConst) -> Result<BitTiming, CanError> {
    if bitrate == 0 || fclk == 0 {
        return Err(CanError::Unsupported("bit rate and adapter clock must be above zero".into()));
    }
    let target = if bitrate > 1_000_000 { 0.75 } else if bitrate > 800_000 { 0.80 } else { 0.875 };
    let mut best: Option<(f64, BitTiming)> = None;
    let mut brp = c.brp_min.max(1);
    while brp <= c.brp_max {
        let div = brp as u64 * bitrate as u64;
        if (fclk as u64).is_multiple_of(div) {
            let tq = (fclk as u64 / div) as u32;
            let (min_tq, max_tq) = (1 + c.tseg1_min + c.tseg2_min, 1 + c.tseg1_max + c.tseg2_max);
            if (min_tq..=max_tq).contains(&tq) && tq >= 8 {
                let mut tseg1 = ((target * tq as f64).round() as u32).saturating_sub(1);
                tseg1 = tseg1.clamp(c.tseg1_min, c.tseg1_max);
                let mut tseg2 = tq - 1 - tseg1;
                if tseg2 < c.tseg2_min {
                    tseg2 = c.tseg2_min;
                    tseg1 = tq - 1 - tseg2;
                }
                if tseg2 <= c.tseg2_max && (c.tseg1_min..=c.tseg1_max).contains(&tseg1) {
                    let bt = BitTiming { prop_seg: 1, phase_seg1: tseg1 - 1, phase_seg2: tseg2, sjw: c.sjw_max.min(tseg2).max(1), brp };
                    let err = (bt.sample_point() - target).abs();
                    // Prefer the closer sample point, then more quanta per bit (finer resolution).
                    if best.as_ref().is_none_or(|(e, b)| err < *e - 1e-9 || ((err - *e).abs() < 1e-9 && bt.tq() > b.tq())) {
                        best = Some((err, bt));
                    }
                }
            }
        }
        brp += c.brp_inc;
    }
    best.map(|(_, b)| b).ok_or_else(|| {
        CanError::Unsupported(format!("this adapter ({} MHz clock) cannot run at exactly {} bit/s", fclk as f64 / 1e6, bitrate))
    })
}

/// A `gs_host_frame` for a frame to send.
pub fn encode_frame(f: &CanFrame, echo_id: u32, channel: u8) -> Vec<u8> {
    let mut id = f.id;
    if f.extended {
        id |= CAN_EFF_FLAG;
    }
    if f.rtr {
        id |= CAN_RTR_FLAG;
    }
    let mut flags = 0u8;
    if f.fd {
        flags |= FRAME_FLAG_FD;
        if f.brs {
            flags |= FRAME_FLAG_BRS;
        }
    }
    let data_len = if f.fd { 64 } else { 8 };
    let mut out = Vec::with_capacity(HEADER + data_len);
    out.extend_from_slice(&echo_id.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&[f.dlc(), channel, flags, 0]);
    let mut data = [0u8; 64];
    data[..f.data.len()].copy_from_slice(&f.data);
    out.extend_from_slice(&data[..data_len]);
    out
}

/// What one `gs_host_frame` from the adapter means.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Rx { frame: CanFrame, channel: u8, overflow: bool },
    /// A frame we sent is on the bus.
    Echo { echo_id: u32, channel: u8 },
    /// An error frame: the Linux CAN error class bits and data.
    Error { class: u32, data: [u8; 8] },
}

pub fn decode_frame(b: &[u8]) -> Result<Incoming, CanError> {
    if b.len() < HEADER {
        return Err(CanError::Adapter(format!("short frame from the adapter ({} bytes)", b.len())));
    }
    let echo_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let can_id = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let (dlc, channel, flags) = (b[8], b[9], b[10]);
    let payload = &b[HEADER..];
    if echo_id != RX_ECHO_ID {
        return Ok(Incoming::Echo { echo_id, channel });
    }
    if can_id & CAN_ERR_FLAG != 0 {
        let mut data = [0u8; 8];
        let n = payload.len().min(8);
        data[..n].copy_from_slice(&payload[..n]);
        return Ok(Incoming::Error { class: can_id & 0x1FFF_FFFF, data });
    }
    let extended = can_id & CAN_EFF_FLAG != 0;
    let id = if extended { can_id & 0x1FFF_FFFF } else { can_id & 0x7FF };
    let fd = flags & FRAME_FLAG_FD != 0;
    let overflow = flags & FRAME_FLAG_OVERFLOW != 0;
    let frame = if can_id & CAN_RTR_FLAG != 0 && !fd {
        CanFrame { id, extended, rtr: true, fd: false, brs: false, data: Vec::new(), rtr_len: dlc.min(8) }
    } else {
        let len = if fd { dlc_to_len(dlc) } else { (dlc as usize).min(8) };
        if payload.len() < len {
            return Err(CanError::Adapter(format!("frame data shorter than its length ({} < {len})", payload.len())));
        }
        CanFrame { id, extended, rtr: false, fd, brs: fd && flags & FRAME_FLAG_BRS != 0, data: payload[..len].to_vec(), rtr_len: 0 }
    };
    Ok(Incoming::Rx { frame, channel, overflow })
}

/// `GS_USB_BREQ_DEVICE_CONFIG`: channel count, software and hardware version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConfig {
    pub channels: u8,
    pub sw_version: u32,
    pub hw_version: u32,
}

pub fn parse_device_config(b: &[u8]) -> Result<DeviceConfig, CanError> {
    if b.len() < 12 {
        return Err(CanError::Adapter("short device configuration from the adapter".into()));
    }
    Ok(DeviceConfig {
        channels: b[3].saturating_add(1),
        sw_version: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        hw_version: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
    })
}

fn mode_bytes(mode: u32, flags: u32) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&mode.to_le_bytes());
    o[4..].copy_from_slice(&flags.to_le_bytes());
    o
}

fn error_text(class: u32) -> String {
    const NAMES: [&str; 9] = [
        "transmit timeout",
        "arbitration lost",
        "controller problem",
        "protocol violation",
        "transceiver status",
        "no acknowledge",
        "bus off",
        "bus error",
        "controller restarted",
    ];
    let parts: Vec<&str> = (0..9).filter(|b| class & (1 << b) != 0).map(|b| NAMES[b]).collect();
    if parts.is_empty() { "error frame".into() } else { format!("error frame: {}", parts.join(", ")) }
}

// ---- USB ----------------------------------------------------------------------------------------

/// A frame handed to the adapter, waiting for its echo.
type InFlight = (CanFrame, tokio::sync::oneshot::Sender<Result<(), CanError>>);

/// A gs_usb adapter on the USB bus.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GsUsbDevice {
    /// Serial number, or `bus-address` when the device has none. Pass it back to open the device.
    pub id: String,
    pub description: String,
    pub vid: u16,
    pub pid: u16,
}

fn usb_err(what: &str, e: impl std::fmt::Display) -> CanError {
    CanError::Adapter(format!("{what}: {e}"))
}

fn device_key(d: &nusb::DeviceInfo) -> String {
    d.serial_number().map(str::to_string).unwrap_or_else(|| format!("{:04X}:{:04X}@{:?}", d.vendor_id(), d.product_id(), d.id()))
}

/// gs_usb adapters that are plugged in.
pub async fn list_devices() -> Result<Vec<GsUsbDevice>, CanError> {
    let devs = nusb::list_devices().await.map_err(|e| usb_err("cannot list USB devices", e))?;
    Ok(devs
        .filter(|d| DEVICE_IDS.contains(&(d.vendor_id(), d.product_id())))
        .map(|d| {
            let name = d.product_string().or(d.manufacturer_string()).unwrap_or("gs_usb adapter").to_string();
            let serial = d.serial_number().map(|s| format!(" {s}")).unwrap_or_default();
            GsUsbDevice { id: device_key(&d), description: format!("{name}{serial}"), vid: d.vendor_id(), pid: d.product_id() }
        })
        .collect())
}

struct Opened {
    iface: nusb::Interface,
    ep_in: u8,
    ep_out: u8,
    bt: BtConst,
    config: DeviceConfig,
    product: String,
}

async fn vendor_out(iface: &nusb::Interface, request: u8, value: u16, data: &[u8]) -> Result<(), CanError> {
    iface
        .control_out(
            ControlOut { control_type: ControlType::Vendor, recipient: Recipient::Interface, request, value, index: iface.interface_number() as u16, data },
            TIMEOUT,
        )
        .await
        .map_err(|e| usb_err(&format!("adapter refused request {request}"), e))
}

async fn vendor_in(iface: &nusb::Interface, request: u8, value: u16, length: u16) -> Result<Vec<u8>, CanError> {
    iface
        .control_in(
            ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Interface, request, value, index: iface.interface_number() as u16, length },
            TIMEOUT,
        )
        .await
        .map_err(|e| usb_err(&format!("adapter refused request {request}"), e))
}

async fn open_device(id: Option<&str>, channel: u8) -> Result<Opened, CanError> {
    let devs = nusb::list_devices().await.map_err(|e| usb_err("cannot list USB devices", e))?;
    let mut found = devs.filter(|d| DEVICE_IDS.contains(&(d.vendor_id(), d.product_id())));
    let info = match id {
        Some(want) => found.find(|d| device_key(d) == want),
        None => found.next(),
    }
    .ok_or_else(|| CanError::Adapter("the gs_usb adapter is not plugged in (or another app has it open)".into()))?;
    let product = info.product_string().unwrap_or("gs_usb adapter").to_string();
    let device = info.open().await.map_err(|e| usb_err("cannot open the adapter", e))?;
    // Interface 0 carries the CAN channels; on Linux the kernel's gs_usb driver has it, so detach it.
    let iface = device.detach_and_claim_interface(0).await.map_err(|e| {
        usb_err("cannot claim the adapter (is another program, or the Linux gs_usb driver, using it?)", e)
    })?;
    let desc = iface.descriptor().ok_or_else(|| CanError::Adapter("the adapter has no interface descriptor".into()))?;
    let mut ep_in = None;
    let mut ep_out = None;
    for ep in desc.endpoints() {
        if ep.transfer_type() == TransferType::Bulk {
            match ep.direction() {
                UsbDir::In if ep_in.is_none() => ep_in = Some(ep.address()),
                UsbDir::Out if ep_out.is_none() => ep_out = Some(ep.address()),
                _ => {}
            }
        }
    }
    let (Some(ep_in), Some(ep_out)) = (ep_in, ep_out) else {
        return Err(CanError::Adapter("the adapter has no bulk endpoints; is it running candleLight firmware?".into()));
    };
    // Byte order of the host. The firmware ignores it today, but the Linux driver sends it.
    vendor_out(&iface, REQ_HOST_FORMAT, 1, &0x0000_BEEFu32.to_le_bytes()).await?;
    let config = parse_device_config(&vendor_in(&iface, REQ_DEVICE_CONFIG, 1, 12).await?)?;
    if channel >= config.channels {
        return Err(CanError::Adapter(format!("the adapter has {} channel(s); channel {} does not exist", config.channels, channel + 1)));
    }
    let mut bt = parse_bt_const(&vendor_in(&iface, REQ_BT_CONST, channel as u16, 40).await?)?;
    if bt.feature & FEATURE_BT_CONST_EXT != 0
        && let Ok(b) = vendor_in(&iface, REQ_BT_CONST_EXT, channel as u16, 72).await
            && let Ok(ext) = parse_bt_const(&b) {
                bt = ext;
            }
    Ok(Opened { iface, ep_in, ep_out, bt, config, product })
}

pub(crate) async fn open(
    id: Option<&str>,
    channel: u8,
    bitrate: u32,
    data_bitrate: Option<u32>,
    listen_only: bool,
    description: String,
) -> Result<CanHandle, CanError> {
    let o = open_device(id, channel).await?;
    let ch = channel as u16;
    let nominal = calc_bittiming(o.bt.fclk, bitrate, &o.bt.nominal)?;
    let mut flags = 0u32;
    if listen_only {
        if o.bt.feature & FEATURE_LISTEN_ONLY == 0 {
            return Err(CanError::Unsupported("this adapter has no listen-only mode".into()));
        }
        flags |= MODE_FLAG_LISTEN_ONLY;
    }
    if o.bt.feature & FEATURE_BERR_REPORTING != 0 {
        flags |= MODE_FLAG_BERR_REPORTING;
    }
    // Stop the channel in case it was left running, then configure and start it.
    vendor_out(&o.iface, REQ_MODE, ch, &mode_bytes(MODE_RESET, 0)).await?;
    vendor_out(&o.iface, REQ_BITTIMING, ch, &nominal.bytes()).await?;
    if let Some(dr) = data_bitrate {
        if o.bt.feature & FEATURE_FD == 0 {
            return Err(CanError::Unsupported("this adapter does not do CAN FD (pick Classic CAN)".into()));
        }
        let dc = o.bt.data.unwrap_or(o.bt.nominal);
        let data = calc_bittiming(o.bt.fclk, dr, &dc)?;
        vendor_out(&o.iface, REQ_DATA_BITTIMING, ch, &data.bytes()).await?;
        flags |= MODE_FLAG_FD;
    }
    vendor_out(&o.iface, REQ_MODE, ch, &mode_bytes(MODE_START, flags)).await?;

    let mut ep_in = o.iface.endpoint::<Bulk, In>(o.ep_in).map_err(|e| usb_err("bulk IN endpoint", e))?;
    let mut ep_out = o.iface.endpoint::<Bulk, Out>(o.ep_out).map_err(|e| usb_err("bulk OUT endpoint", e))?;
    let mps = ep_in.max_packet_size().max(64);
    // A 76-byte FD frame spans two full-speed packets; a transfer ends at the first short packet.
    let rx_len = mps * (HEADER + 64).div_ceil(mps);
    for _ in 0..8 {
        ep_in.submit(Buffer::new(rx_len));
    }

    let (mut d, make) = crate::bus::channels(description);
    let fd_txt = if o.bt.feature & FEATURE_FD != 0 { ", CAN FD capable" } else { "" };
    let _ = d.events.send(BusEvent::Info(format!(
        "{} (firmware {}, hardware {}, {} channel(s), {} MHz clock{fd_txt}); nominal timing brp {} tq {} sample point {:.1}%",
        o.product,
        o.config.sw_version,
        o.config.hw_version,
        o.config.channels,
        o.bt.fclk as f64 / 1e6,
        nominal.brp,
        nominal.tq(),
        nominal.sample_point() * 100.0
    )));
    let iface = o.iface.clone();
    let task = tokio::spawn(async move {
        // echo id -> (frame, done) for frames handed to the adapter and not yet on the bus.
        let mut in_flight: Vec<Option<InFlight>> =
            (0..MAX_IN_FLIGHT).map(|_| None).collect();
        let mut queued: std::collections::VecDeque<crate::bus::SendReq> = Default::default();
        let mut out_busy = 0usize;
        // When the oldest unechoed frame is given up on.
        let mut give_up: Option<tokio::time::Instant> = None;
        let reason = loop {
            // Hand queued frames to the adapter while it has room.
            while let Some(slot) = in_flight.iter().position(Option::is_none) {
                let Some((frame, done)) = queued.pop_front() else { break };
                if listen_only {
                    let _ = done.send(Err(CanError::Unsupported("the channel is open in listen-only mode".into())));
                    continue;
                }
                ep_out.submit(Buffer::from(encode_frame(&frame, slot as u32, channel)));
                out_busy += 1;
                in_flight[slot] = Some((frame, done));
                give_up.get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_secs(2));
            }
            let room = in_flight.iter().any(Option::is_none) && queued.len() < 256;
            tokio::select! {
                _ = &mut d.stop => break None,
                c = ep_in.next_complete() => {
                    match c.status {
                        Ok(()) => {
                            match decode_frame(&c.buffer[..c.actual_len]) {
                                Ok(Incoming::Rx { frame, channel: fch, overflow }) => {
                                    if fch == channel {
                                        if overflow {
                                            let _ = d.events.send(BusEvent::Error("the adapter's receive buffer overflowed; frames were lost".into()));
                                        }
                                        let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Rx, frame }));
                                    }
                                }
                                Ok(Incoming::Echo { echo_id, .. }) => {
                                    if let Some(Some((frame, done))) = in_flight.get_mut(echo_id as usize).map(Option::take) {
                                        let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }));
                                        let _ = done.send(Ok(()));
                                        give_up = in_flight.iter().any(Option::is_some).then(|| tokio::time::Instant::now() + Duration::from_secs(2));
                                    }
                                }
                                Ok(Incoming::Error { class, .. }) => { let _ = d.events.send(BusEvent::Error(error_text(class))); }
                                Err(e) => { let _ = d.events.send(BusEvent::Error(e.to_string())); }
                            }
                            let mut buf = c.buffer;
                            buf.clear();
                            buf.set_requested_len(rx_len);
                            ep_in.submit(buf);
                        }
                        Err(nusb::transfer::TransferError::Disconnected) => break Some("the adapter was unplugged".to_string()),
                        Err(e) => {
                            let _ = d.events.send(BusEvent::Error(format!("USB receive error: {e}")));
                            ep_in.submit(Buffer::new(rx_len));
                        }
                    }
                }
                c = ep_out.next_complete(), if out_busy > 0 => {
                    out_busy -= 1;
                    if let Err(e) = c.status {
                        if matches!(e, nusb::transfer::TransferError::Disconnected) {
                            break Some("the adapter was unplugged".to_string());
                        }
                        let _ = d.events.send(BusEvent::Error(format!("USB send error: {e}")));
                    }
                }
                req = d.requests.recv(), if room => match req {
                    Some(r) => queued.push_back(r),
                    None => break None,
                },
                // Frames the adapter never echoes (no other node acknowledges them) are given up on.
                _ = tokio::time::sleep_until(give_up.unwrap_or_else(tokio::time::Instant::now)), if give_up.is_some() => {
                    give_up = None;
                    for slot in in_flight.iter_mut() {
                        if let Some((_, done)) = slot.take() {
                            let _ = done.send(Err(CanError::Adapter("no other node acknowledged the frame (is the bus connected and terminated?)".into())));
                        }
                    }
                    let _ = vendor_out(&iface, REQ_MODE, ch, &mode_bytes(MODE_RESET, 0)).await;
                    let _ = vendor_out(&iface, REQ_MODE, ch, &mode_bytes(MODE_START, flags)).await;
                }
            }
        };
        let _ = vendor_out(&iface, REQ_MODE, ch, &mode_bytes(MODE_RESET, 0)).await;
        let _ = d.events.send(BusEvent::Closed(reason));
    });
    Ok(make(task))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// STM32F072 candleLight: 48 MHz, bxCAN limits.
    const F0: TimingConst = TimingConst { tseg1_min: 1, tseg1_max: 16, tseg2_min: 1, tseg2_max: 8, sjw_max: 4, brp_min: 1, brp_max: 1024, brp_inc: 1 };
    /// STM32G431 (CANable 2 / candleLight FD): 80 MHz FDCAN, data phase limits.
    const G4_DATA: TimingConst = TimingConst { tseg1_min: 1, tseg1_max: 32, tseg2_min: 1, tseg2_max: 16, sjw_max: 16, brp_min: 1, brp_max: 32, brp_inc: 1 };

    #[test]
    fn bit_timing_hits_the_cia_sample_points() {
        for (rate, sp) in [(125_000, 0.875), (250_000, 0.875), (500_000, 0.875), (1_000_000, 0.80)] {
            let bt = calc_bittiming(48_000_000, rate, &F0).unwrap();
            assert_eq!(48_000_000 / (bt.brp * bt.tq()), rate);
            assert!((bt.sample_point() - sp).abs() < 0.03, "{rate}: {}", bt.sample_point());
            assert!(bt.phase_seg2 <= F0.tseg2_max && bt.prop_seg + bt.phase_seg1 <= F0.tseg1_max);
        }
        let bt = calc_bittiming(48_000_000, 500_000, &F0).unwrap();
        assert_eq!((bt.brp, bt.tq(), bt.phase_seg2), (6, 16, 2));
        let d = calc_bittiming(80_000_000, 2_000_000, &G4_DATA).unwrap();
        assert_eq!(80_000_000 / (d.brp * d.tq()), 2_000_000);
        assert!(calc_bittiming(48_000_000, 83_333, &F0).is_err());
        assert_eq!(bt.bytes()[16..20], 6u32.to_le_bytes());
    }

    #[test]
    fn frames_round_trip_through_the_wire_format() {
        let f = CanFrame::new(0x123, &[1, 2, 3]).unwrap();
        let b = encode_frame(&f, 4, 0);
        assert_eq!(b.len(), 20);
        assert_eq!(&b[..12], &[4, 0, 0, 0, 0x23, 0x01, 0, 0, 3, 0, 0, 0]);
        // The adapter echoes it, then a received copy has echo id 0xFFFFFFFF.
        assert_eq!(decode_frame(&b).unwrap(), Incoming::Echo { echo_id: 4, channel: 0 });
        let mut rx = b.clone();
        rx[..4].copy_from_slice(&RX_ECHO_ID.to_le_bytes());
        assert_eq!(decode_frame(&rx).unwrap(), Incoming::Rx { frame: f, channel: 0, overflow: false });

        let e = CanFrame::with_ext(0x18FE_EE00, true, &[0xAA; 8]).unwrap();
        let mut b = encode_frame(&e, 0, 1);
        b[..4].copy_from_slice(&RX_ECHO_ID.to_le_bytes());
        let Incoming::Rx { frame, channel, .. } = decode_frame(&b).unwrap() else { panic!() };
        assert_eq!((frame, channel), (e, 1));

        let fd = CanFrame::fd(0x10, false, true, &[7; 24]).unwrap();
        let mut b = encode_frame(&fd, 0, 0);
        assert_eq!(b.len(), 76);
        assert_eq!(b[8], 12); // DLC code for 24 bytes
        b[..4].copy_from_slice(&RX_ECHO_ID.to_le_bytes());
        let Incoming::Rx { frame, .. } = decode_frame(&b).unwrap() else { panic!() };
        assert_eq!(frame, fd);

        let r = CanFrame::remote(0x7DF, false, 8).unwrap();
        let mut b = encode_frame(&r, 0, 0);
        b[..4].copy_from_slice(&RX_ECHO_ID.to_le_bytes());
        let Incoming::Rx { frame, .. } = decode_frame(&b).unwrap() else { panic!() };
        assert_eq!(frame, r);
    }

    #[test]
    fn errors_and_short_frames() {
        let mut b = vec![0xFF; 4];
        b.extend_from_slice(&(CAN_ERR_FLAG | 0x40).to_le_bytes());
        b.extend_from_slice(&[8, 0, 0, 0]);
        b.extend_from_slice(&[0; 8]);
        let Incoming::Error { class, .. } = decode_frame(&b).unwrap() else { panic!() };
        assert_eq!(error_text(class), "error frame: bus off");
        assert!(decode_frame(&[0; 5]).is_err());
        let mut short = encode_frame(&CanFrame::new(1, &[1; 8]).unwrap(), 0, 0);
        short[..4].copy_from_slice(&RX_ECHO_ID.to_le_bytes());
        short.truncate(15);
        assert!(decode_frame(&short).is_err());
    }

    #[test]
    fn device_replies() {
        let cfg = parse_device_config(&[0, 0, 0, 1, 2, 0, 0, 0, 1, 0, 0, 0]).unwrap();
        assert_eq!(cfg, DeviceConfig { channels: 2, sw_version: 2, hw_version: 1 });
        let mut b = Vec::new();
        for v in [FEATURE_LISTEN_ONLY | FEATURE_FD, 80_000_000, 1, 256, 1, 128, 128, 1, 512, 1] {
            b.extend_from_slice(&u32::to_le_bytes(v));
        }
        let c = parse_bt_const(&b).unwrap();
        assert_eq!((c.fclk, c.nominal.tseg1_max, c.data), (80_000_000, 256, None));
        for v in [1, 32, 1, 16, 16, 1, 32, 1] {
            b.extend_from_slice(&u32::to_le_bytes(v));
        }
        assert_eq!(parse_bt_const(&b).unwrap().data, Some(G4_DATA));
        assert!(parse_bt_const(&b[..20]).is_err());
    }
}
