use std::fmt;

use serde::{Deserialize, Serialize};

/// The EtherNet/IP encapsulation port, TCP for sessions and UDP for
/// ListIdentity discovery (0xAF12).
pub const ENIP_PORT: u16 = 44818;

/// Read a Logix controller's tags by name and CIP objects' attributes
/// (assemblies and others), poll them and write them; send any explicit
/// message from the inspector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnipClientSpec {
    /// The device's IP address or host name.
    pub host: String,
    pub port: u16,
    /// CIP route to the controller, e.g. `1,0` (backplane port 1, slot 0)
    /// for a ControlLogix behind an Ethernet module; empty for devices that
    /// end CIP at their Ethernet port (CompactLogix, Micro800, adapters).
    #[serde(default)]
    pub route_path: String,
    pub poll_ms: u64,
    /// The points to read. Empty reads every tag the controller lists.
    #[serde(default)]
    pub points: Vec<EnipPoint>,
}

impl Default for EnipClientSpec {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: ENIP_PORT,
            route_path: String::new(),
            poll_ms: 1000,
            points: Vec::new(),
        }
    }
}

/// One value to read from an EtherNet/IP device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnipPoint {
    /// What the app calls it; empty shows the address.
    #[serde(default)]
    pub name: String,
    pub address: EnipAddress,
    #[serde(default)]
    pub unit: String,
}

impl EnipPoint {
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            self.address.to_string()
        } else {
            self.name.clone()
        }
    }
}

/// Where a point's value lives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnipAddress {
    /// A Logix tag by name, read with Read Tag; its type comes from the
    /// device: `Speed`, `Program:Main.Motor.Speed`, `Recipe[3]`, `Status.5`.
    Tag { tag: String },
    /// Bytes of a CIP attribute read with Get_Attribute_Single and written
    /// back with Set_Attribute_Single: an assembly's data (class 0x04,
    /// attribute 3) or any object's attribute, as `data_type` at `offset`.
    Attribute {
        class: u16,
        instance: u32,
        attribute: u16,
        offset: u16,
        data_type: CipType,
    },
}

impl EnipAddress {
    /// Parse what an engineer types: a tag name, or
    /// `@class/instance/attribute[+offset][:TYPE]` for an attribute, with
    /// numbers in decimal or `0x` hex, e.g. `@0x04/100/3+4:REAL`. An attribute
    /// without a type is read as a DINT.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix('@') {
            return parse_attribute(rest);
        }
        valid_tag(text)?;
        Ok(Self::Tag {
            tag: text.to_owned(),
        })
    }
}

impl fmt::Display for EnipAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag { tag } => f.write_str(tag),
            Self::Attribute {
                class,
                instance,
                attribute,
                offset,
                data_type,
            } => {
                write!(f, "@0x{class:02X}/{instance}/{attribute}")?;
                if *offset != 0 {
                    write!(f, "+{offset}")?;
                }
                write!(f, ":{}", data_type.name())
            }
        }
    }
}

fn parse_number<T: TryFrom<u64>>(text: &str, what: &str) -> Result<T, String> {
    let text = text.trim();
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse::<u64>(),
    };
    parsed
        .ok()
        .and_then(|value| T::try_from(value).ok())
        .ok_or_else(|| format!("{what} “{text}” is not a number in range"))
}

fn parse_attribute(rest: &str) -> Result<EnipAddress, String> {
    let (path, data_type) = match rest.split_once(':') {
        Some((path, name)) => (
            path,
            CipType::parse(name).ok_or_else(|| {
                format!(
                    "“{}” is not a CIP type: use one of {}",
                    name.trim(),
                    CipType::ALL
                        .iter()
                        .map(|t| t.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
        ),
        None => (rest, CipType::Dint),
    };
    let (path, offset) = match path.split_once('+') {
        Some((path, offset)) => (path, parse_number::<u16>(offset, "Offset")?),
        None => (path, 0),
    };
    let parts: Vec<&str> = path.split('/').collect();
    let [class, instance, attribute] = parts.as_slice() else {
        return Err("Write an attribute as @class/instance/attribute, e.g. @0x04/100/3".into());
    };
    Ok(EnipAddress::Attribute {
        class: parse_number(class, "Class")?,
        instance: parse_number(instance, "Instance")?,
        attribute: parse_number(attribute, "Attribute")?,
        offset,
        data_type,
    })
}

/// The rules a Logix tag name follows: letters, digits and `_`, members after
/// `.`, array indices in brackets, a `Program:` scope, at most 255
/// characters and not starting with a digit.
fn valid_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err(
            "Enter a tag name, e.g. Program:Main.Speed, or @class/instance/attribute".into(),
        );
    }
    if tag.len() > 255 {
        return Err("A tag name has at most 255 characters".into());
    }
    if tag.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("A tag name cannot start with a digit".into());
    }
    if let Some(bad) = tag
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.' | '[' | ']' | ',' | ':'))
    {
        return Err(format!("“{bad}” cannot be part of a tag name"));
    }
    let mut depth = 0i32;
    for c in tag.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
        if !(0..=1).contains(&depth) {
            return Err("Brackets in the tag name do not match".into());
        }
    }
    if depth != 0 || tag.contains("..") || tag.ends_with('.') {
        return Err("The tag name is incomplete".into());
    }
    Ok(())
}

