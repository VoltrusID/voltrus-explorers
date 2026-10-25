use serde::{Deserialize, Serialize};

/// Connect to an MQTT broker and turn what it publishes into points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttClientSpec {
    pub host: String,
    pub port: u16,
    /// Empty: the engine picks a unique id per session.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// MQTTS, verified against the system's trust store.
    #[serde(default)]
    pub tls: bool,
    /// Seconds between keep-alive pings; 0 turns them off.
    pub keep_alive_s: u16,
    pub subscriptions: Vec<MqttSubscription>,
}

impl Default for MqttClientSpec {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 1883,
            client_id: String::new(),
            username: String::new(),
            password: String::new(),
            tls: false,
            keep_alive_s: 30,
            subscriptions: vec![MqttSubscription::new("#")],
        }
    }
}

/// A topic filter the client subscribes to. Every concrete topic it receives
/// becomes a point: a number, a boolean, each numeric field of a JSON object
/// (or only the one at `json_path`), or each metric of a Sparkplug B payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSubscription {
    pub filter: String,
    pub qos: MqttQos,
    /// JSON payloads: take only this field, e.g. `ac.power` or `phases[2].v`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub json_path: String,
}

impl MqttSubscription {
    pub fn new(filter: impl Into<String>) -> Self {
        Self {
            filter: filter.into(),
            qos: MqttQos::AtMostOnce,
            json_path: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MqttQos {
    #[default]
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

impl MqttQos {
    pub const ALL: [MqttQos; 3] = [
        MqttQos::AtMostOnce,
        MqttQos::AtLeastOnce,
        MqttQos::ExactlyOnce,
    ];

    pub fn level(self) -> u8 {
        match self {
            Self::AtMostOnce => 0,
            Self::AtLeastOnce => 1,
            Self::ExactlyOnce => 2,
        }
    }

    pub fn from_level(level: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|qos| qos.level() == level)
    }

    /// `QoS 0`, `QoS 1`, `QoS 2`.
    pub fn name(self) -> &'static str {
        match self {
            Self::AtMostOnce => "QoS 0",
            Self::AtLeastOnce => "QoS 1",
            Self::ExactlyOnce => "QoS 2",
        }
    }
}

/// Run an MQTT broker on this computer, optionally with a publisher that
/// simulates a device template.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttBrokerSpec {
    pub bind: String,
    pub port: u16,
    /// Device template the publisher simulates; `None` runs only the broker.
    pub template: Option<String>,
    /// Topic the device's values are published under, e.g. `plant/inverter`:
    /// one retained number per value at `<prefix>/<value>`, and a JSON
    /// nameplate at `<prefix>/status`.
    pub prefix: String,
    pub publish_ms: u64,
    /// Also publish the device as a Sparkplug B edge node.
    #[serde(default)]
    pub sparkplug: bool,
}

impl MqttBrokerSpec {
    pub const MIN_PUBLISH_MS: u64 = 100;

    /// A client that subscribes to everything the simulated device publishes.
    pub fn client(&self) -> MqttClientSpec {
        let host = match self.bind.as_str() {
            "0.0.0.0" => "127.0.0.1".to_owned(),
            "::" => "::1".to_owned(),
            bind => bind.to_owned(),
        };
        let mut subscriptions = vec![MqttSubscription::new(format!("{}/#", self.prefix))];
        if self.sparkplug {
            subscriptions.push(MqttSubscription::new(format!(
                "{SPARKPLUG_NAMESPACE}/{SPARKPLUG_GROUP}/#"
            )));
        }
        MqttClientSpec {
            host,
            port: self.port,
            subscriptions,
            ..MqttClientSpec::default()
        }
    }
}

impl Default for MqttBrokerSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 1883,
            template: Some("sma-inverter".into()),
            prefix: "plant/inverter".into(),
            publish_ms: 1000,
            sparkplug: false,
        }
    }
}

