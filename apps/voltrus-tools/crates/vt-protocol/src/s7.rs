//! Siemens S7 (ISO-on-TCP) sessions: tag addresses as engineers write them,
//! the client and simulator specs, and what the PLC says about itself.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The ISO-on-TCP port every S7 CPU listens on.
pub const S7_PORT: u16 = 102;
/// The PDU size a client asks for unless told otherwise; the CPU answers
/// with what it can do (S7-300: 240, S7-1500: 960).
pub const S7_DEFAULT_PDU: u16 = 480;
/// The PDU sizes a session may ask for or a simulator offer.
pub const S7_MIN_PDU: u16 = 240;
pub const S7_MAX_PDU: u16 = 960;

/// A memory area of the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7Area {
    /// A data block, `DB<n>.DB…`.
    Db,
    /// Process image of the inputs, `I…`.
    Input,
    /// Process image of the outputs, `Q…`.
    Output,
    /// Bit memory (merker), `M…`.
    Marker,
    Timer,
    Counter,
}

/// How much an address covers: the size letter of `DBX`, `MB`, `IW`, `QD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum S7Width {
    Bit,
    Byte,
    Word,
    DWord,
}

impl S7Width {
    pub fn bytes(self) -> u16 {
        match self {
            Self::Bit | Self::Byte => 1,
            Self::Word => 2,
            Self::DWord => 4,
        }
    }

    fn letter(self) -> char {
        match self {
            Self::Bit => 'X',
            Self::Byte => 'B',
            Self::Word => 'W',
            Self::DWord => 'D',
        }
    }
}

/// An S7 address, e.g. `DB1.DBD0`, `DB1.DBX4.0`, `MW20`, `I0.3`, `QB2`,
/// `T5` or `C10`. Saved as that text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct S7Address {
    pub area: S7Area,
    /// The DB number; 0 outside data blocks.
    pub db: u16,
    /// Byte offset, or the timer or counter number.
    pub offset: u16,
    /// Bit 0 to 7 of a bit address; 0 otherwise.
    pub bit: u8,
    /// Timers and counters are one word each.
    pub width: S7Width,
}

impl S7Address {
    /// The data types this address can be read as, the usual one first.
    pub fn types(&self) -> &'static [S7Type] {
        match (self.area, self.width) {
            (S7Area::Timer, _) => &[S7Type::Timer],
            (S7Area::Counter, _) => &[S7Type::Counter],
            (_, S7Width::Bit) => &[S7Type::Bool],
            (_, S7Width::Byte) => &[S7Type::Byte, S7Type::String],
            (_, S7Width::Word) => &[S7Type::Int, S7Type::Word],
            (_, S7Width::DWord) => &[S7Type::Real, S7Type::DInt, S7Type::DWord],
        }
    }

    /// The bytes after `offset` that a value of this address reads, past
    /// what `u16` offsets can reach: `None` when it runs off the area.
    pub fn end(&self) -> Option<u32> {
        let end = u32::from(self.offset) + u32::from(self.width.bytes());
        (end <= 65_536).then_some(end)
    }
}

impl fmt::Display for S7Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bit = if self.width == S7Width::Bit {
            format!(".{}", self.bit)
        } else {
            String::new()
        };
        let letter = self.width.letter();
        match self.area {
            S7Area::Db => write!(f, "DB{}.DB{letter}{}{bit}", self.db, self.offset),
            S7Area::Timer => write!(f, "T{}", self.offset),
            S7Area::Counter => write!(f, "C{}", self.offset),
            area => {
                let prefix = match area {
                    S7Area::Input => 'I',
                    S7Area::Output => 'Q',
                    _ => 'M',
                };
                if self.width == S7Width::Bit {
                    write!(f, "{prefix}{}{bit}", self.offset)
                } else {
                    write!(f, "{prefix}{letter}{}", self.offset)
                }
            }
        }
    }
}

fn number<T: FromStr>(text: &str, what: &str, address: &str) -> Result<T, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("“{address}” needs a {what} number"));
    }
    text.parse()
        .map_err(|_| format!("The {what} in “{address}” is too large"))
}

fn bit(text: &str, address: &str) -> Result<u8, String> {
    match text.parse::<u8>() {
        Ok(bit) if bit <= 7 && text.len() == 1 => Ok(bit),
        _ => Err(format!("The bit in “{address}” must be 0 to 7")),
    }
}

