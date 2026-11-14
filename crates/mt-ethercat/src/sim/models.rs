//! The simulated devices: an EK1100 coupler, EL1008 digital inputs, EL2008 digital outputs, EL3102 analog inputs,
//! EL4102 analog outputs (identities and PDO layouts as the real Beckhoff terminals report them), and a generic
//! CiA 402 servo drive. Each has its SII image, object dictionary and a model of its behaviour.

use crate::coe::{ACC_RO, ACC_RW, ACC_RXPDO, ACC_TXPDO};
use crate::esc::AlState;
use crate::sii::{self, SiiBuilder, SiiPdo, SiiPdoEntry, SiiSyncManager, SmType};
use crate::sim::device::{DeviceSpec, Model, Passive};
use crate::sim::od::{MapEntry, Od};

pub const BECKHOFF: u32 = 0x0000_0002;

/// Small deterministic noise source.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    }
}

fn sm(start: u16, len: u16, control: u8, enable: u8, kind: SmType) -> SiiSyncManager {
    SiiSyncManager { start, len, control, enable, kind }
}

/// One PDO per channel with a single BOOL entry (`obj_base + 0x10 * channel`, sub 1), as EL1xxx/EL2xxx do.
fn bit_pdos(pdo_base: u16, obj_base: u16, sm: u8, channels: u16, entry_name: &str) -> Vec<SiiPdo> {
    (0..channels)
        .map(|c| SiiPdo {
            index: pdo_base + c,
            sm,
            name: format!("Channel {}", c + 1),
            entries: vec![SiiPdoEntry { index: obj_base + 0x10 * c, sub: 1, name: entry_name.into(), data_type: 1, bits: 1 }],
        })
        .collect()
}

/// `dc_sync`: the device can run DC SYNC signals (ESC feature bit 8); without it, DC is a time reference only.
fn ebus_terminal(esc_type: u8, dc_sync: bool, eeprom: Vec<u8>, od: Od, model: Box<dyn Model>) -> DeviceSpec {
    DeviceSpec {
        esc_type,
        esc_revision: 0x02,
        esc_build: 0x0004,
        fmmus: 3,
        sms: 4,
        port_desc: 0x0A,
        features: if dc_sync { 0x010C } else { 0x000C },
        eeprom,
        od,
        model,
        delay_ns: 110,
    }
}

// ---- EK1100 -------------------------------------------------------------------------------------------------

pub fn ek1100(serial: u32) -> DeviceSpec {
    let eeprom = SiiBuilder::new(BECKHOFF, 0x044C_2C52, 0x0012_0000, serial)
        .general("SystemBk", "EK1100", "EK1100 EtherCAT Coupler (2A E-Bus)", 0, -2000)
        .build();
    DeviceSpec {
        esc_type: 0x11,
        esc_revision: 0x00,
        esc_build: 0x0003,
        fmmus: 8,
        sms: 8,
        port_desc: 0xCB,
        features: 0x010C,
        eeprom,
        od: Od::default(),
        model: Box::new(Passive),
        delay_ns: 320,
    }
}

// ---- EL1008 -------------------------------------------------------------------------------------------------

pub struct DigitalInputs {
    t: f64,
    wired: [bool; 4],
    start_button: bool,
    estop_ok: bool,
}

impl Model for DigitalInputs {
    fn tick(&mut self, dt: f64, od: &mut Od, _op: bool) {
        self.t += dt;
        let chans = [
            self.wired[0],
            self.wired[1],
            self.wired[2],
            self.wired[3],
            // A part passing a proximity sensor: present for 0.6 s of every 1.5 s.
            self.t % 1.5 < 0.6,
            // A 1 Hz blinker.
            self.t % 1.0 < 0.5,
            self.start_button,
            self.estop_ok,
        ];
        for (c, v) in chans.into_iter().enumerate() {
            od.set_u64(0x6000 + 0x10 * c as u16, 1, v as u64);
        }
    }

    fn action(&mut self, action: &str, _od: &mut Od) -> bool {
        match action {
            "start" => self.start_button = !self.start_button,
            "estop" => self.estop_ok = !self.estop_ok,
            _ => return false,
        }
        true
    }

    fn actions(&self) -> Vec<(&'static str, &'static str)> {
        vec![("start", "Toggle start button (input 7)"), ("estop", "Toggle E-stop (input 8)")]
    }

    fn summary(&self, od: &Od) -> Vec<(String, String)> {
        let bits: String = (0..8).map(|c| if od.get_bool(0x6000 + 0x10 * c, 1) { '1' } else { '0' }).collect();
        vec![
            ("Inputs 1-8".into(), bits),
            ("Inputs 1-4".into(), "wired from EL2008 outputs 1-4".into()),
            ("Input 5".into(), "part sensor, 0.6 s of every 1.5 s".into()),
            ("Input 6".into(), "1 Hz blinker".into()),
            ("Start button (7)".into(), if self.start_button { "pressed" } else { "released" }.into()),
            ("E-stop OK (8)".into(), if self.estop_ok { "yes" } else { "no, tripped" }.into()),
        ]
    }

