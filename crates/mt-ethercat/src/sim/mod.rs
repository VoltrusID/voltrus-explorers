//! A simulated EtherCAT segment: devices in a line, each processing every frame on its way out as real devices do
//! (auto-increment addressing, working counters, broadcast OR, logical access through FMMUs), with propagation
//! delays for distributed clocks, wiring between devices, and faults to inject (a pulled cable, CRC errors).

pub mod device;
pub mod models;
pub mod od;

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::esc::al_status_text;
use crate::wire::{self, Cmd};
use device::{Device, DeviceSpec, Pass};

/// Simulation time starts here (ns since 2000-01-01, a plausible EtherCAT system time).
const TIME_BASE: u64 = 838_000_000_000_000_000;

/// A signal from one device's physical output to another device's input.
#[derive(Debug, Clone, Copy)]
pub struct Wire {
    pub from: (usize, usize),
    pub to: (usize, usize),
}

pub struct Segment {
    pub devices: Vec<Device>,
    start: Instant,
    /// A pulled cable after this position: later devices are unreachable.
    pub break_after: Option<usize>,
    pub wires: Vec<Wire>,
    pub frames: u64,
    pub template: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Template {
    /// EK1100 with EL1008, EL2008, EL3102, EL4102, and two servo drives.
    Machine,
    /// EK1100 with the four I/O terminals.
    IoStation,
    /// Two servo drives.
    Drives,
}

impl Template {
    pub fn label(self) -> &'static str {
        match self {
            Template::Machine => "Machine: I/O terminals and two servo axes",
            Template::IoStation => "I/O station: EK1100 with four terminals",
            Template::Drives => "Two CiA 402 servo drives",
        }
    }
}

impl Segment {
    pub fn new(specs: Vec<DeviceSpec>, wires: Vec<Wire>, template: &str) -> Segment {
        let devices = specs
            .into_iter()
            .enumerate()
            // Each device powered on at a different moment, so DC offsets have something to compensate.
            .map(|(i, s)| Device::new(s, 1_000_000_000 * (i as u64 + 1) + 7_919 * i as u64))
            .collect();
        Segment { devices, start: Instant::now(), break_after: None, wires, frames: 0, template: template.to_string() }
    }

    pub fn template(t: Template) -> Segment {
        use models::*;
        match t {
            Template::Machine => Segment::new(
                vec![ek1100(0), el1008(0), el2008(0), el3102(2104), el4102(1977), servo(240_118), servo(240_131)],
                machine_wires(1, 2, 3, 4),
                "machine",
            ),
            Template::IoStation => {
                Segment::new(vec![ek1100(0), el1008(0), el2008(0), el3102(2104), el4102(1977)], machine_wires(1, 2, 3, 4), "io")
            }
            Template::Drives => Segment::new(vec![servo(240_118), servo(240_131)], Vec::new(), "drives"),
        }
    }

    pub fn now_ns(&self) -> u64 {
        TIME_BASE + self.start.elapsed().as_nanos() as u64
    }

    fn reachable(&self) -> usize {
        match self.break_after {
            Some(k) => (k + 1).min(self.devices.len()),
            None => self.devices.len(),
        }
    }

    /// Passes a frame through the segment and back. `false` when nothing answers (no device reachable or not an
    /// EtherCAT frame): the master sees the frame lost.
    pub fn process(&mut self, frame: &mut [u8]) -> bool {
        let Some(spans) = wire::datagrams(frame) else { return false };
        let n = self.reachable();
        if n == 0 {
            return false;
        }
        let t0 = self.now_ns();
        // Arrival at each device's port 0, then the return trip through port 1 from the far end.
        let cable = 25u64;
        let mut t_in = Vec::with_capacity(n);
        let mut t = t0 + cable;
        for d in &self.devices[..n] {
            t_in.push(t);
            t += d.delay_ns + cable;
        }
        let mut t_ret = vec![None; n];
        let mut back = t_in[n - 1] + self.devices[n - 1].delay_ns;
        for i in (0..n.saturating_sub(1)).rev() {
            back += cable;
            t_ret[i] = Some(back);
            back += 40;
        }

        for (i, d) in self.devices[..n].iter_mut().enumerate() {
            d.port1_link = i + 1 < n;
            d.tick(t_in[i]);
        }
        for w in self.wires.clone() {
            if w.from.0 < n && w.to.0 < n {
                let v = self.devices[w.from.0].model.output(w.from.1);
                self.devices[w.to.0].model.set_input(w.to.1, v);
            }
        }

        for (i, dev) in self.devices[..n].iter_mut().enumerate() {
            dev.frames += 1;
            let pass = Pass { t_in: t_in[i], t_return: t_ret[i] };
            for s in &spans {
                process_datagram(dev, frame, s, pass);
            }
        }
        // The ESC sets the locally administered bit of the source address when the frame leaves through port 0.
        frame[6] |= 0x02;
        self.frames += 1;
        true
    }

