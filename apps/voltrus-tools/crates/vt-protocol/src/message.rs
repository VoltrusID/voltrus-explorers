use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BacnetDeviceInfo, BacnetDiscoverSpec, PointInfo, SessionSpec, TrafficFrame, ValueFrame,
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
    /// Answer to [`Command::DiscoverBacnet`]: the devices that sent an I-Am.
    BacnetDevices(Result<Vec<BacnetDeviceInfo>, String>),
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
                spec: crate::BacnetDiscoverSpec {
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
            Ok(vec![crate::BacnetDeviceInfo {
                instance: 1001,
                address: "192.168.1.40".into(),
                port: 47808,
                vendor: "ASHRAE".into(),
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
}