    fn set_input(&mut self, port: usize, value: f64) {
        if let Some(w) = self.wired.get_mut(port) {
            *w = value > 0.5;
        }
    }
}

pub fn el1008(serial: u32) -> DeviceSpec {
    let pdos = bit_pdos(0x1A00, 0x6000, 0, 8, "Input");
    let eeprom = SiiBuilder::new(BECKHOFF, 0x03F0_3052, 0x0010_0000, serial)
        .general("DigIn", "EL1008", "EL1008 8Ch. Dig. Input 24V, 3ms", 0, 90)
        .fmmus(&[2])
        .sync_managers(&[sm(0x1000, 1, 0x00, 1, SmType::Inputs)])
        .pdos(true, &pdos)
        .build();
    let mut od = Od::default();
    for c in 0..8u16 {
        od.record(0x6000 + 0x10 * c, &format!("Channel {}", c + 1), None, ACC_RO, &[(1, "Input", 0x0001, 1, ACC_RO | ACC_TXPDO, &[0])]);
    }
    let model = DigitalInputs { t: 0.0, wired: [false; 4], start_button: false, estop_ok: true };
    ebus_terminal(0x12, false, eeprom, od, Box::new(model))
}

// ---- EL2008 -------------------------------------------------------------------------------------------------

pub struct DigitalOutputs {
    on: [bool; 8],
}

impl Model for DigitalOutputs {
    fn tick(&mut self, _dt: f64, od: &mut Od, op: bool) {
        for c in 0..8 {
            self.on[c] = op && od.get_bool(0x7000 + 0x10 * c as u16, 1);
        }
    }

    fn state_changed(&mut self, state: AlState, od: &mut Od) {
        if state != AlState::Op {
            for c in 0..8u16 {
                od.set_u64(0x7000 + 0x10 * c, 1, 0);
            }
        }
    }

    fn summary(&self, _od: &Od) -> Vec<(String, String)> {
        let bits: String = self.on.iter().map(|b| if *b { '1' } else { '0' }).collect();
        vec![("Outputs 1-8".into(), bits), ("Outputs 1-4".into(), "wired to EL1008 inputs 1-4".into())]
    }

    fn output(&self, port: usize) -> f64 {
        self.on.get(port).map(|b| *b as u8 as f64).unwrap_or(0.0)
    }
}

pub fn el2008(serial: u32) -> DeviceSpec {
    let pdos = bit_pdos(0x1600, 0x7000, 0, 8, "Output");
    let eeprom = SiiBuilder::new(BECKHOFF, 0x07D8_3052, 0x0010_0000, serial)
        .general("DigOut", "EL2008", "EL2008 8Ch. Dig. Output 24V, 0.5A", 0, 110)
        .fmmus(&[1])
        .sync_managers(&[sm(0x0F00, 1, 0x44, 1, SmType::Outputs)])
        .pdos(false, &pdos)
        .build();
    let mut od = Od::default();
    for c in 0..8u16 {
        od.record(0x7000 + 0x10 * c, &format!("Channel {}", c + 1), None, ACC_RO, &[(1, "Output", 0x0001, 1, ACC_RW | ACC_RXPDO, &[0])]);
    }
    ebus_terminal(0x12, false, eeprom, od, Box::new(DigitalOutputs { on: [false; 8] }))
}

// ---- EL3102 -------------------------------------------------------------------------------------------------

fn mailbox_sms(tx_start: u16, rest: &[SiiSyncManager]) -> Vec<SiiSyncManager> {
    let mut v = vec![sm(0x1000, 128, 0x26, 1, SmType::MailboxOut), sm(tx_start, 128, 0x22, 1, SmType::MailboxIn)];
    v.extend_from_slice(rest);
    v
}

struct Ident<'a> {
    device_type: u32,
    name: &'a str,
    hw: &'a str,
    sw: &'a str,
    vendor: u32,
    product: u32,
    revision: u32,
    serial: u32,
}

fn common_objects(od: &mut Od, id: Ident) {
    od.var(0x1000, "Device type", 0x0007, 32, ACC_RO, &id.device_type.to_le_bytes());
    od.string(0x1008, "Device name", id.name);
    od.string(0x1009, "Hardware version", id.hw);
    od.string(0x100A, "Software version", id.sw);
    od.identity(id.vendor, id.product, id.revision, id.serial);
}

