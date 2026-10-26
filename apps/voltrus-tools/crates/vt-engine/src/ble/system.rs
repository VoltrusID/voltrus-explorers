//! This computer's Bluetooth radio, through `mt-ble`'s manager.

use mt_ble::{BleEvent, BleManager, BleService};
use tokio::sync::mpsc;
use vt_protocol::{BleDeviceInfo, BleManufacturerData, BleServiceData};

use super::central::{Central, Notification};
use super::decode;

/// Notifications in flight between the radio and the session.
const NOTIFY_QUEUE: usize = 256;

pub(crate) struct SystemCentral {
    manager: BleManager,
}

impl SystemCentral {
    /// Open the first Bluetooth adapter. Fails with a message the app can
    /// show when there is none or it is off.
    pub(crate) async fn open() -> Result<(Self, mpsc::Receiver<Notification>), String> {
        let (events, mut raw) = mpsc::channel::<BleEvent>(NOTIFY_QUEUE);
        let manager = BleManager::new(events);
        manager.init().await.map_err(|error| {
            format!(
                "{error}. Turn Bluetooth on and allow Voltrus Tools to use it in System Settings › Privacy & Security › Bluetooth."
            )
        })?;
        let (notify, notifications) = mpsc::channel(NOTIFY_QUEUE);
        tokio::spawn(async move {
            while let Some(event) = raw.recv().await {
                let BleEvent::CharacteristicNotify {
                    device_id,
                    char_uuid,
                    value,
                } = event
                else {
                    continue;
                };
                let Ok(data) = decode::parse_hex(&value) else {
                    continue;
                };
                let notification = Notification {
                    peripheral: device_id,
                    characteristic: char_uuid,
                    data,
                };
                if notify.send(notification).await.is_err() {
                    break;
                }
            }
        });
        Ok((Self { manager }, notifications))
    }
}

/// A device as `mt-ble` lists it, in the app's terms.
fn device_info(device: mt_ble::BleDevice) -> BleDeviceInfo {
    let advertisement = device.advertisement;
    let mut manufacturer: Vec<BleManufacturerData> = advertisement
        .manufacturer_data
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(company, data)| {
            Some(BleManufacturerData {
                company,
                data: decode::parse_hex(&data).ok()?,
            })
        })
        .collect();
    manufacturer.sort_by_key(|data| data.company);
    let mut service_data: Vec<BleServiceData> = advertisement
        .service_data
        .into_iter()
        .filter_map(|(service, data)| {
            Some(BleServiceData {
                service,
                data: decode::parse_hex(&data).ok()?,
            })
        })
        .collect();
    service_data.sort_by(|a, b| a.service.cmp(&b.service));
    BleDeviceInfo {
        id: device.id,
        name: device.name,
        address: device.address,
        // mt-ble reports -128 when the radio gave no signal strength.
        rssi: (device.rssi > -128).then_some(device.rssi),
        tx_power: advertisement.tx_power_level.map(i16::from),
        services: advertisement.service_uuids,
        manufacturer,
        service_data,
        connected: device.connected,
    }
}

impl Central for SystemCentral {
    async fn start_scan(&self) -> Result<(), String> {
        self.manager.start_scan().await
    }

    async fn stop_scan(&self) -> Result<(), String> {
        self.manager.stop_scan().await
    }

    async fn peripherals(&self) -> Vec<BleDeviceInfo> {
        self.manager
            .get_devices()
            .await
            .into_iter()
            .map(device_info)
            .collect()
    }

    async fn connect(&self, peripheral: &str) -> Result<(), String> {
        self.manager.connect(peripheral).await
    }

    async fn disconnect(&self, peripheral: &str) -> Result<(), String> {
        self.manager.disconnect(peripheral).await
    }

    async fn is_connected(&self, peripheral: &str) -> Result<bool, String> {
        self.manager.is_connected(peripheral).await
    }

    async fn services(&self, peripheral: &str) -> Result<Vec<BleService>, String> {
        self.manager.discover_services(peripheral).await
    }

    async fn read(&self, peripheral: &str, characteristic: &str) -> Result<Vec<u8>, String> {
        let hex = self
            .manager
            .read_characteristic(peripheral, characteristic)
            .await?;
        decode::parse_hex(&hex)
    }

    async fn write(
        &self,
        peripheral: &str,
        characteristic: &str,
        data: &[u8],
    ) -> Result<(), String> {
        let hex: String = data.iter().map(|byte| format!("{byte:02x}")).collect();
        self.manager
            .write_characteristic(peripheral, characteristic, &hex)
            .await
    }

    async fn subscribe(&self, peripheral: &str, characteristic: &str) -> Result<(), String> {
        self.manager
            .subscribe_characteristic(peripheral, characteristic)
            .await
    }

    async fn unsubscribe(&self, peripheral: &str, characteristic: &str) -> Result<(), String> {
        self.manager
            .unsubscribe_characteristic(peripheral, characteristic)
            .await
    }
}
