//! One simulated EtherCAT device: an ESC register file with process RAM, the SII EEPROM behind its interface
//! registers, sync managers (mailbox and buffered), FMMUs, the AL state machine with real AL status codes, DC
//! receive times and system time, the process data watchdog, and a CoE server (SDO, SDO Info) on its mailbox.

use std::collections::VecDeque;

use crate::coe::{self, SdoRequest};
use crate::esc::{self, AlState};
use crate::sii::{Sii, SmType};
use crate::sim::od::{MapEntry, Od, total_bits};

/// Register file plus 8 KB of process RAM (0x1000-0x2FFF), as an ET1100.
pub const MEM_SIZE: usize = 0x3000;

/// The application behind the ESC: reads outputs from and writes inputs to the object dictionary.
pub trait Model: Send {
    /// Advances the model by `dt` seconds. `op` is false when outputs are not valid (not in OP).
    fn tick(&mut self, dt: f64, od: &mut Od, op: bool);
    /// A named action from the simulator panel (`fault`, `input:7:on`...). Returns false if unknown.
    fn action(&mut self, _action: &str, _od: &mut Od) -> bool {
        false
    }
    /// What the simulator panel shows for this device: `(label, value)`.
    fn summary(&self, _od: &Od) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Called when the EtherCAT state changes, e.g. to drop a drive out of operation.
    fn state_changed(&mut self, _state: AlState, _od: &mut Od) {}
    /// Actions the simulator panel offers: `(id, label)`.
    fn actions(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
    /// A signal wired into this device (volts, or 0/1 for digital), by port.
    fn set_input(&mut self, _port: usize, _value: f64) {}
    /// A physical output of this device by port, for wiring into another device.
    fn output(&self, _port: usize) -> f64 {
        0.0
    }
}

/// A device that does nothing (couplers).
pub struct Passive;

impl Model for Passive {
    fn tick(&mut self, _dt: f64, _od: &mut Od, _op: bool) {}
}

/// Static description of a device for [`Device::new`].
pub struct DeviceSpec {
    pub esc_type: u8,
    pub esc_revision: u8,
    pub esc_build: u16,
    pub fmmus: u8,
    pub sms: u8,
    pub port_desc: u8,
    /// ESC features (0x0008): 0x0004 DC, 0x0008 64-bit DC.
    pub features: u16,
    pub eeprom: Vec<u8>,
    pub od: Od,
    pub model: Box<dyn Model>,
    /// Forward delay through this device in nanoseconds (EBUS terminals about 100, MII devices about 500).
    pub delay_ns: u64,
}

#[derive(Debug, Default)]
struct Mailbox {
    rx_sm: Option<usize>,
    tx_sm: Option<usize>,
    /// The device-to-master mailbox holds a message the master has not read.
    out_full: bool,
    queue: VecDeque<Vec<u8>>,
    last_counter: u8,
    counter: u8,
    upload: Option<(Vec<u8>, usize, bool)>,
    download: Option<(u16, u8, u32, Vec<u8>, bool)>,
}

/// What the segment knows about a frame as it passes this device.
#[derive(Debug, Clone, Copy)]
pub struct Pass {
    /// Simulation time (ns) at which the frame enters port 0.
    pub t_in: u64,
    /// When the returning frame enters port 1, if port 1 leads somewhere.
    pub t_return: Option<u64>,
}

pub struct Device {
    pub mem: Vec<u8>,
    pub eeprom: Vec<u8>,
    pub sii: Sii,
    pub od: Od,
    pub model: Box<dyn Model>,
    pub delay_ns: u64,
    /// Local clock = simulation time + this (each device powered on at a different moment).
    pub clock_skew: u64,
    pub coe: bool,
    mbx: Mailbox,
    out_map: Vec<MapEntry>,
    in_map: Vec<MapEntry>,
    out_sm: Option<usize>,
    in_sm: Option<usize>,
    last_output_ns: u64,
    last_tick_ns: Option<u64>,
    /// Port 1 has a neighbour (set by the segment from the topology).
    pub port1_link: bool,
    /// Frames this device has seen, and frames carrying a valid process data write.
    pub frames: u64,
}

fn rd16(m: &[u8], a: u16) -> u16 {
    u16::from_le_bytes([m[a as usize], m[a as usize + 1]])
}

fn rd32(m: &[u8], a: u16) -> u32 {
    u32::from_le_bytes(m[a as usize..a as usize + 4].try_into().unwrap())
}

fn rd64(m: &[u8], a: u16) -> u64 {
    u64::from_le_bytes(m[a as usize..a as usize + 8].try_into().unwrap())
}

fn wr16(m: &mut [u8], a: u16, v: u16) {
    m[a as usize..a as usize + 2].copy_from_slice(&v.to_le_bytes());
}

fn wr32(m: &mut [u8], a: u16, v: u32) {
    m[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr64(m: &mut [u8], a: u16, v: u64) {
    m[a as usize..a as usize + 8].copy_from_slice(&v.to_le_bytes());
}

/// A sync manager as configured in its registers.
#[derive(Debug, Clone, Copy)]
struct Sm {
    start: u16,
    len: u16,
    control: u8,
    enabled: bool,
}

impl Sm {
    fn mailbox(&self) -> bool {
        self.control & 0b11 == 0b10
    }
    /// The master writes into it (outputs, master-to-device mailbox).
    fn ecat_writes(&self) -> bool {
        (self.control >> 2) & 0b11 == 0b01
    }
    fn overlaps(&self, a: u32, n: u32) -> bool {
        self.enabled && self.len > 0 && a < self.start as u32 + self.len as u32 && a + n > self.start as u32
    }
    fn covers_last(&self, a: u32, n: u32) -> bool {
        let last = self.start as u32 + self.len as u32 - 1;
        self.enabled && self.len > 0 && a <= last && a + n > last
    }
}

impl Device {
    pub fn new(spec: DeviceSpec, clock_skew: u64) -> Device {
        let sii = Sii::parse(&spec.eeprom);
        let mut mem = vec![0u8; MEM_SIZE];
        mem[esc::TYPE as usize] = spec.esc_type;
        mem[esc::REVISION as usize] = spec.esc_revision;
        wr16(&mut mem, esc::BUILD, spec.esc_build);
        mem[esc::FMMU_COUNT as usize] = spec.fmmus;
        mem[esc::SM_COUNT as usize] = spec.sms;
        mem[esc::RAM_SIZE as usize] = 8;
        mem[esc::PORT_DESC as usize] = spec.port_desc;
        wr16(&mut mem, esc::FEATURES, spec.features);
        // Alias and PDI control come from the EEPROM at power-on.
        wr16(&mut mem, esc::STATION_ALIAS, sii.alias);
        mem[esc::PDI_CONTROL as usize] = sii.pdi_control as u8;
        mem[esc::ESC_CONFIG as usize] = (sii.pdi_control >> 8) as u8;
        wr16(&mut mem, esc::AL_STATUS, AlState::Init.code() as u16);
        wr32(&mut mem, esc::DL_CONTROL, 0x0007_0001);
        // SII interface: 8-byte reads, EEPROM loaded; checksum error flag if the header CRC is wrong.
        wr16(&mut mem, esc::SII_CONTROL, 0x0040 | if sii.checksum_ok { 0 } else { 0x0800 });
        // Watchdog defaults: divider 2498 (100 us), process data watchdog 1000 (100 ms), not expired.
        wr16(&mut mem, esc::WATCHDOG_DIVIDER, 2498);
        wr16(&mut mem, esc::WATCHDOG_PDI, 1000);
        wr16(&mut mem, esc::WATCHDOG_PD, 1000);
        let coe = sii.mailbox_protocols & crate::sii::MBX_COE != 0 && sii.std_tx.1 > 0;
        let mut mbx = Mailbox::default();
        for (i, s) in sii.sync_managers.iter().enumerate() {
            match s.kind {
                SmType::MailboxOut => mbx.rx_sm = Some(i),
                SmType::MailboxIn => mbx.tx_sm = Some(i),
                _ => {}
            }
        }
        let out_sm = sii.sync_managers.iter().position(|s| s.kind == SmType::Outputs);
        let in_sm = sii.sync_managers.iter().position(|s| s.kind == SmType::Inputs);
        Device {
            mem,
            eeprom: spec.eeprom,
            sii,
            od: spec.od,
            model: spec.model,
            delay_ns: spec.delay_ns,
            clock_skew,
            coe,
            mbx,
            out_map: Vec::new(),
            in_map: Vec::new(),
            out_sm,
            in_sm,
            last_output_ns: 0,
            last_tick_ns: None,
            port1_link: false,
            frames: 0,
        }
    }

    pub fn station_address(&self) -> u16 {
        rd16(&self.mem, esc::STATION_ADDR)
    }

    pub fn state(&self) -> AlState {
        AlState::from_code(self.mem[esc::AL_STATUS as usize])
    }

    pub fn al_error(&self) -> Option<u16> {
        (self.mem[esc::AL_STATUS as usize] & 0x10 != 0).then(|| rd16(&self.mem, esc::AL_STATUS_CODE))
    }

    pub fn name(&self) -> String {
        self.sii.general.as_ref().map(|g| g.order.clone()).unwrap_or_default()
    }

    pub fn local_time(&self, t: u64) -> u64 {
        t.wrapping_add(self.clock_skew)
    }

    fn system_time(&self, t: u64) -> u64 {
        self.local_time(t).wrapping_add(rd64(&self.mem, esc::DC_SYSTEM_OFFSET))
    }

    fn sm(&self, n: usize) -> Sm {
        let a = esc::sm(n as u16);
        Sm {
            start: rd16(&self.mem, a),
            len: rd16(&self.mem, a + 2),
            control: self.mem[a as usize + 4],
            enabled: self.mem[a as usize + 6] & 1 != 0,
        }
    }

    fn set_sm_full(&mut self, n: usize, full: bool) {
        let a = esc::sm(n as u16) as usize + 5;
        if full {
            self.mem[a] |= 0x08;
        } else {
            self.mem[a] &= !0x08;
        }
    }

    /// Advances the application and checks the process data watchdog. Called for every frame that reaches the
    /// device, before the frame is processed.
    pub fn tick(&mut self, now: u64) {
        let dt = self.last_tick_ns.map(|t| now.saturating_sub(t) as f64 / 1e9).unwrap_or(0.0).min(0.5);
        self.last_tick_ns = Some(now);
        let op = self.state() == AlState::Op && self.al_error().is_none();
        self.model.tick(dt, &mut self.od, op);

        // Process data watchdog: outputs must keep coming in OP.
        if op && !self.out_map.is_empty() {
            if let Some(n) = self.out_sm {
                let sm = self.sm(n);
                let divider = rd16(&self.mem, esc::WATCHDOG_DIVIDER) as u64;
                let ticks = rd16(&self.mem, esc::WATCHDOG_PD) as u64;
                let wd_ns = (divider + 2) * 40 * ticks;
                if sm.control & 0x40 != 0 && wd_ns > 0 && now.saturating_sub(self.last_output_ns) > wd_ns {
                    self.mem[esc::WATCHDOG_STATUS as usize] &= !1;
                    self.mem[esc::WATCHDOG_COUNTER_PD as usize] = self.mem[esc::WATCHDOG_COUNTER_PD as usize].saturating_add(1);
                    self.enter(AlState::SafeOp, Some(0x001B));
                }
            }
        }
    }

    /// Sets the AL status, with an error code or without.
    fn enter(&mut self, state: AlState, error: Option<u16>) {
        let before = self.state();
        self.mem[esc::AL_STATUS as usize] = state.code() | if error.is_some() { 0x10 } else { 0 };
        wr16(&mut self.mem, esc::AL_STATUS_CODE, error.unwrap_or(0));
        if state != before {
            if state == AlState::Init {
                self.mbx.queue.clear();
                self.mbx.out_full = false;
                self.mbx.upload = None;
                self.mbx.download = None;
                self.mbx.last_counter = 0;
                if let Some(n) = self.mbx.tx_sm {
                    self.set_sm_full(n, false);
                }
            }
            self.model.state_changed(state, &mut self.od);
        }
    }

    /// The master wrote AL control.
    fn al_control(&mut self, now: u64) {
        let ctl = rd16(&self.mem, esc::AL_CONTROL);
        let req = AlState::from_code(ctl as u8);
        let ack = ctl & 0x10 != 0;
        let cur = self.state();
        let in_error = self.al_error().is_some();
        if in_error && !ack && req.code() > cur.code() {
            return;
        }
        if req == AlState::Unknown {
            return self.enter(cur, Some(0x0012));
        }
        let result = match (cur, req) {
            (_, AlState::Init) => Ok(()),
            (_, s) if s == cur => Ok(()),
            (AlState::Init, AlState::PreOp) => self.check_mailbox(),
            (AlState::Init, AlState::Boot) => Err(0x0013),
            (AlState::PreOp, AlState::SafeOp) => self.check_process_data(),
            (AlState::SafeOp | AlState::Op, AlState::PreOp) => Ok(()),
            (AlState::SafeOp, AlState::Op) => {
                // Outputs must have arrived recently, else the device would have no valid outputs.
                self.last_output_ns = now;
                Ok(())
            }
            (AlState::Op, AlState::SafeOp) => Ok(()),
            _ => Err(0x0011),
        };
        match result {
            Ok(()) => {
                if req == AlState::SafeOp || req == AlState::Op {
                    self.mem[esc::WATCHDOG_STATUS as usize] |= 1;
                }
                self.enter(req, None)
            }
            Err(code) => self.enter(cur, Some(code)),
        }
    }

    fn check_mailbox(&self) -> Result<(), u16> {
        let (Some(rx), Some(tx)) = (self.mbx.rx_sm, self.mbx.tx_sm) else { return Ok(()) };
        let r = self.sm(rx);
        let t = self.sm(tx);
        let ok = r.enabled
            && t.enabled
            && r.mailbox()
            && t.mailbox()
            && r.ecat_writes()
            && !t.ecat_writes()
            && (r.start, r.len) == self.sii.std_rx
            && (t.start, t.len) == self.sii.std_tx;
        if ok { Ok(()) } else { Err(0x0016) }
    }

    fn check_process_data(&mut self) -> Result<(), u16> {
        let (outs, ins) = if self.coe {
            (self.od.assigned_mapping(0x1C12).map_err(|_| 0x0025u16)?, self.od.assigned_mapping(0x1C13).map_err(|_| 0x0024u16)?)
        } else {
            let pick = |pdos: &[crate::sii::SiiPdo]| -> Vec<MapEntry> {
                pdos.iter()
                    .filter(|p| p.sm != 0xFF)
                    .flat_map(|p| p.entries.iter().map(|e| MapEntry { index: e.index, sub: e.sub, bits: e.bits }))
                    .collect()
            };
            (pick(&self.sii.rx_pdos), pick(&self.sii.tx_pdos))
        };
        for (map, sm, code) in [(&outs, self.out_sm, 0x001Du16), (&ins, self.in_sm, 0x001Eu16)] {
            let bytes = total_bits(map).div_ceil(8);
            if bytes == 0 {
                continue;
            }
            let Some(n) = sm else { return Err(code) };
            let s = self.sm(n);
            if !s.enabled || s.len as usize != bytes || s.mailbox() {
                return Err(code);
            }
        }
        self.out_map = outs;
        self.in_map = ins;
        Ok(())
    }

    /// Whether a master write may land on register byte `a`.
    fn writable(a: u16) -> bool {
        matches!(a,
            0x0010..=0x0013 | 0x0100..=0x0103 | 0x0120..=0x0121 | 0x0200..=0x0203 | 0x0400..=0x0401 | 0x0410..=0x0411
            | 0x0420..=0x0421 | 0x0500..=0x050F | 0x0600..=0x06FF | 0x0920..=0x092B | 0x0930..=0x0935 | 0x0980..=0x09FF
        ) || ((0x0800..0x0880).contains(&a) && (a - 0x0800) % 8 != 5)
    }

    /// Physical read of `buf.len()` bytes at `addr` (register or RAM). `false` when the ESC refuses the access
    /// (an empty device-to-master mailbox, a master-to-device mailbox), so the working counter stays.
    pub fn read(&mut self, addr: u16, buf: &mut [u8], now: u64) -> bool {
        let a = addr as u32;
        let n = buf.len() as u32;
        let sms: Vec<(usize, Sm)> = (0..16).map(|i| (i, self.sm(i))).filter(|(_, s)| s.overlaps(a, n)).collect();
        for (i, s) in &sms {
            if s.mailbox() && (s.ecat_writes() || !self.mbx.out_full || Some(*i) != self.mbx.tx_sm) {
                return false;
            }
        }
        // Inputs are copied into their buffer when the master reads it.
        if let Some(n) = self.in_sm {
            if sms.iter().any(|(i, _)| *i == n) && matches!(self.state(), AlState::SafeOp | AlState::Op) {
                let s = self.sm(n);
                let mut img = vec![0u8; s.len as usize];
                self.od.pack(&self.in_map, &mut img);
                let start = s.start as usize;
                self.mem[start..start + img.len()].copy_from_slice(&img);
            }
        }
        self.refresh_dynamic(now);
        for (i, b) in buf.iter_mut().enumerate() {
            let at = addr as usize + i;
            *b = if at < MEM_SIZE { self.mem[at] } else { 0 };
        }
        if let Some(tx) = self.mbx.tx_sm {
            if sms.iter().any(|(i, s)| *i == tx && s.covers_last(a, n)) {
                self.mbx.out_full = false;
                self.set_sm_full(tx, false);
                self.next_response();
            }
        }
        true
    }

    fn refresh_dynamic(&mut self, now: u64) {
        let mut dl: u16 = 0;
        // Port 0 always linked and communicating (the frame got here); port 1 linked if a neighbour is there;
        // ports without a link are closed. Ports 2 and 3 unused.
        dl |= 1 << 4 | 1 << 9;
        if self.port1_link {
            dl |= 1 << 5 | 1 << 11;
        } else {
            dl |= 1 << 10;
        }
        dl |= 1 << 12 | 1 << 14;
        // PDI operational, watchdog status, EEPROM loaded.
        dl |= 0x0001 | 0x0002 | 0x0004;
        wr16(&mut self.mem, esc::DL_STATUS, dl);
        let st = self.system_time(now);
        wr64(&mut self.mem, esc::DC_SYSTEM_TIME, st);
    }

    /// Physical write. `false` when refused (the mailbox is full, or the area belongs to a device-to-master
    /// buffer). Side effects (AL control, EEPROM commands, error counter reset, mailbox, outputs) follow.
    pub fn write(&mut self, addr: u16, data: &[u8], pass: Pass) -> bool {
        let a = addr as u32;
        let n = data.len() as u32;
        let sms: Vec<(usize, Sm)> = (0..16).map(|i| (i, self.sm(i))).filter(|(_, s)| s.overlaps(a, n)).collect();
        for (i, s) in &sms {
            if !s.ecat_writes() {
                return false;
            }
            if s.mailbox() && Some(*i) != self.mbx.rx_sm {
                return false;
            }
        }
        let in_sm_area = |at: u16| sms.iter().any(|(_, s)| at >= s.start && at < s.start + s.len);
        for (i, &b) in data.iter().enumerate() {
            let at = addr as usize + i;
            if at >= MEM_SIZE {
                break;
            }
            let at16 = at as u16;
            if at16 >= esc::PROCESS_RAM || in_sm_area(at16) || Self::writable(at16) {
                self.mem[at] = b;
            }
        }
        let touches = |reg: u16, len: u16| a < reg as u32 + len as u32 && a + n > reg as u32;
        let t = pass.t_in;
        if touches(esc::AL_CONTROL, 2) {
            self.al_control(t);
        }
        if touches(esc::RX_ERROR, 0x14) {
            for b in &mut self.mem[esc::RX_ERROR as usize..esc::RX_ERROR as usize + 0x14] {
                *b = 0;
            }
        }
        if touches(esc::WATCHDOG_COUNTER_PD, 2) {
            self.mem[esc::WATCHDOG_COUNTER_PD as usize] = 0;
            self.mem[esc::WATCHDOG_COUNTER_PD as usize + 1] = 0;
        }
        if touches(esc::SII_CONTROL, 2) {
            self.eeprom_command();
        }
        if touches(esc::DC_RECV_TIME, 4) {
            // Latch the receive times of every port (32-bit local times) and of the processing unit (64-bit).
            let p0 = self.local_time(t);
            wr32(&mut self.mem, esc::DC_RECV_TIME, p0 as u32);
            let p1 = pass.t_return.map(|r| self.local_time(r) as u32).unwrap_or(0);
            wr32(&mut self.mem, esc::DC_RECV_TIME + 4, p1);
            wr32(&mut self.mem, esc::DC_RECV_TIME + 8, 0);
            wr32(&mut self.mem, esc::DC_RECV_TIME + 12, 0);
            wr64(&mut self.mem, esc::DC_RECV_TIME_PU, p0);
        }
        if touches(esc::DC_SYSTEM_TIME, 8) && n >= 4 {
            // Drift control input: compare the reference time (plus our delay) with our own system time.
            let written = if n >= 8 && a == esc::DC_SYSTEM_TIME as u32 {
                u64::from_le_bytes(data[..8].try_into().unwrap())
            } else {
                u32::from_le_bytes(data[..4].try_into().unwrap()) as u64
            };
            let delay = rd32(&self.mem, esc::DC_SYSTEM_DELAY) as u64;
            let own = self.system_time(t);
            let diff = (written.wrapping_add(delay) as i64).wrapping_sub(own as i64);
            let mag = diff.unsigned_abs().min(0x7FFF_FFFF) as u32;
            wr32(&mut self.mem, esc::DC_SYSTEM_DIFF, mag | if diff < 0 { 0x8000_0000 } else { 0 });
        }
        if let Some(rx) = self.mbx.rx_sm {
            if sms.iter().any(|(i, s)| *i == rx && s.covers_last(a, n)) {
                self.mailbox_request();
            }
        }
        if let Some(o) = self.out_sm {
            if sms.iter().any(|(i, s)| *i == o && s.covers_last(a, n)) {
                self.last_output_ns = t;
                if self.state() == AlState::Op && self.al_error().is_none() {
                    let s = self.sm(o);
                    let img = self.mem[s.start as usize..s.start as usize + s.len as usize].to_vec();
                    let map = self.out_map.clone();
                    self.od.unpack(&map, &img);
                }
            }
        }
        true
    }

    fn eeprom_command(&mut self) {
        let ctl = rd16(&self.mem, esc::SII_CONTROL);
        let word = rd32(&self.mem, esc::SII_ADDRESS) as usize;
        let cmd = (ctl >> 8) & 0x07;
        let mut status = ctl & 0x0841 & !0x0700;
        status |= 0x0040;
        match cmd {
            0b001 => {
                for i in 0..8 {
                    let at = word * 2 + i;
                    self.mem[esc::SII_DATA as usize + i] = self.eeprom.get(at).copied().unwrap_or(0xFF);
                }
            }
            0b010 => {
                // Write one word; needs the write enable bit in the same access.
                if ctl & 0x0001 != 0 && word * 2 + 1 < self.eeprom.len() {
                    self.eeprom[word * 2] = self.mem[esc::SII_DATA as usize];
                    self.eeprom[word * 2 + 1] = self.mem[esc::SII_DATA as usize + 1];
                    self.sii = Sii::parse(&self.eeprom);
                } else {
                    status |= 0x2000;
                }
            }
            0b100 => {
                // Reload: alias and PDI configuration from the EEPROM.
                wr16(&mut self.mem, esc::STATION_ALIAS, self.sii.alias);
            }
            _ => {}
        }
        if !self.sii.checksum_ok {
            status |= 0x0800;
        }
        wr16(&mut self.mem, esc::SII_CONTROL, status);
    }

    // ---- mailbox ---------------------------------------------------------------------------------------------

    fn mbx_size(&self) -> usize {
        self.mbx.tx_sm.map(|n| self.sm(n).len as usize).unwrap_or(0)
    }

    fn mailbox_request(&mut self) {
        let Some(rx) = self.mbx.rx_sm else { return };
        let s = self.sm(rx);
        let raw = self.mem[s.start as usize..s.start as usize + s.len as usize].to_vec();
        let Some(h) = coe::MbxHeader::parse(&raw) else { return };
        // A repeated counter means the master sent the same message again: ignore it.
        if h.counter != 0 && h.counter == self.mbx.last_counter {
            return;
        }
        self.mbx.last_counter = h.counter;
        let len = (h.len as usize).min(raw.len() - coe::MBX_HEADER);
        let body = raw[coe::MBX_HEADER..coe::MBX_HEADER + len].to_vec();
        let replies: Vec<(u8, Vec<u8>)> = if h.kind == coe::MBX_TYPE_COE && self.coe {
            self.coe_request(&body).into_iter().map(|b| (coe::MBX_TYPE_COE, b)).collect()
        } else {
            // Mailbox error: protocol not supported.
            let mut b = 0x0001u16.to_le_bytes().to_vec();
            b.extend_from_slice(&0x0002u16.to_le_bytes());
            vec![(coe::MBX_TYPE_ERROR, b)]
        };
        for (kind, body) in replies {
            self.mbx.counter = if self.mbx.counter >= 7 { 1 } else { self.mbx.counter + 1 };
            self.mbx.queue.push_back(coe::mailbox(kind, self.mbx.counter, &body));
        }
        if !self.mbx.out_full {
            self.next_response();
        }
    }

    fn next_response(&mut self) {
        let Some(tx) = self.mbx.tx_sm else { return };
        let Some(msg) = self.mbx.queue.pop_front() else { return };
        let s = self.sm(tx);
        let start = s.start as usize;
        let len = s.len as usize;
        for b in &mut self.mem[start..start + len] {
            *b = 0;
        }
        let n = msg.len().min(len);
        self.mem[start..start + n].copy_from_slice(&msg[..n]);
        self.mbx.out_full = true;
        self.set_sm_full(tx, true);
    }

    fn readable_now(&self, access: u16) -> bool {
        match self.state() {
            AlState::Op => access & coe::ACC_READ_OP != 0,
            AlState::SafeOp => access & coe::ACC_READ_SAFEOP != 0,
            _ => access & coe::ACC_READ_PREOP != 0,
        }
    }

    fn writable_now(&self, access: u16) -> bool {
        match self.state() {
            AlState::Op => access & coe::ACC_WRITE_OP != 0,
            AlState::SafeOp => access & coe::ACC_WRITE_SAFEOP != 0,
            _ => access & coe::ACC_WRITE_PREOP != 0,
        }
    }

    fn coe_request(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        match coe::coe_service(body) {
            Some(coe::SVC_SDO_REQ) => self.sdo(body).into_iter().collect(),
            Some(coe::SVC_SDO_INFO) => self.sdo_info(body),
            _ => {
                let mut b = 0x0001u16.to_le_bytes().to_vec();
                b.extend_from_slice(&0x0004u16.to_le_bytes());
                vec![b]
            }
        }
    }

    /// Handles an SDO request; `None` for an abort from the master, which gets no answer.
    fn sdo(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        let req = match coe::parse_sdo_request(body) {
            Ok(r) => r,
            Err(code) => return Some(coe::abort_response(0, 0, code)),
        };
        if let SdoRequest::Abort { .. } = req {
            self.mbx.upload = None;
            self.mbx.download = None;
            return None;
        }
        Some(self.sdo_reply(req))
    }

    fn sdo_reply(&mut self, req: SdoRequest) -> Vec<u8> {
        let mbx = self.mbx_size();
        match req {
            SdoRequest::Upload { index, sub, complete } => {
                self.mbx.upload = None;
                if complete {
                    return coe::abort_response(index, sub, 0x0601_0000);
                }
                let Some(obj) = self.od.objects.get(&index) else { return coe::abort_response(index, sub, 0x0602_0000) };
                let Some(e) = obj.entries.get(&sub) else { return coe::abort_response(index, sub, 0x0609_0011) };
                if !self.readable_now(e.access) {
                    return coe::abort_response(index, sub, if e.access & coe::ACC_RO == 0 { 0x0601_0001 } else { 0x0800_0022 });
                }
                let data = e.value.clone();
                let (resp, n) = coe::upload_response(index, sub, &data, mbx);
                if n < data.len() && data.len() > 4 {
                    self.mbx.upload = Some((data, n, false));
                }
                resp
            }
            SdoRequest::UploadSegment { toggle } => {
                let Some((data, pos, expect)) = self.mbx.upload.take() else { return coe::abort_response(0, 0, 0x0504_0001) };
                if toggle != expect {
                    return coe::abort_response(0, 0, 0x0503_0000);
                }
                let room = mbx.saturating_sub(coe::MBX_HEADER + 3).max(7);
                let end = (pos + room).min(data.len());
                let last = end == data.len();
                let resp = coe::upload_segment_response(toggle, last, &data[pos..end]);
                if !last {
                    self.mbx.upload = Some((data, end, !toggle));
                }
                resp
            }
            SdoRequest::Download { index, sub, complete, data, size } => {
                self.mbx.download = None;
                if complete {
                    return coe::abort_response(index, sub, 0x0601_0000);
                }
                let Some(obj) = self.od.objects.get(&index) else { return coe::abort_response(index, sub, 0x0602_0000) };
                let Some(e) = obj.entries.get(&sub) else { return coe::abort_response(index, sub, 0x0609_0011) };
                if e.access & (coe::ACC_WRITE_PREOP | coe::ACC_WRITE_SAFEOP | coe::ACC_WRITE_OP) == 0 {
                    return coe::abort_response(index, sub, 0x0601_0002);
                }
                if !self.writable_now(e.access) {
                    return coe::abort_response(index, sub, 0x0800_0022);
                }
                if (data.len() as u32) < size {
                    self.mbx.download = Some((index, sub, size, data, false));
                    return coe::download_response(index, sub);
                }
                match self.apply_write(index, sub, data) {
                    Ok(()) => coe::download_response(index, sub),
                    Err(code) => coe::abort_response(index, sub, code),
                }
            }
            SdoRequest::DownloadSegment { toggle, last, data } => {
                let Some((index, sub, size, mut buf, expect)) = self.mbx.download.take() else {
                    return coe::abort_response(0, 0, 0x0504_0001);
                };
                if toggle != expect {
                    return coe::abort_response(index, sub, 0x0503_0000);
                }
                buf.extend_from_slice(&data);
                if !last {
                    self.mbx.download = Some((index, sub, size, buf, !toggle));
                    return coe::download_segment_response(toggle);
                }
                buf.truncate(size as usize);
                match self.apply_write(index, sub, buf) {
                    Ok(()) => coe::download_segment_response(toggle),
                    Err(code) => coe::abort_response(index, sub, code),
                }
            }
            SdoRequest::Abort { .. } => unreachable!("handled in sdo()"),
        }
    }

    fn apply_write(&mut self, index: u16, sub: u8, data: Vec<u8>) -> Result<(), u32> {
        let obj = self.od.objects.get(&index).ok_or(0x0602_0000u32)?;
        let existing_max = obj.entries.keys().copied().max().unwrap_or(0);
        let is_container = obj.code != coe::OBJ_VAR;
        let e = obj.entries.get(&sub).ok_or(0x0609_0011u32)?;
        if crate::coe::data_type_bits(e.data_type).is_some() {
            let want = e.value.len();
            if data.len() < want {
                return Err(0x0607_0013);
            }
            if data.len() > want {
                return Err(0x0607_0012);
            }
        }
        if is_container && sub == 0 && data[0] > existing_max {
            return Err(0x0609_0031);
        }
        let e = self.od.entry_mut(index, sub).ok_or(0x0609_0011u32)?;
        e.value = data;
        Ok(())
    }

    fn sdo_info(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        let Ok(f) = coe::parse_info(body) else { return vec![coe::info_error(0x0504_0001)] };
        let p = &f.payload;
        let room = self.mbx_size().saturating_sub(coe::MBX_HEADER + 6).max(2) & !1;
        match f.opcode {
            coe::INFO_OD_LIST_REQ if p.len() >= 2 => {
                let list = u16::from_le_bytes([p[0], p[1]]);
                let mut payload = list.to_le_bytes().to_vec();
                if list == 0 {
                    for t in 1..=5u16 {
                        payload.extend_from_slice(&(self.od.list(t).len() as u16).to_le_bytes());
                    }
                } else {
                    for i in self.od.list(list) {
                        payload.extend_from_slice(&i.to_le_bytes());
                    }
                }
                let chunks: Vec<&[u8]> = payload.chunks(room).collect();
                let total = chunks.len();
                chunks
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| coe::info_response(coe::INFO_OD_LIST_RESP, i + 1 < total, (total - 1 - i) as u16, c))
                    .collect()
            }
            coe::INFO_OBJ_DESC_REQ if p.len() >= 2 => {
                let index = u16::from_le_bytes([p[0], p[1]]);
                let Some(o) = self.od.objects.get(&index) else { return vec![coe::info_error(0x0602_0000)] };
                let mut r = index.to_le_bytes().to_vec();
                r.extend_from_slice(&o.data_type.to_le_bytes());
                r.push(o.max_sub());
                r.push(o.code);
                r.extend_from_slice(o.name.as_bytes());
                vec![coe::info_response(coe::INFO_OBJ_DESC_RESP, false, 0, &r)]
            }
            coe::INFO_ENTRY_DESC_REQ if p.len() >= 4 => {
                let index = u16::from_le_bytes([p[0], p[1]]);
                let sub = p[2];
                let Some(o) = self.od.objects.get(&index) else { return vec![coe::info_error(0x0602_0000)] };
                let Some(e) = o.entries.get(&sub) else { return vec![coe::info_error(0x0609_0011)] };
                let mut r = index.to_le_bytes().to_vec();
                r.push(sub);
                r.push(0);
                r.extend_from_slice(&e.data_type.to_le_bytes());
                r.extend_from_slice(&e.bits.to_le_bytes());
                r.extend_from_slice(&e.access.to_le_bytes());
                r.extend_from_slice(e.name.as_bytes());
                vec![coe::info_response(coe::INFO_ENTRY_DESC_RESP, false, 0, &r)]
            }
            _ => vec![coe::info_error(0x0504_0001)],
        }
    }

    /// Adds to the RX error counters of `port` (invalid frames), as a bad cable or connector would.
    pub fn add_rx_errors(&mut self, port: usize, n: u8) {
        let a = esc::RX_ERROR as usize + 2 * port.min(3);
        self.mem[a] = self.mem[a].saturating_add(n);
        self.mem[a + 1] = self.mem[a + 1].saturating_add(n / 2);
    }

    pub fn add_lost_link(&mut self, port: usize) {
        let a = esc::LOST_LINK as usize + port.min(3);
        self.mem[a] = self.mem[a].saturating_add(1);
    }

    /// Process data sizes in bits, as mapped for the current SAFE-OP/OP (0 before).
    pub fn pd_bits(&self) -> (usize, usize) {
        (total_bits(&self.out_map), total_bits(&self.in_map))
    }

    /// Logical access through the FMMUs. Returns the working counter increment for this device.
    pub fn logical(&mut self, cmd: crate::wire::Cmd, log_addr: u32, data: &mut [u8], pass: Pass) -> u16 {
        use crate::wire::Cmd;
        let dlen = data.len() as u64;
        let dg_start = log_addr as u64 * 8;
        let dg_end = dg_start + dlen * 8;
        let mut did_read = false;
        let mut did_write = false;
        let count = self.mem[esc::FMMU_COUNT as usize].min(16) as u16;
        for f in 0..count {
            let b = esc::fmmu(f) as usize;
            if self.mem[b + 12] & 1 == 0 {
                continue;
            }
            let lstart = u32::from_le_bytes(self.mem[b..b + 4].try_into().unwrap()) as u64;
            let len = u16::from_le_bytes([self.mem[b + 4], self.mem[b + 5]]) as u64;
            let lsb = self.mem[b + 6] as u64 & 7;
            let leb = self.mem[b + 7] as u64 & 7;
            let pstart = u16::from_le_bytes([self.mem[b + 8], self.mem[b + 9]]) as u64;
            let psb = self.mem[b + 10] as u64 & 7;
            let kind = self.mem[b + 11];
            if len == 0 {
                continue;
            }
            let f_start = lstart * 8 + lsb;
            let f_end = (lstart + len - 1) * 8 + leb + 1;
            let s = f_start.max(dg_start);
            let e = f_end.min(dg_end);
            if s >= e {
                continue;
            }
            let reading = kind & 1 != 0 && matches!(cmd, Cmd::Lrd | Cmd::Lrw);
            let writing = kind & 2 != 0 && matches!(cmd, Cmd::Lwr | Cmd::Lrw);
            if !reading && !writing {
                continue;
            }
            // Physical byte range touched by this FMMU's part of the datagram.
            let p_first = pstart * 8 + psb + (s - f_start);
            let p_last = pstart * 8 + psb + (e - f_start) - 1;
            let pa = (p_first / 8) as u16;
            let pn = (p_last / 8 - p_first / 8 + 1) as usize;
            if reading {
                let mut phys = vec![0u8; pn];
                if self.read(pa, &mut phys, pass.t_in) {
                    for bit in s..e {
                        let pb = pstart * 8 + psb + (bit - f_start) - pa as u64 * 8;
                        let v = phys[(pb / 8) as usize] >> (pb % 8) & 1;
                        let db = bit - dg_start;
                        let byte = &mut data[(db / 8) as usize];
                        if v != 0 { *byte |= 1 << (db % 8) } else { *byte &= !(1 << (db % 8)) }
                    }
                    did_read = true;
                }
            }
            if writing {
                let mut phys = self.mem[pa as usize..pa as usize + pn].to_vec();
                for bit in s..e {
                    let pb = pstart * 8 + psb + (bit - f_start) - pa as u64 * 8;
                    let db = bit - dg_start;
                    let v = data[(db / 8) as usize] >> (db % 8) & 1;
                    let byte = &mut phys[(pb / 8) as usize];
                    if v != 0 { *byte |= 1 << (pb % 8) } else { *byte &= !(1 << (pb % 8)) }
                }
                if self.write(pa, &phys, pass) {
                    did_write = true;
                }
            }
        }
        match cmd {
            Cmd::Lrw => did_read as u16 + 2 * did_write as u16,
            Cmd::Lrd => did_read as u16,
            Cmd::Lwr => did_write as u16,
            _ => 0,
        }
    }
}
