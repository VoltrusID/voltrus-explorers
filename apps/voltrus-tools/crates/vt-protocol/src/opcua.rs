use serde::{Deserialize, Serialize};

/// The registered OPC UA TCP port.
pub const OPCUA_PORT: u16 = 4840;

/// How messages between client and server are secured (the endpoint's SecurityPolicy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcuaSecurityPolicy {
    #[default]
    None,
    Basic128Rsa15,
    Basic256,
    Basic256Sha256,
    Aes128Sha256RsaOaep,
    Aes256Sha256RsaPss,
}

impl OpcuaSecurityPolicy {
    pub const ALL: [OpcuaSecurityPolicy; 6] = [
        Self::None,
        Self::Basic128Rsa15,
        Self::Basic256,
        Self::Basic256Sha256,
        Self::Aes128Sha256RsaOaep,
        Self::Aes256Sha256RsaPss,
    ];

    /// The name after the `#` of the policy URI, e.g. `Basic256Sha256`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Basic128Rsa15 => "Basic128Rsa15",
            Self::Basic256 => "Basic256",
            Self::Basic256Sha256 => "Basic256Sha256",
            Self::Aes128Sha256RsaOaep => "Aes128_Sha256_RsaOaep",
            Self::Aes256Sha256RsaPss => "Aes256_Sha256_RsaPss",
        }
    }

    /// Basic128Rsa15 and Basic256 use SHA-1 and are deprecated by the OPC Foundation.
    pub fn deprecated(self) -> bool {
        matches!(self, Self::Basic128Rsa15 | Self::Basic256)
    }

    /// The policy of a SecurityPolicyUri (`http://opcfoundation.org/UA/SecurityPolicy#None`).
    pub fn from_uri(uri: &str) -> Option<Self> {
        let name = uri.rsplit('#').next()?;
        Self::ALL.into_iter().find(|policy| policy.name() == name)
    }
}

/// Whether messages are signed, or signed and encrypted (the endpoint's MessageSecurityMode).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcuaSecurityMode {
    #[default]
    None,
    Sign,
    SignAndEncrypt,
}

impl OpcuaSecurityMode {
    pub const ALL: [OpcuaSecurityMode; 3] = [Self::None, Self::Sign, Self::SignAndEncrypt];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Sign => "Sign",
            Self::SignAndEncrypt => "Sign & Encrypt",
        }
    }

    /// The enumeration name the standard (and the protocol crate) uses, e.g. `SignAndEncrypt`.
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Sign => "Sign",
            Self::SignAndEncrypt => "SignAndEncrypt",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

/// Who the client signs in as.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpcuaAuth {
    #[default]
    Anonymous,
    UserName {
        username: String,
        password: String,
    },
    /// An X.509 user certificate (DER or PEM file) and its private key (PEM file).
    Certificate {
        certificate: String,
        private_key: String,
    },
}

impl OpcuaAuth {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Anonymous => "Anonymous",
            Self::UserName { .. } => "User name",
            Self::Certificate { .. } => "Certificate",
        }
    }
}

/// A variable the client monitors: one point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaNode {
    /// As the server prints it, e.g. `ns=2;s=Plant.Reactor.Temperature` or `i=2258`.
    pub node_id: String,
    /// Shown in the points table; empty takes the variable's display name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl OpcuaNode {
    pub fn new(node_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            name: name.into(),
        }
    }
}

/// Connect to an OPC UA server, browse its address space and monitor variables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaClientSpec {
    /// `opc.tcp://host:port/path`.
    pub endpoint: String,
    pub policy: OpcuaSecurityPolicy,
    pub mode: OpcuaSecurityMode,
    pub auth: OpcuaAuth,
    /// How often the server sends the subscription's changes.
    pub publishing_ms: u32,
    /// How often the server samples each monitored variable.
    pub sampling_ms: u32,
    pub nodes: Vec<OpcuaNode>,
}

