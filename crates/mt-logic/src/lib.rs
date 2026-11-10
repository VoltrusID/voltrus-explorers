//! `mt-logic`: the logic analyzer behind Voltrus Logic Analyzer.
//!
//! - [`capture`]: 8-channel sample buffers with a summary pyramid for fast drawing and edge search
//! - [`trigger`] and [`engine`]: edge and pattern triggers, pre-trigger, bounded-memory capture from a stream
//! - [`decode`]: UART, Modbus RTU, SPI (with MCP2515), I2C and CAN decoders
//! - [`format`]: our `.vla` files, sigrok `.sr` import and export, VCD and CSV export
//! - [`synth`] and [`sim`]: waveform synthesis and the simulated device with industrial scenarios
//! - [`fx2`]: the driver for Cypress FX2 boards running the fx2lafw firmware (USB behind a trait)

pub mod capture;
pub mod decode;
pub mod engine;
pub mod error;
pub mod format;
pub mod fx2;
pub mod sim;
pub mod synth;
pub mod trigger;

pub use capture::{Capture, ChannelInfo, fmt_time};
pub use decode::{Annotation, DecodeOutput, DecoderConfig, Level, Table, TableRow};
pub use error::{LogicError, Result};
pub use trigger::Trigger;

/// Formats a sample rate: `24 MHz`, `500 kHz`, `20 kHz`.
pub fn fmt_rate(hz: u64) -> String {
    let (v, unit) = if hz >= 1_000_000 {
        (hz as f64 / 1e6, "MHz")
    } else if hz >= 1_000 {
        (hz as f64 / 1e3, "kHz")
    } else {
        (hz as f64, "Hz")
    };
    if v.fract() == 0.0 { format!("{v:.0} {unit}") } else { format!("{v} {unit}") }
}

/// Parses `24 MHz`, `500kHz`, `1 GHz`, `100 Hz` or a bare number of Hz (sigrok metadata style).
pub fn parse_rate(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    let (num, mul) = if let Some(n) = t.strip_suffix("ghz") {
        (n, 1e9)
    } else if let Some(n) = t.strip_suffix("mhz") {
        (n, 1e6)
    } else if let Some(n) = t.strip_suffix("khz") {
        (n, 1e3)
    } else if let Some(n) = t.strip_suffix("hz") {
        (n, 1.0)
    } else {
        (t.as_str(), 1.0)
    };
    let v: f64 = num.parse().ok()?;
    (v > 0.0).then(|| (v * mul).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(fmt_rate(24_000_000), "24 MHz");
        assert_eq!(fmt_rate(500_000), "500 kHz");
        assert_eq!(fmt_rate(2_500_000), "2.5 MHz");
        assert_eq!(parse_rate("1 MHz"), Some(1_000_000));
        assert_eq!(parse_rate("500 kHz"), Some(500_000));
        assert_eq!(parse_rate("24000000"), Some(24_000_000));
        assert_eq!(parse_rate("1.5 GHz"), Some(1_500_000_000));
        assert_eq!(parse_rate("fast"), None);
    }
}
