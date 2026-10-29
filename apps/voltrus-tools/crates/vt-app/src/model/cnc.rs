//! The MTConnect and FANUC parts of the model: their palette commands, a
//! client beside a new simulator, and the demos.

use std::net::TcpListener;
use std::ops::Range;

use gpui::Context;
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{
    FocasSimulatorSpec, MtconnectAgentSpec, MtconnectTemplate, Protocol, SessionSpec,
};
use vt_ui::Icon;

use super::AppModel;
use super::palette::{Command, Entry};

/// Ports the MTConnect demo agent tries. 5000 is left alone: macOS's AirPlay
/// Receiver listens there.
const MTCONNECT_DEMO_PORTS: Range<u16> = 5100..5200;
/// Ports the FANUC demo simulator tries, from FOCAS's own.
const FOCAS_DEMO_PORTS: Range<u16> = 8193..8293;

/// The palette's MTConnect and FANUC commands.
pub(super) fn palette_entries() -> Vec<Entry> {
    vec![
        Entry {
            title: "New MTConnect connection…".into(),
            detail: "Probe an agent: its devices become points, kept current by the live stream"
                .into(),
            icon: Icon::Plus,
            command: Command::NewMtconnectClient,
        },
        Entry {
            title: "New MTConnect agent simulator…".into(),
            detail: "Play a machining centre or lathe over HTTP, with an optional SHDR adapter"
                .into(),
            icon: Icon::Server,
            command: Command::NewMtconnectAgent,
        },
        Entry {
            title: "Start the MTConnect demo".into(),
            detail: "A simulated machining centre and a client streaming it".into(),
            icon: Icon::Zap,
            command: Command::MtconnectDemo,
        },
        Entry {
            title: "New FANUC CNC connection (beta)…".into(),
            detail:
                "Read status, axes, spindle, program and alarms over FOCAS/Ethernet, write macros"
                    .into(),
            icon: Icon::Plus,
            command: Command::NewFocasClient,
        },
        Entry {
            title: "New FANUC CNC simulator…".into(),
            detail: "Play a 31i-B or 0i-F running a demo program on this computer".into(),
            icon: Icon::Server,
            command: Command::NewFocasSimulator,
        },
        Entry {
            title: "Start the FANUC CNC demo".into(),
            detail: "A simulated 31i-B mill and a client polling it".into(),
            icon: Icon::Zap,
            command: Command::FocasDemo,
        },
    ]
}

/// The address a client on this computer uses for a simulator bound to
/// `bind`.
fn loopback(bind: &str) -> &str {
    match bind {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        bind => bind,
    }
}

