//! The MTConnect and FANUC client and simulator fields of the connection
//! editor.

use gpui::{App, Context, Entity};
use vt_app_core::forms::{
    FocasClientForm, FocasSimulatorForm, MtconnectAgentForm, MtconnectClientForm,
};
use vt_protocol::{
    FocasClientSpec, FocasProfile, FocasSimulatorSpec, MtconnectAgentSpec, MtconnectClientSpec,
    MtconnectTemplate, SessionSpec,
};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, input};

/// Names of a new client or simulator until the user names it.
pub const NEW_MTCONNECT_CLIENT_NAME: &str = "MTConnect agent";
pub const NEW_MTCONNECT_AGENT_NAME: &str = "MTConnect simulator";
pub const NEW_FOCAS_CLIENT_NAME: &str = "FANUC CNC";
pub const NEW_FOCAS_SIMULATOR_NAME: &str = "FANUC CNC simulator";

pub struct CncFields {
    pub url: Entity<TextInput>,
    pub device: Entity<TextInput>,
    pub interval: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub agent_bind: Entity<TextInput>,
    pub agent_port: Entity<TextInput>,
    pub agent_template: MtconnectTemplate,
    pub shdr: bool,
    pub shdr_port: Entity<TextInput>,
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub focas_timeout: Entity<TextInput>,
    pub macros: Entity<TextInput>,
    pub sim_bind: Entity<TextInput>,
    pub sim_port: Entity<TextInput>,
    pub profile: FocasProfile,
    pub run: bool,
}

impl CncFields {
    /// Fields for `editing` when it is an MTConnect or FANUC connection,
    /// defaults otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::MtconnectClient(spec)) => spec.clone(),
            _ => MtconnectClientSpec::default(),
        };
        let agent = match editing {
            Some(SessionSpec::MtconnectAgent(spec)) => spec.clone(),
            _ => MtconnectAgentSpec::default(),
        };
        let focas = match editing {
            Some(SessionSpec::FocasClient(spec)) => spec.clone(),
            _ => FocasClientSpec::default(),
        };
        let simulator = match editing {
            Some(SessionSpec::FocasSimulator(spec)) => spec.clone(),
            _ => FocasSimulatorSpec::default(),
        };
        let client = MtconnectClientForm::from_spec("", &client);
        let agent = MtconnectAgentForm::from_spec("", &agent);
        let focas = FocasClientForm::from_spec("", &focas);
        let simulator = FocasSimulatorForm::from_spec("", &simulator);
        let unsigned = InputKind::Unsigned;
        let text = InputKind::Text;
        Self {
            url: input(client.url, "http://localhost:5000", text, true, cx),
            device: input(client.device, "All devices", text, false, cx),
            interval: input(client.interval_ms, "1000", unsigned, true, cx),
            timeout: input(client.timeout_ms, "10000", unsigned, true, cx),
            agent_bind: input(agent.bind, "127.0.0.1", text, true, cx),
            agent_port: input(agent.port, "5100", unsigned, true, cx),
            agent_template: agent.template,
            shdr: agent.shdr,
            shdr_port: input(agent.shdr_port, "7878", unsigned, true, cx),
            host: input(focas.host, "192.168.1.50", text, true, cx),
            port: input(focas.port, "8193", unsigned, true, cx),
            poll: input(focas.poll_ms, "500", unsigned, true, cx),
            focas_timeout: input(focas.timeout_ms, "5000", unsigned, true, cx),
            macros: input(focas.macros, "500, 501", text, true, cx),
            sim_bind: input(simulator.bind, "127.0.0.1", text, true, cx),
            sim_port: input(simulator.port, "8193", unsigned, true, cx),
            profile: simulator.profile,
            run: simulator.run,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        vec![
            self.url.clone(),
            self.device.clone(),
            self.interval.clone(),
            self.timeout.clone(),
            self.agent_bind.clone(),
            self.agent_port.clone(),
            self.shdr_port.clone(),
            self.host.clone(),
            self.port.clone(),
            self.poll.clone(),
            self.focas_timeout.clone(),
            self.macros.clone(),
            self.sim_bind.clone(),
            self.sim_port.clone(),
        ]
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn mtconnect_client_form(&self, name: String, cx: &App) -> MtconnectClientForm {
        MtconnectClientForm {
            name,
            url: Self::text(&self.url, cx),
            device: Self::text(&self.device, cx),
            interval_ms: Self::text(&self.interval, cx),
            timeout_ms: Self::text(&self.timeout, cx),
        }
    }

    pub fn mtconnect_agent_form(&self, name: String, cx: &App) -> MtconnectAgentForm {
        MtconnectAgentForm {
            name,
            bind: Self::text(&self.agent_bind, cx),
            port: Self::text(&self.agent_port, cx),
            template: self.agent_template,
            shdr: self.shdr,
            shdr_port: Self::text(&self.shdr_port, cx),
        }
    }

    pub fn focas_client_form(&self, name: String, cx: &App) -> FocasClientForm {
        FocasClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            poll_ms: Self::text(&self.poll, cx),
            timeout_ms: Self::text(&self.focas_timeout, cx),
            macros: Self::text(&self.macros, cx),
        }
    }

    pub fn focas_simulator_form(&self, name: String, cx: &App) -> FocasSimulatorForm {
        FocasSimulatorForm {
            name,
            bind: Self::text(&self.sim_bind, cx),
            port: Self::text(&self.sim_port, cx),
            profile: self.profile,
            run: self.run,
        }
    }
}