impl Default for OpcuaClientSpec {
    fn default() -> Self {
        Self {
            endpoint: format!("opc.tcp://localhost:{OPCUA_PORT}"),
            policy: OpcuaSecurityPolicy::None,
            mode: OpcuaSecurityMode::None,
            auth: OpcuaAuth::Anonymous,
            publishing_ms: 500,
            sampling_ms: 250,
            nodes: Vec::new(),
        }
    }
}

/// The `host:port` of an `opc.tcp://` URL, if it is one.
pub fn opcua_host_port(url: &str) -> Option<&str> {
    let rest = url.trim().strip_prefix("opc.tcp://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    (!authority.is_empty() && !authority.starts_with(':')).then_some(authority)
}

/// Run an OPC UA server on this computer serving a simulated plant: folders, machines,
/// variables with units and ranges, and methods.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaServerSpec {
    /// Address to listen on: `127.0.0.1` for this computer only, `::` or `0.0.0.0` for every
    /// interface. Specs saved before it existed listen on this computer only.
    #[serde(default = "loopback")]
    pub bind: String,
    pub port: u16,
    /// Clients may sign in without a user.
    pub anonymous: bool,
    /// A user-name user; an empty name turns user-name sign-in off.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
}

fn loopback() -> String {
    "127.0.0.1".into()
}

impl Default for OpcuaServerSpec {
    fn default() -> Self {
        Self {
            bind: loopback(),
            port: OPCUA_PORT,
            anonymous: true,
            username: "operator".into(),
            password: "voltrus".into(),
        }
    }
}

impl OpcuaServerSpec {
    /// The host a client on this computer reaches the server by: the bind address itself, or
    /// `localhost` when the server listens on every interface.
    pub fn host(&self) -> String {
        let bind = self.bind.trim();
        let bare = bind
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(bind);
        match bare.parse::<std::net::IpAddr>() {
            Ok(ip) if ip.is_unspecified() => "localhost".into(),
            Ok(std::net::IpAddr::V6(v6)) => format!("[{v6}]"),
            Ok(std::net::IpAddr::V4(v4)) => v4.to_string(),
            Err(_) => "localhost".into(),
        }
    }

    /// A client of this server on this computer, signed in the way the server allows first.
    pub fn client(&self, nodes: Vec<OpcuaNode>) -> OpcuaClientSpec {
        OpcuaClientSpec {
            endpoint: format!("opc.tcp://{}:{}", self.host(), self.port),
            auth: if self.anonymous || self.username.is_empty() {
                OpcuaAuth::Anonymous
            } else {
                OpcuaAuth::UserName {
                    username: self.username.clone(),
                    password: self.password.clone(),
                }
            },
            nodes,
            ..OpcuaClientSpec::default()
        }
    }
}

/// App → engine requests of the OPC UA client (and discovery, which needs no session).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum OpcuaRequest {
    /// GetEndpoints from a server: answered with [`OpcuaEvent::Endpoints`] under the command's
    /// session id, which names the request (no session runs under it).
    Endpoints { url: String },
    /// One level of the address space under `node_id` (`Objects` for the objects folder).
    Browse { node_id: String },
    /// A node's attributes and properties, for the inspector.
    Attributes { node_id: String },
    /// Monitor more variables: they join the session's points.
    Monitor { nodes: Vec<OpcuaNode> },
    /// Write `text` (parsed as the point's data type) to a point.
    Write { point: u32, text: String },
    /// A method's input and output arguments.
    Method { method_id: String },
    /// Call a method of an object with arguments typed as text.
    Call {
        object_id: String,
        method_id: String,
        arguments: Vec<String>,
    },
}

/// What a node is (its NodeClass).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcuaNodeClass {
    Object,
    Variable,
    Method,
    ObjectType,
    VariableType,
    ReferenceType,
    DataType,
    View,
    Unspecified,
}

