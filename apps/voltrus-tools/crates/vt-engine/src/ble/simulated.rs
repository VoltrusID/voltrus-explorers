//! Simulated peripherals behind the [`Central`] trait: a heart rate monitor,
//! an environment sensor with a writable setpoint, and a beacon that takes
//! no connections. The demo and the tests run the real session against them.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mt_ble::{BleCharProperties, BleCharacteristic, BleService};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use vt_protocol::{BleDeviceInfo, BleManufacturerData, BleServiceData};

use super::central::{Central, Notification};
use super::gatt::uuid16;

/// How often subscriptions are checked for a value due.
const NOTIFY_TICK: Duration = Duration::from_millis(100);
/// A connection takes a moment, as over the air.
const CONNECT_DELAY: Duration = Duration::from_millis(120);
const NOTIFY_QUEUE: usize = 256;
/// Nordic Semiconductor's company identifier, as on many dev boards.
const NORDIC: u16 = 0x0059;

pub(crate) const HRM_ID: &str = "6F1D2C3B-5A49-4E8F-9C1B-2D3E4F5A6B01";
pub(crate) const HRM_NAME: &str = "Voltrus HRM";
pub(crate) const ENV_ID: &str = "6F1D2C3B-5A49-4E8F-9C1B-2D3E4F5A6B02";
pub(crate) const ENV_NAME: &str = "Voltrus Env Sensor";
const BEACON_ID: &str = "6F1D2C3B-5A49-4E8F-9C1B-2D3E4F5A6B03";

/// The control service of the simulated environment sensor and its
/// float32 setpoint.
pub(crate) const CONTROL_SERVICE: &str = "7d2e0001-6c1a-4b7e-9e4f-3a5c8d1b0e21";
pub(crate) const SETPOINT: &str = "7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21";
const UART_SERVICE: &str = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
const UART_RX: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";
const UART_TX: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";

/// What a simulated characteristic holds.
#[derive(Clone, Debug)]
enum Kind {
    Fixed(Vec<u8>),
    HeartRate,
    Battery {
        start: u8,
    },
    Temperature,
    Humidity,
    Pressure,
    BodyTemperature,
    /// Float32 LE, written by clients.
    Setpoint,
    AlertLevel,
    ControlPoint,
    UartRx,
    UartTx,
}

#[derive(Clone, Debug)]
struct SimCharacteristic {
    uuid: String,
    properties: BleCharProperties,
    kind: Kind,
    /// How often a subscribed client gets the value.
    every: Option<Duration>,
}

struct SimPeripheral {
    id: &'static str,
    name: &'static str,
    address: &'static str,
    rssi: i16,
    tx_power: Option<i16>,
    advertised: Vec<String>,
    manufacturer: Vec<BleManufacturerData>,
    service_data: Vec<BleServiceData>,
    connectable: bool,
    services: Vec<(String, Vec<SimCharacteristic>)>,
}

fn props(read: bool, write: bool, write_without_response: bool, notify: bool) -> BleCharProperties {
    BleCharProperties {
        read,
        write,
        write_without_response,
        notify,
        indicate: false,
    }
}

fn characteristic(
    uuid: impl Into<String>,
    properties: BleCharProperties,
    kind: Kind,
    every: Option<Duration>,
) -> SimCharacteristic {
    SimCharacteristic {
        uuid: uuid.into(),
        properties,
        kind,
        every,
    }
}

fn read_only(short: u16, kind: Kind) -> SimCharacteristic {
    characteristic(uuid16(short), props(true, false, false, false), kind, None)
}

fn text(value: &str) -> Kind {
    Kind::Fixed(value.as_bytes().to_vec())
}

fn device_information(model: &str, serial: &str) -> (String, Vec<SimCharacteristic>) {
    (
        uuid16(0x180A),
        vec![
            read_only(0x2A29, text("Voltrus")),
            read_only(0x2A24, text(model)),
            read_only(0x2A25, text(serial)),
            read_only(0x2A26, text("2.4.1")),
        ],
    )
}

