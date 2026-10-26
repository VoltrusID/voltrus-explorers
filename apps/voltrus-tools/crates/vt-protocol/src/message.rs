use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BacnetDeviceInfo, BacnetDiscoverSpec, BleCharacteristicInfo, BleRequest, BleScan, BleScanSpec,
    MqttQos, PointInfo, SessionSpec, TrafficFrame, ValueFrame,
};

/// Identifies a running session. Chosen by the app, unique per engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s{}", self.0)
    }
}

/// App → engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Start a session. Opening an id that is already running replaces it.
    Open {
        session: SessionId,
        spec: SessionSpec,
    },
    /// Stop a session; the engine answers with a final `Closed` state.
    Close { session: SessionId },
    /// Write `value` to point `point`. Masters write to the device; simulators
    /// set the value they serve.
    Write {
        session: SessionId,
        point: u32,
        value: f64,
    },
    /// Simulators only: hold a simulated value instead of letting it drift.
    Lock {
        session: SessionId,
        point: u32,
        locked: bool,
    },
    /// Message protocols (MQTT): publish `payload` to `topic`. Clients send it
    /// to their broker; a broker simulator delivers it as if a client had.
    Publish {
        session: SessionId,
        topic: String,
        payload: String,
        qos: MqttQos,
        retain: bool,
    },
    /// Stop every session and the engine itself.
    Shutdown,
    /// BACnet clients: give up the command at the session's write priority
    /// (WriteProperty of NULL), so a lower priority or the relinquish default
    /// drives the point again. Answered by [`EventKind::Written`].
    Relinquish { session: SessionId, point: u32 },
    /// Send a BACnet Who-Is; the devices that answer come back as
    /// [`EventKind::BacnetDevices`] under `session`, which names the request
    /// (no session runs under it).
    DiscoverBacnet {
        session: SessionId,
        spec: BacnetDiscoverSpec,
    },
    /// Scan for BLE peripherals; what it sees comes back as
    /// [`EventKind::BleDevices`] under `session`, which names the scan (no
    /// session runs under it), until an answer that is `done`.
    ScanBle {
        session: SessionId,
        spec: BleScanSpec,
    },
    /// BLE sessions: read, write, subscribe to or re-decode a characteristic.
    Ble {
        session: SessionId,
        request: BleRequest,
    },
}

/// Engine → app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub session: SessionId,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum EventKind {
    State(SessionState),
    /// The points of the session; sent before the first values.
    Points(Vec<PointInfo>),
    Values(ValueFrame),
    Traffic(Vec<TrafficFrame>),
    Log(LogEntry),
    Stats(Stats),
    /// Outcome of a [`Command::Write`].
    Written {
        point: u32,
        result: Result<(), String>,
    },
    /// Simulators: a point is now held at its value (or released to drift
    /// again), after an app write or lock, or a write from an external master.
    Locked {
        point: u32,
        locked: bool,
    },
    /// More points joined the session (topics seen for the first time). Their
    /// ids continue from the points already reported; values so far are kept.
    PointsAdded(Vec<PointInfo>),
    /// The latest message behind points whose values arrive as messages
    /// (MQTT payloads), as text for the inspector.
    Payloads(Vec<PointText>),
    /// Outcome of a [`Command::Publish`].
    Published {
        topic: String,
        result: Result<(), String>,
    },
    /// Answer to [`Command::DiscoverBacnet`]: the devices that sent an I-Am.
    BacnetDevices(Result<Vec<BacnetDeviceInfo>, String>),
    /// Answers to [`Command::ScanBle`]: the peripherals seen so far.
    BleDevices(Result<BleScan, String>),
    /// BLE sessions: each point's characteristic, its service and properties,
    /// whether it notifies and how it is decoded. Sent after `Points` and
    /// whenever one of them changes.
    BleGatt(Vec<BleCharacteristicInfo>),
}

/// The text behind one point's latest value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointText {
    pub point: u32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Connected,
    /// Serving on the given address.
    Listening {
        address: String,
    },
    /// Lost the device; trying again after `retry_in_ms`.
    Reconnecting {
        attempt: u32,
        retry_in_ms: u64,
        reason: String,
    },
    /// Stopped by the app or by a fatal error.
    Closed {
        error: Option<String>,
    },
}