/// Byte and bit after an area's letters: `4.0` for a bit, `20` otherwise.
fn offset_and_bit(rest: &str, width: S7Width, address: &str) -> Result<(u16, u8), String> {
    match (rest.split_once('.'), width) {
        (Some((byte, b)), S7Width::Bit) => Ok((number(byte, "byte", address)?, bit(b, address)?)),
        (None, S7Width::Bit) => Err(format!(
            "“{address}” is a bit address: add the bit, e.g. {address}.0"
        )),
        (Some(_), _) => Err(format!(
            "Only bit addresses take a .bit; “{address}” is a {} address",
            match width {
                S7Width::Byte => "byte",
                S7Width::Word => "word",
                _ => "double word",
            }
        )),
        (None, _) => Ok((number(rest, "byte", address)?, 0)),
    }
}

fn width_of(letter: Option<char>) -> Option<S7Width> {
    Some(match letter? {
        'X' => S7Width::Bit,
        'B' => S7Width::Byte,
        'W' => S7Width::Word,
        'D' => S7Width::DWord,
        _ => return None,
    })
}

impl FromStr for S7Address {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let upper = text.trim().trim_start_matches('%').to_ascii_uppercase();
        let address = upper.as_str();
        if address.is_empty() {
            return Err("Enter an address, e.g. DB1.DBD0, MW20 or I0.3".into());
        }
        if let Some(rest) = address.strip_prefix("DB") {
            let Some((db, item)) = rest.split_once('.') else {
                return Err(format!(
                    "“{address}” needs the item in the block, e.g. {address}.DBD0"
                ));
            };
            let db: u16 = number(db, "DB", address)?;
            if db == 0 {
                return Err("Data blocks are numbered from DB1".into());
            }
            let Some(item) = item.strip_prefix("DB") else {
                return Err(format!(
                    "Write the item as DBX, DBB, DBW or DBD, e.g. DB{db}.DBW0"
                ));
            };
            let Some(width) = width_of(item.chars().next()) else {
                return Err(format!(
                    "Write the item as DBX, DBB, DBW or DBD, e.g. DB{db}.DBW0"
                ));
            };
            let (offset, bit) = offset_and_bit(&item[1..], width, address)?;
            return Self::checked(Self {
                area: S7Area::Db,
                db,
                offset,
                bit,
                width,
            });
        }
        let mut chars = address.chars();
        let area = match chars.next() {
            Some('I' | 'E') => S7Area::Input,
            Some('Q' | 'A') => S7Area::Output,
            Some('M') => S7Area::Marker,
            Some('T') => S7Area::Timer,
            Some('C' | 'Z') => S7Area::Counter,
            _ => {
                return Err(format!(
                    "“{address}” is not an S7 address: use DB, I, Q, M, T or C, e.g. DB1.DBD0"
                ));
            }
        };
        let rest = chars.as_str();
        if matches!(area, S7Area::Timer | S7Area::Counter) {
            return Ok(Self {
                area,
                db: 0,
                offset: number(
                    rest,
                    if area == S7Area::Timer {
                        "timer"
                    } else {
                        "counter"
                    },
                    address,
                )?,
                bit: 0,
                width: S7Width::Word,
            });
        }
        let (width, rest) = match width_of(rest.chars().next()) {
            Some(width) => (width, &rest[1..]),
            None if rest.contains('.') => (S7Width::Bit, rest),
            None => {
                return Err(format!(
                    "Say how much of “{address}” to read: {address}.0 for a bit, or B, W, D for a byte, word or double word"
                ));
            }
        };
        let (offset, bit) = offset_and_bit(rest, width, address)?;
        Self::checked(Self {
            area,
            db: 0,
            offset,
            bit,
            width,
        })
    }
}

impl S7Address {
    fn checked(self) -> Result<Self, String> {
        match self.end() {
            Some(_) => Ok(self),
            None => Err(format!("{self} runs past byte 65535")),
        }
    }
}

impl TryFrom<String> for S7Address {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl From<S7Address> for String {
    fn from(address: S7Address) -> Self {
        address.to_string()
    }
}

/// How a tag's bytes are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7Type {
    Bool,
    Byte,
    /// Unsigned 16 bits.
    Word,
    /// Signed 16 bits.
    Int,
    /// Unsigned 32 bits.
    #[serde(rename = "dword")]
    DWord,
    /// Signed 32 bits.
    #[serde(rename = "dint")]
    DInt,
    Real,
    /// An S7 string at a byte address: maximum length, length, characters.
    String,
    /// S5 time, read in milliseconds.
    Timer,
    /// Three BCD digits, 0 to 999.
    Counter,
}

