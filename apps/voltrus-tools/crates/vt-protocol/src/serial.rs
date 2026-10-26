use serde::{Deserialize, Serialize};

use crate::{Direction, SerialLink};

/// Open a serial port and show what crosses it: a terminal of the bytes,
/// sends and macros, and received lines parsed into points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialMonitorSpec {
    pub link: SerialLink,
    #[serde(default)]
    pub flow: FlowControl,
    /// Appended to every text line sent (not to hex).
    #[serde(default)]
    pub line_ending: LineEnding,
    /// How received lines become points.
    #[serde(default)]
    pub parser: LineParser,
    /// Saved sends, one click (or ⌃1…⌃9) away.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<SerialMacro>,
}

impl Default for SerialMonitorSpec {
    fn default() -> Self {
        Self {
            link: SerialLink {
                baud: 115_200,
                ..SerialLink::default()
            },
            flow: FlowControl::None,
            line_ending: LineEnding::Lf,
            parser: LineParser::KeyValue,
            macros: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowControl {
    #[default]
    None,
    /// RTS/CTS.
    Hardware,
    /// XON/XOFF.
    Software,
}

impl FlowControl {
    pub const ALL: [FlowControl; 3] = [Self::None, Self::Hardware, Self::Software];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Hardware => "RTS/CTS",
            Self::Software => "XON/XOFF",
        }
    }
}

/// What ends a line the app sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    None,
    #[default]
    Lf,
    Cr,
    CrLf,
}

impl LineEnding {
    pub const ALL: [LineEnding; 4] = [Self::None, Self::Lf, Self::Cr, Self::CrLf];

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::Cr => b"\r",
            Self::CrLf => b"\r\n",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Lf => "LF",
            Self::Cr => "CR",
            Self::CrLf => "CR LF",
        }
    }
}

/// How a received line (ended by CR, LF or CR LF) becomes point values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LineParser {
    /// Terminal only.
    None,
    /// `name=value` or `name:value` pairs separated by spaces, commas, tabs
    /// or semicolons, e.g. `T=21.4C RH=45%`. A unit may follow the number.
    #[default]
    KeyValue,
    /// Delimited numbers, one point per column. A line of names (a header)
    /// names the columns; so do `names`, in order.
    Csv {
        delimiter: char,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
    },
    /// A regular expression: each capture group is a point, named after the
    /// group (`(?P<temp>…)`) or numbered.
    Regex { pattern: String },
}

impl LineParser {
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::KeyValue => "Key=value",
            Self::Csv { .. } => "CSV",
            Self::Regex { .. } => "Regex",
        }
    }
}

/// A saved send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialMacro {
    pub name: String,
    /// Text (with `\r`, `\n`, `\t`, `\xNN` escapes) or hex bytes.
    pub payload: String,
    #[serde(default)]
    pub hex: bool,
}

impl SerialMacro {
    /// The bytes it sends: hex as written, text with the line ending.
    pub fn bytes(&self, ending: LineEnding) -> Result<Vec<u8>, String> {
        encode_send(&self.payload, self.hex, ending)
    }
}

/// The bytes of a send: hex as written, or text (escapes resolved) followed
/// by the line ending.
pub fn encode_send(text: &str, hex: bool, ending: LineEnding) -> Result<Vec<u8>, String> {
    if hex {
        let bytes = parse_hex(text)?;
        if bytes.is_empty() {
            return Err("Enter the bytes to send, e.g. 01 03 00 00".into());
        }
        Ok(bytes)
    } else {
        let mut bytes = unescape(text)?;
        bytes.extend_from_slice(ending.bytes());
        Ok(bytes)
    }
}

/// Hex bytes as engineers write them: `01 03 0A`, `01030A`, `0x01,0x03`
/// or `01:03`.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let mut digits = String::with_capacity(text.len());
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == ':' || c == '-') {
        let token = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if let Some(bad) = token.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(format!("“{bad}” is not a hex digit"));
        }
        // A lone digit between separators is one byte: `1 2` is 01 02.
        if token.len() % 2 == 1 {
            digits.push('0');
        }
        digits.push_str(token);
    }
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| "not a hex byte".to_owned())
        })
        .collect()
}

/// Text with C escapes resolved: `\r`, `\n`, `\t`, `\0`, `\\` and `\xNN`.
pub fn unescape(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        match chars.next() {
            Some('r') => bytes.push(b'\r'),
            Some('n') => bytes.push(b'\n'),
            Some('t') => bytes.push(b'\t'),
            Some('0') => bytes.push(0),
            Some('\\') => bytes.push(b'\\'),
            Some('x') => {
                let pair: String = chars.by_ref().take(2).collect();
                let byte = (pair.len() == 2)
                    .then(|| u8::from_str_radix(&pair, 16).ok())
                    .flatten()
                    .ok_or_else(|| format!("\\x{pair} needs two hex digits"))?;
                bytes.push(byte);
            }
            Some(other) => {
                return Err(format!(
                    "\\{other} is not an escape (\\r \\n \\t \\0 \\\\ \\xNN)"
                ));
            }
            None => return Err("A \\ at the end escapes nothing".into()),
        }
    }
    Ok(bytes)
}

/// Play an instrument on a virtual serial port (a pseudo-terminal): it
/// prints a device template's drifting values as lines and takes commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialDeviceSpec {
    /// Device template (the Modbus templates' values and units).
    pub template: String,
    pub format: SerialFormat,
    /// Milliseconds between printed lines.
    pub interval_ms: u64,
    /// Name of the stable link to the port in the temporary folder, so a
    /// monitor finds the device again after a restart.
    pub alias: String,
}

