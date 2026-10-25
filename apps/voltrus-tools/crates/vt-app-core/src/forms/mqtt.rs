//! MQTT client and broker simulator forms.

use vt_protocol::{
    MqttBrokerSpec, MqttClientSpec, MqttQos, MqttSubscription, parse_json_path, valid_filter,
    valid_topic,
};

use super::{Field, FormError, error, name, number};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionField {
    Filter,
    JsonPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionForm {
    pub filter: String,
    pub qos: MqttQos,
    pub json_path: String,
}

impl SubscriptionForm {
    /// A blank row for a new subscription.
    pub fn blank() -> Self {
        Self {
            filter: String::new(),
            qos: MqttQos::AtMostOnce,
            json_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub tls: bool,
    pub keep_alive: String,
    pub subscriptions: Vec<SubscriptionForm>,
}

impl MqttClientForm {
    pub fn from_spec(name: &str, spec: &MqttClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            client_id: spec.client_id.clone(),
            username: spec.username.clone(),
            password: spec.password.clone(),
            tls: spec.tls,
            keep_alive: spec.keep_alive_s.to_string(),
            subscriptions: spec
                .subscriptions
                .iter()
                .map(|subscription| SubscriptionForm {
                    filter: subscription.filter.clone(),
                    qos: subscription.qos,
                    json_path: subscription.json_path.clone(),
                })
                .collect(),
        }
    }

    pub fn build(&self) -> Result<(String, MqttClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the broker's IP address or host name",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let username = self.username.trim();
        if username.is_empty() && !self.password.is_empty() {
            return Err(error(Field::Username, "A password needs a username"));
        }
        let keep_alive_s = number(&self.keep_alive, Field::KeepAlive, "Keep-alive", 0, 3600)?;
        if self.subscriptions.is_empty() {
            return Err(error(
                Field::Subscriptions,
                "Add at least one topic filter to subscribe to, e.g. #",
            ));
        }
        let mut subscriptions = Vec::with_capacity(self.subscriptions.len());
        for (index, row) in self.subscriptions.iter().enumerate() {
            let filter = row.filter.trim();
            if !valid_filter(filter) {
                return Err(error(
                    Field::Subscription(index, SubscriptionField::Filter),
                    if filter.is_empty() {
                        "Enter a topic filter, e.g. plant/+/power or #".to_owned()
                    } else {
                        format!(
                            "“{filter}” is not a topic filter: # goes last, + and # take a whole level"
                        )
                    },
                ));
            }
            let json_path = row.json_path.trim();
            if let Err(problem) = parse_json_path(json_path) {
                return Err(error(
                    Field::Subscription(index, SubscriptionField::JsonPath),
                    format!("JSON path {problem}"),
                ));
            }
            subscriptions.push(MqttSubscription {
                filter: filter.to_owned(),
                qos: row.qos,
                json_path: json_path.to_owned(),
            });
        }
        Ok((
            name,
            MqttClientSpec {
                host: host.to_owned(),
                port,
                client_id: self.client_id.trim().to_owned(),
                username: username.to_owned(),
                password: if username.is_empty() {
                    String::new()
                } else {
                    self.password.clone()
                },
                tls: self.tls,
                keep_alive_s,
                subscriptions,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttBrokerForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub template: Option<String>,
    pub prefix: String,
    pub publish_ms: String,
    pub sparkplug: bool,
}

impl MqttBrokerForm {
    pub fn from_spec(name: &str, spec: &MqttBrokerSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            template: spec.template.clone(),
            prefix: spec.prefix.clone(),
            publish_ms: spec.publish_ms.to_string(),
            sparkplug: spec.sparkplug,
        }
    }

    pub fn build(&self) -> Result<(String, MqttBrokerSpec), FormError> {
        let name = name(&self.name)?;
        let bind = self.bind.trim();
        let bare = bind
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(bind);
        let Ok(ip) = bare.parse::<std::net::IpAddr>() else {
            return Err(error(
                Field::Bind,
                "Listen on an IP address, e.g. 127.0.0.1, 0.0.0.0 or ::",
            ));
        };
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let prefix = self.prefix.trim().trim_matches('/');
        if self.template.is_some() && !valid_topic(prefix) {
            return Err(error(
                Field::Prefix,
                "Enter the topic the device publishes under, e.g. plant/inverter (no + or #)",
            ));
        }
        let publish_ms = number(
            &self.publish_ms,
            Field::PublishMs,
            "Publish interval",
            MqttBrokerSpec::MIN_PUBLISH_MS,
            60_000,
        )?;
        Ok((
            name,
            MqttBrokerSpec {
                bind: ip.to_string(),
                port,
                template: self.template.clone(),
                prefix: prefix.to_owned(),
                publish_ms,
                sparkplug: self.sparkplug && self.template.is_some(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_forms_round_trip_to_the_same_specs() {
        let client = MqttClientSpec::default();
        assert_eq!(
            MqttClientForm::from_spec("Broker", &client).build(),
            Ok(("Broker".to_owned(), client))
        );
        let broker = MqttBrokerSpec::default();
        assert_eq!(
            MqttBrokerForm::from_spec("Sim", &broker).build(),
            Ok(("Sim".to_owned(), broker))
        );
    }

    #[test]
    fn client_errors_point_at_the_offending_field() {
        let form = || MqttClientForm::from_spec("Broker", &MqttClientSpec::default());

        let mut bad = form();
        bad.subscriptions[0].filter = "plant/#/power".into();
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Subscription(0, SubscriptionField::Filter)
        );

        let mut bad = form();
        bad.subscriptions.push(SubscriptionForm {
            filter: "meter".into(),
            qos: MqttQos::AtLeastOnce,
            json_path: "phases[x]".into(),
        });
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Subscription(1, SubscriptionField::JsonPath)
        );

        let mut bad = form();
        bad.subscriptions.clear();
        assert_eq!(bad.build().unwrap_err().field, Field::Subscriptions);

        let mut bad = form();
        bad.password = "secret".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Username);

        let mut bad = form();
        bad.keep_alive = "5000".into();
        assert_eq!(bad.build().unwrap_err().field, Field::KeepAlive);
    }

    #[test]
    fn client_fields_are_trimmed() {
        let mut form = MqttClientForm::from_spec("Broker", &MqttClientSpec::default());
        form.host = " broker.local ".into();
        form.username = " operator ".into();
        form.password = " keep spaces ".into();
        form.subscriptions[0].filter = " plant/+/power ".into();
        form.subscriptions[0].json_path = " ac.power ".into();
        let (_, spec) = form.build().expect("a valid form");
        assert_eq!(spec.host, "broker.local");
        assert_eq!(spec.username, "operator");
        assert_eq!(
            spec.password, " keep spaces ",
            "passwords are taken as typed"
        );
        assert_eq!(spec.subscriptions[0].filter, "plant/+/power");
        assert_eq!(spec.subscriptions[0].json_path, "ac.power");
    }

    #[test]
    fn broker_prefixes_are_concrete_topics() {
        let mut form = MqttBrokerForm::from_spec("Sim", &MqttBrokerSpec::default());
        form.prefix = "/plant/inverter/".into();
        assert_eq!(
            form.build().expect("a valid prefix").1.prefix,
            "plant/inverter"
        );
        form.prefix = "plant/+".into();
        assert_eq!(form.build().unwrap_err().field, Field::Prefix);
        // A broker without a device publishes nothing: no prefix needed, and
        // nothing to publish as Sparkplug.
        form.template = None;
        form.sparkplug = true;
        let (_, spec) = form.build().expect("a plain broker");
        assert!(!spec.sparkplug);
        form.bind = "localhost".into();
        assert_eq!(form.build().unwrap_err().field, Field::Bind);
        form.bind = "[::1]".into();
        form.publish_ms = "50".into();
        assert_eq!(form.build().unwrap_err().field, Field::PublishMs);
    }
}