/// CIP elementary types an attribute's bytes can be read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CipType {
    Bool,
    Sint,
    Int,
    Dint,
    Lint,
    Usint,
    Uint,
    Udint,
    Ulint,
    Real,
    Lreal,
}

impl CipType {
    pub const ALL: [CipType; 11] = [
        Self::Bool,
        Self::Sint,
        Self::Int,
        Self::Dint,
        Self::Lint,
        Self::Usint,
        Self::Uint,
        Self::Udint,
        Self::Ulint,
        Self::Real,
        Self::Lreal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Sint => "SINT",
            Self::Int => "INT",
            Self::Dint => "DINT",
            Self::Lint => "LINT",
            Self::Usint => "USINT",
            Self::Uint => "UINT",
            Self::Udint => "UDINT",
            Self::Ulint => "ULINT",
            Self::Real => "REAL",
            Self::Lreal => "LREAL",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(name))
    }

    /// Bytes one value takes.
    pub fn size(self) -> usize {
        match self {
            Self::Bool | Self::Sint | Self::Usint => 1,
            Self::Int | Self::Uint => 2,
            Self::Dint | Self::Udint | Self::Real => 4,
            Self::Lint | Self::Ulint | Self::Lreal => 8,
        }
    }
}

/// Play an EtherNet/IP adapter on this computer: a Logix-style target whose
/// tags a plant template drives, with its assemblies and ListIdentity answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnipAdapterSpec {
    /// TCP port of the encapsulation session, on every interface; the
    /// adapter answers ListIdentity on the same UDP port.
    pub port: u16,
    /// Built-in plant template id.
    pub template: String,
}

impl Default for EnipAdapterSpec {
    fn default() -> Self {
        Self {
            port: ENIP_PORT,
            template: "drive".into(),
        }
    }
}

/// ListIdentity: find the EtherNet/IP devices that answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnipDiscoverSpec {
    /// Broadcast on this UDP port on every interface.
    pub broadcast_port: Option<u16>,
    /// Addresses (`host:port`) that also get a request of their own: devices
    /// on other ports, behind routers, or on this computer.
    pub directed: Vec<String>,
    /// How long to collect answers.
    pub wait_ms: u64,
}

/// A device that answered ListIdentity, once per identity, at the address to
/// use (one the request was sent to, else loopback, else the first).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnipDeviceInfo {
    pub address: String,
    /// The TCP port the device advertises for sessions.
    pub port: u16,
    pub product: String,
    pub vendor: String,
    pub device_type: String,
    /// `major.minor`.
    pub revision: String,
    pub serial: u32,
    /// Other addresses the same device answered from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}

/// An explicit message: one CIP service to an object of the connected
/// device, sent unconnected (routed when the session has a route path).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CipRequest {
    pub service: u8,
    pub class: u16,
    pub instance: u32,
    pub attribute: Option<u16>,
    /// Request data after the path, as sent.
    #[serde(default)]
    pub data: Vec<u8>,
}

/// The device's answer to a [`CipRequest`] that succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CipResponse {
    pub request: CipRequest,
    /// Reply data after the status.
    pub data: Vec<u8>,
    pub latency_us: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_and_print_back() {
        for text in [
            "Speed",
            "Program:Main.Motor1.Speed",
            "Recipe[3]",
            "Grid[1,2].Value",
            "Status.5",
            "@0x04/100/3:REAL",
            "@0x6C/1/2+12:UINT",
        ] {
            let address = EnipAddress::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(address.to_string(), text);
            assert_eq!(EnipAddress::parse(&address.to_string()), Ok(address));
        }
        assert_eq!(
            EnipAddress::parse(" @4/0x64/3+8:lreal "),
            Ok(EnipAddress::Attribute {
                class: 4,
                instance: 100,
                attribute: 3,
                offset: 8,
                data_type: CipType::Lreal,
            })
        );
        // No type reads a DINT.
        assert!(matches!(
            EnipAddress::parse("@1/1/1"),
            Ok(EnipAddress::Attribute {
                data_type: CipType::Dint,
                ..
            })
        ));
    }

    #[test]
    fn bad_addresses_say_why() {
        for (text, problem) in [
            ("", "Enter a tag name"),
            ("3Speed", "cannot start with a digit"),
            ("Motor Speed", "“ ” cannot be part"),
            ("Recipe[3", "incomplete"),
            ("Recipe]3[", "do not match"),
            ("Motor.", "incomplete"),
            ("@4/100", "@class/instance/attribute"),
            ("@4/100/3:FLOAT", "not a CIP type"),
            ("@0x10000/1/1", "Class"),
            ("@4/x/3", "Instance"),
            ("@4/100/3+70000", "Offset"),
        ] {
            let error = EnipAddress::parse(text).expect_err(text);
            assert!(error.contains(problem), "{text}: {error}");
        }
    }

    #[test]
    fn types_have_their_wire_sizes() {
        let sizes: Vec<usize> = CipType::ALL.iter().map(|t| t.size()).collect();
        assert_eq!(sizes, [1, 1, 2, 4, 8, 1, 2, 4, 8, 4, 8]);
        assert_eq!(CipType::parse("real"), Some(CipType::Real));
        assert_eq!(CipType::parse("STRING"), None);
    }
}