impl S7Type {
    pub const ALL: &'static [S7Type] = &[
        Self::Bool,
        Self::Byte,
        Self::Word,
        Self::Int,
        Self::DWord,
        Self::DInt,
        Self::Real,
        Self::String,
        Self::Timer,
        Self::Counter,
    ];

    /// The name TIA Portal gives it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Byte => "Byte",
            Self::Word => "Word",
            Self::Int => "Int",
            Self::DWord => "DWord",
            Self::DInt => "DInt",
            Self::Real => "Real",
            Self::String => "String",
            Self::Timer => "Timer",
            Self::Counter => "Counter",
        }
    }

    /// Whether the app writes it: timers and counters belong to the CPU,
    /// and text has no number to type.
    pub fn writable(self) -> bool {
        !matches!(self, Self::String | Self::Timer | Self::Counter)
    }

    /// `value` as this type would hold it, or why it cannot.
    pub fn check(self, value: f64) -> Result<(), String> {
        let whole = |min: f64, max: f64| {
            if value.fract() != 0.0 || !(min..=max).contains(&value) {
                Err(format!(
                    "A {} holds whole numbers from {min} to {max}",
                    self.name()
                ))
            } else {
                Ok(())
            }
        };
        match self {
            _ if !value.is_finite() => Err("Enter a number".into()),
            Self::Bool if value == 0.0 || value == 1.0 => Ok(()),
            Self::Bool => Err("Write 1 for ON, 0 for OFF".into()),
            Self::Byte => whole(0.0, 255.0),
            Self::Word => whole(0.0, 65_535.0),
            Self::Int => whole(-32_768.0, 32_767.0),
            Self::DWord => whole(0.0, 4_294_967_295.0),
            Self::DInt => whole(-2_147_483_648.0, 2_147_483_647.0),
            Self::Real if value.abs() <= f64::from(f32::MAX) => Ok(()),
            Self::Real => Err("Too large for a Real".into()),
            Self::String | Self::Timer | Self::Counter => {
                Err(format!("A {} is read-only here", self.name()))
            }
        }
    }
}

/// One value to read from the PLC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7TagSpec {
    pub name: String,
    pub address: S7Address,
    pub data_type: S7Type,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

/// Read and write a Siemens S7 CPU's tags (S7-300/400/1200/1500, LOGO!,
/// and S7-compatible PLCs) over ISO-on-TCP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7ClientSpec {
    pub host: String,
    pub port: u16,
    pub rack: u8,
    /// S7-300: 2; S7-1200/1500: 1 (or 0); S7-400: see the hardware configuration.
    pub slot: u8,
    /// PDU size to ask for.
    pub pdu_size: u16,
    pub poll_ms: u64,
    pub timeout_ms: u64,
    pub tags: Vec<S7TagSpec>,
}

impl Default for S7ClientSpec {
    fn default() -> Self {
        Self {
            host: "192.168.0.1".into(),
            port: S7_PORT,
            rack: 0,
            slot: 1,
            pdu_size: S7_DEFAULT_PDU,
            poll_ms: 1000,
            timeout_ms: 3000,
            tags: vec![S7TagSpec {
                name: "Value".into(),
                address: S7Address {
                    area: S7Area::Db,
                    db: 1,
                    offset: 0,
                    bit: 0,
                    width: S7Width::DWord,
                },
                data_type: S7Type::Real,
                unit: String::new(),
            }],
        }
    }
}

/// Play an S7 CPU on this computer: a template's data blocks, inputs,
/// outputs, markers, timers and counters, with simulated values, CPU
/// identity, diagnostic buffer and blocks for any S7 client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7ServerSpec {
    /// Address to listen on: `0.0.0.0` for every interface. Files saved
    /// before this field existed listen on loopback only.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// TCP port; S7 clients expect 102.
    pub port: u16,
    /// Built-in template id.
    pub template: String,
    /// Most PDU bytes the simulated CPU offers.
    pub pdu_size: u16,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

impl Default for S7ServerSpec {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: S7_PORT,
            template: "motor".into(),
            pdu_size: S7_DEFAULT_PDU,
        }
    }
}