fn world() -> Vec<SimPeripheral> {
    let second = Some(Duration::from_secs(1));
    let two_seconds = Some(Duration::from_secs(2));
    vec![
        SimPeripheral {
            id: HRM_ID,
            name: HRM_NAME,
            address: "C4:7F:51:0A:3E:01",
            rssi: -52,
            tx_power: Some(4),
            advertised: vec![uuid16(0x180D), uuid16(0x180F)],
            manufacturer: vec![BleManufacturerData {
                company: NORDIC,
                data: vec![0x01, 0x48, 0x52],
            }],
            service_data: Vec::new(),
            connectable: true,
            services: vec![
                (
                    uuid16(0x1800),
                    vec![
                        read_only(0x2A00, text(HRM_NAME)),
                        read_only(0x2A01, Kind::Fixed(0x0341u16.to_le_bytes().to_vec())),
                    ],
                ),
                (
                    uuid16(0x180D),
                    vec![
                        characteristic(
                            uuid16(0x2A37),
                            props(false, false, false, true),
                            Kind::HeartRate,
                            second,
                        ),
                        read_only(0x2A38, Kind::Fixed(vec![1])),
                        characteristic(
                            uuid16(0x2A39),
                            props(false, true, false, false),
                            Kind::ControlPoint,
                            None,
                        ),
                    ],
                ),
                (
                    uuid16(0x180F),
                    vec![characteristic(
                        uuid16(0x2A19),
                        props(true, false, false, true),
                        Kind::Battery { start: 87 },
                        Some(Duration::from_secs(10)),
                    )],
                ),
                (
                    uuid16(0x1802),
                    vec![characteristic(
                        uuid16(0x2A06),
                        props(false, false, true, false),
                        Kind::AlertLevel,
                        None,
                    )],
                ),
                (
                    UART_SERVICE.into(),
                    vec![
                        characteristic(
                            UART_RX,
                            props(false, true, true, false),
                            Kind::UartRx,
                            None,
                        ),
                        characteristic(
                            UART_TX,
                            props(false, false, false, true),
                            Kind::UartTx,
                            None,
                        ),
                    ],
                ),
                device_information("VT-HRM-1", "HRM-0042"),
            ],
        },
        SimPeripheral {
            id: ENV_ID,
            name: ENV_NAME,
            address: "C4:7F:51:0A:3E:02",
            rssi: -64,
            tx_power: Some(0),
            advertised: vec![uuid16(0x181A)],
            manufacturer: vec![BleManufacturerData {
                company: NORDIC,
                data: vec![0x02, 0x45, 0x4E, 0x56],
            }],
            service_data: Vec::new(),
            connectable: true,
            services: vec![
                (uuid16(0x1800), vec![read_only(0x2A00, text(ENV_NAME))]),
                (
                    uuid16(0x181A),
                    vec![
                        characteristic(
                            uuid16(0x2A6E),
                            props(true, false, false, true),
                            Kind::Temperature,
                            two_seconds,
                        ),
                        characteristic(
                            uuid16(0x2A6F),
                            props(true, false, false, true),
                            Kind::Humidity,
                            two_seconds,
                        ),
                        read_only(0x2A6D, Kind::Pressure),
                    ],
                ),
                (
                    uuid16(0x1809),
                    vec![characteristic(
                        uuid16(0x2A1C),
                        BleCharProperties {
                            indicate: true,
                            ..BleCharProperties::default()
                        },
                        Kind::BodyTemperature,
                        two_seconds,
                    )],
                ),
                (
                    CONTROL_SERVICE.into(),
                    vec![characteristic(
                        SETPOINT,
                        props(true, true, false, false),
                        Kind::Setpoint,
                        None,
                    )],
                ),
                (
                    uuid16(0x180F),
                    vec![characteristic(
                        uuid16(0x2A19),
                        props(true, false, false, false),
                        Kind::Battery { start: 64 },
                        None,
                    )],
                ),
                device_information("VT-ENV-2", "ENV-0007"),
            ],
        },
        SimPeripheral {
            id: BEACON_ID,
            name: "",
            address: "D2:41:9A:77:10:5C",
            rssi: -79,
            tx_power: Some(-8),
            advertised: vec![uuid16(0xFEAA)],
            manufacturer: Vec::new(),
            // An Eddystone-UID frame.
            service_data: vec![BleServiceData {
                service: uuid16(0xFEAA),
                data: vec![
                    0x00, 0xEE, 0x56, 0x6F, 0x6C, 0x74, 0x72, 0x75, 0x73, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01,
                ],
            }],
            connectable: false,
            services: Vec::new(),
        },
    ]
}

