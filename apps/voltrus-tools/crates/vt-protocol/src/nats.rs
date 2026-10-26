use serde::{Deserialize, Serialize};

/// The NATS client port.
pub const NATS_PORT: u16 = 4222;

/// Connect to a NATS server and turn the messages of its subjects into
/// points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsClientSpec {
    pub host: String,
    pub port: u16,
    /// `tls://`, verified against the built-in web trust roots.
    #[serde(default)]
    pub tls: bool,
    #[serde(default)]
    pub auth: NatsAuth,
    /// Connection name the server lists; empty: the engine picks one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// May be empty: a client that only publishes, requests and browses
    /// JetStream.
    pub subscriptions: Vec<NatsSubscription>,
}

impl Default for NatsClientSpec {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: NATS_PORT,
            tls: false,
            auth: NatsAuth::None,
            name: String::new(),
            subscriptions: vec![NatsSubscription::new(">")],
        }
    }
}

/// How a client proves who it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NatsAuth {
    #[default]
    None,
    Token {
        token: String,
    },
    UserPassword {
        username: String,
        password: String,
    },
    /// An NKey seed (`SU…`) that signs the server's nonce.
    Nkey {
        seed: String,
    },
    /// A decentralized-auth `.creds` file, read when it was chosen: `file`
    /// is its name for display, `creds` its contents (user JWT and seed).
    Credentials {
        file: String,
        creds: String,
    },
}

impl NatsAuth {
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Token { .. } => "Token",
            Self::UserPassword { .. } => "User",
            Self::Nkey { .. } => "NKey",
            Self::Credentials { .. } => "Creds",
        }
    }
}

/// A subject filter the client subscribes to. Every concrete subject it
/// receives becomes a point: a number, a boolean, or each numeric field of a
/// JSON message (only the one at `json_path` when given).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsSubscription {
    pub subject: String,
    /// Queue group: the server hands each message to one member only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub queue: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub json_path: String,
}

impl NatsSubscription {
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            queue: String::new(),
            json_path: String::new(),
        }
    }
}

/// Run a NATS server on this computer (with an in-memory JetStream),
/// optionally publishing a device template.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsServerSpec {
    pub bind: String,
    pub port: u16,
    /// Device template the publisher simulates; `None` runs only the server.
    pub template: Option<String>,
    /// Subject the device's values are published under, e.g.
    /// `plant.inverter`: a number per value at `<prefix>.<value>` and a JSON
    /// nameplate at `<prefix>.status`. `<prefix>.<value>.set` takes a value,
    /// `<prefix>.get` answers with every value.
    pub prefix: String,
    pub publish_ms: u64,
    /// Keep what the device publishes in a JetStream stream with a durable
    /// consumer, to browse.
    #[serde(default)]
    pub jetstream: bool,
    /// Clients must log in with this user when set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
}

impl NatsServerSpec {
    pub const MIN_PUBLISH_MS: u64 = 100;
    /// The durable consumer the simulator's stream comes with.
    pub const CONSUMER: &'static str = "historian";

    /// A client that subscribes to everything the simulated device publishes.
    pub fn client(&self) -> NatsClientSpec {
        let host = match self.bind.as_str() {
            "0.0.0.0" => "127.0.0.1".to_owned(),
            "::" => "::1".to_owned(),
            bind => bind.to_owned(),
        };
        NatsClientSpec {
            host,
            port: self.port,
            auth: if self.username.is_empty() {
                NatsAuth::None
            } else {
                NatsAuth::UserPassword {
                    username: self.username.clone(),
                    password: self.password.clone(),
                }
            },
            subscriptions: vec![NatsSubscription::new(format!("{}.>", self.prefix))],
            ..NatsClientSpec::default()
        }
    }

    /// Name of the JetStream stream that keeps the device's messages:
    /// the prefix in capitals, `plant.inverter` → `PLANT_INVERTER`.
    pub fn stream_name(&self) -> String {
        self.prefix
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect()
    }
}

impl Default for NatsServerSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: NATS_PORT,
            template: Some("sma-inverter".into()),
            prefix: "plant.inverter".into(),
            publish_ms: 1000,
            jetstream: true,
            username: String::new(),
            password: String::new(),
        }
    }
}

/// Whether `subject` is a valid NATS subscription subject: dot-separated
/// non-empty tokens without whitespace, `*` only as a whole token, `>` only
/// as the whole last token.
pub fn valid_subject_filter(subject: &str) -> bool {
    if subject.is_empty() || subject.chars().any(char::is_whitespace) {
        return false;
    }
    let tokens: Vec<&str> = subject.split('.').collect();
    tokens.iter().enumerate().all(|(index, token)| {
        !token.is_empty()
            && (!token.contains('*') || *token == "*")
            && (!token.contains('>') || (*token == ">" && index == tokens.len() - 1))
    })
}

/// Whether `subject` is a subject to publish to: a valid subject without
/// wildcards.
pub fn valid_subject(subject: &str) -> bool {
    valid_subject_filter(subject) && !subject.split('.').any(|token| token == "*" || token == ">")
}

/// One NATS message header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsHeader {
    pub name: String,
    pub value: String,
}

