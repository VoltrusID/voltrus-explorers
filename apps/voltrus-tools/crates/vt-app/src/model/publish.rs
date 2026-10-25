//! The inspector's publish form, for protocols that carry messages (MQTT).

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use vt_app_core::{ConnectionId, Workspace};
use vt_protocol::{Command as EngineCommand, MqttQos, SessionSpec};
use vt_ui::text_input::{InputEvent, TextInput};

use super::AppModel;
use super::live::Live;

pub struct PublishForm {
    pub topic: Entity<TextInput>,
    pub payload: Entity<TextInput>,
    pub qos: MqttQos,
    pub retain: bool,
    _subscriptions: Vec<Subscription>,
}

impl PublishForm {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let topic = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("plant/inverter/limit")
                .mono()
        });
        let payload = cx.new(|cx| TextInput::new(cx).placeholder("42 or {\"kw\": 42}").mono());
        let subscriptions = [&topic, &payload]
            .into_iter()
            .map(|field| {
                cx.subscribe_in(field, window, |model, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Submitted => model.publish_message(window, cx),
                        InputEvent::Cancelled => window.focus(&model.focus),
                        InputEvent::Changed | InputEvent::Blurred => {}
                    }
                })
            })
            .collect();
        Self {
            topic,
            payload,
            qos: MqttQos::AtMostOnce,
            retain: false,
            _subscriptions: subscriptions,
        }
    }
}

/// The topic a point's messages arrive on, when the point is a whole topic
/// of an MQTT session.
pub fn topic_of(
    workspace: &Workspace,
    connection: ConnectionId,
    live: &Live,
    point: u32,
) -> Option<String> {
    let spec = &workspace.connection(connection)?.spec;
    if !matches!(
        spec,
        SessionSpec::MqttClient(_) | SessionSpec::MqttBroker(_)
    ) {
        return None;
    }
    let (info, _) = live.point(point)?;
    // JSON fields and Sparkplug metrics are named `<topic or node> › <field>`.
    (!info.name.contains(" › ")).then(|| info.name.clone())
}

impl AppModel {
    /// Publish the form's payload to its topic from the focused session.
    pub fn publish_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let topic = self.publish.topic.read(cx).value().trim().to_owned();
        let payload = self.publish.payload.read(cx).value().to_owned();
        let (qos, retain) = (self.publish.qos, self.publish.retain);
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let Some(session) = live.session else {
            return;
        };
        if !vt_protocol::valid_topic(&topic) {
            live.publish_result = Some((
                topic,
                Err("Enter a topic to publish to, without + or #".into()),
            ));
            cx.notify();
            return;
        }
        live.publish_result = None;
        self.engine.send(EngineCommand::Publish {
            session,
            topic,
            payload,
            qos,
            retain,
        });
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn set_publish_qos(&mut self, qos: MqttQos, cx: &mut Context<Self>) {
        self.publish.qos = qos;
        cx.notify();
    }

    pub fn toggle_publish_retain(&mut self, cx: &mut Context<Self>) {
        self.publish.retain = !self.publish.retain;
        cx.notify();
    }
}