struct World {
    peripherals: Vec<SimPeripheral>,
    started: Instant,
    scanning: bool,
    connected: HashSet<String>,
    /// Subscribed characteristics and when each last notified (`None`: not
    /// yet, so at once).
    subscribed: HashMap<(String, String), Option<Instant>>,
    /// Values clients wrote, by peripheral and characteristic.
    written: HashMap<(String, String), Vec<u8>>,
    /// Pushed without waiting for a period (the UART echo).
    pending: Vec<Notification>,
}

impl World {
    fn peripheral(&self, id: &str) -> Result<&SimPeripheral, String> {
        self.peripherals
            .iter()
            .find(|peripheral| peripheral.id == id)
            .ok_or_else(|| format!("Device not found: {id}"))
    }

    fn connected(&self, id: &str) -> Result<&SimPeripheral, String> {
        let peripheral = self.peripheral(id)?;
        if self.connected.contains(id) {
            Ok(peripheral)
        } else {
            Err(format!("{} is not connected", label(peripheral)))
        }
    }

    fn characteristic(&self, id: &str, uuid: &str) -> Result<SimCharacteristic, String> {
        self.connected(id)?
            .services
            .iter()
            .flat_map(|(_, characteristics)| characteristics)
            .find(|characteristic| characteristic.uuid.eq_ignore_ascii_case(uuid))
            .cloned()
            .ok_or_else(|| format!("Characteristic not found: {uuid}"))
    }

    /// The value of a characteristic now.
    fn value(&self, id: &str, characteristic: &SimCharacteristic) -> Vec<u8> {
        let t = self.started.elapsed().as_secs_f64();
        let written = self
            .written
            .get(&(id.to_owned(), characteristic.uuid.clone()))
            .cloned();
        match &characteristic.kind {
            Kind::Fixed(bytes) => bytes.clone(),
            Kind::HeartRate => {
                let bpm = 72.0 + 9.0 * (t / 23.0).sin() + 3.0 * (t / 5.3).sin();
                let rr = (60.0 / bpm * 1024.0).round() as u16;
                let [low, high] = rr.to_le_bytes();
                // uint8 rate, sensor contact detected, one RR interval.
                vec![0x16, bpm.round() as u8, low, high]
            }
            Kind::Battery { start } => {
                let drained = (t / 60.0).floor() as u8;
                vec![start.saturating_sub(drained).max(5)]
            }
            Kind::Temperature => {
                let celsius = 21.5 + 1.2 * (t / 40.0).sin() + 0.1 * (t / 3.0).sin();
                ((celsius * 100.0).round() as i16).to_le_bytes().to_vec()
            }
            Kind::Humidity => {
                let percent = 46.0 + 4.0 * (t / 55.0).sin();
                ((percent * 100.0).round() as u16).to_le_bytes().to_vec()
            }
            Kind::Pressure => {
                let pascal = 101_325.0 + 120.0 * (t / 90.0).sin();
                ((pascal * 10.0).round() as u32).to_le_bytes().to_vec()
            }
            Kind::BodyTemperature => {
                let celsius = 36.6 + 0.2 * (t / 30.0).sin();
                // IEEE 11073 FLOAT: hundredths, exponent -2.
                let mantissa = (celsius * 100.0).round() as u32 & 0x00FF_FFFF;
                let mut bytes = vec![0x00];
                bytes.extend_from_slice(&(0xFE00_0000 | mantissa).to_le_bytes());
                bytes
            }
            Kind::Setpoint => written.unwrap_or_else(|| 22.0f32.to_le_bytes().to_vec()),
            Kind::UartTx => written.unwrap_or_else(|| b"ready".to_vec()),
            Kind::AlertLevel | Kind::ControlPoint | Kind::UartRx => written.unwrap_or_default(),
        }
    }
}

