//! The simulated logic analyzer: industrial scenarios rendered at the chosen sample rate and
//! streamed in real time, through the same capture engine as the FX2 driver. It is what the app
//! uses without hardware, and what the demos and screenshots show.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::capture::ChannelInfo;
use crate::decode::DecoderConfig;
use crate::decode::can::{CanConfig, CanFrameSpec, frame_bits};
use crate::decode::i2c::I2cConfig;
use crate::decode::modbus::ModbusConfig;
use crate::decode::spi::{SpiConfig, SpiDevice};
use crate::decode::uart::{Parity, UartConfig};
use crate::engine::SampleStream;
use crate::error::{LogicError, Result};
use crate::synth::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScenarioId {
    ModbusRtu,
    CanBus,
    SpiMcp2515,
    I2cSensors,
    UartConsole,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioInfo {
    pub id: ScenarioId,
    pub name: &'static str,
    pub description: &'static str,
    pub default_rate: u64,
    /// Below this the signals are still generated, but too coarse to decode.
    pub min_rate: u64,
    /// Length of one loop of the scenario, seconds.
    pub duration: f64,
    pub channels: Vec<ChannelInfo>,
    /// The decoder stack the app adds for it.
    pub decoders: Vec<DecoderConfig>,
}

pub const SCENARIOS: [ScenarioId; 5] = [ScenarioId::ModbusRtu, ScenarioId::CanBus, ScenarioId::SpiMcp2515, ScenarioId::I2cSensors, ScenarioId::UartConsole];

fn chans(names: &[&str]) -> Vec<ChannelInfo> {
    (0..8).map(|i| ChannelInfo::named(names.get(i).copied().unwrap_or(&format!("D{i}")).to_string())).collect()
}

pub const MODBUS_BAUD: f64 = 19200.0;
pub const CAN_BITRATE: f64 = 500_000.0;
pub const MCP_CAN_BITRATE: f64 = 125_000.0;

pub fn info(id: ScenarioId) -> ScenarioInfo {
    match id {
        ScenarioId::ModbusRtu => ScenarioInfo {
            id,
            name: "Modbus RTU on RS-485",
            description: "A PLC polls three drives and a power meter at 19200 baud 8E1: holding and input registers, coils, writes, an exception, a CRC error from line noise, a slave that never answers and a broadcast. D0 is the bus seen by a listening transceiver, D1 to D4 the master's and slave's TX and driver enable, D5 a 10 ms PLC scan output.",
            default_rate: 1_000_000,
            min_rate: 100_000,
            duration: 1.0,
            channels: chans(&["RS-485 RO", "Master TX", "Master DE", "Slave TX", "Slave DE", "PLC scan"]),
            decoders: vec![DecoderConfig::ModbusRtu(ModbusConfig { request_channel: Some(1), response_channel: Some(3), baud: MODBUS_BAUD, parity: Parity::Even, stop_bits: 1.0, inverted: false, timeout_ms: 100.0 })],
        },
        ScenarioId::CanBus => ScenarioInfo {
            id,
            name: "CAN bus, J1939 and CANopen",
            description: "500 kbit/s machine bus: J1939 engine and temperature frames, CANopen heartbeat and PDOs, a remote frame, a frame nobody acknowledges (followed by an error flag) and a CRC error. D0 is CAN_RX of a transceiver, D1 the engine ECU's CAN_TX.",
            default_rate: 8_000_000,
            min_rate: 4_000_000,
            duration: 0.01,
            channels: chans(&["CAN RX", "ECU TX"]),
            decoders: vec![DecoderConfig::Can(CanConfig { channel: 0, bitrate: CAN_BITRATE, ..Default::default() })],
        },
        ScenarioId::SpiMcp2515 => ScenarioInfo {
            id,
            name: "SPI to an MCP2515 CAN controller",
            description: "A microcontroller sets up an MCP2515 over SPI (mode 0, 1 MHz), loads and sends a CAN frame at 125 kbit/s, then reads the frame its interrupt reports. D0 to D3 are CS, SCK, SI, SO; D4 the INT line; D5 TXCAN and D6 RXCAN.",
            default_rate: 8_000_000,
            min_rate: 4_000_000,
            duration: 0.004,
            channels: chans(&["CS", "SCK", "SI (MOSI)", "SO (MISO)", "INT", "TXCAN", "RXCAN"]),
            decoders: vec![
                DecoderConfig::Spi(SpiConfig { clk: 1, mosi: Some(2), miso: Some(3), cs: Some(0), mode: 0, word_bits: 8, lsb_first: false, cs_active_high: false, device: SpiDevice::Mcp2515 }),
                DecoderConfig::Can(CanConfig { channel: 6, bitrate: MCP_CAN_BITRATE, ..Default::default() }),
            ],
        },
        ScenarioId::I2cSensors => ScenarioInfo {
            id,
            name: "I2C sensors and EEPROM",
            description: "100 kHz I2C: a temperature sensor read with a repeated start, an EEPROM page write, and an address nobody answers. D0 SCL, D1 SDA.",
            default_rate: 2_000_000,
            min_rate: 1_000_000,
            duration: 0.01,
            channels: chans(&["SCL", "SDA"]),
            decoders: vec![DecoderConfig::I2c(I2cConfig { scl: 0, sda: 1, min_pulse: 0 })],
        },
        ScenarioId::UartConsole => ScenarioInfo {
            id,
            name: "UART service console",
            description: "A drive's 115200 8N1 service port: status lines on TX, commands on RX, with a noise glitch on the idle line.",
            default_rate: 2_000_000,
            min_rate: 500_000,
            duration: 0.1,
            channels: chans(&["TX", "RX"]),
            decoders: vec![
                DecoderConfig::Uart(UartConfig { channel: 0, baud: 115200.0, data_bits: 8, parity: Parity::None, stop_bits: 1.0, inverted: false, msb_first: false }),
                DecoderConfig::Uart(UartConfig { channel: 1, baud: 115200.0, data_bits: 8, parity: Parity::None, stop_bits: 1.0, inverted: false, msb_first: false }),
            ],
        },
    }
}

/// Renders one loop of a scenario at `rate`.
pub fn render(id: ScenarioId, rate: u64) -> Vec<u8> {
    let i = info(id);
    let len = (i.duration * rate as f64).round() as u64;
    let s = match id {
        ScenarioId::ModbusRtu => modbus(rate),
        ScenarioId::CanBus => can(rate),
        ScenarioId::SpiMcp2515 => mcp2515(rate),
        ScenarioId::I2cSensors => i2c(rate),
        ScenarioId::UartConsole => uart(rate),
    };
    let mut d = s.render(len);
    d.truncate(len as usize);
    d
}

fn modbus(rate: u64) -> Synth {
    // Idle: RS-485 lines high (mark), DE low.
    let mut s = Synth::new(rate, [true, true, false, true, false, false, false, false]);
    let u = modbus_uart(MODBUS_BAUD, 0);
    let ct = 11.0 / MODBUS_BAUD;
    // PLC scan output: 10 ms period, 30 % high.
    let mut t = 0.0;
    while t < 1.0 {
        s.set_t(5, t, true);
        s.set_t(5, t + 0.003, false);
        t += 0.010;
    }
    // (request, response, response delay s); response None = no answer.
    let mut noisy = modbus_frame(&[1, 3, 8, 0x00, 0xEB, 0x01, 0x2C, 0x05, 0xDC, 0x00, 0x04]);
    noisy[6] ^= 0x22;
    let exchanges: Vec<(Vec<u8>, Option<Vec<u8>>, f64)> = vec![
        // Drive 1: setpoint, actual, speed, state (40001-40004).
        (modbus_frame(&[1, 3, 0, 0, 0, 4]), Some(modbus_frame(&[1, 3, 8, 0x00, 0xEB, 0x01, 0x2C, 0x05, 0xDC, 0x00, 0x04])), 0.0031),
        // Power meter: voltage, current (30001-30004).
        (modbus_frame(&[2, 4, 0, 0, 0, 4]), Some(modbus_frame(&[2, 4, 8, 0x08, 0xFC, 0x00, 0x7D, 0x00, 0x00, 0x01, 0xF4])), 0.0062),
        // Drive 1: new speed setpoint 1800.
        (modbus_frame(&[1, 6, 0, 10, 0x07, 0x08]), Some(modbus_frame(&[1, 6, 0, 10, 0x07, 0x08])), 0.0028),
        // Remote I/O: 16 coils.
        (modbus_frame(&[3, 1, 0, 0, 0, 16]), Some(modbus_frame(&[3, 1, 2, 0b0010_0101, 0b1000_0001])), 0.0019),
        // Power meter: a register that does not exist.
        (modbus_frame(&[2, 3, 0x27, 0x0F, 0, 2]), Some(modbus_frame(&[2, 0x83, 2])), 0.0045),
        // Drive 4 is switched off.
        (modbus_frame(&[4, 3, 0, 0, 0, 2]), None, 0.0),
        // Drive 1: ramp times (40021-40022).
        (modbus_frame(&[1, 16, 0, 20, 0, 2, 4, 0, 50, 0, 80]), Some(modbus_frame(&[1, 16, 0, 20, 0, 2])), 0.0035),
        // Drive 1 again, answer hit by noise.
        (modbus_frame(&[1, 3, 0, 0, 0, 4]), Some(noisy), 0.0031),
        // Broadcast: reset alarms on every drive.
        (modbus_frame(&[0, 6, 0, 99, 0, 1]), None, 0.0),
    ];
    let mut t = 0.02;
    for (req, resp, delay) in exchanges {
        // Master: DE up 0.2 ms before the first start bit, down 0.2 ms after the last stop bit.
        s.set_t(2, t - 0.0002, true);
        let end = uart_tx(&mut s, 1, t, &req, &u, MODBUS_BAUD, 0.0, UartFault::default());
        uart_tx(&mut s, 0, t, &req, &u, MODBUS_BAUD, 0.0, UartFault::default());
        s.set_t(2, end + 0.0002, false);
        t = end;
        if let Some(resp) = resp {
            let rt = end + delay;
            s.set_t(4, rt - 0.0002, true);
            let e2 = uart_tx(&mut s, 3, rt, &resp, &u, MODBUS_BAUD, 0.0, UartFault::default());
            uart_tx(&mut s, 0, rt, &resp, &u, MODBUS_BAUD, 0.0, UartFault::default());
            s.set_t(4, e2 + 0.0002, false);
            t = e2;
        } else if req[0] != 0 {
            // The master waits for its 100 ms response timeout.
            t += 0.1;
        }
        t += 0.012 + 3.5 * ct;
    }
    // A noise burst on the bus when a contactor switches.
    let n = s.at(0.92);
    for k in 0..6 {
        s.glitch(0, n + k * (rate / 50_000).max(2), 1);
    }
    s
}

fn can(rate: u64) -> Synth {
    let mut s = Synth::new(rate, [true; 8]);
    let mut t = 0.0003;
    let ecu = |f: &CanFrameSpec| f.id & 0xFF == 0x00 && f.extended;
    let frames: Vec<CanFrameSpec> = vec![
        // J1939 EEC1 from the engine (SA 0x00): 1500 rpm.
        CanFrameSpec::data(0x0CF00400, true, &[0xF0, 0x7D, 0x7D, 0xE0, 0x2E, 0x00, 0xF0, 0x7D]),
        // CANopen heartbeat of node 5: operational.
        CanFrameSpec::data(0x705, false, &[0x05]),
        // J1939 ET1 engine temperature: 88 °C coolant.
        CanFrameSpec::data(0x18FEEE00, true, &[0x80, 0x82, 0x20, 0x3B, 0xFF, 0xFF, 0xFF, 0xFF]),
        // CANopen TPDO1 of node 5: digital inputs and an analog value.
        CanFrameSpec::data(0x185, false, &[0x00, 0x00, 0x00, 0x00, 0x10, 0x27]),
        // A remote request for node 5's TPDO2.
        CanFrameSpec { rtr: true, dlc: 4, data: vec![], ..CanFrameSpec::data(0x285, false, &[]) },
        // A node at the wrong bit rate is gone: nobody acknowledges this frame.
        CanFrameSpec { ack: false, ..CanFrameSpec::data(0x18FF5021, true, &[1, 2, 3, 4]) },
        // A frame damaged on the cable: the receivers' CRC check fails.
        CanFrameSpec { corrupt_crc: true, ack: false, ..CanFrameSpec::data(0x186, false, &[0x55, 0xAA]) },
        CanFrameSpec::data(0x0CF00400, true, &[0xF0, 0x7D, 0x7D, 0xE8, 0x2E, 0x00, 0xF0, 0x7D]),
        CanFrameSpec::data(0x000, false, &[0x01, 0x00]),
    ];
    for f in &frames {
        let start = t;
        t = can_tx(&mut s, 0, t, CAN_BITRATE, f);
        if ecu(f) {
            can_tx(&mut s, 1, start, CAN_BITRATE, &CanFrameSpec { ack: false, ..f.clone() });
        } else if f.ack {
            // The ECU drives the ACK slot of everyone else's frames.
            let n = frame_bits(f).len();
            let ack_at = start + (n - 9) as f64 / CAN_BITRATE;
            s.set_t(1, ack_at, false);
            s.set_t(1, ack_at + 1.0 / CAN_BITRATE, true);
        }
        if !f.ack {
            // An error flag right after the ACK delimiter (ACK error) or the CRC delimiter.
            let n = frame_bits(f).len();
            let flag_at = start + (n - 7) as f64 / CAN_BITRATE;
            t = can_error_frame(&mut s, 0, flag_at, CAN_BITRATE);
        }
        t += 0.0002 + 0.0003 * ((f.id % 7) as f64 / 7.0);
    }
    s
}

fn mcp2515(rate: u64) -> Synth {
    // CS idle high, SCK idle low (mode 0), INT idle high, TXCAN/RXCAN recessive high.
    let mut s = Synth::new(rate, [true, false, false, false, true, true, true, false]);
    let l = SpiLines { cs: 0, clk: 1, mosi: 2, miso: 3, cpol: false, cpha: false };
    let hz = 1_000_000.0;
    let gap = 0.00002;
    let mut t = 0.0001;
    let x = |s: &mut Synth, mo: &[u8], mi: &[u8], t: &mut f64| {
        *t = spi_transfer(s, l, *t, hz, mo, mi) + gap;
    };
    x(&mut s, &[0xC0], &[], &mut t); // RESET
    t += 0.0001;
    x(&mut s, &[0x03, 0x0E, 0x00], &[0, 0, 0x80], &mut t); // READ CANSTAT: configuration mode
    // 125 kbit/s with an 8 MHz crystal: CNF1=0x01, CNF2=0xB1, CNF3=0x05.
    x(&mut s, &[0x02, 0x28, 0x05, 0xB1, 0x01], &[], &mut t);
    x(&mut s, &[0x02, 0x2B, 0x01], &[], &mut t); // CANINTE: RX0IE
    x(&mut s, &[0x05, 0x0F, 0xE0, 0x00], &[], &mut t); // BIT MODIFY CANCTRL: normal mode
    x(&mut s, &[0x03, 0x0E, 0x00], &[0, 0, 0x00], &mut t);
    // LOAD TX BUFFER 0: ID 0x123, 4 bytes.
    x(&mut s, &[0x40, 0x24, 0x60, 0x00, 0x00, 0x04, 0xDE, 0xAD, 0xBE, 0xEF], &[], &mut t);
    x(&mut s, &[0x81], &[], &mut t); // RTS TXB0
    // The frame goes out on TXCAN and comes back on RXCAN (the transceiver echoes the bus).
    let f = CanFrameSpec::data(0x123, false, &[0xDE, 0xAD, 0xBE, 0xEF]);
    let t0 = t + 0.00005;
    can_tx(&mut s, 5, t0, MCP_CAN_BITRATE, &CanFrameSpec { ack: false, ..f.clone() });
    let end = can_tx(&mut s, 6, t0, MCP_CAN_BITRATE, &f);
    // Another node answers with ID 0x321, and the MCP2515 raises INT once it is received.
    let reply = CanFrameSpec::data(0x321, false, &[0x01, 0x02]);
    let end2 = can_tx(&mut s, 6, end + 0.0002, MCP_CAN_BITRATE, &reply);
    let nbits = frame_bits(&reply).len();
    // ...and acknowledges it on TXCAN.
    let ack_at = end + 0.0002 + (nbits - 9) as f64 / MCP_CAN_BITRATE;
    s.set_t(5, ack_at, false);
    s.set_t(5, ack_at + 1.0 / MCP_CAN_BITRATE, true);
    s.set_t(4, end2 - 0.00002, false);
    t = end2 + 0.00003;
    x(&mut s, &[0xA0, 0x00], &[0x00, 0x01], &mut t); // READ STATUS: RX0IF
    // READ RX BUFFER 0 from SIDH: 0x321 = SIDH 0x64, SIDL 0x20.
    x(&mut s, &[0x90, 0, 0, 0, 0, 0, 0, 0], &[0x00, 0x64, 0x20, 0x00, 0x00, 0x02, 0x01, 0x02], &mut t);
    s.set_t(4, t - gap, true);
    x(&mut s, &[0x05, 0x2C, 0x01, 0x00], &[], &mut t); // clear RX0IF
    s
}

fn i2c(rate: u64) -> Synth {
    let mut s = Synth::new(rate, [true; 8]);
    let l = I2cLines { scl: 0, sda: 1 };
    let mut w = I2cWriter::new(&mut s, l, 0.0002, 100_000.0);
    // TMP102 at 0x48: pointer 0, repeated start, read 25.0 °C (0x19 0x00).
    w.start();
    w.byte(0x48 << 1, true);
    w.byte(0x00, true);
    w.start();
    w.byte(0x48 << 1 | 1, true);
    w.byte(0x19, true);
    w.byte(0x00, false);
    w.stop();
    w.t += 0.0005;
    // 24C02 EEPROM at 0x50: write 4 bytes at 0x10.
    w.start();
    w.byte(0x50 << 1, true);
    for b in [0x10, b'V', b'L', b'A', 0x01] {
        w.byte(b, true);
    }
    w.stop();
    w.t += 0.0005;
    // Nobody at 0x27 (an LCD backpack that is not fitted).
    w.start();
    w.byte(0x27 << 1, false);
    w.stop();
    w.t += 0.0005;
    // The EEPROM is busy writing: it NACKs until done, then reads back.
    w.start();
    w.byte(0x50 << 1, false);
    w.stop();
    w.t += 0.0006;
    w.start();
    w.byte(0x50 << 1, true);
    w.byte(0x10, true);
    w.start();
    w.byte(0x50 << 1 | 1, true);
    w.byte(b'V', true);
    w.byte(b'L', true);
    w.byte(b'A', false);
    w.stop();
    s
}

fn uart(rate: u64) -> Synth {
    let mut s = Synth::new(rate, [true; 8]);
    let u = UartConfig { channel: 0, baud: 115200.0, data_bits: 8, parity: Parity::None, stop_bits: 1.0, inverted: false, msb_first: false };
    let mut t = 0.001;
    let lines: [&[u8]; 3] = [b"VFD ready, fw 2.14\r\n", b"STAT RUN 49.98Hz 12.4A 61C\r\n", b"STAT RUN 50.00Hz 12.6A 61C\r\n"];
    for line in lines {
        t = uart_tx(&mut s, 0, t, line, &u, 115200.0, 0.0, UartFault::default()) + 0.006;
    }
    uart_tx(&mut s, 1, 0.012, b"STAT?\r", &u, 115200.0, 0.00002, UartFault::default());
    uart_tx(&mut s, 1, 0.060, b"FREQ 50.0\r", &u, 115200.0, 0.0, UartFault::default());
    let g = s.at(0.05);
    s.glitch(0, g, 1);
    s.glitch(1, g + 3, 1);
    let t = uart_tx(&mut s, 0, 0.07, b"OK FREQ 50.0\r\n", &u, 115200.0, 0.0, UartFault::default());
    let _ = t;
    s
}

/// The simulated device: streams a scenario, looped, in real time (or as fast as possible).
pub struct SimStream {
    data: Vec<u8>,
    pos: usize,
    rate: u64,
    realtime: bool,
    started: Option<Instant>,
    emitted: u64,
}

impl SimStream {
    pub fn new(id: ScenarioId, rate: u64, realtime: bool) -> Result<Self> {
        if !(1_000..=100_000_000).contains(&rate) {
            return Err(LogicError::Config("the simulator runs from 1 kHz to 100 MHz".into()));
        }
        Ok(Self { data: render(id, rate), pos: 0, rate, realtime, started: None, emitted: 0 })
    }
}

impl SampleStream for SimStream {
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        // 20 ms of samples per chunk.
        let n = (self.rate / 50).max(1) as usize;
        if self.realtime {
            let start = *self.started.get_or_insert_with(Instant::now);
            let due = Duration::from_secs_f64((self.emitted + n as u64) as f64 / self.rate as f64);
            let el = start.elapsed();
            if due > el {
                std::thread::sleep(due - el);
            }
        }
        let mut need = n;
        while need > 0 {
            if self.pos >= self.data.len() {
                self.pos = 0;
            }
            let k = need.min(self.data.len() - self.pos);
            buf.extend_from_slice(&self.data[self.pos..self.pos + k]);
            self.pos += k;
            need -= k;
        }
        self.emitted += n as u64;
        Ok(true)
    }

    fn sample_rate(&self) -> u64 {
        self.rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Capture;
    use crate::decode::{can, i2c, modbus, spi, uart};

    fn cap(id: ScenarioId) -> Capture {
        let i = info(id);
        Capture::new(i.default_rate, i.channels.clone(), render(id, i.default_rate))
    }

    #[test]
    fn modbus_scenario_decodes_as_designed() {
        let c = cap(ScenarioId::ModbusRtu);
        let DecoderConfig::ModbusRtu(cfg) = &info(ScenarioId::ModbusRtu).decoders[0] else { panic!() };
        let r = modbus::decode(&c, cfg).unwrap();
        let st: Vec<_> = r.transactions.iter().map(|t| t.status.clone()).collect();
        use modbus::Status::*;
        assert_eq!(st, vec![Ok, Ok, Ok, Ok, Exception { code: 2 }, NoResponse, Ok, CrcError, Broadcast]);
        assert_eq!(r.frames[1].values, vec![235, 300, 1500, 4]);
        // The bus channel (both directions on one line) pairs the same way.
        let bus = ModbusConfig { request_channel: Some(0), response_channel: None, ..cfg.clone() };
        let r2 = modbus::decode(&c, &bus).unwrap();
        assert_eq!(r2.transactions.iter().map(|t| t.status.clone()).collect::<Vec<_>>(), st);
        // The PLC scan output measures 100 Hz, 30 %.
        let m = c.measure(5, 0, c.len()).unwrap();
        assert!((m.frequency - 100.0).abs() < 0.01 && (m.duty - 0.3).abs() < 0.001);
    }

    #[test]
    fn can_scenario_decodes_as_designed() {
        let c = cap(ScenarioId::CanBus);
        let DecoderConfig::Can(cfg) = &info(ScenarioId::CanBus).decoders[0] else { panic!() };
        let r = can::decode(&c, cfg);
        let f: Vec<_> = r.frames.iter().map(|f| (f.id, f.error.clone())).collect();
        let good = r.frames.iter().filter(|f| f.error.is_none()).count();
        assert_eq!(good, 7, "{f:x?}");
        assert!(r.frames.iter().any(|f| f.id == 0x18FF5021 && f.error.as_deref() == Some("No ACK, error flag follows")), "{f:x?}");
        assert!(r.frames.iter().any(|f| f.id == 0x186 && !f.crc_ok && f.header_ok), "{f:x?}");
        assert!(r.frames.iter().any(|f| f.id == 0x285 && f.rtr));
    }

    #[test]
    fn mcp2515_scenario_decodes_as_designed() {
        let c = cap(ScenarioId::SpiMcp2515);
        let i = info(ScenarioId::SpiMcp2515);
        let DecoderConfig::Spi(sc) = &i.decoders[0] else { panic!() };
        let ts = spi::decode(&c, sc);
        let desc: Vec<String> = ts.iter().map(spi::mcp2515_describe).collect();
        assert_eq!(desc[0], "RESET");
        assert!(desc.contains(&"WRITE CNF3=05 CNF2=B1 CNF1=01".to_string()), "{desc:?}");
        assert!(desc.contains(&"LOAD TX BUFFER 0: ID 0x123 [4] DE AD BE EF".to_string()), "{desc:?}");
        assert!(desc.contains(&"READ RX BUFFER 0: ID 0x321 [2] 01 02".to_string()), "{desc:?}");
        let DecoderConfig::Can(cc) = &i.decoders[1] else { panic!() };
        let r = can::decode(&c, cc);
        assert_eq!(r.frames.iter().filter(|f| f.error.is_none()).map(|f| f.id).collect::<Vec<_>>(), vec![0x123, 0x321]);
    }

    #[test]
    fn i2c_and_uart_scenarios() {
        let c = cap(ScenarioId::I2cSensors);
        let r = i2c::decode(&c, &I2cConfig::default());
        assert_eq!(r.len(), 7);
        assert_eq!(r[1].bytes(), vec![0x19, 0x00]);
        assert_eq!(r.iter().filter(|t| !t.address_ack).count(), 2);
        let c = cap(ScenarioId::UartConsole);
        let DecoderConfig::Uart(u) = &info(ScenarioId::UartConsole).decoders[0] else { panic!() };
        let r = uart::decode(&c, u);
        let text = String::from_utf8(r.bytes()).unwrap();
        assert!(text.starts_with("VFD ready"));
        assert!(text.ends_with("OK FREQ 50.0\r\n"));
        assert_eq!(r.glitches.len(), 1);
        let DecoderConfig::Uart(u1) = &info(ScenarioId::UartConsole).decoders[1] else { panic!() };
        assert_eq!(uart::decode(&c, u1).bytes(), b"STAT?\rFREQ 50.0\r");
    }

    #[test]
    fn every_scenario_decodes_through_the_generic_api() {
        for id in SCENARIOS {
            let c = cap(id);
            for d in info(id).decoders {
                let out = d.decode(&c).unwrap();
                assert!(!out.annotations.is_empty(), "{id:?}");
            }
        }
    }

    #[test]
    fn stream_loops_and_paces() {
        let mut s = SimStream::new(ScenarioId::I2cSensors, 1_000_000, false).unwrap();
        let mut buf = Vec::new();
        for _ in 0..3 {
            s.read(&mut buf).unwrap();
        }
        assert_eq!(buf.len(), 60_000);
        let one = render(ScenarioId::I2cSensors, 1_000_000);
        assert_eq!(&buf[..one.len()], &one[..]);
        assert_eq!(&buf[one.len()..one.len() * 2], &one[..]);
        let mut rt = SimStream::new(ScenarioId::I2cSensors, 100_000, true).unwrap();
        let t = Instant::now();
        let mut b = Vec::new();
        for _ in 0..5 {
            rt.read(&mut b).unwrap();
        }
        assert!(t.elapsed() >= Duration::from_millis(90));
    }
}