const AI_ENTRIES: [(u8, &str, u16, u8); 9] = [
    (0x01, "Underrange", 0x0001, 1),
    (0x02, "Overrange", 0x0001, 1),
    (0x03, "Limit 1", 0x0031, 2),
    (0x05, "Limit 2", 0x0031, 2),
    (0x07, "Error", 0x0001, 1),
    (0x0E, "Sync error", 0x0001, 1),
    (0x0F, "TxPDO State", 0x0001, 1),
    (0x10, "TxPDO Toggle", 0x0001, 1),
    (0x11, "Value", 0x0003, 16),
];

fn ai_map(base: u16) -> Vec<MapEntry> {
    let e = |sub, bits| MapEntry { index: base, sub, bits };
    vec![e(1, 1), e(2, 1), e(3, 2), e(5, 2), e(7, 1), MapEntry { index: 0, sub: 0, bits: 6 }, e(0x0E, 1), e(0x0F, 1), e(0x10, 1), e(0x11, 16)]
}

pub struct AnalogInputs {
    t: f64,
    rng: Rng,
    /// Volts arriving at channel 2 (wired from an EL4102).
    wired: f64,
    filtered: [f64; 2],
    toggle: bool,
}

impl AnalogInputs {
    fn channel(&mut self, c: usize, volts: f64, od: &mut Od) {
        let base = 0x6000 + 0x10 * c as u16;
        let set = 0x8000 + 0x10 * c as u16;
        let v = if od.get_bool(set, 0x06) {
            self.filtered[c] += (volts - self.filtered[c]) * 0.2;
            self.filtered[c]
        } else {
            self.filtered[c] = volts;
            volts
        };
        let under = v < -10.0;
        let over = v > 10.0;
        let mut raw = (v / 10.0 * 32767.0).round().clamp(-32768.0, 32767.0) as i64;
        if od.get_bool(set, 0x0A) {
            let gain = od.get_i64(set, 0x12) as f64 / 65536.0;
            raw = ((raw as f64) * gain + od.get_i64(set, 0x11) as f64).round().clamp(-32768.0, 32767.0) as i64;
        }
        let limit = |enabled: bool, l: i64| -> u64 {
            if !enabled {
                0
            } else if raw > l {
                1
            } else if raw < l {
                2
            } else {
                3
            }
        };
        let l1 = limit(od.get_bool(set, 0x07), od.get_i64(set, 0x13));
        let l2 = limit(od.get_bool(set, 0x08), od.get_i64(set, 0x14));
        od.set_u64(base, 0x01, under as u64);
        od.set_u64(base, 0x02, over as u64);
        od.set_u64(base, 0x03, l1);
        od.set_u64(base, 0x05, l2);
        od.set_u64(base, 0x07, (under || over) as u64);
        od.set_u64(base, 0x10, self.toggle as u64);
        od.set_i64(base, 0x11, raw);
    }
}

impl Model for AnalogInputs {
    fn tick(&mut self, dt: f64, od: &mut Od, _op: bool) {
        if dt <= 0.0 {
            return;
        }
        self.t += dt;
        self.toggle = !self.toggle;
        // Channel 1: a level transmitter on a tank that fills and drains over 20 s (0.5 to 4.5 V), with noise.
        let level = 2.5 + 2.0 * (self.t * std::f64::consts::TAU / 20.0).sin() + self.rng.next() * 0.01;
        self.channel(0, level, od);
        let wired = self.wired + self.rng.next() * 0.004;
        self.channel(1, wired, od);
    }

    fn summary(&self, od: &Od) -> Vec<(String, String)> {
        let v = |c: u16| od.get_i64(0x6000 + 0x10 * c, 0x11) as f64 * 10.0 / 32767.0;
        vec![
            ("Ch.1 tank level".into(), format!("{:.3} V", v(0))),
            ("Ch.2 (wired from EL4102 ch.1)".into(), format!("{:.3} V", v(1))),
        ]
    }

    fn set_input(&mut self, _port: usize, value: f64) {
        self.wired = value;
    }
}