fn label(peripheral: &SimPeripheral) -> &str {
    if peripheral.name.is_empty() {
        peripheral.address
    } else {
        peripheral.name
    }
}

/// A central over the simulated peripherals. Each has its own world: what
/// one session writes, another doesn't see.
pub(crate) struct SimCentral {
    world: Arc<Mutex<World>>,
    ticker: JoinHandle<()>,
}

impl Drop for SimCentral {
    fn drop(&mut self) {
        self.ticker.abort();
    }
}

fn lock(world: &Mutex<World>) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SimCentral {
    /// Must be called on the engine's runtime: it starts the task that
    /// sends notifications.
    pub(crate) fn new() -> (Self, mpsc::Receiver<Notification>) {
        let world = Arc::new(Mutex::new(World {
            peripherals: world(),
            started: Instant::now(),
            scanning: false,
            connected: HashSet::new(),
            subscribed: HashMap::new(),
            written: HashMap::new(),
            pending: Vec::new(),
        }));
        let (notify, notifications) = mpsc::channel(NOTIFY_QUEUE);
        let shared = world.clone();
        let ticker = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(NOTIFY_TICK);
            loop {
                ticker.tick().await;
                let due = {
                    let mut world = lock(&shared);
                    due(&mut world)
                };
                for notification in due {
                    if notify.send(notification).await.is_err() {
                        return;
                    }
                }
            }
        });
        (Self { world, ticker }, notifications)
    }

    fn world(&self) -> MutexGuard<'_, World> {
        lock(&self.world)
    }

    /// The peripheral went out of range: the link is gone until the next
    /// connect.
    #[cfg(test)]
    pub(crate) fn drop_link(&self, id: &str) {
        let mut world = self.world();
        world.connected.remove(id);
        world
            .subscribed
            .retain(|(peripheral, _), _| peripheral != id);
    }

    /// Whether a scan is running.
    #[cfg(test)]
    pub(crate) fn scanning(&self) -> bool {
        self.world().scanning
    }
}

/// Notifications due now: pending echoes, and subscribed values whose period
/// passed.
fn due(world: &mut World) -> Vec<Notification> {
    let now = Instant::now();
    let mut out = std::mem::take(&mut world.pending);
    let keys: Vec<(String, String)> = world.subscribed.keys().cloned().collect();
    for (id, uuid) in keys {
        let Ok(characteristic) = world.characteristic(&id, &uuid) else {
            continue;
        };
        let Some(every) = characteristic.every else {
            continue;
        };
        let last = world
            .subscribed
            .get(&(id.clone(), uuid.clone()))
            .copied()
            .flatten();
        if last.is_some_and(|last| now.duration_since(last) < every) {
            continue;
        }
        let data = world.value(&id, &characteristic);
        world
            .subscribed
            .insert((id.clone(), uuid.clone()), Some(now));
        out.push(Notification {
            peripheral: id,
            characteristic: characteristic.uuid,
            data,
        });
    }
    out
}

impl Central for SimCentral {
    async fn start_scan(&self) -> Result<(), String> {
        self.world().scanning = true;
        Ok(())
    }

    async fn stop_scan(&self) -> Result<(), String> {
        self.world().scanning = false;
        Ok(())
    }

    async fn peripherals(&self) -> Vec<BleDeviceInfo> {
        let world = self.world();
        let t = world.started.elapsed().as_secs_f64();
        world
            .peripherals
            .iter()
            .enumerate()
            .filter(|(_, peripheral)| world.scanning || world.connected.contains(peripheral.id))
            .map(|(index, peripheral)| {
                // Signal strength wanders a few dB, as it does in a room.
                let wander = 3.0 * (t * 1.7 + index as f64).sin();
                BleDeviceInfo {
                    id: peripheral.id.into(),
                    name: peripheral.name.into(),
                    address: peripheral.address.into(),
                    rssi: Some(peripheral.rssi + wander.round() as i16),
                    tx_power: peripheral.tx_power,
                    services: peripheral.advertised.clone(),
                    manufacturer: peripheral.manufacturer.clone(),
                    service_data: peripheral.service_data.clone(),
                    connected: world.connected.contains(peripheral.id),
                }
            })
            .collect()
    }

