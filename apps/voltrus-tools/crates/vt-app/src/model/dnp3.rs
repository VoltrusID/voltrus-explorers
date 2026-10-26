//! The DNP3 parts of the model: station commands and controls from the
//! inspector and the palette, the master's control mode, what a session
//! last reported about its outstation, and the DNP3 demo plant.

use std::net::TcpListener;

use gpui::Context;
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{
    Command as EngineCommand, DNP3_PORT, Dnp3Action, Dnp3ControlMode, Dnp3Iin, Dnp3Operation,
    Dnp3OutstationSpec, SessionSpec,
};
use vt_ui::Icon;

use super::AppModel;
use super::palette::{Command, Entry};

/// The plant the demo and new simulators' masters start from.
const DEMO_PLANT: &str = "feeder";

/// What a DNP3 session last reported about its outstation.
#[derive(Debug, Default)]
pub struct Dnp3Live {
    /// The internal indications of the last response (masters).
    pub iin: Option<Dnp3Iin>,
    /// The station command sent and not answered yet.
    pub busy: Option<Dnp3Action>,
    /// The last station command's outcome.
    pub last: Option<(Dnp3Action, Result<String, String>)>,
}

impl Dnp3Live {
    /// Whether `action` is the command waiting for its answer.
    pub fn is_busy(&self, action: &Dnp3Action) -> bool {
        self.busy.as_ref() == Some(action)
    }
}

impl AppModel {
    /// Send a station command (poll, time sync, restart, control) to a
    /// connection's running session.
    pub fn dnp3_action(
        &mut self,
        connection: ConnectionId,
        action: Dnp3Action,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let Some(session) = live.session else {
            self.show_toast("Connect first", true, cx);
            return;
        };
        live.dnp3.busy = Some(action);
        live.dnp3.last = None;
        if let Dnp3Action::Operate { .. } | Dnp3Action::SetOnline { .. } = action {
            live.write_result = None;
        }
        self.engine.send(EngineCommand::Dnp3 { session, action });
        cx.notify();
    }

    /// Command the focused master's selected output the way its connection
    /// is set to (select-before-operate or direct operate).
    pub fn dnp3_operate_selected(&mut self, operation: Dnp3Operation, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(SessionSpec::Dnp3Master(spec)) = self
            .workspace
            .connection(connection)
            .map(|connection| &connection.spec)
        else {
            return;
        };
        let mode = spec.control_mode;
        let Some(point) = self.live.get(&connection).and_then(|live| live.selected) else {
            return;
        };
        self.dnp3_action(
            connection,
            Dnp3Action::Operate {
                point,
                operation,
                mode,
            },
            cx,
        );
    }

    /// Take the focused simulator's selected point offline or back online.
    pub fn dnp3_set_online_selected(&mut self, online: bool, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(point) = self.live.get(&connection).and_then(|live| live.selected) else {
            return;
        };
        self.dnp3_action(connection, Dnp3Action::SetOnline { point, online }, cx);
    }

    /// Save how a master commands outputs. A running session takes the mode
    /// with each command, so it needs no reconnect.
    pub fn set_dnp3_control_mode(
        &mut self,
        connection: ConnectionId,
        mode: Dnp3ControlMode,
        cx: &mut Context<Self>,
    ) {
        let Some(saved) = self.workspace.connection(connection) else {
            return;
        };
        let SessionSpec::Dnp3Master(spec) = &saved.spec else {
            return;
        };
        if spec.control_mode == mode {
            return;
        }
        let name = saved.name.clone();
        let mut spec = spec.clone();
        spec.control_mode = mode;
        self.workspace
            .update_connection(connection, name, SessionSpec::Dnp3Master(spec));
        self.save(cx);
        cx.notify();
    }

    /// After a new DNP3 simulator is saved: a master of it, side by side.
    pub(super) fn add_dnp3_master(
        &mut self,
        name: &str,
        outstation: &Dnp3OutstationSpec,
        cx: &mut Context<Self>,
    ) {
        let Some(plant) = templates::find_dnp3(&outstation.template) else {
            return;
        };
        let host = match outstation.bind.as_str() {
            "0.0.0.0" => "127.0.0.1",
            "::" => "::1",
            bind => bind,
        };
        let client = self.workspace.add_connection(
            format!("{name} master"),
            SessionSpec::Dnp3Master(templates::dnp3_master_for(
                plant,
                host,
                outstation.port,
                outstation.outstation_address,
                outstation.master_address,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated distribution feeder and a master polling it, side by side.
    pub fn start_dnp3_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        let Some(plant) = templates::find_dnp3(DEMO_PLANT) else {
            self.show_toast("The demo plant template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_dnp3_port() else {
            self.show_toast(
                "No free TCP port between 20000 and 20099 for the outstation",
                true,
                cx,
            );
            return None;
        };
        let outstation = Dnp3OutstationSpec {
            port,
            template: plant.id.clone(),
            ..Dnp3OutstationSpec::default()
        };
        let simulator = self.workspace.add_connection(
            format!("{} outstation", plant.name),
            SessionSpec::Dnp3Outstation(outstation.clone()),
        );
        let client = self.workspace.add_connection(
            plant.name.clone(),
            SessionSpec::Dnp3Master(templates::dnp3_master_for(
                plant,
                "127.0.0.1",
                port,
                outstation.outstation_address,
                outstation.master_address,
            )),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A TCP port from the DNP3 port up that no saved simulator uses and
    /// nothing on this computer listens on.
    fn free_dnp3_port(&self) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::Dnp3Outstation(outstation) => Some(outstation.port),
                _ => None,
            })
            .collect();
        (DNP3_PORT..DNP3_PORT + 100)
            .find(|port| !taken.contains(port) && TcpListener::bind(("127.0.0.1", *port)).is_ok())
    }
}

/// The station commands of a DNP3 master, in the order the inspector shows them.
pub const STATION: [(Dnp3Action, &str, Icon); 8] = [
    (
        Dnp3Action::IntegrityPoll,
        "Read every point now (class 0, 1, 2, 3)",
        Icon::RefreshCw,
    ),
    (
        Dnp3Action::ClassPoll { class: 1 },
        "Read class 1 events",
        Icon::Inbox,
    ),
    (
        Dnp3Action::ClassPoll { class: 2 },
        "Read class 2 events",
        Icon::Inbox,
    ),
    (
        Dnp3Action::ClassPoll { class: 3 },
        "Read class 3 events",
        Icon::Inbox,
    ),
    (
        Dnp3Action::TimeSync,
        "Write this computer's time to the outstation",
        Icon::Clock,
    ),
    (
        Dnp3Action::FreezeCounters,
        "Copy every counter to its frozen counter",
        Icon::Hash,
    ),
    (
        Dnp3Action::WarmRestart,
        "Restart the outstation's application",
        Icon::RefreshCw,
    ),
    (
        Dnp3Action::ColdRestart,
        "Restart the whole outstation",
        Icon::Zap,
    ),
];

/// Palette commands for the focused DNP3 master.
pub fn palette_entries(model: &AppModel) -> Vec<Entry> {
    let Some(connection) = model.focused_connection() else {
        return Vec::new();
    };
    let Some(saved) = model.workspace.connection(connection) else {
        return Vec::new();
    };
    if !matches!(saved.spec, SessionSpec::Dnp3Master(_)) {
        return Vec::new();
    }
    STATION
        .iter()
        .map(|(action, detail, icon)| Entry {
            title: format!("DNP3: {}", action.label()).into(),
            detail: format!("{detail} · {}", saved.name).into(),
            icon: *icon,
            command: Command::Dnp3(connection, *action),
        })
        .collect()
}