pub fn el3102(serial: u32) -> DeviceSpec {
    let (product, rev) = (0x0C1E_3052, 0x0014_0000);
    let pdo = |c: u16| SiiPdo {
        index: 0x1A00 + 2 * c,
        sm: 3,
        name: format!("AI TxPDO-Map Standard Ch.{}", c + 1),
        entries: ai_map(0x6000 + 0x10 * c)
            .into_iter()
            .map(|m| {
                let (name, dt) =
                    AI_ENTRIES.iter().find(|e| e.0 == m.sub && m.index != 0).map(|e| (e.1, e.2 as u8)).unwrap_or(("", 0));
                SiiPdoEntry { index: m.index, sub: m.sub, name: name.into(), data_type: dt, bits: m.bits }
            })
            .collect(),
    };
    let eeprom = SiiBuilder::new(BECKHOFF, product, rev, serial)
        .mailbox((0x1000, 128), (0x1080, 128), sii::MBX_COE)
        .general("AnaIn", "EL3102", "EL3102 2Ch. Ana. Input +/-10V, Diff.", sii::COE_SDO | sii::COE_SDO_INFO | sii::COE_PDO_ASSIGN, 130)
        .fmmus(&[2, 3])
        .sync_managers(&mailbox_sms(0x1080, &[sm(0x1100, 0, 0x24, 0, SmType::Outputs), sm(0x1180, 8, 0x20, 1, SmType::Inputs)]))
        .pdos(true, &[pdo(0), pdo(1)])
        .build();
    let mut od = Od::default();
    common_objects(&mut od, Ident { device_type: 0x0000_1389, name: "EL3102", hw: "04", sw: "05", vendor: BECKHOFF, product, revision: rev, serial });
    for c in 0..2u16 {
        od.pdo_mapping(0x1A00 + 2 * c, &format!("AI TxPDO-Map Standard Ch.{}", c + 1), &ai_map(0x6000 + 0x10 * c));
        let subs: Vec<(u8, &str, u16, u16, u16, &[u8])> =
            AI_ENTRIES.iter().map(|&(s, n, dt, bits)| (s, n, dt, bits as u16, ACC_RO | ACC_TXPDO, &[0u8, 0][..])).collect();
        od.record(0x6000 + 0x10 * c, &format!("AI Inputs Ch.{}", c + 1), None, ACC_RO, &subs);
        let gain = 65536i32.to_le_bytes();
        od.record(
            0x8000 + 0x10 * c,
            &format!("AI Settings Ch.{}", c + 1),
            None,
            ACC_RO,
            &[
                (0x06, "Enable filter", 0x0001, 1, ACC_RW, &[0]),
                (0x07, "Enable limit 1", 0x0001, 1, ACC_RW, &[0]),
                (0x08, "Enable limit 2", 0x0001, 1, ACC_RW, &[0]),
                (0x0A, "Enable user scale", 0x0001, 1, ACC_RW, &[0]),
                (0x11, "User scale offset", 0x0003, 16, ACC_RW, &[0, 0]),
                (0x12, "User scale gain", 0x0004, 32, ACC_RW, &gain),
                (0x13, "Limit 1", 0x0003, 16, ACC_RW, &[0, 0]),
                (0x14, "Limit 2", 0x0003, 16, ACC_RW, &[0, 0]),
            ],
        );
    }
    od.sm_types(&[1, 2, 3, 4]);
    od.pdo_assign(0x1C12, "RxPDO assign", &[]);
    od.pdo_assign(0x1C13, "TxPDO assign", &[0x1A00, 0x1A02]);
    let model = AnalogInputs { t: 0.0, rng: Rng(0x9E37_79B9_7F4A_7C15 ^ serial as u64), wired: 0.0, filtered: [0.0; 2], toggle: false };
    ebus_terminal(0x12, true, eeprom, od, Box::new(model))
}

// ---- EL4102 -------------------------------------------------------------------------------------------------

pub struct AnalogOutputs {
    volts: [f64; 2],
}

impl Model for AnalogOutputs {
    fn tick(&mut self, _dt: f64, od: &mut Od, op: bool) {
        for c in 0..2u16 {
            let raw = if op { od.get_i64(0x7000 + 0x10 * c, 1) } else { od.get_i64(0x8000 + 0x10 * c, 0x13) };
            self.volts[c as usize] = (raw as f64 * 10.0 / 32767.0).clamp(0.0, 10.0);
        }
    }

    fn summary(&self, _od: &Od) -> Vec<(String, String)> {
        vec![
            ("Ch.1 (wired to EL3102 ch.2)".into(), format!("{:.3} V", self.volts[0])),
            ("Ch.2".into(), format!("{:.3} V", self.volts[1])),
        ]
    }

    fn output(&self, port: usize) -> f64 {
        self.volts.get(port).copied().unwrap_or(0.0)
    }
}

