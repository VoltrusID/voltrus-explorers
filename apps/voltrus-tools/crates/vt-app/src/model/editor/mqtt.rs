//! The MQTT client and broker simulator fields of the connection editor.

use gpui::{App, AppContext as _, Context, Entity, Window};
use vt_app_core::forms::{MqttBrokerForm, MqttClientForm, SubscriptionForm};
use vt_protocol::{MqttBrokerSpec, MqttClientSpec, MqttQos};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, Editor, input};

pub struct SubscriptionRow {
    pub filter: Entity<TextInput>,
    pub json_path: Entity<TextInput>,
    pub qos: MqttQos,
}

impl SubscriptionRow {
    fn new(form: SubscriptionForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            filter: input(form.filter, "plant/+/power or #", InputKind::Text, true, cx),
            json_path: input(form.json_path, "every number", InputKind::Text, true, cx),
            qos: form.qos,
        }
    }
}

pub struct MqttFields {
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub client_id: Entity<TextInput>,
    pub username: Entity<TextInput>,
    pub password: Entity<TextInput>,
    pub keep_alive: Entity<TextInput>,
    pub tls: bool,
    pub subscriptions: Vec<SubscriptionRow>,
    pub bind: Entity<TextInput>,
    pub broker_port: Entity<TextInput>,
    pub prefix: Entity<TextInput>,
    pub publish_ms: Entity<TextInput>,
    pub template: Option<String>,
    pub sparkplug: bool,
}

impl MqttFields {
    pub(super) fn new(
        client: &MqttClientSpec,
        broker: &MqttBrokerSpec,
        cx: &mut Context<AppModel>,
    ) -> Self {
        let client = MqttClientForm::from_spec("", client);
        let broker = MqttBrokerForm::from_spec("", broker);
        let password = client.password.clone();
        Self {
            host: input(client.host, "broker.local", InputKind::Text, true, cx),
            port: input(client.port, "1883", InputKind::Unsigned, true, cx),
            client_id: input(client.client_id, "automatic", InputKind::Text, true, cx),
            username: input(client.username, "none", InputKind::Text, false, cx),
            password: cx.new(|cx| {
                TextInput::new(cx)
                    .text(password)
                    .placeholder("none")
                    .secure()
            }),
            keep_alive: input(client.keep_alive, "30", InputKind::Unsigned, true, cx),
            tls: client.tls,
            subscriptions: client
                .subscriptions
                .into_iter()
                .map(|form| SubscriptionRow::new(form, cx))
                .collect(),
            bind: input(broker.bind, "127.0.0.1", InputKind::Text, true, cx),
            broker_port: input(broker.port, "1883", InputKind::Unsigned, true, cx),
            prefix: input(broker.prefix, "plant/inverter", InputKind::Text, true, cx),
            publish_ms: input(broker.publish_ms, "1000", InputKind::Unsigned, true, cx),
            template: broker.template,
            sparkplug: broker.sparkplug,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.host.clone(),
            self.port.clone(),
            self.client_id.clone(),
            self.username.clone(),
            self.password.clone(),
            self.keep_alive.clone(),
            self.bind.clone(),
            self.broker_port.clone(),
            self.prefix.clone(),
            self.publish_ms.clone(),
        ];
        for row in &self.subscriptions {
            fields.push(row.filter.clone());
            fields.push(row.json_path.clone());
        }
        fields
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> MqttClientForm {
        MqttClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            client_id: Self::text(&self.client_id, cx),
            username: Self::text(&self.username, cx),
            password: Self::text(&self.password, cx),
            tls: self.tls,
            keep_alive: Self::text(&self.keep_alive, cx),
            subscriptions: self
                .subscriptions
                .iter()
                .map(|row| SubscriptionForm {
                    filter: Self::text(&row.filter, cx),
                    qos: row.qos,
                    json_path: Self::text(&row.json_path, cx),
                })
                .collect(),
        }
    }

    pub fn broker_form(&self, name: String, cx: &App) -> MqttBrokerForm {
        MqttBrokerForm {
            name,
            bind: Self::text(&self.bind, cx),
            port: Self::text(&self.broker_port, cx),
            template: self.template.clone(),
            prefix: Self::text(&self.prefix, cx),
            publish_ms: Self::text(&self.publish_ms, cx),
            sparkplug: self.sparkplug,
        }
    }
}

impl Editor {
    pub fn add_subscription(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let row = SubscriptionRow::new(SubscriptionForm::blank(), cx);
        self.watch(&row.filter, window, cx);
        self.watch(&row.json_path, window, cx);
        row.filter.update(cx, |filter, _| filter.focus(window));
        self.mqtt.subscriptions.push(row);
    }
}
