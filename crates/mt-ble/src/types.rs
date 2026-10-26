use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub address: String,
    pub connected: bool,
    /// What the device last advertised: manufacturer and service data, the
    /// services it offers and its transmit power.
    #[serde(default)]
    pub advertisement: BleAdvertisement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleService {
    pub uuid: String,
    pub is_primary: bool,
    pub characteristics: Vec<BleCharacteristic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleCharacteristic {
    pub uuid: String,
    pub properties: BleCharProperties,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleCharProperties {
    pub read: bool,
    pub write: bool,
    pub write_without_response: bool,
    pub notify: bool,
    pub indicate: bool,
}

impl Default for BleCharProperties {
    fn default() -> Self {
        Self {
            read: false,
            write: false,
            write_without_response: false,
            notify: false,
            indicate: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BleAdvertisement {
    pub local_name: Option<String>,
    /// Manufacturer data as hex, keyed by Bluetooth SIG company identifier.
    pub manufacturer_data: Option<HashMap<u16, String>>,
    pub service_uuids: Vec<String>,
    pub tx_power_level: Option<i8>,
    /// Service data as hex, keyed by service UUID.
    #[serde(default)]
    pub service_data: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BleEvent {
    ScanStarted,
    ScanStopped,
    DeviceFound { device: BleDevice },
    DeviceUpdated { device: BleDevice },
    DeviceConnected { id: String },
    DeviceDisconnected { id: String },
    ServicesDiscovered { device_id: String, services: Vec<BleService> },
    CharacteristicRead { device_id: String, char_uuid: String, value: String },
    CharacteristicWritten { device_id: String, char_uuid: String },
    CharacteristicNotify { device_id: String, char_uuid: String, value: String },
    Error { message: String },
}