    pub fn action(&mut self, a: &SimAction) -> Result<(), String> {
        match a {
            SimAction::BreakCable { after } => {
                if let Some(k) = after {
                    if *k >= self.devices.len() {
                        return Err(format!("There is no device at position {k}"));
                    }
                    self.devices[*k].add_lost_link(1);
                    if let Some(next) = self.devices.get_mut(k + 1) {
                        next.add_lost_link(0);
                    }
                }
                self.break_after = *after;
            }
            SimAction::RxErrors { device, port, count } => {
                self.devices.get_mut(*device).ok_or("No such device")?.add_rx_errors(*port, *count);
            }
            SimAction::Device { device, action } => {
                let d = self.devices.get_mut(*device).ok_or("No such device")?;
                if !d.model.action(action, &mut d.od) {
                    return Err(format!("{} has no action {action}", d.name()));
                }
            }
        }
        Ok(())
    }

    pub fn state(&self) -> SimState {
        let n = self.reachable();
        SimState {
            template: self.template.clone(),
            frames: self.frames,
            break_after: self.break_after,
            devices: self
                .devices
                .iter()
                .enumerate()
                .map(|(i, d)| SimDeviceState {
                    position: i,
                    name: d.name(),
                    description: d.sii.display_name(),
                    reachable: i < n,
                    address: d.station_address(),
                    state: d.state().label().to_string(),
                    error: d.al_error().map(|c| format!("0x{c:04X} {}", al_status_text(c))),
                    summary: d.model.summary(&d.od),
                    actions: d.model.actions().into_iter().map(|(id, label)| (id.to_string(), label.to_string())).collect(),
                })
                .collect(),
        }
    }
}

/// EL2008 outputs 1-4 to EL1008 inputs 1-4, EL4102 channel 1 to EL3102 channel 2.
fn machine_wires(di: usize, dout: usize, ai: usize, ao: usize) -> Vec<Wire> {
    let mut w: Vec<Wire> = (0..4).map(|c| Wire { from: (dout, c), to: (di, c) }).collect();
    w.push(Wire { from: (ao, 0), to: (ai, 1) });
    w
}

