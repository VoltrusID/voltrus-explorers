//! Capture to file: every RX and TX chunk of every open session, one CSV row each.
//!
//! Rows are flushed as they are written, so a crash or a pulled cable still leaves a complete file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

use serde::Serialize;

const HEADER: &str = "timestamp_utc,port,direction,hex,ascii\n";

struct Writer {
    out: BufWriter<File>,
    path: String,
    rows: u64,
}

/// What the frontend shows while a capture runs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CaptureStatus {
    pub path: String,
    pub rows: u64,
}

#[derive(Default)]
pub struct Capture {
    inner: Mutex<Option<Writer>>,
}

impl Capture {
    /// Create (or truncate) the file at `path` and start capturing into it, replacing a running capture.
    pub fn start(&self, path: &str) -> Result<(), String> {
        let file = File::create(path).map_err(|e| format!("Cannot create {path}: {e}"))?;
        let mut out = BufWriter::new(file);
        out.write_all(HEADER.as_bytes())
            .and_then(|_| out.flush())
            .map_err(|e| format!("Cannot write {path}: {e}"))?;
        *self.inner.lock().unwrap_or_else(|p| p.into_inner()) = Some(Writer {
            out,
            path: path.to_string(),
            rows: 0,
        });
        Ok(())
    }

    /// Stop capturing; returns the number of rows written, or `None` if no capture was running.
    pub fn stop(&self) -> Option<u64> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        guard.take().map(|mut w| {
            let _ = w.out.flush();
            w.rows
        })
    }

    pub fn status(&self) -> Option<CaptureStatus> {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().map(|w| CaptureStatus { path: w.path.clone(), rows: w.rows })
    }

    /// Append one row. A write failure closes the capture and returns the reason.
    /// A no-op when no capture is running.
    pub fn record(&self, port: &str, direction: &str, bytes: &[u8], ts_ms: u64) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(w) = guard.as_mut() else { return Ok(()) };
        let row = format_row(port, direction, bytes, ts_ms);
        match w.out.write_all(row.as_bytes()).and_then(|_| w.out.flush()) {
            Ok(()) => {
                w.rows += 1;
                Ok(())
            }
            Err(e) => {
                let path = w.path.clone();
                *guard = None;
                Err(format!("Capture to {path} stopped: {e}"))
            }
        }
    }
}

fn format_row(port: &str, direction: &str, bytes: &[u8], ts_ms: u64) -> String {
    let hex = bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
    let ascii: String = bytes
        .iter()
        .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' })
        .collect();
    format!(
        "{},{},{direction},{hex},{}\n",
        iso_utc(ts_ms),
        csv_field(port),
        csv_field(&ascii)
    )
}

/// Quote a text field. Cells that spreadsheets would read as a formula get a leading apostrophe.
fn csv_field(s: &str) -> String {
    let guarded = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

/// `2023-11-14T22:13:20.000Z` from UNIX epoch milliseconds.
fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let milli = ms % 1000;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_known_instants() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        // Leap day
        assert_eq!(iso_utc(1_709_208_000_000), "2024-02-29T12:00:00.000Z");
    }

    #[test]
    fn row_has_hex_and_printable_ascii() {
        let row = format_row("/dev/ttyUSB0", "RX", &[0x48, 0x69, 0x00, 0x0A], 0);
        assert_eq!(
            row,
            "1970-01-01T00:00:00.000Z,\"/dev/ttyUSB0\",RX,48 69 00 0A,\"Hi..\"\n"
        );
    }

    #[test]
    fn csv_field_escapes_quotes_and_formulas() {
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_field("@x"), "\"'@x\"");
    }

    #[test]
    fn capture_writes_rows_and_stops() {
        let path = std::env::temp_dir().join(format!("serial-capture-test-{}.csv", std::process::id()));
        let p = path.to_str().unwrap();
        let cap = Capture::default();
        // Recording with no capture running is a no-op.
        assert!(cap.record("p", "RX", b"x", 0).is_ok());
        assert_eq!(cap.status(), None);

        cap.start(p).unwrap();
        cap.record("COM1", "TX", &[0x01, 0xFF], 1000).unwrap();
        cap.record("COM2", "RX", b"ok", 2000).unwrap();
        assert_eq!(cap.status(), Some(CaptureStatus { path: p.to_string(), rows: 2 }));
        assert_eq!(cap.stop(), Some(2));
        assert_eq!(cap.stop(), None);

        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "timestamp_utc,port,direction,hex,ascii");
        assert_eq!(lines[1], "1970-01-01T00:00:01.000Z,\"COM1\",TX,01 FF,\"..\"");
        assert_eq!(lines[2], "1970-01-01T00:00:02.000Z,\"COM2\",RX,6F 6B,\"ok\"");
    }

    #[test]
    fn start_reports_unwritable_path() {
        let cap = Capture::default();
        assert!(cap.start("/nonexistent-dir-for-serial-test/out.csv").is_err());
        assert_eq!(cap.status(), None);
    }
}
