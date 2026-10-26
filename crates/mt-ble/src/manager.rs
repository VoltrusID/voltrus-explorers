use crate::types::*;
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub struct BleManager {
    adapter: Arc<Mutex<Option<Adapter>>>,
    peripherals: Arc<Mutex<HashMap<String, Peripheral>>>,
    characteristic_map: Arc<Mutex<HashMap<String, Characteristic>>>,
    scanning: Arc<Mutex<bool>>,
    event_tx: mpsc::Sender<BleEvent>,
    notify_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl BleManager {
    pub fn new(event_tx: mpsc::Sender<BleEvent>) -> Self {
        Self {
            adapter: Arc::new(Mutex::new(None)),
            peripherals: Arc::new(Mutex::new(HashMap::new())),
            characteristic_map: Arc::new(Mutex::new(HashMap::new())),
            scanning: Arc::new(Mutex::new(false)),
            event_tx,
            notify_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn init(&self) -> Result<(), String> {
        let manager = Manager::new()
            .await
            .map_err(|e| format!("Failed to create BLE manager: {}", e))?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| format!("Failed to get adapters: {}", e))?;
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or("No BLE adapter found. Is Bluetooth enabled?")?;
        *self.adapter.lock().await = Some(adapter);
        Ok(())
    }

    pub async fn start_scan(&self) -> Result<(), String> {
        let adapter_guard = self.adapter.lock().await;
        let adapter = adapter_guard
            .as_ref()
            .ok_or("BLE adapter not initialized")?;

        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| format!("Failed to start scan: {}", e))?;

        *self.scanning.lock().await = true;
        Ok(())
    }

    pub async fn stop_scan(&self) -> Result<(), String> {
        let adapter_guard = self.adapter.lock().await;
        let adapter = adapter_guard
            .as_ref()
            .ok_or("BLE adapter not initialized")?;

        adapter
            .stop_scan()
            .await
            .map_err(|e| format!("Failed to stop scan: {}", e))?;

        *self.scanning.lock().await = false;
        Ok(())
    }

    pub async fn is_scanning(&self) -> bool {
        *self.scanning.lock().await
    }

    pub async fn get_devices(&self) -> Vec<BleDevice> {
        let adapter_guard = self.adapter.lock().await;
        let adapter = match adapter_guard.as_ref() {
            Some(a) => a,
            None => return Vec::new(),
        };

        let discovered = match adapter.peripherals().await {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        let mut cache = self.peripherals.lock().await;
        let mut devices = Vec::new();

        for peripheral in &discovered {
            let id = format!("{}", peripheral.id());
            let props = match peripheral.properties().await {
                Ok(Some(p)) => p,
                _ => {
                    cache.insert(id.clone(), peripheral.clone());
                    continue;
                }
            };

            let advertisement = advertisement_of(&props);
            let name = props.local_name.unwrap_or_default();
            let rssi = props.rssi.unwrap_or(-128);
            let address = format!("{}", peripheral.address());

            let is_connected = match peripheral.is_connected().await {
                Ok(c) => c,
                Err(_) => false,
            };

            devices.push(BleDevice {
                id: id.clone(),
                name,
                rssi,
                address,
                connected: is_connected,
                advertisement,
            });

            cache.insert(id, peripheral.clone());
        }

        devices
    }

    pub async fn connect(&self, id: &str) -> Result<(), String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", id))?
        };

        peripheral
            .connect()
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;

        Ok(())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        // Stop notification listener for this device
        let task = {
            let mut tasks = self.notify_tasks.lock().await;
            tasks.remove(id)
        };
        if let Some(handle) = task {
            handle.abort();
        }

        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", id))?
        };

        peripheral
            .disconnect()
            .await
            .map_err(|e| format!("Failed to disconnect: {}", e))?;

        let mut char_map = self.characteristic_map.lock().await;
        char_map.retain(|k, _| !k.starts_with(id));

        Ok(())
    }

    /// Whether the device is connected now. A device no scan has seen is an
    /// error.
    pub async fn is_connected(&self, id: &str) -> Result<bool, String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", id))?
        };
        peripheral
            .is_connected()
            .await
            .map_err(|e| format!("Failed to query the connection: {}", e))
    }

    pub async fn discover_services(&self, id: &str) -> Result<Vec<BleService>, String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", id))?
        };

        peripheral
            .discover_services()
            .await
            .map_err(|e| format!("Failed to discover services: {}", e))?;

        let services = peripheral.services();
        let mut char_map = self.characteristic_map.lock().await;
        let mut result = Vec::new();

        for service in services {
            let mut characteristics = Vec::new();
            for ch in &service.characteristics {
                let props = &ch.properties;

                let char_key = format!("{}:{}", id, ch.uuid);
                char_map.insert(char_key, ch.clone());

                characteristics.push(BleCharacteristic {
                    uuid: format!("{}", ch.uuid),
                    properties: BleCharProperties {
                        read: props.contains(CharPropFlags::READ),
                        write: props.contains(CharPropFlags::WRITE),
                        write_without_response: props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE),
                        notify: props.contains(CharPropFlags::NOTIFY),
                        indicate: props.contains(CharPropFlags::INDICATE),
                    },
                    value: None,
                });
            }

            result.push(BleService {
                uuid: format!("{}", service.uuid),
                is_primary: service.primary,
                characteristics,
            });
        }

        Ok(result)
    }

    pub async fn read_characteristic(
        &self,
        device_id: &str,
        char_uuid: &str,
    ) -> Result<String, String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(device_id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", device_id))?
        };

        let ch = self.find_characteristic(device_id, char_uuid).await?;

        let data = peripheral
            .read(&ch)
            .await
            .map_err(|e| format!("Failed to read: {}", e))?;

        let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
        Ok(hex)
    }

    pub async fn write_characteristic(
        &self,
        device_id: &str,
        char_uuid: &str,
        value: &str,
    ) -> Result<(), String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(device_id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", device_id))?
        };

        let ch = self.find_characteristic(device_id, char_uuid).await?;
        let data = hex_to_bytes(value)?;

        let write_type = if ch.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };

        peripheral
            .write(&ch, &data, write_type)
            .await
            .map_err(|e| format!("Failed to write: {}", e))?;

        Ok(())
    }

    pub async fn subscribe_characteristic(
        &self,
        device_id: &str,
        char_uuid: &str,
    ) -> Result<(), String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(device_id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", device_id))?
        };

        let ch = self.find_characteristic(device_id, char_uuid).await?;

        peripheral
            .subscribe(&ch)
            .await
            .map_err(|e| format!("Failed to subscribe: {}", e))?;

        // Start notification listener if not already running
        let mut tasks = self.notify_tasks.lock().await;
        if !tasks.contains_key(device_id) {
            let peripheral_clone = peripheral.clone();
            let device_id_owned = device_id.to_string();
            let tx = self.event_tx.clone();

            let handle = tokio::spawn(async move {
                match peripheral_clone.notifications().await {
                    Ok(mut stream) => {
                        while let Some(notification) = stream.next().await {
                            let hex: String = notification
                                .value
                                .iter()
                                .map(|b| format!("{:02x}", b))
                                .collect();
                            let event = BleEvent::CharacteristicNotify {
                                device_id: device_id_owned.clone(),
                                char_uuid: format!("{}", notification.uuid),
                                value: hex,
                            };
                            if tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to get notification stream: {}", e);
                    }
                }
            });

            tasks.insert(device_id.to_string(), handle);
        }

        Ok(())
    }

    pub async fn unsubscribe_characteristic(
        &self,
        device_id: &str,
        char_uuid: &str,
    ) -> Result<(), String> {
        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(device_id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", device_id))?
        };

        let ch = self.find_characteristic(device_id, char_uuid).await?;

        peripheral
            .unsubscribe(&ch)
            .await
            .map_err(|e| format!("Failed to unsubscribe: {}", e))
    }

    async fn find_characteristic(
        &self,
        device_id: &str,
        char_uuid: &str,
    ) -> Result<Characteristic, String> {
        let char_key = format!("{}:{}", device_id, char_uuid);
        {
            let char_map = self.characteristic_map.lock().await;
            if let Some(ch) = char_map.get(&char_key) {
                return Ok(ch.clone());
            }
        }

        let uuid = Uuid::parse_str(char_uuid)
            .map_err(|e| format!("Invalid UUID: {}", e))?;

        let peripheral = {
            let peripherals = self.peripherals.lock().await;
            peripherals
                .get(device_id)
                .cloned()
                .ok_or_else(|| format!("Device not found: {}", device_id))?
        };

        peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == uuid)
            .ok_or_else(|| format!("Characteristic not found: {}", char_uuid))
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn advertisement_of(props: &btleplug::api::PeripheralProperties) -> BleAdvertisement {
    BleAdvertisement {
        local_name: props.local_name.clone(),
        manufacturer_data: (!props.manufacturer_data.is_empty()).then(|| {
            props
                .manufacturer_data
                .iter()
                .map(|(company, data)| (*company, to_hex(data)))
                .collect()
        }),
        service_uuids: props.services.iter().map(|uuid| uuid.to_string()).collect(),
        tx_power_level: props
            .tx_power_level
            .map(|level| level.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8),
        service_data: props
            .service_data
            .iter()
            .map(|(uuid, data)| (uuid.to_string(), to_hex(data)))
            .collect(),
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let hex: String = hex.trim().replace("0x", "").replace(" ", "");
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    if hex.len() % 2 != 0 {
        return Err("Hex string must have even length".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use btleplug::api::PeripheralProperties;

    #[test]
    fn advertisements_carry_their_data_as_hex() {
        let heart_rate = Uuid::from_u128(0x0000180d_0000_1000_8000_00805f9b34fb);
        let eddystone = Uuid::from_u128(0x0000feaa_0000_1000_8000_00805f9b34fb);
        let props = PeripheralProperties {
            local_name: Some("HRM".into()),
            tx_power_level: Some(-300),
            manufacturer_data: HashMap::from([(0x0059, vec![0x01, 0xab])]),
            service_data: HashMap::from([(eddystone, vec![0x00, 0xee])]),
            services: vec![heart_rate],
            ..PeripheralProperties::default()
        };
        let advertisement = advertisement_of(&props);
        assert_eq!(advertisement.local_name.as_deref(), Some("HRM"));
        assert_eq!(
            advertisement.manufacturer_data,
            Some(HashMap::from([(0x0059, "01ab".to_string())]))
        );
        assert_eq!(
            advertisement.service_data.get(&eddystone.to_string()).map(String::as_str),
            Some("00ee")
        );
        assert_eq!(advertisement.service_uuids, vec![heart_rate.to_string()]);
        // Out of the i8 range: clamped, not wrapped.
        assert_eq!(advertisement.tx_power_level, Some(i8::MIN));

        let silent = advertisement_of(&PeripheralProperties::default());
        assert_eq!(silent.manufacturer_data, None);
        assert!(silent.service_data.is_empty() && silent.service_uuids.is_empty());
    }

    #[test]
    fn devices_saved_before_advertisements_still_load() {
        let saved = r#"{"id":"a","name":"","rssi":-60,"address":"a","connected":false}"#;
        let device: BleDevice = serde_json::from_str(saved).unwrap();
        assert!(device.advertisement.service_uuids.is_empty());
    }
}