impl OpcuaNodeClass {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Object" => Self::Object,
            "Variable" => Self::Variable,
            "Method" => Self::Method,
            "ObjectType" => Self::ObjectType,
            "VariableType" => Self::VariableType,
            "ReferenceType" => Self::ReferenceType,
            "DataType" => Self::DataType,
            "View" => Self::View,
            _ => Self::Unspecified,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::Variable => "Variable",
            Self::Method => "Method",
            Self::ObjectType => "ObjectType",
            Self::VariableType => "VariableType",
            Self::ReferenceType => "ReferenceType",
            Self::DataType => "DataType",
            Self::View => "View",
            Self::Unspecified => "Node",
        }
    }
}

/// An endpoint a server offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaEndpoint {
    pub url: String,
    /// `None` for a policy this app does not speak.
    pub policy: Option<OpcuaSecurityPolicy>,
    pub policy_uri: String,
    pub mode: OpcuaSecurityMode,
    /// The server's ranking: higher is more secure.
    pub security_level: u8,
    /// How users may sign in: `Anonymous`, `UserName`, `Certificate`, `IssuedToken`.
    pub user_tokens: Vec<String>,
}

/// One child of a browsed node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaBrowseNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub class: OpcuaNodeClass,
    /// Variables: their data type, e.g. `Double`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data_type: String,
    pub has_children: bool,
}

/// An attribute or property of a node, as text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaAttribute {
    pub name: String,
    pub value: String,
    /// `Good`, or the status code that kept the attribute from being read.
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaArgument {
    pub name: String,
    pub data_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcuaMethodInfo {
    pub inputs: Vec<OpcuaArgument>,
    pub outputs: Vec<OpcuaArgument>,
}

/// Engine → app answers of the OPC UA client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", content = "data", rename_all = "snake_case")]
pub enum OpcuaEvent {
    Endpoints(Result<Vec<OpcuaEndpoint>, String>),
    Children {
        parent: String,
        result: Result<Vec<OpcuaBrowseNode>, String>,
    },
    Attributes {
        node_id: String,
        result: Result<Vec<OpcuaAttribute>, String>,
    },
    /// Which variables of a [`OpcuaRequest::Monitor`] became points and why others did not.
    Monitored(Vec<(String, Result<(), String>)>),
    Method {
        method_id: String,
        result: Result<OpcuaMethodInfo, String>,
    },
    Called {
        method_id: String,
        result: Result<Vec<String>, String>,
    },
}

/// A value typed as text, parsed for a data type: what writes and method arguments send.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OpcuaScalar {
    Boolean(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
}

/// Whether values of `data_type` can be typed as text and written.
pub fn opcua_writable_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "Boolean"
            | "SByte"
            | "Byte"
            | "Int16"
            | "UInt16"
            | "Int32"
            | "UInt32"
            | "Int64"
            | "UInt64"
            | "Float"
            | "Double"
            | "String"
            | "LocalizedText"
            | "DateTime"
    )
}