impl SessionState {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Connected | Self::Listening { .. })
    }

    pub fn is_running(&self) -> bool {
        !matches!(self, Self::Closed { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Listening { .. } => "Listening",
            Self::Reconnecting { .. } => "Reconnecting",
            Self::Closed { error: Some(_) } => "Failed",
            Self::Closed { error: None } => "Stopped",
        }
    }
}

/// Running counters of a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub polls: u64,
    pub errors: u64,
    /// Requests served (simulators) or frames exchanged (clients).
    pub frames: u64,
    pub last_latency_us: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModbusMasterSpec, SessionSpec};

    // The engine may move out of process: every message must survive a
    // round trip through its serialized form unchanged.
    #[test]
    fn messages_round_trip_through_json() {
        let open = Command::Open {
            session: SessionId(7),
            spec: SessionSpec::ModbusMaster(ModbusMasterSpec::default()),
        };
        let text = serde_json::to_string(&open).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), open);

        let event = Event {
            session: SessionId(7),
            kind: EventKind::Written {
                point: 3,
                result: Err("Illegal data address".into()),
            },
        };
        let text = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);

        let locked = Event {
            session: SessionId(7),
            kind: EventKind::Locked {
                point: 4,
                locked: true,
            },
        };
        let text = serde_json::to_string(&locked).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), locked);
    }

    #[test]
    fn mqtt_messages_round_trip_through_json() {
        use crate::{MqttBrokerSpec, MqttClientSpec, MqttQos, MqttSubscription};

        let client = MqttClientSpec {
            host: "broker.example".into(),
            port: 8883,
            client_id: "panel-7".into(),
            username: "operator".into(),
            password: "s3cret".into(),
            tls: true,
            keep_alive_s: 15,
            subscriptions: vec![
                MqttSubscription::new("plant/+/telemetry"),
                MqttSubscription {
                    filter: "site/meter".into(),
                    qos: MqttQos::ExactlyOnce,
                    json_path: "phases[2].v".into(),
                },
            ],
        };
        let broker = MqttBrokerSpec {
            sparkplug: true,
            template: None,
            ..MqttBrokerSpec::default()
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::MqttClient(client),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::MqttBroker(broker),
            },
            Command::Publish {
                session: SessionId(1),
                topic: "plant/inverter/limit".into(),
                payload: "{\"kw\": 42.5}".into(),
                qos: MqttQos::AtLeastOnce,
                retain: true,
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let events = [
            EventKind::PointsAdded(vec![PointInfo {
                id: 3,
                name: "plant/inverter/ac_power".into(),
                address: "QoS 0".into(),
                data_type: "Number".into(),
                unit: String::new(),
                writable: true,
                boolean: false,
                states: Vec::new(),
            }]),
            EventKind::Payloads(vec![PointText {
                point: 3,
                text: "1234.5".into(),
            }]),
            EventKind::Published {
                topic: "plant/inverter/limit".into(),
                result: Err("Not connected".into()),
            },
        ];
        for kind in events {
            let event = Event {
                session: SessionId(1),
                kind,
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // Optional fields may be missing from a saved workspace.
        let saved = r##"{"kind":"mqtt_client","host":"h","port":1883,"keep_alive_s":30,
            "subscriptions":[{"filter":"#","qos":"at_most_once"}]}"##;
        let SessionSpec::MqttClient(spec) = serde_json::from_str(saved).unwrap() else {
            panic!("an MQTT client spec");
        };
        assert!(spec.password.is_empty() && !spec.tls);
        assert!(spec.subscriptions[0].json_path.is_empty());
    }

    #[test]
    fn bacnet_messages_round_trip_through_json() {
        let client = crate::BacnetClientSpec {
            host: "plant.local".into(),
            device_instance: Some(4_194_302),
            local_port: crate::BACNET_PORT,
            cov: false,
            priority: 16,
            ..crate::BacnetClientSpec::default()
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::BacnetClient(client),
            },
            // An unknown instance is not the same as instance 0.
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::BacnetClient(crate::BacnetClientSpec {
                    device_instance: None,
                    ..crate::BacnetClientSpec::default()
                }),
            },
            Command::Open {
                session: SessionId(3),
                spec: SessionSpec::BacnetDevice(crate::BacnetDeviceSpec::default()),
            },
            Command::Relinquish {
                session: SessionId(1),
                point: 9,
            },
            Command::DiscoverBacnet {
                session: SessionId(4),
                spec: BacnetDiscoverSpec {
                    bind: "0.0.0.0".into(),
                    broadcast_port: None,
                    directed: vec!["127.0.0.1:47809".into(), "[::1]:47808".into()],
                    wait_ms: 1500,
                },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        for answer in [
            Ok(vec![BacnetDeviceInfo {
                instance: 1001,
                address: "192.168.1.40".into(),
                port: 47808,
                vendor: "ASHRAE".into(),
                also: vec!["127.0.0.1:47808".into()],
            }]),
            Ok(Vec::new()),
            Err("Who-Is broadcast could not be sent".into()),
        ] {
            let event = Event {
                session: SessionId(4),
                kind: EventKind::BacnetDevices(answer),
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }
    }

    #[test]
    fn ble_messages_round_trip_through_json() {
        use crate::{
            BleAdapter, BleClientSpec, BleDecoder, BleDeviceInfo, BleFormat, BleManufacturerData,
            BleProperties, BleServiceData,
        };

        let mut spec = BleClientSpec {
            adapter: BleAdapter::Simulated,
            peripheral: "5C3A1F2E-0000-4000-8000-00000000BEEF".into(),
            name: "Voltrus HRM".into(),
            poll_ms: 0,
            notify: false,
            decoders: Vec::new(),
        };
        spec.set_decoder(
            "00002a6e-0000-1000-8000-00805f9b34fb",
            BleDecoder {
                format: BleFormat::I16Be,
                offset: 2,
                scale: 0.01,
            },
        );
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::BleClient(spec),
            },
            Command::ScanBle {
                session: SessionId(2),
                spec: BleScanSpec {
                    adapter: BleAdapter::System,
                    duration_ms: 5000,
                },
            },
            Command::Ble {
                session: SessionId(1),
                request: BleRequest::Write {
                    point: 4,
                    data: vec![0x00, 0xff, 0x7f],
                },
            },
            Command::Ble {
                session: SessionId(1),
                request: BleRequest::Notify {
                    point: 0,
                    enabled: false,
                },
            },
            Command::Ble {
                session: SessionId(1),
                request: BleRequest::Decode {
                    point: 2,
                    decoder: BleDecoder::of(BleFormat::F64Le),
                },
            },
            Command::Ble {
                session: SessionId(1),
                request: BleRequest::Read { point: 3 },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let device = BleDeviceInfo {
            id: "AA:BB:CC:DD:EE:FF".into(),
            name: String::new(),
            address: "AA:BB:CC:DD:EE:FF".into(),
            rssi: Some(-67),
            tx_power: None,
            services: vec!["0000180d-0000-1000-8000-00805f9b34fb".into()],
            manufacturer: vec![BleManufacturerData {
                company: 0x0059,
                data: vec![1, 2, 3],
            }],
            service_data: vec![BleServiceData {
                service: "0000feaa-0000-1000-8000-00805f9b34fb".into(),
                data: Vec::new(),
            }],
            connected: false,
        };
        let events = [
            EventKind::BleDevices(Ok(BleScan {
                devices: vec![device],
                done: false,
            })),
            EventKind::BleDevices(Err("Bluetooth is off".into())),
            EventKind::BleGatt(vec![BleCharacteristicInfo {
                point: 0,
                service: "0000180d-0000-1000-8000-00805f9b34fb".into(),
                uuid: "00002a37-0000-1000-8000-00805f9b34fb".into(),
                properties: BleProperties {
                    notify: true,
                    ..BleProperties::default()
                },
                notifying: true,
                decoder: BleDecoder::default(),
            }]),
        ];
        for kind in events {
            let event = Event {
                session: SessionId(2),
                kind,
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }
    }
}
