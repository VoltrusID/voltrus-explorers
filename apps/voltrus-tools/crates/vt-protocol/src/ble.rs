use serde::{Deserialize, Serialize};

/// Which central a BLE session or scan uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BleAdapter {
    /// This computer's Bluetooth radio.
    #[default]
    System,
    /// Peripherals simulated in the engine (a heart rate monitor, an
    /// environment sensor): the whole workflow without hardware.
    Simulated,
}

impl BleAdapter {
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "Bluetooth",
            Self::Simulated => "Simulated",
        }
    }
}

/// Connect to one BLE peripheral, browse its GATT services and turn every
/// characteristic into a point: read, polled, or pushed by notifications.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BleClientSpec {
    #[serde(default)]
    pub adapter: BleAdapter,
    /// The peripheral's identifier as a scan reports it (a UUID on macOS,
    /// the MAC address elsewhere).
    pub peripheral: String,
    /// Its advertised name: shown, and used to find it again when the
    /// identifier no longer matches.
    #[serde(default)]
    pub name: String,
    /// Re-read readable characteristics that don't notify this often; 0 reads
    /// them once after connecting.
    pub poll_ms: u64,
    /// Subscribe to every characteristic that notifies or indicates.
    pub notify: bool,
    /// How to decode characteristics that should not use the automatic decoder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decoders: Vec<BleDecoderRule>,
}

impl Default for BleClientSpec {
    fn default() -> Self {
        Self {
            adapter: BleAdapter::System,
            peripheral: String::new(),
            name: String::new(),
            poll_ms: 2000,
            notify: true,
            decoders: Vec::new(),
        }
    }
}

impl BleClientSpec {
    /// The decoder chosen for `characteristic` (a full UUID), if any.
    pub fn decoder(&self, characteristic: &str) -> Option<BleDecoder> {
        self.decoders
            .iter()
            .find(|rule| rule.characteristic.eq_ignore_ascii_case(characteristic))
            .map(|rule| rule.decoder)
    }

    /// Use `decoder` for `characteristic`; the automatic decoder drops the rule.
    pub fn set_decoder(&mut self, characteristic: &str, decoder: BleDecoder) {
        self.decoders
            .retain(|rule| !rule.characteristic.eq_ignore_ascii_case(characteristic));
        if decoder != BleDecoder::default() {
            self.decoders.push(BleDecoderRule {
                characteristic: characteristic.to_ascii_lowercase(),
                decoder,
            });
        }
    }
}

/// The decoder of one characteristic, by its UUID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BleDecoderRule {
    pub characteristic: String,
    pub decoder: BleDecoder,
}

/// How a characteristic's bytes become a value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BleDecoder {
    pub format: BleFormat,
    /// Byte the number starts at (numeric formats).
    #[serde(default)]
    pub offset: u16,
    /// Multiplies the number read (0.01 for hundredths of a degree).
    #[serde(default = "unit_scale")]
    pub scale: f64,
}

fn unit_scale() -> f64 {
    1.0
}

impl Default for BleDecoder {
    fn default() -> Self {
        Self::of(BleFormat::Auto)
    }
}

impl BleDecoder {
    pub const fn of(format: BleFormat) -> Self {
        Self {
            format,
            offset: 0,
            scale: 1.0,
        }
    }
}

/// The layouts a characteristic value can be read as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BleFormat {
    /// The standard layout of a Bluetooth SIG characteristic (heart rate,
    /// battery, temperature…), else text when printable, else bytes.
    #[default]
    Auto,
    U8,
    I8,
    U16Le,
    U16Be,
    I16Le,
    I16Be,
    U32Le,
    U32Be,
    I32Le,
    I32Be,
    F32Le,
    F32Be,
    F64Le,
    F64Be,
    Utf8,
    Hex,
}

impl BleFormat {
    pub const ALL: [BleFormat; 17] = [
        Self::Auto,
        Self::U8,
        Self::I8,
        Self::U16Le,
        Self::U16Be,
        Self::I16Le,
        Self::I16Be,
        Self::U32Le,
        Self::U32Be,
        Self::I32Le,
        Self::I32Be,
        Self::F32Le,
        Self::F32Be,
        Self::F64Le,
        Self::F64Be,
        Self::Utf8,
        Self::Hex,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::U8 => "uint8",
            Self::I8 => "int8",
            Self::U16Le => "uint16 LE",
            Self::U16Be => "uint16 BE",
            Self::I16Le => "int16 LE",
            Self::I16Be => "int16 BE",
            Self::U32Le => "uint32 LE",
            Self::U32Be => "uint32 BE",
            Self::I32Le => "int32 LE",
            Self::I32Be => "int32 BE",
            Self::F32Le => "float32 LE",
            Self::F32Be => "float32 BE",
            Self::F64Le => "float64 LE",
            Self::F64Be => "float64 BE",
            Self::Utf8 => "UTF-8",
            Self::Hex => "Hex",
        }
    }

    /// Bytes a numeric format reads; `None` for text, bytes and automatic.
    pub fn size(self) -> Option<usize> {
        match self {
            Self::U8 | Self::I8 => Some(1),
            Self::U16Le | Self::U16Be | Self::I16Le | Self::I16Be => Some(2),
            Self::U32Le | Self::U32Be | Self::I32Le | Self::I32Be | Self::F32Le | Self::F32Be => {
                Some(4)
            }
            Self::F64Le | Self::F64Be => Some(8),
            Self::Auto | Self::Utf8 | Self::Hex => None,
        }
    }
}