/// What a CPU says about itself: identity, operating mode, diagnostic
/// buffer and blocks. Each part may fail on its own: an S7-1200 without
/// PUT/GET, for one, refuses some of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7PlcReport {
    /// The PDU size the CPU agreed to.
    pub pdu_size: u16,
    pub cpu: Result<S7CpuInfo, String>,
    /// `RUN`, `STOP`, `STARTUP`, `HOLD`, `DEFECT` or `UNKNOWN`.
    pub mode: Result<String, String>,
    /// Newest first.
    pub diagnostics: Result<Vec<S7DiagEntry>, String>,
    pub blocks: Result<Vec<S7Block>, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7CpuInfo {
    pub module_type: String,
    pub order_code: String,
    pub firmware: String,
    pub serial_number: String,
    /// The station (automation system) name.
    pub station: String,
    pub module_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7DiagEntry {
    /// `YYYY-MM-DD HH:MM:SS.mmm`, as the CPU logged it.
    pub timestamp: String,
    pub event_id: u16,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S7Block {
    /// `OB`, `FB`, `FC`, `DB`, `SDB`, `SFC` or `SFB`.
    pub kind: String,
    pub number: u16,
    /// Programming language, e.g. `LAD`, `STL`, `SCL`.
    pub language: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_listens_on_loopback_unless_told_otherwise() {
        assert_eq!(S7ServerSpec::default().bind, "127.0.0.1");
        let old: S7ServerSpec =
            serde_json::from_str(r#"{"port":102,"template":"motor","pdu_size":480}"#).unwrap();
        assert_eq!(old.bind, "127.0.0.1");
    }

    fn parse(text: &str) -> S7Address {
        text.parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn addresses_read_back_in_their_canonical_form() {
        for (text, canonical) in [
            ("DB1.DBD0", "DB1.DBD0"),
            ("db10.dbx4.7", "DB10.DBX4.7"),
            ("%DB2.DBW30", "DB2.DBW30"),
            (" DB65535.DBB65535 ", "DB65535.DBB65535"),
            ("MW20", "MW20"),
            ("%M10.3", "M10.3"),
            ("MX10.3", "M10.3"),
            ("IB0", "IB0"),
            ("I0.0", "I0.0"),
            ("E1.2", "I1.2"),
            ("QD4", "QD4"),
            ("A0.1", "Q0.1"),
            ("T5", "T5"),
            ("%C10", "C10"),
            ("Z3", "C3"),
        ] {
            assert_eq!(parse(text).to_string(), canonical, "{text}");
        }
        let bit = parse("DB3.DBX12.5");
        assert_eq!((bit.db, bit.offset, bit.bit), (3, 12, 5));
        assert_eq!(bit.width, S7Width::Bit);
    }

    #[test]
    fn bad_addresses_say_what_is_wrong() {
        for (text, hint) in [
            ("", "Enter an address"),
            ("DB0.DBW0", "from DB1"),
            ("DB1", "needs the item"),
            ("DB1.DBQ0", "DBX, DBB, DBW or DBD"),
            ("DB1.W0", "DBX, DBB, DBW or DBD"),
            ("DB1.DBX0", "add the bit"),
            ("DB1.DBX0.8", "0 to 7"),
            ("DB1.DBX0.10", "0 to 7"),
            ("DB1.DBW0.1", "Only bit addresses"),
            ("M10", "how much"),
            ("MW", "byte number"),
            ("MW-1", "byte number"),
            ("MW70000", "too large"),
            ("DB1.DBD65534", "runs past"),
            ("T", "timer number"),
            ("T5.0", "timer number"),
            ("X0", "not an S7 address"),
            ("DB70000.DBW0", "too large"),
        ] {
            let error = text.parse::<S7Address>().expect_err(text);
            assert!(error.contains(hint), "{text}: {error}");
        }
    }

    #[test]
    fn each_width_offers_the_types_that_fit_it() {
        assert_eq!(parse("M0.0").types(), [S7Type::Bool]);
        assert_eq!(parse("DB1.DBD0").types()[0], S7Type::Real);
        assert!(parse("MW2").types().contains(&S7Type::Int));
        assert!(!parse("MW2").types().contains(&S7Type::Real));
        assert_eq!(parse("T1").types(), [S7Type::Timer]);
        assert_eq!(parse("C1").types(), [S7Type::Counter]);
    }

    #[test]
    fn values_are_checked_against_the_type() {
        assert!(S7Type::Int.check(-32_768.0).is_ok());
        assert!(S7Type::Int.check(32_768.0).is_err());
        assert!(S7Type::Word.check(-1.0).is_err());
        assert!(S7Type::Byte.check(2.5).is_err());
        assert!(S7Type::DInt.check(-2_147_483_648.0).is_ok());
        assert!(S7Type::DWord.check(4_294_967_296.0).is_err());
        assert!(S7Type::Bool.check(1.0).is_ok());
        assert!(S7Type::Bool.check(2.0).is_err());
        assert!(S7Type::Real.check(-1.5e30).is_ok());
        assert!(S7Type::Real.check(1e39).is_err());
        assert!(S7Type::Real.check(f64::NAN).is_err());
        assert!(S7Type::Timer.check(10.0).is_err());
    }

    #[test]
    fn addresses_are_saved_as_text() {
        let tag = S7TagSpec {
            name: "Speed".into(),
            address: parse("DB1.DBD0"),
            data_type: S7Type::Real,
            unit: "rpm".into(),
        };
        let text = serde_json::to_string(&tag).unwrap();
        assert!(text.contains(r#""address":"DB1.DBD0""#), "{text}");
        assert_eq!(serde_json::from_str::<S7TagSpec>(&text).unwrap(), tag);
        let bad = r#"{"name":"x","address":"DB1.DBX0","data_type":"bool"}"#;
        assert!(serde_json::from_str::<S7TagSpec>(bad).is_err());
    }
}