pub fn el4102(serial: u32) -> DeviceSpec {
    let (product, rev) = (0x1006_3052, 0x0013_0000);
    let pdo = |c: u16| SiiPdo {
        index: 0x1600 + c,
        sm: 2,
        name: format!("AO RxPDO-Map Ch.{}", c + 1),
        entries: vec![SiiPdoEntry { index: 0x7000 + 0x10 * c, sub: 1, name: "Analog output".into(), data_type: 3, bits: 16 }],
    };
    let eeprom = SiiBuilder::new(BECKHOFF, product, rev, serial)
        .mailbox((0x1000, 128), (0x1080, 128), sii::MBX_COE)
        .general("AnaOut", "EL4102", "EL4102 2Ch. Ana. Output 0-10V, 16bit", sii::COE_SDO | sii::COE_SDO_INFO | sii::COE_PDO_ASSIGN, 180)
        .fmmus(&[1, 3])
        .sync_managers(&mailbox_sms(0x1080, &[sm(0x1100, 4, 0x64, 1, SmType::Outputs), sm(0x1180, 0, 0x20, 0, SmType::Inputs)]))
        .pdos(false, &[pdo(0), pdo(1)])
        .build();
    let mut od = Od::default();
    common_objects(&mut od, Ident { device_type: 0x0000_1389, name: "EL4102", hw: "03", sw: "04", vendor: BECKHOFF, product, revision: rev, serial });
    for c in 0..2u16 {
        od.pdo_mapping(0x1600 + c, &format!("AO RxPDO-Map Ch.{}", c + 1), &[MapEntry { index: 0x7000 + 0x10 * c, sub: 1, bits: 16 }]);
        od.record(0x7000 + 0x10 * c, &format!("AO Outputs Ch.{}", c + 1), None, ACC_RO, &[(1, "Analog output", 0x0003, 16, ACC_RW | ACC_RXPDO, &[0, 0])]);
        od.record(
            0x8000 + 0x10 * c,
            &format!("AO Settings Ch.{}", c + 1),
            None,
            ACC_RO,
            &[(0x05, "Watchdog", 0x0031, 2, ACC_RW, &[0]), (0x13, "Default output", 0x0003, 16, ACC_RW, &[0, 0])],
        );
    }
    od.sm_types(&[1, 2, 3, 4]);
    od.pdo_assign(0x1C12, "RxPDO assign", &[0x1600, 0x1601]);
    od.pdo_assign(0x1C13, "TxPDO assign", &[]);
    ebus_terminal(0x12, true, eeprom, od, Box::new(AnalogOutputs { volts: [0.0; 2] }))
}

// ---- CiA 402 servo drive --------------------------------------------------------------------------------------

/// Encoder counts per motor revolution (17-bit).
pub const COUNTS_PER_REV: f64 = 131_072.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ds402 {
    SwitchOnDisabled,
    ReadyToSwitchOn,
    SwitchedOn,
    OperationEnabled,
    QuickStopActive,
    Fault,
}

impl Ds402 {
    pub fn label(self) -> &'static str {
        match self {
            Ds402::SwitchOnDisabled => "Switch on disabled",
            Ds402::ReadyToSwitchOn => "Ready to switch on",
            Ds402::SwitchedOn => "Switched on",
            Ds402::OperationEnabled => "Operation enabled",
            Ds402::QuickStopActive => "Quick stop active",
            Ds402::Fault => "Fault",
        }
    }

    fn bits(self) -> u16 {
        match self {
            Ds402::SwitchOnDisabled => 0x0040,
            Ds402::ReadyToSwitchOn => 0x0021,
            Ds402::SwitchedOn => 0x0023,
            Ds402::OperationEnabled => 0x0027,
            Ds402::QuickStopActive => 0x0007,
            Ds402::Fault => 0x0008,
        }
    }
}

pub fn mode_name(m: i8) -> &'static str {
    match m {
        0 => "none",
        1 => "Profile position (PP)",
        3 => "Profile velocity (PV)",
        6 => "Homing",
        8 => "Cyclic sync position (CSP)",
        9 => "Cyclic sync velocity (CSV)",
        10 => "Cyclic sync torque (CST)",
        _ => "unsupported",
    }
}

pub struct Servo {
    state: Ds402,
    pos: f64,
    vel: f64,
    last_cw: u16,
    fault: u16,
    pp_target: Option<f64>,
    pp_ack: bool,
    temp: f64,
    torque: f64,
    rng: Rng,
}

impl Servo {
    fn new(seed: u64) -> Servo {
        Servo {
            state: Ds402::SwitchOnDisabled,
            pos: 0.0,
            vel: 0.0,
            last_cw: 0,
            fault: 0,
            pp_target: None,
            pp_ack: false,
            temp: 31.0,
            torque: 0.0,
            rng: Rng(seed | 1),
        }
    }

    fn command(&mut self, cw: u16) {
        use Ds402::*;
        let fault_reset = cw & 0x80 != 0 && self.last_cw & 0x80 == 0;
        if self.state == Fault {
            if fault_reset {
                self.state = SwitchOnDisabled;
                self.fault = 0;
            }
            return;
        }
        let disable_voltage = cw & 0x02 == 0;
        let quick_stop = cw & 0x06 == 0x02;
        let shutdown = cw & 0x87 == 0x06;
        let switch_on = cw & 0x8F == 0x07;
        let enable = cw & 0x8F == 0x0F;
        self.state = match self.state {
            _ if disable_voltage => SwitchOnDisabled,
            OperationEnabled if quick_stop => QuickStopActive,
            _ if quick_stop => SwitchOnDisabled,
            SwitchOnDisabled if shutdown => ReadyToSwitchOn,
            ReadyToSwitchOn if switch_on => SwitchedOn,
            ReadyToSwitchOn | SwitchedOn if enable => OperationEnabled,
            SwitchedOn | OperationEnabled if shutdown => ReadyToSwitchOn,
            OperationEnabled if switch_on => SwitchedOn,
            s => s,
        };
    }

