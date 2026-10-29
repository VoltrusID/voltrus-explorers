//! The SECS/GEM part of the model: its palette commands, a host beside a new
//! equipment simulator, and the demo.

use std::ops::Range;

use gpui::Context;
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{SecsGemSimulatorSpec, SessionSpec};
use vt_ui::Icon;

use super::AppModel;
use super::cnc::loopback;
use super::palette::{Command, Entry};

/// Ports the SECS/GEM demo simulator tries, from the simulator's default.
const DEMO_PORTS: Range<u16> = 5600..5700;

/// The palette's SECS/GEM commands.
pub(super) fn palette_entries() -> Vec<Entry> {
    vec![
        Entry {
            title: "New SECS/GEM connection (beta)…".into(),
            detail: "Host a piece of equipment over HSMS: status variables, equipment constants, events and alarms"
                .into(),
            icon: Icon::Plus,
            command: Command::NewSecsGemClient,
        },
        Entry {
            title: "New SECS/GEM equipment simulator…".into(),
            detail: "Play a wafer processing chamber behind a GEM interface on this computer"
                .into(),
            icon: Icon::Server,
            command: Command::NewSecsGemSimulator,
        },
        Entry {
            title: "Start the SECS/GEM demo".into(),
            detail: "A simulated chamber and a host reading it".into(),
            icon: Icon::Zap,
            command: Command::SecsGemDemo,
        },
    ]
}

impl AppModel {
    /// After a new SECS/GEM simulator is saved: a host reading it, side by
    /// side.
    pub(super) fn add_secsgem_client(
        &mut self,
        name: &str,
        simulator: &SecsGemSimulatorSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} host"),
            SessionSpec::SecsGemClient(templates::secsgem_client_for(
                loopback(&simulator.bind),
                simulator.port,
                simulator.device_id,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated wafer processing chamber and a host reading it, side by
    /// side.
    pub fn start_secsgem_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(port) = self.free_cnc_port(DEMO_PORTS, |spec| match spec {
            SessionSpec::SecsGemSimulator(simulator) => Some(simulator.port),
            _ => None,
        }) else {
            self.show_toast(
                "No free port between 5600 and 5699 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let simulator = self.workspace.add_connection(
            "SECS/GEM chamber simulator",
            SessionSpec::SecsGemSimulator(SecsGemSimulatorSpec {
                port,
                ..SecsGemSimulatorSpec::default()
            }),
        );
        let client = self.workspace.add_connection(
            "Wafer chamber (SECS/GEM)",
            SessionSpec::SecsGemClient(templates::secsgem_client_for("127.0.0.1", port, 0)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_commands_open_one_editor_per_role_and_a_demo() {
        let entries = palette_entries();
        let commands: Vec<Command> = entries.iter().map(|entry| entry.command.clone()).collect();
        for command in [
            Command::NewSecsGemClient,
            Command::NewSecsGemSimulator,
            Command::SecsGemDemo,
        ] {
            assert!(commands.contains(&command), "{command:?}");
        }
        assert!(
            entries
                .iter()
                .any(|entry| entry.title.contains("SECS/GEM connection (beta)")),
            "the host says it is beta"
        );
    }
}