/// Parse `text` as a value of the built-in type `data_type`, checking the type's range.
pub fn parse_opcua_value(data_type: &str, text: &str) -> Result<OpcuaScalar, String> {
    let trimmed = text.trim();
    let signed = |min: i64, max: i64| -> Result<OpcuaScalar, String> {
        match trimmed.parse::<i64>() {
            Ok(value) if (min..=max).contains(&value) => Ok(OpcuaScalar::Int(value)),
            _ => Err(format!(
                "“{trimmed}” is not a {data_type}: a whole number from {min} to {max}"
            )),
        }
    };
    let unsigned = |max: u64| -> Result<OpcuaScalar, String> {
        match trimmed.parse::<u64>() {
            Ok(value) if value <= max => Ok(OpcuaScalar::UInt(value)),
            _ => Err(format!(
                "“{trimmed}” is not a {data_type}: a whole number from 0 to {max}"
            )),
        }
    };
    match data_type {
        "Boolean" => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "1" | "on" => Ok(OpcuaScalar::Boolean(true)),
            "false" | "0" | "off" => Ok(OpcuaScalar::Boolean(false)),
            _ => Err(format!("“{trimmed}” is not a Boolean: true or false")),
        },
        "SByte" => signed(i64::from(i8::MIN), i64::from(i8::MAX)),
        "Byte" => unsigned(u64::from(u8::MAX)),
        "Int16" => signed(i64::from(i16::MIN), i64::from(i16::MAX)),
        "UInt16" => unsigned(u64::from(u16::MAX)),
        "Int32" => signed(i64::from(i32::MIN), i64::from(i32::MAX)),
        "UInt32" => unsigned(u64::from(u32::MAX)),
        "Int64" => signed(i64::MIN, i64::MAX),
        "UInt64" => unsigned(u64::MAX),
        "Float" | "Double" => match trimmed.parse::<f64>() {
            Ok(value) if value.is_finite() => {
                if data_type == "Float" && value.abs() > f64::from(f32::MAX) {
                    Err(format!("“{trimmed}” is too large for a Float"))
                } else {
                    Ok(OpcuaScalar::Float(value))
                }
            }
            _ => Err(format!("“{trimmed}” is not a number")),
        },
        // Text keeps its spaces.
        "String" | "LocalizedText" => Ok(OpcuaScalar::Text(text.to_owned())),
        "DateTime" => {
            if looks_like_rfc3339(trimmed) {
                Ok(OpcuaScalar::Text(trimmed.to_owned()))
            } else {
                Err(format!(
                    "“{trimmed}” is not a time like 2026-10-26T08:30:00Z"
                ))
            }
        }
        "" | "Unknown" => {
            Err("The variable's data type is unknown, so it cannot be written".into())
        }
        other => Err(format!("Writing {other} values is not supported")),
    }
}

