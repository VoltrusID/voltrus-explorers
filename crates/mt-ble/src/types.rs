use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub address: String,
    pub connected: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleAdvertisement {
    pub local_name: Option<String>,
    pub manufacturer_data: Option<std::collections::HashMap<u16, String>>,
    pub service_uuids: Vec<String>,
    pub tx_power_level: Option<i8>,
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
