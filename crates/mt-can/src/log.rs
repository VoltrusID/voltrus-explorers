//! Trace files: the candump log format (`candump -l`, read by `canplayer`, SavvyCAN and Wireshark's
//! importers) and a CSV layout for spreadsheets.
//!
//! ```text
//! (1699999999.123456) can0 123#DEADBEEF
//! (1699999999.223456) can0 12345678#R
//! (1699999999.323456) can0 123##1AABBCCDDEEFF0011
//! ```

use crate::error::CanError;
use crate::frame::{CanFrame, Direction, TimedFrame, hex_bytes, parse_hex_bytes};

/// One candump line (without a newline).
pub fn candump_line(f: &TimedFrame, iface: &str) -> String {
    let secs = f.ts_us / 1_000_000;
    let micros = f.ts_us % 1_000_000;
    format!("({secs}.{micros:06}) {iface} {}", candump_frame(&f.frame))
}

/// `123#DEADBEEF`, `123#R`, `123##1AABB` (cansend's syntax too).
pub fn candump_frame(f: &CanFrame) -> String {
    let id = f.id_text();
    if f.rtr {
        return if f.rtr_len > 0 { format!("{id}#R{}", f.rtr_len) } else { format!("{id}#R") };
    }
    let data: String = f.data.iter().map(|b| format!("{b:02X}")).collect();
    if f.fd {
        // The flags nibble is the kernel's: 1 = bit rate switch, 2 = error state indicator.
        let flags = f.brs as u8 | (f.esi as u8) << 1;
        format!("{id}##{flags:X}{data}")
    } else {
        format!("{id}#{data}")
    }
}

/// Parses cansend syntax: `123#DEADBEEF`, `1F334455#11.22`, `123#R`, `123#R3`, `123##1AABB`
/// (CAN FD: the digit after `##` holds the flags, 1 = bit rate switch, 2 = error state indicator).
/// Three hex digits or fewer is an 11-bit identifier, eight is 29-bit (as cansend decides).
pub fn parse_frame(text: &str) -> Result<CanFrame, CanError> {
    let text = text.trim();
    let bad = |m: &str| CanError::Frame(format!("{m}: \"{text}\" (expected e.g. 123#DEADBEEF)"));
    let (id_s, rest) = text.split_once('#').ok_or_else(|| bad("no '#'"))?;
    if id_s.is_empty() || id_s.len() > 8 {
        return Err(bad("identifier must be 1 to 8 hex digits"));
    }
    let id = u32::from_str_radix(id_s, 16).map_err(|_| bad("identifier is not hex"))?;
    let extended = id_s.len() > 3;
    if let Some(fd) = rest.strip_prefix('#') {
        let mut chars = fd.chars();
        let flags = chars.next().and_then(|c| c.to_digit(16)).ok_or_else(|| bad("CAN FD frames need a flags digit after ##"))?;
        let data = parse_hex_bytes(chars.as_str())?;
        let mut f = CanFrame::fd(id, extended, flags & 1 != 0, &data)?;
        f.esi = flags & 2 != 0;
        return Ok(f);
    }
    if let Some(r) = rest.strip_prefix('R').or_else(|| rest.strip_prefix('r')) {
        let len = if r.is_empty() { 0 } else { r.parse::<u8>().map_err(|_| bad("remote length must be 0 to 8"))? };
        return CanFrame::remote(id, extended, len);
    }
    CanFrame::with_ext(id, extended, &parse_hex_bytes(rest)?)
}

/// A frame read back from a log, with the interface name it was recorded on.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    pub ts_us: u64,
    pub iface: String,
    pub frame: CanFrame,
}

/// Reads a candump log. Blank lines and lines starting with `#` are skipped.
pub fn parse_candump(text: &str) -> Result<Vec<LogRecord>, CanError> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let err = |m: String| CanError::Log { line: n + 1, msg: m };
        let rest = line.strip_prefix('(').ok_or_else(|| err("expected \"(seconds.micros) iface id#data\"".into()))?;
        let (ts, rest) = rest.split_once(')').ok_or_else(|| err("no closing ')' after the time".into()))?;
        let ts_us = parse_ts(ts).ok_or_else(|| err(format!("bad time {ts}")))?;
        let mut parts = rest.split_whitespace();
        let iface = parts.next().ok_or_else(|| err("no interface name".into()))?.to_string();
        let frame_s = parts.next().ok_or_else(|| err("no frame".into()))?;
        let frame = parse_frame(frame_s).map_err(|e| err(e.to_string()))?;
        out.push(LogRecord { ts_us, iface, frame });
    }
    Ok(out)
}

fn parse_ts(s: &str) -> Option<u64> {
    let (sec, frac) = s.split_once('.').unwrap_or((s, "0"));
    let sec: u64 = sec.trim().parse().ok()?;
    let frac = frac.trim();
    if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut digits: String = frac.chars().take(6).collect();
    while digits.len() < 6 {
        digits.push('0');
    }
    Some(sec * 1_000_000 + digits.parse::<u64>().ok()?)
}