/// Sparkplug B topic namespace.
pub const SPARKPLUG_NAMESPACE: &str = "spBv1.0";
/// Sparkplug group the simulator's edge node belongs to.
pub const SPARKPLUG_GROUP: &str = "Voltrus";

/// Whether `filter` is a valid MQTT topic filter: not empty, `#` only as the
/// whole last level, `+` only as a whole level.
pub fn valid_filter(filter: &str) -> bool {
    if filter.is_empty() {
        return false;
    }
    let levels: Vec<&str> = filter.split('/').collect();
    levels.iter().enumerate().all(|(index, level)| {
        (!level.contains('#') || (*level == "#" && index == levels.len() - 1))
            && (!level.contains('+') || *level == "+")
    })
}

/// Whether `topic` is a valid topic to publish to: not empty, no wildcards.
pub fn valid_topic(topic: &str) -> bool {
    !topic.is_empty() && !topic.contains(['#', '+', '\0'])
}

/// One step of a JSON path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathStep {
    Field(String),
    Index(usize),
}

/// Parse a JSON path such as `ac.power`, `phases[2].v` or `[0]`.
pub fn parse_json_path(path: &str) -> Result<Vec<PathStep>, String> {
    let path = path.trim();
    let mut steps = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let end = after
                .find(']')
                .ok_or_else(|| format!("“{path}”: [ without ]"))?;
            let index = after[..end]
                .trim()
                .parse()
                .map_err(|_| format!("“{path}”: [{}] is not an index", &after[..end]))?;
            steps.push(PathStep::Index(index));
            rest = &after[end + 1..];
        } else {
            let rest_field = rest.strip_prefix('.').unwrap_or(rest);
            if rest_field.is_empty() || (steps.is_empty() && rest.starts_with('.')) {
                return Err(format!("“{path}”: a field name is missing"));
            }
            let end = rest_field.find(['.', '[']).unwrap_or(rest_field.len());
            let field = &rest_field[..end];
            if field.is_empty() {
                return Err(format!("“{path}”: a field name is missing"));
            }
            steps.push(PathStep::Field(field.to_owned()));
            rest = &rest_field[end..];
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_filters_follow_the_wildcard_rules() {
        for good in [
            "#",
            "a/#",
            "+/b",
            "a/+/c",
            "plant/inverter/ac_power",
            "a//b",
        ] {
            assert!(valid_filter(good), "{good}");
        }
        for bad in ["", "a/#/b", "a#", "a/b+", "#/a", "a/+x"] {
            assert!(!valid_filter(bad), "{bad}");
        }
        assert!(valid_topic("plant/x"));
        assert!(!valid_topic("plant/+"));
        assert!(!valid_topic(""));
    }

    #[test]
    fn json_paths_parse_fields_and_indexes() {
        assert_eq!(
            parse_json_path("phases[2].v"),
            Ok(vec![
                PathStep::Field("phases".into()),
                PathStep::Index(2),
                PathStep::Field("v".into()),
            ])
        );
        assert_eq!(parse_json_path("[0]"), Ok(vec![PathStep::Index(0)]));
        assert_eq!(parse_json_path(""), Ok(Vec::new()));
        assert!(parse_json_path("a.").is_err());
        assert!(parse_json_path(".a").is_err());
        assert!(parse_json_path("a[x]").is_err());
        assert!(parse_json_path("a[1").is_err());
        assert!(parse_json_path("a..b").is_err());
    }

    #[test]
    fn a_broker_client_subscribes_to_what_it_publishes() {
        let broker = MqttBrokerSpec {
            bind: "0.0.0.0".into(),
            sparkplug: true,
            ..MqttBrokerSpec::default()
        };
        let client = broker.client();
        assert_eq!(client.host, "127.0.0.1");
        assert_eq!(client.port, broker.port);
        let filters: Vec<&str> = client
            .subscriptions
            .iter()
            .map(|subscription| subscription.filter.as_str())
            .collect();
        assert_eq!(filters, ["plant/inverter/#", "spBv1.0/Voltrus/#"]);
    }
}
