//! Serial: a monitor on `mt-serial`'s async port (terminal bytes, sends,
//! logging, lines parsed into points) and a device simulator that prints a
//! template's values on a virtual port (a pseudo-terminal).

pub(crate) mod device;
pub(crate) mod monitor;
mod parser;

use std::fs::File;
use std::io::{self, BufWriter, Write};

use vt_protocol::{Direction, SerialChunk, SerialPortInfo};

pub(crate) use parser::{Lines, Parser};

/// Answer a [`vt_protocol::Command::ListSerialPorts`]: the system's ports,
/// with macOS's call-in twin (`/dev/tty.X`) dropped where its call-out port
/// (`/dev/cu.X`, the one to open) is listed.
pub(crate) async fn list_ports() -> Result<Vec<SerialPortInfo>, String> {
    let ports = tokio::task::spawn_blocking(mt_serial::list_ports)
        .await
        .map_err(|error| format!("Listing ports failed: {error}"))?
        .map_err(|error| error.to_string())?;
    let ports: Vec<SerialPortInfo> = ports
        .into_iter()
        .map(|port| SerialPortInfo {
            description: [port.manufacturer, port.product]
                .into_iter()
                .flatten()
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            kind: port.port_type,
            path: port.port_name,
        })
        .collect();
    Ok(without_call_in_twins(ports))
}

fn without_call_in_twins(ports: Vec<SerialPortInfo>) -> Vec<SerialPortInfo> {
    let call_out: Vec<String> = ports
        .iter()
        .filter_map(|port| port.path.strip_prefix("/dev/cu.").map(str::to_owned))
        .collect();
    let mut kept: Vec<SerialPortInfo> = ports
        .into_iter()
        .filter(|port| {
            port.path
                .strip_prefix("/dev/tty.")
                .is_none_or(|name| !call_out.iter().any(|other| other == name))
        })
        .collect();
    kept.sort_by(|a, b| a.path.cmp(&b.path));
    kept.dedup_by(|a, b| a.path == b.path);
    kept
}

/// A CSV capture of every chunk that crosses the port, in the Serial
/// Monitor app's format: `timestamp_utc,port,direction,hex,ascii`.
pub(crate) struct LogFile {
    out: BufWriter<File>,
    pub path: String,
    port: String,
}

const LOG_HEADER: &str = "timestamp_utc,port,direction,hex,ascii\n";

impl LogFile {
    /// Create (or truncate) the file at `path`.
    pub fn create(path: &str, port: &str) -> Result<Self, String> {
        let file = File::create(path).map_err(|error| format!("Cannot create {path}: {error}"))?;
        let mut out = BufWriter::new(file);
        out.write_all(LOG_HEADER.as_bytes())
            .and_then(|()| out.flush())
            .map_err(|error| format!("Cannot write {path}: {error}"))?;
        Ok(Self {
            out,
            path: path.to_owned(),
            port: port.to_owned(),
        })
    }

    pub fn record(&mut self, chunk: &SerialChunk) -> io::Result<()> {
        self.out.write_all(log_row(&self.port, chunk).as_bytes())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

fn log_row(port: &str, chunk: &SerialChunk) -> String {
    format!(
        "{},{},{},{},{}\n",
        iso_time(chunk.at_ms),
        csv_field(port),
        match chunk.direction {
            Direction::Rx => "RX",
            Direction::Tx => "TX",
        },
        hex(&chunk.bytes),
        csv_field(&printable(&chunk.bytes)),
    )
}

/// `01 0D FF`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

/// Printable ASCII as is, everything else as a dot.
pub(crate) fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// `2026-10-26T09:41:05.123Z` from milliseconds since the Unix epoch.
pub(crate) fn iso_time(at_ms: u64) -> String {
    let days = i64::try_from(at_ms / 86_400_000).unwrap_or(0);
    let in_day = at_ms % 86_400_000;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_utc_iso_8601() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
        // 2000-02-29, a leap day of a century leap year.
        assert_eq!(iso_time(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(1_793_093_465_123), "2026-10-27T09:31:05.123Z");
        assert_eq!(iso_time(4_102_444_799_999), "2099-12-31T23:59:59.999Z");
    }

    #[test]
    fn log_rows_quote_what_csv_needs() {
        let chunk = SerialChunk {
            at_ms: 0,
            direction: Direction::Tx,
            bytes: b"a,\"b\"\r\n\x01".to_vec(),
        };
        assert_eq!(
            log_row("/dev/cu.x", &chunk),
            "1970-01-01T00:00:00.000Z,/dev/cu.x,TX,61 2C 22 62 22 0D 0A 01,\"a,\"\"b\"\"...\"\n"
        );
    }

    #[test]
    fn call_in_twins_of_listed_call_out_ports_are_dropped() {
        let port = |path: &str| SerialPortInfo {
            path: path.into(),
            kind: "USB".into(),
            description: String::new(),
        };
        let kept = without_call_in_twins(vec![
            port("/dev/tty.usbserial-1"),
            port("/dev/cu.usbserial-1"),
            // The call-out port may come first.
            port("/dev/cu.usbmodem-2"),
            port("/dev/tty.usbmodem-2"),
            // A call-in port with no call-out twin stays.
            port("/dev/tty.usbmodem-3"),
            port("/dev/tty.Bluetooth-Incoming-Port"),
            port("/dev/ttyUSB0"),
            port("/dev/ttyUSB0"),
        ]);
        let paths: Vec<&str> = kept.iter().map(|port| port.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/dev/cu.usbmodem-2",
                "/dev/cu.usbserial-1",
                "/dev/tty.Bluetooth-Incoming-Port",
                "/dev/tty.usbmodem-3",
                "/dev/ttyUSB0"
            ]
        );
    }
}