    async fn connect(&self, peripheral: &str) -> Result<(), String> {
        tokio::time::sleep(CONNECT_DELAY).await;
        let mut world = self.world();
        let found = world.peripheral(peripheral)?;
        if !found.connectable {
            return Err(format!(
                "Failed to connect: {} accepts no connections",
                label(found)
            ));
        }
        world.connected.insert(peripheral.to_owned());
        Ok(())
    }

    async fn disconnect(&self, peripheral: &str) -> Result<(), String> {
        let mut world = self.world();
        world.peripheral(peripheral)?;
        world.connected.remove(peripheral);
        world
            .subscribed
            .retain(|(subscribed, _), _| subscribed != peripheral);
        Ok(())
    }

    async fn is_connected(&self, peripheral: &str) -> Result<bool, String> {
        let world = self.world();
        world.peripheral(peripheral)?;
        Ok(world.connected.contains(peripheral))
    }

    async fn services(&self, peripheral: &str) -> Result<Vec<BleService>, String> {
        let world = self.world();
        Ok(world
            .connected(peripheral)?
            .services
            .iter()
            .map(|(uuid, characteristics)| BleService {
                uuid: uuid.clone(),
                is_primary: true,
                characteristics: characteristics
                    .iter()
                    .map(|characteristic| BleCharacteristic {
                        uuid: characteristic.uuid.clone(),
                        properties: characteristic.properties.clone(),
                        value: None,
                    })
                    .collect(),
            })
            .collect())
    }

    async fn read(&self, peripheral: &str, characteristic: &str) -> Result<Vec<u8>, String> {
        let world = self.world();
        let found = world.characteristic(peripheral, characteristic)?;
        if !found.properties.read {
            return Err("Failed to read: read not permitted (ATT error 0x02)".into());
        }
        Ok(world.value(peripheral, &found))
    }

    async fn write(
        &self,
        peripheral: &str,
        characteristic: &str,
        data: &[u8],
    ) -> Result<(), String> {
        let mut world = self.world();
        let found = world.characteristic(peripheral, characteristic)?;
        if !found.properties.write && !found.properties.write_without_response {
            return Err("Failed to write: write not permitted (ATT error 0x03)".into());
        }
        let key = |uuid: &str| (peripheral.to_owned(), uuid.to_owned());
        match found.kind {
            Kind::Setpoint if data.len() != 4 => {
                return Err(
                    "Failed to write: the setpoint takes 4 bytes, a float32 (ATT error 0x0D)"
                        .into(),
                );
            }
            Kind::AlertLevel if !matches!(data, [0..=2]) => {
                return Err(
                    "Failed to write: alert level is one byte, 0 to 2 (ATT error 0x13)".into(),
                );
            }
            Kind::ControlPoint if data != [0x01] => {
                return Err(
                    "Failed to write: control point value not supported (ATT error 0x80)".into(),
                );
            }
            Kind::UartRx => {
                // The board echoes what it receives on its TX characteristic.
                world.written.insert(key(UART_TX), data.to_vec());
                if world.subscribed.contains_key(&key(UART_TX)) {
                    world.pending.push(Notification {
                        peripheral: peripheral.to_owned(),
                        characteristic: UART_TX.to_owned(),
                        data: data.to_vec(),
                    });
                }
            }
            _ => {}
        }
        world.written.insert(key(&found.uuid), data.to_vec());
        Ok(())
    }

    async fn subscribe(&self, peripheral: &str, characteristic: &str) -> Result<(), String> {
        let mut world = self.world();
        let found = world.characteristic(peripheral, characteristic)?;
        if !found.properties.notify && !found.properties.indicate {
            return Err("Failed to subscribe: the characteristic does not notify".into());
        }
        world
            .subscribed
            .entry((peripheral.to_owned(), found.uuid))
            // Due at once: the first value arrives on the next tick.
            .or_insert(None);
        Ok(())
    }

    async fn unsubscribe(&self, peripheral: &str, characteristic: &str) -> Result<(), String> {
        let mut world = self.world();
        let found = world.characteristic(peripheral, characteristic)?;
        world
            .subscribed
            .remove(&(peripheral.to_owned(), found.uuid));
        Ok(())
    }
}
