//! The Siemens S7 parts of the model: reading what a CPU says about itself,
//! a client beside a new simulator, and the S7 demo.

use std::net::TcpListener;

use gpui::Context;
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{Command as EngineCommand, S7ServerSpec, SessionSpec};
use vt_ui::Icon;

use super::AppModel;
use super::palette::{Command, Entry};

/// Whether connections of `spec` have a Device view (an S7 client's PLC).
pub fn has_device_view(spec: &SessionSpec) -> bool {
    matches!(spec, SessionSpec::S7Client(_))
}

/// The palette's S7 commands: new connections, the demo, the PLC view, and
/// reading each running client's CPU again.
pub(super) fn palette_entries(model: &AppModel) -> Vec<Entry> {
    let mut entries = vec![
        Entry {
            title: "New Siemens S7 connection…".into(),
            detail: "Read and write a CPU's DB, I, Q, M, timer and counter tags".into(),
            icon: Icon::Plus,
            command: Command::NewS7Client,
        },
        Entry {
            title: "New S7 PLC simulator…".into(),
            detail: "Play a motor drive, reactor or packaging line CPU on this computer".into(),
            icon: Icon::Server,
            command: Command::NewS7Server,
        },
        Entry {
            title: "Start the S7 demo".into(),
            detail: "A simulated S7-1500 motor drive and a client reading it".into(),
            icon: Icon::Zap,
            command: Command::S7Demo,
        },
    ];
    if model.focused_spec().is_some_and(has_device_view) {
        entries.push(Entry {
            title: "Show PLC".into(),
            detail: "CPU identity, mode, diagnostic buffer and blocks  ⌘5".into(),
            icon: Icon::Cpu,
            command: Command::View(PaneView::Device),
        });
    }
    for connection in &model.workspace.connections {
        let running = model
            .live
            .get(&connection.id)
            .is_some_and(|live| live.state.is_live());
        if running && matches!(connection.spec, SessionSpec::S7Client(_)) {
            entries.push(Entry {
                title: format!("Read CPU diagnostics of {}", connection.name).into(),
                detail: connection.spec.endpoint().into(),
                icon: Icon::RefreshCw,
                command: Command::ReadS7Plc(connection.id),
            });
        }
    }
    entries
}

/// Ports the demo simulator tries: 1102 (what the S7 Explorer app's
/// simulator uses) and up, as 102 may be taken or reserved.
const DEMO_PORTS: std::ops::Range<u16> = 1102..1200;

impl AppModel {
    fn focused_spec(&self) -> Option<&SessionSpec> {
        let connection = self.focused_connection()?;
        self.workspace
            .connection(connection)
            .map(|connection| &connection.spec)
    }

    /// ⌘5: the focused pane's Device view, where its protocol has one.
    pub fn show_device_view(&mut self, cx: &mut Context<Self>) {
        if self.focused_spec().is_some_and(has_device_view) {
            self.set_view(PaneView::Device, cx);
        }
    }

    /// ⌘R: read the focused connection's device again (an S7 client's CPU).
    pub fn refresh_device(&mut self, cx: &mut Context<Self>) {
        if self.focused_spec().is_some_and(has_device_view)
            && let Some(connection) = self.focused_connection()
        {
            self.read_s7_plc(connection, cx);
        }
    }

    /// Ask a running S7 client's CPU for its identity, mode, diagnostic
    /// buffer and blocks again.
    pub fn read_s7_plc(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let Some(session) = live.session else {
            return;
        };
        live.s7_plc_reading = true;
        self.engine.send(EngineCommand::ReadS7Plc { session });
        cx.notify();
    }

    /// After a new S7 simulator is saved: a client that reads all of its
    /// tags, side by side.
    pub(super) fn add_s7_client(
        &mut self,
        name: &str,
        server: &S7ServerSpec,
        cx: &mut Context<Self>,
    ) {
        let Some(template) = templates::find_s7(&server.template) else {
            return;
        };
        let host = match server.bind.as_str() {
            "0.0.0.0" => "127.0.0.1",
            "::" => "::1",
            bind => bind,
        };
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::S7Client(templates::s7_client_for(template, host, server.port)),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated motor drive and a client reading it, side by side.
    pub fn start_s7_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(template) = templates::find_s7("motor") else {
            self.show_toast("The demo CPU template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_s7_port() else {
            self.show_toast(
                "No free port between 1102 and 1199 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let simulator = self.workspace.add_connection(
            format!("{} simulator", template.name),
            SessionSpec::S7Server(S7ServerSpec {
                port,
                template: template.id.clone(),
                ..S7ServerSpec::default()
            }),
        );
        let client = self.workspace.add_connection(
            format!("{} ({})", template.name, template.cpu),
            SessionSpec::S7Client(templates::s7_client_for(template, "127.0.0.1", port)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A TCP port no saved S7 simulator uses and nothing listens on.
    fn free_s7_port(&self) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::S7Server(server) => Some(server.port),
                _ => None,
            })
            .collect();
        DEMO_PORTS
            .clone()
            .find(|port| !taken.contains(port) && TcpListener::bind(("0.0.0.0", *port)).is_ok())
    }
}