    fn trip(&mut self, code: u16) {
        self.state = Ds402::Fault;
        self.fault = code;
        self.vel = 0.0;
    }
}

impl Model for Servo {
    fn tick(&mut self, dt: f64, od: &mut Od, _op: bool) {
        let cw = od.get_u64(0x6040, 0) as u16;
        if cw != self.last_cw {
            let was_enabled = self.state == Ds402::OperationEnabled;
            self.command(cw);
            if self.state == Ds402::OperationEnabled && !was_enabled {
                self.pp_target = None;
            }
        }
        let mode = od.get_i64(0x6060, 0) as i8;
        let accel = od.get_u64(0x6083, 0).max(1) as f64;
        let decel = od.get_u64(0x6084, 0).max(1) as f64;
        let prev_vel = self.vel;
        let mut following = 0.0;
        if dt > 0.0 {
            match self.state {
                Ds402::OperationEnabled => match mode {
                    8 => {
                        let target = od.get_i64(0x607A, 0) as f64;
                        let window = od.get_u64(0x6065, 0) as f64;
                        if window > 0.0 && (target - self.pos).abs() > window {
                            // A set-point far from the actual position (often: enabled with target 0 while the
                            // axis stands elsewhere) trips the drive, as real drives do.
                            self.trip(0x8611);
                        } else {
                            // The position loop follows the set-point with a 5 ms lag.
                            let k = (dt / 0.005).min(1.0);
                            let step = (target - self.pos) * k;
                            self.pos += step;
                            self.vel = step / dt;
                            following = target - self.pos;
                        }
                    }
                    9 | 3 => {
                        let target = od.get_i64(0x60FF, 0) as f64;
                        let a = if target.abs() > self.vel.abs() { accel } else { decel };
                        self.vel += (target - self.vel).clamp(-a * dt, a * dt);
                        self.pos += self.vel * dt;
                    }
                    1 => {
                        let new_setpoint = cw & 0x10 != 0;
                        if new_setpoint && !self.pp_ack {
                            let t = od.get_i64(0x607A, 0) as f64;
                            self.pp_target = Some(if cw & 0x40 != 0 { self.pos + t } else { t });
                        }
                        self.pp_ack = new_setpoint;
                        if let Some(target) = self.pp_target {
                            let vmax = od.get_u64(0x6081, 0).max(1) as f64;
                            let dist = target - self.pos;
                            let stop_dist = self.vel * self.vel / (2.0 * decel);
                            let want = if dist.abs() <= stop_dist + 1.0 { 0.0 } else { vmax * dist.signum() };
                            let a = if want.abs() > self.vel.abs() { accel } else { decel };
                            self.vel += (want - self.vel).clamp(-a * dt, a * dt);
                            self.pos += self.vel * dt;
                            if dist.abs() < 2.0 || (want == 0.0 && self.vel.abs() <= decel * dt) {
                                if dist.abs() < 50.0 {
                                    self.pos = target;
                                    self.vel = 0.0;
                                }
                            }
                        }
                    }
                    _ => self.vel = 0.0,
                },
                Ds402::QuickStopActive => {
                    let qd = od.get_u64(0x6085, 0).max(1) as f64;
                    self.vel += (-self.vel).clamp(-qd * dt, qd * dt);
                    self.pos += self.vel * dt;
                    if self.vel.abs() < 1.0 {
                        self.vel = 0.0;
                        self.state = Ds402::SwitchOnDisabled;
                    }
                }
                _ => self.vel = 0.0,
            }
            let acc = (self.vel - prev_vel) / dt;
            let rated_accel = 20.0 * COUNTS_PER_REV * 60.0;
            self.torque = if self.state == Ds402::OperationEnabled {
                (acc / rated_accel * 1000.0 + self.vel.signum() * 35.0 + self.rng.next() * 6.0).clamp(-3000.0, 3000.0)
            } else {
                0.0
            };
            self.temp += (31.0 + self.torque.abs() / 1000.0 * 40.0 - self.temp) * (dt / 60.0).min(1.0);
            if self.temp > 90.0 {
                self.trip(0x4310);
            }
        }
        self.last_cw = cw;
        let mut sw = self.state.bits() | 0x0010 | 0x0200;
        if self.state == Ds402::OperationEnabled {
            match mode {
                8 => sw |= 0x1000 | if following.abs() < 50.0 { 0x0400 } else { 0 },
                9 | 3 => {
                    if (od.get_i64(0x60FF, 0) as f64 - self.vel).abs() < 10.0 {
                        sw |= 0x0400;
                    }
                }
                1 => {
                    if self.pp_ack {
                        sw |= 0x1000;
                    }
                    if self.pp_target.is_none_or(|t| (t - self.pos).abs() < 2.0 && self.vel == 0.0) {
                        sw |= 0x0400;
                    }
                }
                _ => {}
            }
        }
        od.set_u64(0x6041, 0, sw as u64);
        od.set_i64(0x6061, 0, mode as i64);
        od.set_i64(0x6064, 0, self.pos.round() as i64);
        od.set_i64(0x606C, 0, self.vel.round() as i64);
        od.set_i64(0x6077, 0, self.torque.round() as i64);
        od.set_i64(0x60F4, 0, following.round() as i64);
        od.set_u64(0x603F, 0, self.fault as u64);
        od.set_i64(0x2000, 0, (self.temp * 10.0).round() as i64);
        od.set_u64(0x2001, 0, (3250.0 + self.rng.next() * 8.0).round() as u64);
    }