/// Look for advertising peripherals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleScanSpec {
    pub adapter: BleAdapter,
    /// How long to scan; results arrive along the way.
    pub duration_ms: u64,
}

/// What a scan has seen so far; the last answer of a scan is `done`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BleScan {
    pub devices: Vec<BleDeviceInfo>,
    pub done: bool,
}

/// A peripheral a scan saw, with its latest advertisement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleDeviceInfo {
    /// What a session connects by ([`BleClientSpec::peripheral`]).
    pub id: String,
    /// Advertised local name; empty when it advertises none.
    pub name: String,
    pub address: String,
    /// Signal strength of the last advertisement, in dBm.
    pub rssi: Option<i16>,
    /// Advertised transmit power, in dBm.
    pub tx_power: Option<i16>,
    /// Advertised service UUIDs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manufacturer: Vec<BleManufacturerData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_data: Vec<BleServiceData>,
    pub connected: bool,
}

/// Manufacturer-specific advertisement data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleManufacturerData {
    /// Bluetooth SIG company identifier.
    pub company: u16,
    pub data: Vec<u8>,
}

/// Service data carried in an advertisement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleServiceData {
    pub service: String,
    pub data: Vec<u8>,
}

/// Work on one characteristic (a point) of a running BLE session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum BleRequest {
    /// Read it now.
    Read { point: u32 },
    /// Write these bytes (answered by `Written`).
    Write { point: u32, data: Vec<u8> },
    /// Subscribe to its notifications or indications, or stop.
    Notify { point: u32, enabled: bool },
    /// Decode it differently from now on.
    Decode { point: u32, decoder: BleDecoder },
}

/// The GATT side of one point: where the characteristic lives and what it
/// supports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BleCharacteristicInfo {
    pub point: u32,
    /// Service UUID.
    pub service: String,
    /// Characteristic UUID.
    pub uuid: String,
    pub properties: BleProperties,
    /// Notifications or indications are on.
    pub notifying: bool,
    /// The decoder in use ([`BleFormat::Auto`] unless one was chosen).
    pub decoder: BleDecoder,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleProperties {
    pub read: bool,
    pub write: bool,
    pub write_without_response: bool,
    pub notify: bool,
    pub indicate: bool,
}

impl BleProperties {
    pub fn writable(self) -> bool {
        self.write || self.write_without_response
    }

    pub fn pushes(self) -> bool {
        self.notify || self.indicate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_formats_know_their_size() {
        for format in BleFormat::ALL {
            let numeric = !matches!(format, BleFormat::Auto | BleFormat::Utf8 | BleFormat::Hex);
            assert_eq!(format.size().is_some(), numeric, "{format:?}");
        }
        assert_eq!(BleFormat::I16Be.size(), Some(2));
        assert_eq!(BleFormat::F32Le.size(), Some(4));
        assert_eq!(BleFormat::F64Be.size(), Some(8));
        let labels: std::collections::HashSet<_> =
            BleFormat::ALL.iter().map(|format| format.label()).collect();
        assert_eq!(labels.len(), BleFormat::ALL.len(), "labels are distinct");
    }

    #[test]
    fn a_decoder_rule_per_characteristic_and_none_for_auto() {
        let mut spec = BleClientSpec::default();
        let uuid = "00002A6E-0000-1000-8000-00805F9B34FB";
        spec.set_decoder(
            uuid,
            BleDecoder {
                format: BleFormat::I16Le,
                offset: 0,
                scale: 0.01,
            },
        );
        assert_eq!(spec.decoders.len(), 1);
        // UUIDs match whatever their case.
        assert_eq!(
            spec.decoder(&uuid.to_ascii_lowercase()).map(|d| d.scale),
            Some(0.01)
        );
        spec.set_decoder(uuid, BleDecoder::of(BleFormat::Hex));
        assert_eq!(spec.decoders.len(), 1, "replaced, not added");
        assert_eq!(spec.decoder(uuid).map(|d| d.format), Some(BleFormat::Hex));
        spec.set_decoder(uuid, BleDecoder::default());
        assert!(spec.decoders.is_empty(), "Auto needs no rule");
        assert_eq!(spec.decoder(uuid), None);
    }

    #[test]
    fn saved_specs_may_omit_optional_fields() {
        let saved = r#"{"peripheral":"AA:BB","poll_ms":0,"notify":false,
            "decoders":[{"characteristic":"x","decoder":{"format":"u16_be"}}]}"#;
        let spec: BleClientSpec = serde_json::from_str(saved).unwrap();
        assert_eq!(spec.adapter, BleAdapter::System);
        assert_eq!(spec.name, "");
        assert_eq!(
            spec.decoder("X"),
            Some(BleDecoder {
                format: BleFormat::U16Be,
                offset: 0,
                scale: 1.0
            })
        );
    }
}