/// `YYYY-MM-DDTHH:MM:SS` followed by optional fractions and a `Z` or `±HH:MM` offset; the server
/// side parses it fully.
fn looks_like_rfc3339(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    bytes.len() >= 20
        && digits(0..4)
        && bytes[4] == b'-'
        && digits(5..7)
        && bytes[7] == b'-'
        && digits(8..10)
        && matches!(bytes[10], b'T' | b't' | b' ')
        && digits(11..13)
        && bytes[13] == b':'
        && digits(14..16)
        && bytes[16] == b':'
        && digits(17..19)
        && (text.ends_with('Z') || text.ends_with('z') || text[19..].contains(['+', '-']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_come_back_from_their_uris() {
        assert_eq!(
            OpcuaSecurityPolicy::from_uri(
                "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256"
            ),
            Some(OpcuaSecurityPolicy::Basic256Sha256)
        );
        assert_eq!(
            OpcuaSecurityPolicy::from_uri(
                "http://opcfoundation.org/UA/SecurityPolicy#Aes256_Sha256_RsaPss"
            ),
            Some(OpcuaSecurityPolicy::Aes256Sha256RsaPss)
        );
        assert_eq!(
            OpcuaSecurityPolicy::from_uri("http://opcfoundation.org/UA/SecurityPolicy#None"),
            Some(OpcuaSecurityPolicy::None)
        );
        assert_eq!(OpcuaSecurityPolicy::from_uri("urn:vendor#Custom"), None);
        assert!(OpcuaSecurityPolicy::Basic256.deprecated());
        assert!(!OpcuaSecurityPolicy::Basic256Sha256.deprecated());
        assert_eq!(
            OpcuaSecurityMode::from_key("SignAndEncrypt"),
            Some(OpcuaSecurityMode::SignAndEncrypt)
        );
        assert_eq!(OpcuaSecurityMode::from_key("Invalid"), None);
    }

    #[test]
    fn integers_stay_inside_their_type() {
        assert_eq!(
            parse_opcua_value("SByte", "-128"),
            Ok(OpcuaScalar::Int(-128))
        );
        assert!(parse_opcua_value("SByte", "128").is_err());
        assert_eq!(parse_opcua_value("Byte", "255"), Ok(OpcuaScalar::UInt(255)));
        assert!(parse_opcua_value("Byte", "-1").is_err());
        assert!(parse_opcua_value("UInt16", "65536").is_err());
        assert_eq!(
            parse_opcua_value("Int64", "-9223372036854775808"),
            Ok(OpcuaScalar::Int(i64::MIN))
        );
        assert_eq!(
            parse_opcua_value("UInt64", "18446744073709551615"),
            Ok(OpcuaScalar::UInt(u64::MAX))
        );
        assert!(parse_opcua_value("Int32", "1.5").is_err());
        assert!(
            parse_opcua_value("UInt32", " 7 ").is_ok(),
            "spaces around are fine"
        );
    }

    #[test]
    fn booleans_numbers_text_and_times() {
        assert_eq!(
            parse_opcua_value("Boolean", "ON"),
            Ok(OpcuaScalar::Boolean(true))
        );
        assert_eq!(
            parse_opcua_value("Boolean", "0"),
            Ok(OpcuaScalar::Boolean(false))
        );
        assert!(parse_opcua_value("Boolean", "yes please").is_err());
        assert_eq!(
            parse_opcua_value("Double", "-1e3"),
            Ok(OpcuaScalar::Float(-1000.0))
        );
        assert!(parse_opcua_value("Double", "NaN").is_err());
        assert!(parse_opcua_value("Double", "inf").is_err());
        assert!(parse_opcua_value("Float", "1e39").is_err());
        assert_eq!(
            parse_opcua_value("String", "  batch 7 "),
            Ok(OpcuaScalar::Text("  batch 7 ".into()))
        );
        assert!(parse_opcua_value("DateTime", "2026-10-26T08:30:00Z").is_ok());
        assert!(parse_opcua_value("DateTime", "2026-10-26T08:30:00.250+07:00").is_ok());
        assert!(parse_opcua_value("DateTime", "yesterday").is_err());
        assert!(parse_opcua_value("Unknown", "1").is_err());
        assert!(parse_opcua_value("Guid", "x").is_err());
        assert!(opcua_writable_type("UInt32"));
        assert!(!opcua_writable_type("ns=2;i=3001"));
    }

    #[test]
    fn endpoints_name_their_host_and_port() {
        assert_eq!(
            opcua_host_port("opc.tcp://plc.local:4840/UA/Server"),
            Some("plc.local:4840")
        );
        assert_eq!(opcua_host_port("opc.tcp://[::1]:4841"), Some("[::1]:4841"));
        assert_eq!(opcua_host_port("http://plc.local"), None);
        assert_eq!(opcua_host_port("opc.tcp://"), None);
    }

    #[test]
    fn servers_listen_on_this_computer_unless_told_otherwise() {
        assert_eq!(OpcuaServerSpec::default().bind, "127.0.0.1");
        // A spec saved before the bind address existed stays on this computer.
        let old: OpcuaServerSpec =
            serde_json::from_str(r#"{"port":4840,"anonymous":true}"#).unwrap();
        assert_eq!(old.bind, "127.0.0.1");
        assert_eq!(old.port, 4840);
    }

    #[test]
    fn a_server_spec_makes_a_client_that_can_sign_in() {
        let open = OpcuaServerSpec::default();
        assert_eq!(open.client(Vec::new()).auth, OpcuaAuth::Anonymous);
        let locked = OpcuaServerSpec {
            port: 4850,
            anonymous: false,
            ..OpcuaServerSpec::default()
        };
        let client = locked.client(Vec::new());
        assert_eq!(client.endpoint, "opc.tcp://127.0.0.1:4850");
        let everywhere = OpcuaServerSpec {
            bind: "::".into(),
            ..OpcuaServerSpec::default()
        };
        assert_eq!(
            everywhere.client(Vec::new()).endpoint,
            format!("opc.tcp://localhost:{OPCUA_PORT}")
        );
        assert!(
            matches!(client.auth, OpcuaAuth::UserName { ref username, .. } if username == "operator")
        );
    }
}