impl AppModel {
    /// After a new MTConnect agent simulator is saved: a client streaming
    /// it, side by side.
    pub(super) fn add_mtconnect_client(
        &mut self,
        name: &str,
        agent: &MtconnectAgentSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::MtconnectClient(templates::mtconnect_client_for(
                loopback(&agent.bind),
                agent.port,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// After a new FANUC CNC simulator is saved: a client polling it, side
    /// by side.
    pub(super) fn add_focas_client(
        &mut self,
        name: &str,
        simulator: &FocasSimulatorSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::FocasClient(templates::focas_client_for(
                loopback(&simulator.bind),
                simulator.port,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated machining centre and a client streaming it, side by
    /// side.
    pub fn start_mtconnect_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(port) = self.free_cnc_port(MTCONNECT_DEMO_PORTS, |spec| match spec {
            SessionSpec::MtconnectAgent(agent) => Some(agent.port),
            _ => None,
        }) else {
            self.show_toast(
                "No free port between 5100 and 5199 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let template = MtconnectTemplate::Vmc;
        let simulator = self.workspace.add_connection(
            format!("{} simulator", template.name()),
            SessionSpec::MtconnectAgent(MtconnectAgentSpec {
                port,
                template,
                ..MtconnectAgentSpec::default()
            }),
        );
        let client = self.workspace.add_connection(
            "Machining centre (MTConnect)",
            SessionSpec::MtconnectClient(templates::mtconnect_client_for("127.0.0.1", port)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A simulated 31i-B mill and a client polling it, side by side.
    pub fn start_focas_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(port) = self.free_cnc_port(FOCAS_DEMO_PORTS, |spec| match spec {
            SessionSpec::FocasSimulator(simulator) => Some(simulator.port),
            _ => None,
        }) else {
            self.show_toast(
                "No free port between 8193 and 8292 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let simulator = self.workspace.add_connection(
            "FANUC 31i-B simulator",
            SessionSpec::FocasSimulator(FocasSimulatorSpec {
                port,
                ..FocasSimulatorSpec::default()
            }),
        );
        let client = self.workspace.add_connection(
            "FANUC 31i-B mill",
            SessionSpec::FocasClient(templates::focas_client_for("127.0.0.1", port)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A TCP port in `range` that no saved simulator (`port_of` says which
    /// ports a spec holds) uses and nothing listens on.
    fn free_cnc_port(
        &self,
        range: Range<u16>,
        port_of: impl Fn(&SessionSpec) -> Option<u16>,
    ) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| port_of(&connection.spec))
            .collect();
        range
            .into_iter()
            .find(|port| !taken.contains(port) && TcpListener::bind(("0.0.0.0", *port)).is_ok())
    }
}

/// Whether this protocol's clients are labelled beta in the app: FANUC's
/// connection was only tested against its own simulator.
pub fn is_beta(protocol: Protocol) -> bool {
    protocol == Protocol::Focas
}

/// The explanation shown wherever the Beta label is.
pub const BETA_NOTE: &str = "Tested against the built-in simulator; please report your controller model to support@voltrus.id";

/// What the About sheet says about the names these protocols use. The app
/// is independent of both organisations.
pub const TRADEMARK_NOTES: [&str; 2] = [
    "FANUC and FOCAS are trademarks of FANUC CORPORATION. Voltrus Tools is independent software, not affiliated with, endorsed by or supported by FANUC; the FANUC CNC connection is for FANUC CNCs and is in beta.",
    "MTConnect is a registered trademark of AMT - The Association For Manufacturing Technology. Voltrus Tools is not affiliated with or endorsed by AMT.",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_about_sheet_names_both_trademarks_and_disclaims_affiliation() {
        let notes = TRADEMARK_NOTES.join(" ");
        assert!(notes.contains("FANUC CORPORATION"));
        assert!(notes.contains("not affiliated with, endorsed by or supported by FANUC"));
        assert!(notes.contains("MTConnect is a registered trademark"));
    }

    #[test]
    fn only_the_fanuc_connection_is_beta() {
        for &protocol in Protocol::ALL {
            assert_eq!(
                is_beta(protocol),
                protocol == Protocol::Focas,
                "{protocol:?}"
            );
        }
        assert!(BETA_NOTE.contains("support@voltrus.id"));
    }

    #[test]
    fn a_client_reaches_a_simulator_on_this_computer() {
        assert_eq!(loopback("0.0.0.0"), "127.0.0.1");
        assert_eq!(loopback("::"), "::1");
        assert_eq!(loopback("10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn palette_commands_open_one_editor_per_role_and_a_demo() {
        let entries = palette_entries();
        let commands: Vec<Command> = entries.iter().map(|entry| entry.command.clone()).collect();
        for command in [
            Command::NewMtconnectClient,
            Command::NewMtconnectAgent,
            Command::MtconnectDemo,
            Command::NewFocasClient,
            Command::NewFocasSimulator,
            Command::FocasDemo,
        ] {
            assert!(commands.contains(&command), "{command:?}");
        }
        assert!(
            entries
                .iter()
                .any(|entry| entry.title.contains("FANUC CNC connection (beta)")),
            "the FANUC connection says it is beta"
        );
    }
}
