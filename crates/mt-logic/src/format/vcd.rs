//! Value Change Dump export (IEEE 1364), one 1-bit wire per channel. The timescale is the largest
//! of 1 ps / 1 ns / 1 µs that divides the sample period exactly, so times stay integers.

use std::io::Write;

use crate::capture::Capture;
use crate::error::Result;

fn ident(i: usize) -> char {
    (b'!' + i as u8) as char
}

fn timescale(rate: u64) -> (&'static str, u64) {
    // Period in picoseconds when it is a whole number; otherwise 1 ps with rounding.
    for (name, ps) in [("1 us", 1_000_000u64), ("1 ns", 1_000), ("1 ps", 1)] {
        let units_per_s = 1_000_000_000_000 / ps;
        if units_per_s % rate == 0 {
            return (name, units_per_s / rate);
        }
    }
    ("1 ps", (1e12 / rate as f64).round() as u64)
}

pub fn write_vcd(w: &mut impl Write, cap: &Capture) -> Result<()> {
    let (scale, per_sample) = timescale(cap.sample_rate);
    let n = cap.channels.len().min(8);
    writeln!(w, "$date Voltrus Logic Analyzer export $end")?;
    writeln!(w, "$version mt-logic {} $end", env!("CARGO_PKG_VERSION"))?;
    writeln!(w, "$timescale {scale} $end")?;
    writeln!(w, "$scope module logic $end")?;
    for (i, c) in cap.channels.iter().take(n).enumerate() {
        let name: String = c.name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' }).collect();
        writeln!(w, "$var wire 1 {} {} $end", ident(i), if name.is_empty() { format!("D{i}") } else { name })?;
    }
    writeln!(w, "$upscope $end\n$enddefinitions $end")?;
    let data = cap.data();
    let Some(&first) = data.first() else { return Ok(()) };
    writeln!(w, "#0\n$dumpvars")?;
    for i in 0..n {
        writeln!(w, "{}{}", first >> i & 1, ident(i))?;
    }
    writeln!(w, "$end")?;
    let mask = if n >= 8 { 0xFF } else { (1u8 << n) - 1 };
    let mut prev = first & mask;
    for (s, &v) in data.iter().enumerate().skip(1) {
        let v = v & mask;
        if v == prev {
            continue;
        }
        writeln!(w, "#{}", s as u64 * per_sample)?;
        let diff = v ^ prev;
        for i in 0..n {
            if diff >> i & 1 == 1 {
                writeln!(w, "{}{}", v >> i & 1, ident(i))?;
            }
        }
        prev = v;
    }
    writeln!(w, "#{}", data.len() as u64 * per_sample)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::default_channels;

    #[test]
    fn timescales() {
        assert_eq!(timescale(1_000_000), ("1 us", 1));
        assert_eq!(timescale(24_000_000), ("1 ps", 41_667));
        assert_eq!(timescale(8_000_000), ("1 ns", 125));
        assert_eq!(timescale(500_000), ("1 us", 2));
    }

    #[test]
    fn writes_changes_only() {
        let mut ch = default_channels(2);
        ch[0].name = "RS-485 RO".into();
        let cap = Capture::new(1_000_000, ch, vec![0b01, 0b01, 0b11, 0b10]);
        let mut out = Vec::new();
        write_vcd(&mut out, &cap).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("$var wire 1 ! RS_485_RO $end"));
        assert!(s.contains("$var wire 1 \" D1 $end"));
        assert!(s.contains("#0\n$dumpvars\n1!\n0\"\n$end"));
        assert!(s.contains("#2\n1\"\n#3\n0!\n#4"));
        assert!(!s.contains("#1\n"));
    }
}
