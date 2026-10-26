use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BacnetDeviceInfo, BacnetDiscoverSpec, CipRequest, CipResponse, Dnp3Action, Dnp3Iin,
    EnipDeviceInfo, EnipDiscoverSpec, MqttQos, PointInfo, SessionSpec, TrafficFrame, ValueFrame,
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
    /// OPC UA clients: browse, describe, monitor, write and call (answered by
    /// [`EventKind::Opcua`] and the usual point events). An
    /// [`OpcuaRequest::Endpoints`] needs no session: its answer comes back
    /// under `session`, which names the request.
    Opcua {
        session: SessionId,
        request: crate::OpcuaRequest,
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
    /// Answers to [`Command::Opcua`].
    Opcua(crate::OpcuaEvent),
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
    fn opcua_messages_round_trip_through_json() {
        use crate::{
            OpcuaArgument, OpcuaAttribute, OpcuaAuth, OpcuaBrowseNode, OpcuaClientSpec,
            OpcuaEndpoint, OpcuaEvent, OpcuaMethodInfo, OpcuaNode, OpcuaNodeClass, OpcuaRequest,
            OpcuaSecurityMode, OpcuaSecurityPolicy, OpcuaServerSpec,
        };

        let client = OpcuaClientSpec {
            endpoint: "opc.tcp://plc.local:4840/UA".into(),
            policy: OpcuaSecurityPolicy::Aes256Sha256RsaPss,
            mode: OpcuaSecurityMode::SignAndEncrypt,
            auth: OpcuaAuth::Certificate {
                certificate: "/Users/me/operator.der".into(),
                private_key: "/Users/me/operator.pem".into(),
            },
            publishing_ms: 1000,
            sampling_ms: 100,
            nodes: vec![
                OpcuaNode::new("ns=2;s=Plant.Reactor.Temperature", "Reactor › Temperature"),
                OpcuaNode::new("i=2258", ""),
            ],
        };
        let commands = [
            Command::Open {
                session: SessionId(1),
                spec: SessionSpec::OpcuaClient(client),
            },
            Command::Open {
                session: SessionId(2),
                spec: SessionSpec::OpcuaServer(OpcuaServerSpec::default()),
            },
            Command::Opcua {
                session: SessionId(3),
                request: OpcuaRequest::Endpoints {
                    url: "opc.tcp://localhost:4840".into(),
                },
            },
            Command::Opcua {
                session: SessionId(1),
                request: OpcuaRequest::Browse {
                    node_id: "Objects".into(),
                },
            },
            Command::Opcua {
                session: SessionId(1),
                request: OpcuaRequest::Monitor {
                    nodes: vec![OpcuaNode::new("ns=2;s=Plant.Boiler.SteamPressure", "")],
                },
            },
            Command::Opcua {
                session: SessionId(1),
                request: OpcuaRequest::Write {
                    point: 4,
                    text: "  batch 7 ".into(),
                },
            },
            Command::Opcua {
                session: SessionId(1),
                request: OpcuaRequest::Call {
                    object_id: "ns=2;s=Plant.Reactor".into(),
                    method_id: "ns=2;s=Plant.Reactor.StartBatch".into(),
                    arguments: vec!["Pale ale".into(), "250".into()],
                },
            },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }

        let events = [
            OpcuaEvent::Endpoints(Ok(vec![OpcuaEndpoint {
                url: "opc.tcp://localhost:4840/".into(),
                policy: Some(OpcuaSecurityPolicy::Basic256Sha256),
                policy_uri: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256".into(),
                mode: OpcuaSecurityMode::Sign,
                security_level: 3,
                user_tokens: vec!["Anonymous".into(), "UserName".into()],
            }])),
            OpcuaEvent::Endpoints(Err("BadTcpEndpointUrlInvalid".into())),
            OpcuaEvent::Children {
                parent: "i=85".into(),
                result: Ok(vec![OpcuaBrowseNode {
                    node_id: "ns=2;s=Plant".into(),
                    browse_name: "Plant".into(),
                    display_name: "Plant".into(),
                    class: OpcuaNodeClass::Object,
                    data_type: String::new(),
                    has_children: true,
                }]),
            },
            OpcuaEvent::Attributes {
                node_id: "i=2258".into(),
                result: Ok(vec![OpcuaAttribute {
                    name: "Value".into(),
                    value: "2026-10-26T08:30:00Z".into(),
                    status: "Good".into(),
                }]),
            },
            OpcuaEvent::Monitored(vec![
                ("ns=2;s=Plant.Reactor.Level".into(), Ok(())),
                ("ns=2;s=Nope".into(), Err("BadNodeIdUnknown".into())),
            ]),
            OpcuaEvent::Method {
                method_id: "ns=2;s=Plant.Reactor.StartBatch".into(),
                result: Ok(OpcuaMethodInfo {
                    inputs: vec![OpcuaArgument {
                        name: "Quantity".into(),
                        data_type: "Double".into(),
                        description: "Litres".into(),
                    }],
                    outputs: Vec::new(),
                }),
            },
            OpcuaEvent::Called {
                method_id: "ns=2;s=Plant.Reactor.StartBatch".into(),
                result: Err("BadInvalidArgument".into()),
            },
        ];
        for answer in events {
            let event = Event {
                session: SessionId(1),
                kind: EventKind::Opcua(answer),
            };
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
        }

        // A saved user-name client keeps its credentials; a saved server without a user has none.
        let saved = r#"{"kind":"opcua_server","port":4841,"anonymous":true}"#;
        let SessionSpec::OpcuaServer(server) = serde_json::from_str(saved).unwrap() else {
            panic!("an OPC UA server spec");
        };
        assert!(server.username.is_empty() && server.password.is_empty());
    }
}