    fn action(&mut self, action: &str, od: &mut Od) -> bool {
        match action {
            "fault" => self.trip(0x2310),
            "home" => {
                self.pos = 0.0;
                od.set_i64(0x6064, 0, 0);
            }
            _ => return false,
        }
        true
    }

    fn actions(&self) -> Vec<(&'static str, &'static str)> {
        vec![("fault", "Trip an over-current fault"), ("home", "Set the position to 0")]
    }

    fn state_changed(&mut self, state: AlState, od: &mut Od) {
        if state != AlState::Op {
            od.set_u64(0x6040, 0, 0);
            self.last_cw = 0;
            if self.state != Ds402::Fault {
                self.state = Ds402::SwitchOnDisabled;
            }
            self.vel = 0.0;
        }
    }

    fn summary(&self, od: &Od) -> Vec<(String, String)> {
        vec![
            ("CiA 402 state".into(), self.state.label().into()),
            ("Mode".into(), mode_name(od.get_i64(0x6061, 0) as i8).into()),
            ("Position".into(), format!("{:.3} rev", self.pos / COUNTS_PER_REV)),
            ("Speed".into(), format!("{:.1} rpm", self.vel / COUNTS_PER_REV * 60.0)),
            ("Torque".into(), format!("{:.1} %", self.torque / 10.0)),
            ("Motor temperature".into(), format!("{:.1} °C", self.temp)),
            (
                "Error code".into(),
                if self.fault == 0 { "none".into() } else { format!("0x{:04X} {}", self.fault, ds402_error(self.fault)) },
            ),
        ]
    }
}

pub fn ds402_error(code: u16) -> &'static str {
    match code {
        0x2310 => "Continuous over current",
        0x3210 => "DC link over voltage",
        0x4310 => "Excess temperature drive",
        0x7305 => "Incremental sensor 1 fault",
        0x8611 => "Following error",
        _ => "",
    }
}

fn counts(rev: f64) -> [u8; 4] {
    ((rev * COUNTS_PER_REV) as u32).to_le_bytes()
}

