//! Small helpers: percent-encoding for query strings and UTC timestamp formatting.

use std::time::{SystemTime, UNIX_EPOCH};

/// Percent-encode `s` for use in a URL query component (RFC 3986 unreserved bytes kept).
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode `%XX` escapes and `+` (as space) from a query component. Invalid escapes pass through.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push(hex(bytes[i + 1]).unwrap() << 4 | hex(bytes[i + 2]).unwrap());
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Microseconds since the Unix epoch.
pub fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Format microseconds since the Unix epoch as an MTConnect timestamp
/// (`2026-10-29T02:01:39.506169Z`, UTC, microsecond precision).
pub fn iso_from_micros(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{frac:06}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Current time as an MTConnect timestamp.
pub fn now_iso() -> String {
    iso_from_micros(now_micros())
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_epoch_and_known_dates() {
        assert_eq!(iso_from_micros(0), "1970-01-01T00:00:00.000000Z");
        assert_eq!(iso_from_micros(1_700_000_000_000_000 + 123_456), "2023-11-14T22:13:20.123456Z");
        // 2024-02-29 (leap day) 12:00:00 UTC
        assert_eq!(iso_from_micros(1_709_208_000_000_000), "2024-02-29T12:00:00.000000Z");
        // Before the epoch
        assert_eq!(iso_from_micros(-1), "1969-12-31T23:59:59.999999Z");
    }

    #[test]
    fn percent_round_trip() {
        let s = "//Axes//Linear[@id=\"x pos\"]";
        let enc = percent_encode(s);
        assert!(!enc.contains(' ') && !enc.contains('['));
        assert_eq!(percent_decode(&enc), s);
        assert_eq!(percent_decode("a+b%2"), "a b%2");
    }
}