/// Parse `Name: value` headers separated by `;` or new lines.
pub fn parse_headers(text: &str) -> Result<Vec<NatsHeader>, String> {
    text.split([';', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("“{line}”: write headers as Name: value"))?;
            let name = name.trim();
            if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c.is_control()) {
                return Err(format!("“{name}” is not a header name"));
            }
            Ok(NatsHeader {
                name: name.to_owned(),
                value: value.trim().to_owned(),
            })
        })
        .collect()
}

/// What a NATS client is asked to do besides its subscriptions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum NatsRequest {
    /// Publish a message; answered by [`crate::EventKind::Published`].
    Publish {
        subject: String,
        payload: String,
        #[serde(default)]
        headers: Vec<NatsHeader>,
    },
    /// Send a request and wait for one reply; answered by
    /// [`NatsEvent::Reply`].
    Request {
        subject: String,
        payload: String,
        #[serde(default)]
        headers: Vec<NatsHeader>,
        timeout_ms: u64,
    },
    /// List JetStream streams; answered by [`NatsEvent::Streams`].
    Streams,
    /// A stream's consumers and newest messages; answered by
    /// [`NatsEvent::Stream`].
    Stream { name: String, messages: u32 },
    /// Pull the next messages of a durable consumer and acknowledge them;
    /// answered by [`NatsEvent::Fetched`].
    Fetch {
        stream: String,
        consumer: String,
        batch: u32,
    },
}

/// A NATS client's answers to [`NatsRequest`]s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "nats", content = "data", rename_all = "snake_case")]
pub enum NatsEvent {
    Reply {
        subject: String,
        result: Result<NatsReply, String>,
    },
    Streams(Result<Vec<NatsStream>, String>),
    Stream {
        name: String,
        result: Result<NatsStreamDetail, String>,
    },
    Fetched {
        stream: String,
        consumer: String,
        result: Result<Vec<NatsStoredMessage>, String>,
    },
}

/// The reply to a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsReply {
    /// The reply inbox it arrived on.
    pub subject: String,
    /// Text, or hex when binary, clipped for display.
    pub payload: String,
    pub headers: Vec<NatsHeader>,
    pub bytes: u32,
    /// Round trip from request to reply.
    pub latency_us: u32,
}

/// A JetStream stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsStream {
    pub name: String,
    pub subjects: Vec<String>,
    pub messages: u64,
    pub bytes: u64,
    pub consumers: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub storage: String,
}

/// A stream with its consumers and newest messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsStreamDetail {
    pub stream: NatsStream,
    pub consumers: Vec<NatsConsumer>,
    pub messages: Vec<NatsStoredMessage>,
}

/// A JetStream consumer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsConsumer {
    pub name: String,
    pub durable: bool,
    /// `pull` or `push`.
    pub kind: String,
    /// Messages not yet delivered.
    pub pending: u64,
    pub ack_pending: u64,
    pub redelivered: u64,
}

/// A message kept in a stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsStoredMessage {
    pub sequence: u64,
    pub subject: String,
    /// Text, or hex when binary, clipped for display.
    pub payload: String,
    pub at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_follow_the_wildcard_rules() {
        for good in [
            ">",
            "a.>",
            "*.b",
            "a.*.c",
            "plant.inverter.ac_power",
            "$JS.API.>",
        ] {
            assert!(valid_subject_filter(good), "{good}");
        }
        for bad in [
            "", "a..b", ".a", "a.", "a.>.b", "a>", "a.b*", "a b", "a.\tb",
        ] {
            assert!(!valid_subject_filter(bad), "{bad}");
        }
        assert!(valid_subject("plant.x"));
        assert!(!valid_subject("plant.*"));
        assert!(!valid_subject("plant.>"));
        assert!(!valid_subject(""));
    }

    #[test]
    fn headers_parse_name_value_pairs() {
        assert_eq!(
            parse_headers("Nats-Msg-Id: 42; X-Unit :kW\nEmpty:"),
            Ok(vec![
                NatsHeader {
                    name: "Nats-Msg-Id".into(),
                    value: "42".into()
                },
                NatsHeader {
                    name: "X-Unit".into(),
                    value: "kW".into()
                },
                NatsHeader {
                    name: "Empty".into(),
                    value: String::new()
                },
            ])
        );
        assert_eq!(parse_headers("  "), Ok(Vec::new()));
        // A value may hold a colon (a URL); only the first one splits.
        assert_eq!(
            parse_headers("Link: http://x:1")
                .ok()
                .and_then(|headers| headers.into_iter().next())
                .map(|header| header.value),
            Some("http://x:1".to_owned())
        );
        assert!(parse_headers("no colon").is_err());
        assert!(parse_headers(": v").is_err());
        assert!(parse_headers("Bad Name: v").is_err());
    }

    #[test]
    fn a_server_client_subscribes_to_what_it_publishes() {
        let server = NatsServerSpec {
            bind: "0.0.0.0".into(),
            username: "plant".into(),
            password: "pw".into(),
            ..NatsServerSpec::default()
        };
        let client = server.client();
        assert_eq!(client.host, "127.0.0.1");
        assert_eq!(client.port, server.port);
        assert_eq!(
            client.subscriptions,
            vec![NatsSubscription::new("plant.inverter.>")]
        );
        assert_eq!(
            client.auth,
            NatsAuth::UserPassword {
                username: "plant".into(),
                password: "pw".into()
            }
        );
        assert_eq!(server.stream_name(), "PLANT_INVERTER");
    }
}
