//! Protocol decoders. Each one reads a [`Capture`], returns typed results (tested directly), and
//! turns them into the app's generic [`DecodeOutput`]: annotation rows drawn under the waveform and
//! an optional protocol table.

pub mod can;
pub mod i2c;
pub mod modbus;
pub mod spi;
pub mod uart;

use serde::{Deserialize, Serialize};

use crate::capture::Capture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Payload: bytes, words, values.
    Data,
    /// Structure: addresses, fields, start and stop conditions.
    Control,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Index into [`DecodeOutput::rows`].
    pub row: u8,
    pub start: u64,
    pub end: u64,
    pub text: String,
    /// Shorter text for narrow boxes.
    pub short: String,
    pub level: Level,
}

impl Annotation {
    pub fn new(row: u8, start: u64, end: u64, level: Level, text: impl Into<String>, short: impl Into<String>) -> Self {
        Self { row, start, end, text: text.into(), short: short.into(), level }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    pub start: u64,
    pub end: u64,
    pub level: Level,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DecodeOutput {
    pub rows: Vec<String>,
    pub annotations: Vec<Annotation>,
    pub table: Option<Table>,
    /// Counts for the decoder's header line ("212 frames, 3 CRC errors").
    pub summary: String,
}

/// A decoder and its settings, as the app stores them in a decoder stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DecoderConfig {
    Uart(uart::UartConfig),
    ModbusRtu(modbus::ModbusConfig),
    Spi(spi::SpiConfig),
    I2c(i2c::I2cConfig),
    Can(can::CanConfig),
}

impl DecoderConfig {
    /// Channels this decoder reads; each must exist in the capture.
    pub fn channels(&self) -> Vec<u8> {
        match self {
            DecoderConfig::Uart(c) => vec![c.channel],
            DecoderConfig::ModbusRtu(c) => [c.request_channel, c.response_channel].into_iter().flatten().collect(),
            DecoderConfig::Spi(c) => [Some(c.clk), c.mosi, c.miso, c.cs].into_iter().flatten().collect(),
            DecoderConfig::I2c(c) => vec![c.scl, c.sda],
            DecoderConfig::Can(c) => vec![c.channel],
        }
    }

    pub fn decode(&self, cap: &Capture) -> Result<DecodeOutput, String> {
        let n = cap.channels.len().min(crate::capture::MAX_CHANNELS) as u8;
        if let Some(bad) = self.channels().into_iter().find(|&c| c >= n) {
            return Err(format!("channel D{bad} is not in this capture ({n} channels)"));
        }
        Ok(match self {
            DecoderConfig::Uart(c) => {
                c.validate(cap)?;
                uart::output(c, &uart::decode(cap, c))
            }
            DecoderConfig::ModbusRtu(c) => modbus::output(cap, c, &modbus::decode(cap, c)?),
            DecoderConfig::Spi(c) => spi::output_with_times(cap, c, &spi::decode(cap, c)),
            DecoderConfig::I2c(c) => i2c::output(&i2c::decode(cap, c)),
            DecoderConfig::Can(c) => {
                c.validate(cap)?;
                can::output(cap, &can::decode(cap, c))
            }
        })
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// Time column text for tables, relative to the capture start (or the trigger when there is one).
pub(crate) fn time_cell(cap: &Capture, sample: u64) -> String {
    let zero = cap.trigger.unwrap_or(0) as f64;
    let t = (sample as f64 - zero) / cap.sample_rate as f64;
    format!("{:+.6} s", t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::default_channels;

    fn all_decoders() -> Vec<DecoderConfig> {
        vec![
            DecoderConfig::Uart(uart::UartConfig { baud: 9600.0, ..Default::default() }),
            DecoderConfig::ModbusRtu(modbus::ModbusConfig { request_channel: Some(0), response_channel: Some(1), baud: 9600.0, ..Default::default() }),
            DecoderConfig::ModbusRtu(modbus::ModbusConfig { request_channel: Some(2), response_channel: None, baud: 9600.0, ..Default::default() }),
            DecoderConfig::Spi(spi::SpiConfig { device: spi::SpiDevice::Mcp2515, ..Default::default() }),
            DecoderConfig::Spi(spi::SpiConfig { cs: None, mode: 3, word_bits: 12, ..Default::default() }),
            DecoderConfig::I2c(i2c::I2cConfig { min_pulse: 1, ..Default::default() }),
            DecoderConfig::Can(can::CanConfig { bitrate: 125_000.0, ..Default::default() }),
        ]
    }

    /// Noise, stuck lines and tiny captures must never panic a decoder.
    #[test]
    fn decoders_survive_noise_and_edge_cases() {
        let mut x = 0x1234_5678u32;
        let mut noise = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let mut inputs: Vec<Vec<u8>> = vec![vec![], vec![0], vec![0xFF], vec![0; 50_000], vec![0xFF; 50_000]];
        // White noise, and slow random levels (runs of 1 to 300 samples).
        inputs.push((0..200_000).map(|_| noise() as u8).collect());
        let mut slow = Vec::new();
        while slow.len() < 400_000 {
            let v = noise() as u8;
            let n = 1 + noise() as usize % 300;
            slow.extend(std::iter::repeat_n(v, n));
        }
        inputs.push(slow);
        for data in inputs {
            let cap = Capture::new(2_000_000, default_channels(8), data);
            for d in all_decoders() {
                let _ = d.decode(&cap);
            }
        }
    }

    #[test]
    fn rejects_missing_channels() {
        let cap = Capture::new(1_000_000, default_channels(2), vec![0; 100]);
        let e = DecoderConfig::Spi(spi::SpiConfig::default()).decode(&cap).unwrap_err();
        assert!(e.contains("D2"), "{e}");
    }
}
