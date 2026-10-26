use std::fmt;

use serde::{Deserialize, Serialize};

use crate::S7PlcReport;
use crate::{
    BacnetDeviceInfo, BacnetDiscoverSpec, CipRequest, CipResponse, Dnp3Action, Dnp3Iin,
    EnipDeviceInfo, EnipDiscoverSpec, MqttQos, NatsEvent, NatsRequest, PointInfo, SerialChunk,
    SerialPortInfo, SessionSpec, TrafficFrame, ValueFrame,
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
    /// DNP3 sessions: a station command (poll, time sync, restart, control);
    /// answered by [`EventKind::Dnp3Done`].
    Dnp3 {
        session: SessionId,
        action: Dnp3Action,
    },
    /// Send an EtherNet/IP ListIdentity; the devices that answer come back as
    /// [`EventKind::EnipDevices`] under `session`, which names the request.
    DiscoverEnip {
        session: SessionId,
        spec: EnipDiscoverSpec,
    },
    /// EtherNet/IP clients: send one explicit CIP message to the device,
    /// answered by [`EventKind::CipReply`].
    CipRequest {
        session: SessionId,
        request: CipRequest,
    },
    /// Serial monitors and devices: write raw bytes to the port. Answered by
    /// [`EventKind::SerialSent`]; what went out shows in [`EventKind::SerialData`].
    SerialSend { session: SessionId, bytes: Vec<u8> },
    /// Serial monitors and devices: log every chunk that crosses the port to
    /// a CSV file at `path` (a path the user picked), or stop with `None`.
    /// Answered by [`EventKind::SerialLogging`].
    SerialLog {
        session: SessionId,
        path: Option<String>,
    },
    /// List the system's serial ports; the answer comes back as
    /// [`EventKind::SerialPorts`] under `session`, which names the request.
    ListSerialPorts { session: SessionId },
    /// S7 clients: read the CPU's identity, mode, diagnostic buffer and
    /// blocks over the session's connection. Answered by [`EventKind::S7Plc`].
    ReadS7Plc { session: SessionId },
    /// NATS clients: publish with headers, request/reply and JetStream
    /// browsing; answered by [`EventKind::Nats`] or [`EventKind::Published`].
    Nats {
        session: SessionId,
        request: NatsRequest,
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
    /// DNP3 masters: the outstation's internal indications changed.
    Dnp3Iin(Dnp3Iin),
    /// Outcome of a [`Command::Dnp3`], with what the outstation answered.
    Dnp3Done {
        action: Dnp3Action,
        result: Result<String, String>,
    },
    /// Answer to [`Command::DiscoverEnip`]: the devices that answered ListIdentity.
    EnipDevices(Result<Vec<EnipDeviceInfo>, String>),
    /// Outcome of a [`Command::CipRequest`]: the reply data, or the CIP
    /// status (or transport failure) as text.
    CipReply(Result<CipResponse, String>),
    /// EtherNet/IP: each point's whole address (`Program:Main.Motor1.Speed`,
    /// `@0x04/100/3+24:REAL`), aligned with the points, after `Points`: the
    /// table shows a short one.
    EnipAddresses(Vec<String>),
    /// Bytes that crossed a serial port, in order.
    SerialData(Vec<SerialChunk>),
    /// Outcome of a [`Command::SerialSend`]: the bytes written.
    SerialSent(Result<u32, String>),
    /// Where the session now logs (`None`: it stopped), or why it could not.
    SerialLogging(Result<Option<String>, String>),
    /// Answer to [`Command::ListSerialPorts`].
    SerialPorts(Result<Vec<SerialPortInfo>, String>),
    /// What an S7 CPU says about itself, after connecting and after each
    /// [`Command::ReadS7Plc`]; `Err` when it could not be asked.
    S7Plc(Result<S7PlcReport, String>),
    /// A NATS client's answer to a [`Command::Nats`].
    Nats(NatsEvent),
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
    fn dnp3_messages_round_trip_through_json() {
        use crate::{
            Dnp3ControlMode, Dnp3Crob, Dnp3CrobCode, Dnp3MasterSpec, Dnp3Operation,
            Dnp3OutstationSpec, Dnp3PointKind, Dnp3PointName,
        };

        let master = Dnp3MasterSpec {
            host: "rtu-7.substation".into(),
            outstation_address: 1024,
            event_ms: 0,
            control_mode: Dnp3ControlMode::DirectOperate,
            time_sync: false,
            points: vec![Dnp3PointName {
                kind: Dnp3PointKind::AnalogInput,
                index: 3,
                name: "Feeder current A".into(),
                unit: "A".into(),
            }],
            ..Dnp3MasterSpec::default()
        };
        let crob = Dnp3Crob {
            code: Dnp3CrobCode::Trip,
            count: 2,
            on_ms: 250,
            off_ms: 750,
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::Dnp3Master(master),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::Dnp3Outstation(Dnp3OutstationSpec::default()),
            },
            Command::Dnp3 {
                session: SessionId(1),
                action: Dnp3Action::Operate {
                    point: 12,
                    operation: Dnp3Operation::Crob(crob),
                    mode: Dnp3ControlMode::SelectBeforeOperate,
                },
            },
            Command::Dnp3 {
                session: SessionId(1),
                action: Dnp3Action::Operate {
                    point: 13,
                    operation: Dnp3Operation::Analog { value: -12.25 },
                    mode: Dnp3ControlMode::DirectOperate,
                },
            },
            Command::Dnp3 {
                session: SessionId(1),
                action: Dnp3Action::ClassPoll { class: 2 },
            },
            Command::Dnp3 {
                session: SessionId(2),
                action: Dnp3Action::SetOnline {
                    point: 4,
                    online: false,
                },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        for kind in [
            EventKind::Dnp3Iin(Dnp3Iin {
                iin1: 0x90,
                iin2: 0x02,
            }),
            EventKind::Dnp3Done {
                action: Dnp3Action::TimeSync,
                result: Ok("Time written".into()),
            },
            EventKind::Dnp3Done {
                action: Dnp3Action::ColdRestart,
                result: Err("function code not supported (IIN2.0)".into()),
            },
        ] {
            let event = Event {
                session: SessionId(1),
                kind,
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // A saved master without point names reads back with none.
        let saved = r#"{"kind":"dnp3_master","host":"h","port":20000,"master_address":1,
            "outstation_address":10,"integrity_ms":60000,"event_ms":1000,"timeout_ms":2000,
            "control_mode":"select_before_operate","time_sync":true}"#;
        let SessionSpec::Dnp3Master(spec) = serde_json::from_str(saved).unwrap() else {
            panic!("a DNP3 master spec");
        };
        assert!(spec.points.is_empty());
    }

    #[test]
    fn enip_messages_round_trip_through_json() {
        use crate::{
            CipRequest, CipResponse, CipType, EnipAdapterSpec, EnipAddress, EnipClientSpec,
            EnipDeviceInfo, EnipDiscoverSpec, EnipPoint,
        };

        let client = EnipClientSpec {
            host: "plc-7.local".into(),
            route_path: "1,0".into(),
            poll_ms: 250,
            points: vec![
                EnipPoint {
                    name: "Speed".into(),
                    address: EnipAddress::parse("Program:Main.Motor1.Speed").unwrap(),
                    unit: "RPM".into(),
                },
                EnipPoint {
                    name: String::new(),
                    address: EnipAddress::Attribute {
                        class: 4,
                        instance: 100,
                        attribute: 3,
                        offset: 12,
                        data_type: CipType::Lreal,
                    },
                    unit: String::new(),
                },
            ],
            ..EnipClientSpec::default()
        };
        let request = CipRequest {
            service: 0x10,
            class: 4,
            instance: 150,
            attribute: Some(3),
            data: vec![0, 0x80, 0xFF],
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::EnipClient(client),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::EnipAdapter(EnipAdapterSpec::default()),
            },
            Command::DiscoverEnip {
                session: SessionId(3),
                spec: EnipDiscoverSpec {
                    broadcast_port: None,
                    directed: vec!["127.0.0.1:44819".into()],
                    wait_ms: 1200,
                },
            },
            Command::CipRequest {
                session: SessionId(1),
                request: request.clone(),
            },
            // Get_Attributes_All addresses no attribute: not attribute 0.
            Command::CipRequest {
                session: SessionId(1),
                request: CipRequest {
                    service: 0x01,
                    class: 1,
                    instance: 1,
                    attribute: None,
                    data: Vec::new(),
                },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let events = [
            EventKind::EnipDevices(Ok(vec![EnipDeviceInfo {
                address: "192.168.1.20".into(),
                port: 44818,
                product: "1756-L83E/B".into(),
                vendor: "Rockwell Automation/Allen-Bradley".into(),
                device_type: "Programmable Logic Controller".into(),
                revision: "33.11".into(),
                serial: 0x00C0_FFEE,
                also: vec!["127.0.0.1:44818".into()],
            }])),
            EventKind::EnipDevices(Err("Network is unreachable".into())),
            EventKind::CipReply(Ok(CipResponse {
                request,
                data: Vec::new(),
                latency_us: 812,
            })),
            EventKind::CipReply(Err("CIP error 0x14: Attribute not supported".into())),
            EventKind::EnipAddresses(vec![
                "Program:Main.Motor1.Speed".into(),
                "@0x04/100/3+24:REAL".into(),
            ]),
        ];
        for kind in events {
            let event = Event {
                session: SessionId(1),
                kind,
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // Saved before route paths and points existed: a direct, browse-all client.
        let saved = r#"{"kind":"enip_client","host":"10.0.0.5","port":44818,"poll_ms":1000}"#;
        let SessionSpec::EnipClient(spec) = serde_json::from_str(saved).unwrap() else {
            panic!("an EtherNet/IP client spec");
        };
        assert!(spec.route_path.is_empty() && spec.points.is_empty());
    }

    #[test]
    fn serial_messages_round_trip_through_json() {
        use crate::{
            Direction, LineEnding, LineParser, SerialDeviceSpec, SerialFormat, SerialMacro,
            SerialMonitorSpec,
        };

        let monitor = SerialMonitorSpec {
            line_ending: LineEnding::CrLf,
            parser: LineParser::Csv {
                delimiter: ';',
                names: vec!["Temperature".into(), "Humidity".into()],
            },
            macros: vec![SerialMacro {
                name: "Poll".into(),
                payload: "01 03 00 00 00 0A C5 CD".into(),
                hex: true,
            }],
            ..SerialMonitorSpec::default()
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::SerialMonitor(monitor),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::SerialMonitor(SerialMonitorSpec {
                    parser: LineParser::Regex {
                        pattern: r"T=(?P<temp>-?\d+\.\d+)".into(),
                    },
                    ..SerialMonitorSpec::default()
                }),
            },
            Command::Open {
                session: SessionId(3),
                spec: SessionSpec::SerialDevice(SerialDeviceSpec {
                    format: SerialFormat::Csv,
                    ..SerialDeviceSpec::default()
                }),
            },
            Command::SerialSend {
                session: SessionId(1),
                bytes: vec![0x00, 0x0D, 0x0A, 0xFF],
            },
            Command::SerialLog {
                session: SessionId(1),
                path: Some("/Users/me/Documents/capture.csv".into()),
            },
            Command::SerialLog {
                session: SessionId(1),
                path: None,
            },
            Command::ListSerialPorts {
                session: SessionId(9),
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let events = [
            EventKind::SerialData(vec![
                SerialChunk {
                    at_ms: 1_700_000_000_000,
                    direction: Direction::Rx,
                    bytes: b"pH=7.21\r\n".to_vec(),
                },
                SerialChunk {
                    at_ms: 1_700_000_000_001,
                    direction: Direction::Tx,
                    bytes: Vec::new(),
                },
            ]),
            EventKind::SerialSent(Ok(6)),
            EventKind::SerialSent(Err("Not connected".into())),
            EventKind::SerialLogging(Ok(Some("/tmp/x.csv".into()))),
            EventKind::SerialLogging(Ok(None)),
            EventKind::SerialLogging(Err("Permission denied".into())),
            EventKind::SerialPorts(Ok(vec![SerialPortInfo {
                path: "/dev/cu.usbserial-0001".into(),
                kind: "USB".into(),
                description: "FTDI FT232R".into(),
            }])),
            EventKind::SerialPorts(Err("Enumerating ports failed".into())),
        ];
        for kind in events {
            let event = Event {
                session: SessionId(1),
                kind,
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // A monitor saved with only its link takes the defaults.
        let saved = r#"{"kind":"serial_monitor","link":{"port":"/dev/cu.x","baud":9600,
            "data_bits":8,"parity":"none","stop_bits":1}}"#;
        let SessionSpec::SerialMonitor(spec) = serde_json::from_str(saved).unwrap() else {
            panic!("a serial monitor spec");
        };
        assert_eq!(spec.parser, LineParser::KeyValue);
        assert_eq!(spec.line_ending, LineEnding::Lf);
        assert!(spec.macros.is_empty());
    }

    #[test]
    fn s7_messages_round_trip_through_json() {
        use crate::{S7Block, S7ClientSpec, S7CpuInfo, S7DiagEntry, S7PlcReport, S7ServerSpec};

        let mut client = S7ClientSpec {
            host: "fe80::1".into(),
            rack: 0,
            slot: 2,
            pdu_size: 240,
            ..S7ClientSpec::default()
        };
        for (name, address, data_type) in [
            ("Run", "Q0.1", crate::S7Type::Bool),
            ("Setpoint", "DB10.DBW4", crate::S7Type::Int),
            ("Batch", "DB10.DBB20", crate::S7Type::String),
            ("Delay", "T3", crate::S7Type::Timer),
        ] {
            client.tags.push(crate::S7TagSpec {
                name: name.into(),
                address: address.parse().unwrap(),
                data_type,
                unit: String::new(),
            });
        }
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::S7Client(client),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::S7Server(S7ServerSpec::default()),
            },
            Command::ReadS7Plc {
                session: SessionId(1),
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let report = S7PlcReport {
            pdu_size: 240,
            cpu: Ok(S7CpuInfo {
                module_type: "CPU 315-2 PN/DP".into(),
                order_code: "6ES7 315-2EH14-0AB0".into(),
                firmware: "V3.2.6".into(),
                ..S7CpuInfo::default()
            }),
            mode: Ok("RUN".into()),
            diagnostics: Ok(vec![S7DiagEntry {
                timestamp: "2026-10-26 08:00:00.000".into(),
                event_id: 0x4302,
                text: "Mode transition from STARTUP to RUN".into(),
            }]),
            // Each part fails on its own.
            blocks: Err("function not available in this PLC (0x8104)".into()),
        };
        for answer in [Ok(report.clone()), Err("Not connected".into())] {
            let event = Event {
                session: SessionId(1),
                kind: EventKind::S7Plc(answer),
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }
        let blocks = S7PlcReport {
            blocks: Ok(vec![S7Block {
                kind: "DB".into(),
                number: 1,
                language: "DB".into(),
            }]),
            ..report
        };
        let text = serde_json::to_string(&blocks).unwrap();
        assert_eq!(serde_json::from_str::<S7PlcReport>(&text).unwrap(), blocks);
    }

    #[test]
    fn nats_messages_round_trip_through_json() {
        use crate::{
            NatsAuth, NatsClientSpec, NatsConsumer, NatsHeader, NatsReply, NatsServerSpec,
            NatsStoredMessage, NatsStream, NatsStreamDetail, NatsSubscription,
        };

        let client = NatsClientSpec {
            host: "nats.example".into(),
            port: 4443,
            tls: true,
            auth: NatsAuth::Credentials {
                file: "plant.creds".into(),
                creds: "-----BEGIN NATS USER JWT-----\neyJ0…\n------END NATS USER JWT------".into(),
            },
            name: "panel-7".into(),
            subscriptions: vec![
                NatsSubscription::new("plant.*.telemetry"),
                NatsSubscription {
                    subject: "site.meter".into(),
                    queue: "workers".into(),
                    json_path: "phases[2].v".into(),
                },
            ],
        };
        let headers = vec![NatsHeader {
            name: "Nats-Msg-Id".into(),
            value: "42".into(),
        }];
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::NatsClient(client),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::NatsServer(NatsServerSpec {
                    template: None,
                    username: "u".into(),
                    password: "p".into(),
                    ..NatsServerSpec::default()
                }),
            },
            Command::Nats {
                session: SessionId(1),
                request: NatsRequest::Publish {
                    subject: "plant.limit".into(),
                    payload: "{\"kw\": 42.5}".into(),
                    headers: headers.clone(),
                },
            },
            Command::Nats {
                session: SessionId(1),
                request: NatsRequest::Request {
                    subject: "plant.inverter.get".into(),
                    payload: String::new(),
                    headers: Vec::new(),
                    timeout_ms: 2000,
                },
            },
            Command::Nats {
                session: SessionId(1),
                request: NatsRequest::Streams,
            },
            Command::Nats {
                session: SessionId(1),
                request: NatsRequest::Stream {
                    name: "PLANT".into(),
                    messages: 20,
                },
            },
            Command::Nats {
                session: SessionId(1),
                request: NatsRequest::Fetch {
                    stream: "PLANT".into(),
                    consumer: "historian".into(),
                    batch: 10,
                },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let stream = NatsStream {
            name: "PLANT".into(),
            subjects: vec!["plant.>".into()],
            messages: 1200,
            bytes: 9600,
            consumers: 1,
            first_seq: 801,
            last_seq: 2000,
            storage: "memory".into(),
        };
        let message = NatsStoredMessage {
            sequence: 2000,
            subject: "plant.inverter.ac_power".into(),
            payload: "1234.5".into(),
            at_ms: 1_790_000_000_000,
        };
        let events = [
            NatsEvent::Reply {
                subject: "plant.inverter.get".into(),
                result: Ok(NatsReply {
                    subject: "_INBOX.abc.1".into(),
                    payload: "{\"ac_power\":1234.5}".into(),
                    headers,
                    bytes: 19,
                    latency_us: 812,
                }),
            },
            NatsEvent::Reply {
                subject: "nobody.home".into(),
                result: Err("No reply within 2000 ms".into()),
            },
            NatsEvent::Streams(Ok(vec![stream.clone()])),
            NatsEvent::Streams(Err("JetStream is not enabled".into())),
            NatsEvent::Stream {
                name: "PLANT".into(),
                result: Ok(NatsStreamDetail {
                    stream,
                    consumers: vec![NatsConsumer {
                        name: "historian".into(),
                        durable: true,
                        kind: "pull".into(),
                        pending: 1200,
                        ack_pending: 0,
                        redelivered: 0,
                    }],
                    messages: vec![message.clone()],
                }),
            },
            NatsEvent::Fetched {
                stream: "PLANT".into(),
                consumer: "historian".into(),
                result: Ok(vec![message]),
            },
        ];
        for nats in events {
            let event = Event {
                session: SessionId(1),
                kind: EventKind::Nats(nats),
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // Optional fields may be missing from a saved workspace.
        let saved = r#"{"kind":"nats_client","host":"h","port":4222,
            "subscriptions":[{"subject":">"}]}"#;
        let SessionSpec::NatsClient(spec) = serde_json::from_str(saved).unwrap() else {
            panic!("a NATS client spec");
        };
        assert_eq!(spec.auth, NatsAuth::None);
        assert!(!spec.tls && spec.subscriptions[0].queue.is_empty());
    }
}