impl Default for SerialDeviceSpec {
    fn default() -> Self {
        Self {
            template: "abb-analyzer".into(),
            format: SerialFormat::KeyValue,
            interval_ms: 1000,
            alias: "voltrus-serial-1".into(),
        }
    }
}

impl SerialDeviceSpec {
    pub const MIN_INTERVAL_MS: u64 = 100;

    /// Where monitors open the device: the link in the temporary folder.
    pub fn path(&self) -> String {
        virtual_port_path(&self.alias)
    }

    /// A monitor of this device that parses what it prints.
    pub fn monitor(&self) -> SerialMonitorSpec {
        SerialMonitorSpec {
            link: SerialLink {
                port: self.path(),
                baud: 115_200,
                ..SerialLink::default()
            },
            parser: match self.format {
                SerialFormat::KeyValue => LineParser::KeyValue,
                SerialFormat::Csv => LineParser::Csv {
                    delimiter: ',',
                    names: Vec::new(),
                },
            },
            macros: vec![
                SerialMacro {
                    name: "Read now".into(),
                    payload: "READ".into(),
                    hex: false,
                },
                SerialMacro {
                    name: "Help".into(),
                    payload: "HELP".into(),
                    hex: false,
                },
            ],
            ..SerialMonitorSpec::default()
        }
    }
}

/// How the virtual device prints its values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerialFormat {
    /// `pH=7.21 ORP=245.3mV …`
    #[default]
    KeyValue,
    /// A header of names every 20 lines, then comma-separated numbers.
    Csv,
}

impl SerialFormat {
    pub const ALL: [SerialFormat; 2] = [Self::KeyValue, Self::Csv];

    pub fn name(self) -> &'static str {
        match self {
            Self::KeyValue => "Key=value",
            Self::Csv => "CSV",
        }
    }
}

/// The path of a virtual port's link named `alias` (in the temporary
/// folder, inside the container when sandboxed).
pub fn virtual_port_path(alias: &str) -> String {
    std::env::temp_dir()
        .join(alias)
        .to_string_lossy()
        .into_owned()
}

/// A serial port as shown by a port picker, short where it lives under the
/// temporary folder: `voltrus-serial-1` rather than its full path.
pub fn short_port(port: &str) -> &str {
    if port.starts_with("/dev/") {
        port
    } else {
        port.rsplit('/').next().unwrap_or(port)
    }
}

/// Bytes that crossed a serial port, as they were read or written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialChunk {
    pub at_ms: u64,
    pub direction: Direction,
    pub bytes: Vec<u8>,
}

/// A port the system (or a simulator in the app) offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialPortInfo {
    pub path: String,
    /// `USB`, `Bluetooth`, `PCI`, `Virtual`…
    pub kind: String,
    /// Manufacturer and product, when the system knows them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_takes_every_common_spelling() {
        let expected = vec![0x01, 0x03, 0x0A, 0xFF];
        for text in [
            "01 03 0A FF",
            "01030aff",
            "0x01,0x03,0x0A,0xFF",
            "01:03:0a:ff",
            " 1 3 a ff ",
        ] {
            assert_eq!(parse_hex(text), Ok(expected.clone()), "{text}");
        }
        assert_eq!(parse_hex(""), Ok(Vec::new()));
        assert!(parse_hex("01 0G").unwrap_err().contains('G'));
        assert!(parse_hex("hello").is_err());
    }

    #[test]
    fn escapes_resolve_and_bad_ones_say_why() {
        assert_eq!(unescape(r"AT\r\n"), Ok(b"AT\r\n".to_vec()));
        assert_eq!(unescape(r"\x02DATA\x03"), Ok(b"\x02DATA\x03".to_vec()));
        assert_eq!(unescape(r"a\\b\t\0"), Ok(b"a\\b\t\0".to_vec()));
        assert_eq!(unescape("°C"), Ok("°C".as_bytes().to_vec()));
        assert!(unescape(r"\q").is_err());
        assert!(unescape(r"\x4").is_err());
        assert!(unescape(r"\xZZ").is_err());
        assert!(unescape("end\\").is_err());
    }

    #[test]
    fn sends_end_text_lines_but_not_hex() {
        assert_eq!(
            encode_send("READ", false, LineEnding::CrLf),
            Ok(b"READ\r\n".to_vec())
        );
        assert_eq!(
            encode_send("READ", false, LineEnding::None),
            Ok(b"READ".to_vec())
        );
        assert_eq!(encode_send("", false, LineEnding::Lf), Ok(b"\n".to_vec()));
        assert_eq!(
            encode_send("0D 0A", true, LineEnding::CrLf),
            Ok(vec![0x0D, 0x0A])
        );
        assert!(
            encode_send("  ", true, LineEnding::Lf).is_err(),
            "nothing to send"
        );
    }

    #[test]
    fn a_device_monitor_opens_its_link_and_parses_its_format() {
        let device = SerialDeviceSpec {
            format: SerialFormat::Csv,
            alias: "voltrus-test-port".into(),
            ..SerialDeviceSpec::default()
        };
        let monitor = device.monitor();
        assert!(monitor.link.port.ends_with("voltrus-test-port"));
        assert_eq!(short_port(&monitor.link.port), "voltrus-test-port");
        assert_eq!(short_port("/dev/cu.usbserial-1"), "/dev/cu.usbserial-1");
        assert!(matches!(
            monitor.parser,
            LineParser::Csv { delimiter: ',', .. }
        ));
    }
}
