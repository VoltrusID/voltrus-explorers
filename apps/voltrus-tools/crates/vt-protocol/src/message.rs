use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BacnetDeviceInfo, BacnetDiscoverSpec, MqttQos, NatsEvent, NatsRequest, PointInfo, SessionSpec,
    TrafficFrame, ValueFrame,
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