pub fn servo(serial: u32) -> DeviceSpec {
    let (vendor, product, rev) = (0u32, 0x0000_0400, 0x0001_0004);
    let m = |index, bits| MapEntry { index, sub: 0, bits };
    let rx_default = [m(0x6040, 16), m(0x607A, 32), m(0x60FF, 32), m(0x6071, 16), m(0x6060, 8), m(0, 8)];
    let rx_velocity = [m(0x6040, 16), m(0x60FF, 32), m(0x6060, 8), m(0, 8)];
    let tx_default = [m(0x6041, 16), m(0x6064, 32), m(0x606C, 32), m(0x6077, 16), m(0x60F4, 32), m(0x603F, 16), m(0x6061, 8), m(0, 8)];
    let tx_velocity = [m(0x6041, 16), m(0x606C, 32), m(0x6064, 32), m(0x6061, 8), m(0, 8)];
    let mut od = Od::default();
    common_objects(
        &mut od,
        Ident { device_type: 0x0002_0192, name: "VD400 Servo Drive", hw: "B", sw: "1.4.2", vendor, product, revision: rev, serial },
    );
    od.pdo_mapping(0x1600, "RxPDO 1: CSP/CSV/CST", &rx_default);
    od.pdo_mapping(0x1601, "RxPDO 2: velocity", &rx_velocity);
    od.pdo_mapping(0x1A00, "TxPDO 1: CSP/CSV/CST", &tx_default);
    od.pdo_mapping(0x1A01, "TxPDO 2: velocity", &tx_velocity);
    od.sm_types(&[1, 2, 3, 4]);
    od.pdo_assign(0x1C12, "RxPDO assign", &[0x1600]);
    od.pdo_assign(0x1C13, "TxPDO assign", &[0x1A00]);
    let rx = ACC_RW | ACC_RXPDO;
    let tx = ACC_RO | ACC_TXPDO;
    od.var(0x2000, "Motor temperature (0.1 °C)", 0x0003, 16, tx, &310i16.to_le_bytes());
    od.var(0x2001, "DC link voltage (0.1 V)", 0x0006, 16, tx, &3250u16.to_le_bytes());
    od.var(0x603F, "Error code", 0x0006, 16, tx, &[0, 0]);
    od.var(0x6040, "Controlword", 0x0006, 16, rx, &[0, 0]);
    od.var(0x6041, "Statusword", 0x0006, 16, tx, &0x0250u16.to_le_bytes());
    od.var(0x605A, "Quick stop option code", 0x0003, 16, ACC_RW, &2i16.to_le_bytes());
    od.var(0x6060, "Modes of operation", 0x0002, 8, rx, &[8]);
    od.var(0x6061, "Modes of operation display", 0x0002, 8, tx, &[8]);
    od.var(0x6064, "Position actual value", 0x0004, 32, tx, &[0; 4]);
    od.var(0x6065, "Following error window", 0x0007, 32, ACC_RW, &counts(0.5));
    od.var(0x606C, "Velocity actual value", 0x0004, 32, tx, &[0; 4]);
    od.var(0x6071, "Target torque", 0x0003, 16, rx, &[0, 0]);
    od.var(0x6077, "Torque actual value", 0x0003, 16, tx, &[0, 0]);
    od.var(0x607A, "Target position", 0x0004, 32, rx, &[0; 4]);
    od.var(0x6081, "Profile velocity", 0x0007, 32, ACC_RW, &counts(5.0));
    od.var(0x6083, "Profile acceleration", 0x0007, 32, ACC_RW, &counts(20.0));
    od.var(0x6084, "Profile deceleration", 0x0007, 32, ACC_RW, &counts(20.0));
    od.var(0x6085, "Quick stop deceleration", 0x0007, 32, ACC_RW, &counts(80.0));
    od.var(0x60F4, "Following error actual value", 0x0004, 32, tx, &[0; 4]);
    od.var(0x60FF, "Target velocity", 0x0004, 32, rx, &[0; 4]);
    od.var(0x6502, "Supported drive modes", 0x0007, 32, ACC_RO, &0x0000_0385u32.to_le_bytes());

    let sii_pdo = |index: u16, sm: u8, name: &str, map: &[MapEntry], od: &Od| SiiPdo {
        index,
        sm,
        name: name.into(),
        entries: map
            .iter()
            .map(|m| {
                let e = od.entry(m.index, m.sub);
                SiiPdoEntry {
                    index: m.index,
                    sub: m.sub,
                    name: e.map(|e| e.name.clone()).unwrap_or_default(),
                    data_type: e.map(|e| e.data_type as u8).unwrap_or(0),
                    bits: m.bits,
                }
            })
            .collect(),
    };
    let mut eeprom = SiiBuilder::new(vendor, product, rev, serial)
        .mailbox((0x1000, 128), (0x1400, 128), sii::MBX_COE)
        .general(
            "Drives",
            "VD400",
            "VD400 Servo Drive 400 W (simulated)",
            sii::COE_SDO | sii::COE_SDO_INFO | sii::COE_PDO_ASSIGN | sii::COE_PDO_CONFIG,
            0,
        )
        .fmmus(&[1, 2, 3])
        .sync_managers(&mailbox_sms(0x1400, &[sm(0x1800, 14, 0x64, 1, SmType::Outputs), sm(0x1C00, 20, 0x20, 1, SmType::Inputs)]))
        .pdos(false, &[sii_pdo(0x1600, 2, "RxPDO 1: CSP/CSV/CST", &rx_default, &od), sii_pdo(0x1601, 0xFF, "RxPDO 2: velocity", &rx_velocity, &od)])
        .pdos(true, &[sii_pdo(0x1A00, 3, "TxPDO 1: CSP/CSV/CST", &tx_default, &od), sii_pdo(0x1A01, 0xFF, "TxPDO 2: velocity", &tx_velocity, &od)])
        .build();
    // One CiA 402 axis, in the general category.
    if let Some(pos) = find_category(&eeprom, sii::cat::GENERAL) {
        eeprom[pos + 9] = 1;
    }
    DeviceSpec {
        esc_type: 0x90,
        esc_revision: 0x01,
        esc_build: 0x0002,
        fmmus: 3,
        sms: 4,
        port_desc: 0x0F,
        features: 0x010C,
        eeprom,
        od,
        model: Box::new(Servo::new(serial as u64)),
        delay_ns: 520,
    }
}

/// Byte offset of a category's data in an image.
fn find_category(img: &[u8], cat: u16) -> Option<usize> {
    let mut at = sii::word::CATEGORIES * 2;
    while at + 4 <= img.len() {
        let t = u16::from_le_bytes([img[at], img[at + 1]]);
        if t == sii::cat::END {
            return None;
        }
        let words = u16::from_le_bytes([img[at + 2], img[at + 3]]) as usize;
        if t == cat {
            return Some(at + 4);
        }
        at += 4 + words * 2;
    }
    None
}
