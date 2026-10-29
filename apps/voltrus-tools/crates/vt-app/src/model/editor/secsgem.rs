//! The SECS/GEM host and equipment simulator fields of the connection
//! editor.

use gpui::{App, Context, Entity};
use vt_app_core::forms::{SecsGemClientForm, SecsGemSimulatorForm};
use vt_protocol::{SecsGemClientSpec, SecsGemSimulatorSpec, SessionSpec};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, input};

/// Names of a new host or simulator until the user names it.
pub const NEW_SECSGEM_CLIENT_NAME: &str = "SECS/GEM equipment";
pub const NEW_SECSGEM_SIMULATOR_NAME: &str = "SECS/GEM chamber simulator";

pub struct SecsGemFields {
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub device_id: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub svids: Entity<TextInput>,
    pub ecids: Entity<TextInput>,
    pub events: Entity<TextInput>,
    pub alarms: bool,
    pub sim_bind: Entity<TextInput>,
    pub sim_port: Entity<TextInput>,
    pub sim_device_id: Entity<TextInput>,
    pub run: bool,
}

impl SecsGemFields {
    /// Fields for `editing` when it is a SECS/GEM connection, defaults
    /// otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::SecsGemClient(spec)) => spec.clone(),
            _ => SecsGemClientSpec::default(),
        };
        let simulator = match editing {
            Some(SessionSpec::SecsGemSimulator(spec)) => spec.clone(),
            _ => SecsGemSimulatorSpec::default(),
        };
        let client = SecsGemClientForm::from_spec("", &client);
        let simulator = SecsGemSimulatorForm::from_spec("", &simulator);
        let unsigned = InputKind::Unsigned;
        let text = InputKind::Text;
        Self {
            host: input(client.host, "192.168.1.60", text, true, cx),
            port: input(client.port, "5000", unsigned, true, cx),
            device_id: input(client.device_id, "0", unsigned, true, cx),
            poll: input(client.poll_ms, "1000", unsigned, true, cx),
            timeout: input(client.timeout_ms, "10000", unsigned, true, cx),
            svids: input(client.svids, "All the equipment names", text, false, cx),
            ecids: input(client.ecids, "All the equipment names", text, false, cx),
            events: input(client.events, "None", text, false, cx),
            alarms: client.alarms,
            sim_bind: input(simulator.bind, "127.0.0.1", text, true, cx),
            sim_port: input(simulator.port, "5600", unsigned, true, cx),
            sim_device_id: input(simulator.device_id, "0", unsigned, true, cx),
            run: simulator.run,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        vec![
            self.host.clone(),
            self.port.clone(),
            self.device_id.clone(),
            self.poll.clone(),
            self.timeout.clone(),
            self.svids.clone(),
            self.ecids.clone(),
            self.events.clone(),
            self.sim_bind.clone(),
            self.sim_port.clone(),
            self.sim_device_id.clone(),
        ]
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> SecsGemClientForm {
        SecsGemClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            device_id: Self::text(&self.device_id, cx),
            poll_ms: Self::text(&self.poll, cx),
            timeout_ms: Self::text(&self.timeout, cx),
            svids: Self::text(&self.svids, cx),
            ecids: Self::text(&self.ecids, cx),
            events: Self::text(&self.events, cx),
            alarms: self.alarms,
        }
    }

    pub fn simulator_form(&self, name: String, cx: &App) -> SecsGemSimulatorForm {
        SecsGemSimulatorForm {
            name,
            bind: Self::text(&self.sim_bind, cx),
            port: Self::text(&self.sim_port, cx),
            device_id: Self::text(&self.sim_device_id, cx),
            run: self.run,
        }
    }
}