pub const CSV_HEADER: &str = "time_s,direction,id,extended,fd,brs,rtr,dlc,data";

/// One CSV row (no newline). `t0_us` is subtracted from the time.
pub fn csv_row(f: &TimedFrame, t0_us: u64) -> String {
    let t = f.ts_us.saturating_sub(t0_us) as f64 / 1e6;
    format!(
        "{t:.6},{},{},{},{},{},{},{},{}",
        match f.dir {
            Direction::Rx => "rx",
            Direction::Tx => "tx",
        },
        f.frame.id_text(),
        f.frame.extended as u8,
        f.frame.fd as u8,
        f.frame.brs as u8,
        f.frame.rtr as u8,
        f.frame.dlc(),
        hex_bytes(&f.frame.data)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_syntax() {
        assert_eq!(parse_frame("123#DEADBEEF").unwrap(), CanFrame::new(0x123, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap());
        let e = parse_frame("00000123#11.22").unwrap();
        assert!(e.extended && e.id == 0x123 && e.data == vec![0x11, 0x22]);
        assert_eq!(parse_frame("7DF#R").unwrap(), CanFrame::remote(0x7DF, false, 0).unwrap());
        assert_eq!(parse_frame("7DF#R8").unwrap().rtr_len, 8);
        let fd = parse_frame("123##1AABB").unwrap();
        assert!(fd.fd && fd.brs && fd.data == vec![0xAA, 0xBB]);
        assert!(parse_frame("123").is_err());
        assert!(parse_frame("123#123").is_err());
        assert!(parse_frame("800#00").is_err());
        assert!(parse_frame("123456789#00").is_err());
        let esi = parse_frame("123##3").unwrap();
        assert!(esi.fd && esi.brs && esi.esi && esi.data.is_empty());
        assert!(!parse_frame("123##1").unwrap().esi);
        assert!(parse_frame("123##").is_err());
        assert!(parse_frame("123##1AABBCCDDEEFF001122").is_err(), "9 bytes is not an FD length");
        for f in ["123#DEADBEEF", "12345678#R", "123#R3", "123##0", "001##1", "123##2", "1ABCDEF0##3"] {
            assert_eq!(candump_frame(&parse_frame(f).unwrap()), f);
        }
    }

    #[test]
    fn candump_round_trip() {
        let frames = [TimedFrame { ts_us: 1_699_999_999_123_456, dir: Direction::Rx, frame: CanFrame::new(0x123, &[1, 2]).unwrap() },
            TimedFrame { ts_us: 1_700_000_000_000_001, dir: Direction::Tx, frame: CanFrame::fd(0x1234, true, true, &[0; 12]).unwrap() }];
        let text: String = frames.iter().map(|f| candump_line(f, "can0") + "\n").collect();
        assert!(text.starts_with("(1699999999.123456) can0 123#0102\n"));
        let back = parse_candump(&format!("# comment\n\n{text}")).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].ts_us, frames[0].ts_us);
        assert_eq!(back[1].frame, frames[1].frame);
        assert_eq!(back[1].iface, "can0");
    }

    #[test]
    fn candump_errors_carry_lines() {
        let e = parse_candump("(1.0) can0 123#00\nbad line\n").unwrap_err();
        assert!(matches!(e, CanError::Log { line: 2, .. }));
        assert_eq!(parse_candump("(1.5) vcan0 1#").unwrap()[0].ts_us, 1_500_000);
        assert!(parse_candump("(x.1) can0 1#").is_err());
    }

    #[test]
    fn candump_fd_64_bytes() {
        // A line as the yocto image's can-demo telemetry shows up in `candump -l` on the host.
        let data: String = (0..64).map(|i| format!("{:02X}", i as u8)).collect();
        let line = format!("(1762851234.000100) vcan0 201##1{data}");
        let r = &parse_candump(&line).unwrap()[0];
        assert!(r.frame.fd && r.frame.brs && !r.frame.esi);
        assert_eq!(r.frame.data.len(), 64);
        assert_eq!(r.frame.dlc(), 15);
        assert_eq!(candump_line(&TimedFrame { ts_us: r.ts_us, dir: Direction::Rx, frame: r.frame.clone() }, "vcan0"), line);
        let row = csv_row(&TimedFrame { ts_us: r.ts_us, dir: Direction::Rx, frame: r.frame.clone() }, r.ts_us);
        assert!(row.starts_with("0.000000,rx,201,0,1,1,0,15,00 01 02 "), "{row}");
        assert!(row.ends_with(" 3E 3F"));
    }

    #[test]
    fn csv() {
        let f = TimedFrame { ts_us: 2_500_000, dir: Direction::Tx, frame: CanFrame::new(0x7E8, &[0x41, 0x0C]).unwrap() };
        assert_eq!(csv_row(&f, 1_000_000), "1.500000,tx,7E8,0,0,0,0,2,41 0C");
    }
}