fn process_datagram(dev: &mut Device, frame: &mut [u8], s: &wire::DgramSpan, pass: Pass) {
    let h = s.header;
    let Some(cmd) = Cmd::from_u8(frame[h]) else { return };
    let adp = u16::from_le_bytes([frame[h + 2], frame[h + 3]]);
    let ado = u16::from_le_bytes([frame[h + 4], frame[h + 5]]);
    let mut wkc = u16::from_le_bytes([frame[s.wkc()], frame[s.wkc() + 1]]);
    let mut data = frame[s.data..s.data + s.len].to_vec();
    let now = pass.t_in;
    let auto_inc = matches!(cmd, Cmd::Aprd | Cmd::Apwr | Cmd::Aprw | Cmd::Armw);
    let addressed = match cmd {
        Cmd::Aprd | Cmd::Apwr | Cmd::Aprw | Cmd::Armw => adp == 0,
        Cmd::Fprd | Cmd::Fpwr | Cmd::Fprw | Cmd::Frmw => adp == dev.station_address(),
        Cmd::Brd | Cmd::Bwr | Cmd::Brw => true,
        _ => false,
    };
    match cmd {
        Cmd::Nop => {}
        Cmd::Lrd | Cmd::Lwr | Cmd::Lrw => {
            let log = (ado as u32) << 16 | adp as u32;
            wkc = wkc.wrapping_add(dev.logical(cmd, log, &mut data, pass));
        }
        Cmd::Aprd | Cmd::Fprd if addressed => {
            if dev.read(ado, &mut data, now) {
                wkc = wkc.wrapping_add(1);
            }
        }
        Cmd::Brd => {
            let mut buf = vec![0u8; data.len()];
            if dev.read(ado, &mut buf, now) {
                for (d, b) in data.iter_mut().zip(buf) {
                    *d |= b;
                }
                wkc = wkc.wrapping_add(1);
            }
        }
        Cmd::Apwr | Cmd::Fpwr | Cmd::Bwr if addressed => {
            if dev.write(ado, &data, pass) {
                wkc = wkc.wrapping_add(1);
            }
        }
        Cmd::Aprw | Cmd::Fprw | Cmd::Brw if addressed => {
            let mut buf = vec![0u8; data.len()];
            let r = dev.read(ado, &mut buf, now);
            let w = dev.write(ado, &data, pass);
            if r {
                if cmd == Cmd::Brw {
                    for (d, b) in data.iter_mut().zip(buf) {
                        *d |= b;
                    }
                } else {
                    data = buf;
                }
            }
            wkc = wkc.wrapping_add(r as u16 + 2 * w as u16);
        }
        Cmd::Armw | Cmd::Frmw => {
            if addressed {
                if dev.read(ado, &mut data, now) {
                    wkc = wkc.wrapping_add(1);
                }
            } else if dev.write(ado, &data, pass) {
                wkc = wkc.wrapping_add(1);
            }
        }
        _ => {}
    }
    if auto_inc {
        frame[h + 2..h + 4].copy_from_slice(&adp.wrapping_add(1).to_le_bytes());
    }
    frame[s.data..s.data + s.len].copy_from_slice(&data);
    frame[s.wkc()..s.wkc() + 2].copy_from_slice(&wkc.to_le_bytes());
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SimAction {
    /// Pull the cable after position `after` (devices after it disappear); `None` plugs it back.
    BreakCable { after: Option<usize> },
    /// Add RX errors on a port of a device, as a damaged cable would.
    RxErrors { device: usize, port: usize, count: u8 },
    /// A device's own action (`fault`, `start`...).
    Device { device: usize, action: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDeviceState {
    pub position: usize,
    pub name: String,
    pub description: String,
    pub reachable: bool,
    pub address: u16,
    pub state: String,
    pub error: Option<String>,
    pub summary: Vec<(String, String)>,
    pub actions: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimState {
    pub template: String,
    pub frames: u64,
    pub break_after: Option<usize>,
    pub devices: Vec<SimDeviceState>,
}

/// Frames for tests: run `dgrams` through `seg` and return them decoded.
pub fn exchange(seg: &mut Segment, dgrams: &[(Cmd, u16, u16, Vec<u8>)]) -> Option<Vec<wire::Datagram>> {
    let list: Vec<(Cmd, u8, u16, u16, Vec<u8>)> =
        dgrams.iter().enumerate().map(|(i, (c, a, o, d))| (*c, i as u8, *a, *o, d.clone())).collect();
    let mut f = wire::build_frame([0x10; 6], &list);
    if !seg.process(&mut f) {
        return None;
    }
    let spans = wire::datagrams(&f)?;
    Some(spans.iter().map(|s| wire::read_datagram(&f, s)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coe;
    use crate::esc::{self, AlState};

    fn one(seg: &mut Segment, cmd: Cmd, adp: u16, ado: u16, data: Vec<u8>) -> wire::Datagram {
        exchange(seg, &[(cmd, adp, ado, data)]).unwrap().remove(0)
    }

    #[test]
    fn counting_addressing_and_states() {
        let mut seg = Segment::template(Template::Machine);
        // BRD of the ESC type: every device answers, working counter = device count, types OR-ed.
        let d = one(&mut seg, Cmd::Brd, 0, 0, vec![0; 1]);
        assert_eq!(d.wkc, 7);
        assert_eq!(d.data[0], 0x11 | 0x12 | 0x90);
        // Auto-increment: position 2 is addressed by -2 (0xFFFE).
        for i in 0..7u16 {
            let d = one(&mut seg, Cmd::Apwr, 0u16.wrapping_sub(i), esc::STATION_ADDR, (0x1001 + i).to_le_bytes().to_vec());
            assert_eq!(d.wkc, 1);
        }
        let d = one(&mut seg, Cmd::Fprd, 0x1003, esc::AL_STATUS, vec![0; 2]);
        assert_eq!((d.wkc, d.data[0]), (1, 1));
        // INIT -> SAFE-OP is not allowed: error flag and code 0x0011.
        one(&mut seg, Cmd::Fpwr, 0x1001, esc::AL_CONTROL, vec![4, 0]);
        let d = one(&mut seg, Cmd::Fprd, 0x1001, esc::AL_STATUS, vec![0; 6]);
        assert_eq!(d.data[0], 0x11);
        assert_eq!(u16::from_le_bytes([d.data[4], d.data[5]]), 0x0011);
        // An EK1100 without mailbox goes to PRE-OP directly (after the error is acknowledged).
        one(&mut seg, Cmd::Fpwr, 0x1001, esc::AL_CONTROL, vec![0x12, 0]);
        assert_eq!(seg.devices[0].state(), AlState::PreOp);
        // EL3102 refuses PRE-OP while its mailbox sync managers are not set up.
        one(&mut seg, Cmd::Fpwr, 0x1004, esc::AL_CONTROL, vec![2, 0]);
        assert_eq!(seg.devices[3].al_error(), Some(0x0016));
    }

    #[test]
    fn eeprom_reads_through_the_interface() {
        let mut seg = Segment::template(Template::IoStation);
        one(&mut seg, Cmd::Apwr, 0xFFFF, esc::SII_ADDRESS, 0x0008u32.to_le_bytes().to_vec());
        one(&mut seg, Cmd::Apwr, 0xFFFF, esc::SII_CONTROL, 0x0100u16.to_le_bytes().to_vec());
        let d = one(&mut seg, Cmd::Aprd, 0xFFFF, esc::SII_DATA, vec![0; 8]);
        // Vendor 2, product 0x03F03052 (EL1008).
        assert_eq!(&d.data, &[2, 0, 0, 0, 0x52, 0x30, 0xF0, 0x03]);
    }

    /// Sets up SM0/SM1 like a master would, goes to PRE-OP, and does an SDO upload by hand.
    #[test]
    fn mailbox_sdo_upload_by_hand() {
        let mut seg = Segment::template(Template::IoStation);
        let addr = 0x1004u16;
        one(&mut seg, Cmd::Apwr, 0u16.wrapping_sub(3), esc::STATION_ADDR, addr.to_le_bytes().to_vec());
        let sm = |start: u16, len: u16, ctl: u8| {
            let mut v = start.to_le_bytes().to_vec();
            v.extend_from_slice(&len.to_le_bytes());
            v.extend_from_slice(&[ctl, 0, 1, 0]);
            v
        };
        one(&mut seg, Cmd::Fpwr, addr, esc::sm(0), sm(0x1000, 128, 0x26));
        one(&mut seg, Cmd::Fpwr, addr, esc::sm(1), sm(0x1080, 128, 0x22));
        one(&mut seg, Cmd::Fpwr, addr, esc::AL_CONTROL, vec![2, 0]);
        assert_eq!(seg.devices[3].state(), AlState::PreOp);
        // Reading the empty device-to-master mailbox is refused: working counter 0.
        assert_eq!(one(&mut seg, Cmd::Fprd, addr, 0x1080, vec![0; 128]).wkc, 0);
        let mut req = coe::mailbox(coe::MBX_TYPE_COE, 1, &coe::upload_request(0x1008, 0));
        req.resize(128, 0);
        assert_eq!(one(&mut seg, Cmd::Fpwr, addr, 0x1000, req).wkc, 1);
        let st = one(&mut seg, Cmd::Fprd, addr, esc::sm(1) + 5, vec![0]);
        assert_eq!(st.data[0] & 0x08, 0x08);
        let resp = one(&mut seg, Cmd::Fprd, addr, 0x1080, vec![0; 128]);
        assert_eq!(resp.wkc, 1);
        let h = coe::MbxHeader::parse(&resp.data).unwrap();
        let body = &resp.data[6..6 + h.len as usize];
        assert_eq!(
            coe::parse_sdo_response(body),
            Ok(coe::SdoResponse::Upload { data: b"EL3102".to_vec(), complete_size: Some(6), more: false })
        );
        // Read once, the mailbox is empty again.
        assert_eq!(one(&mut seg, Cmd::Fprd, addr, 0x1080, vec![0; 128]).wkc, 0);
    }

    #[test]
    fn pulled_cable_hides_devices() {
        let mut seg = Segment::template(Template::Machine);
        seg.action(&SimAction::BreakCable { after: Some(2) }).unwrap();
        assert_eq!(one(&mut seg, Cmd::Brd, 0, 0, vec![0]).wkc, 3);
        assert!(seg.state().devices[3..].iter().all(|d| !d.reachable));
        seg.action(&SimAction::BreakCable { after: None }).unwrap();
        assert_eq!(one(&mut seg, Cmd::Brd, 0, 0, vec![0]).wkc, 7);
        // The lost link was counted on port 1 of device 2.
        let d = one(&mut seg, Cmd::Aprd, 0u16.wrapping_sub(2), esc::LOST_LINK, vec![0; 4]);
        assert_eq!(d.data[1], 1);
    }
}
