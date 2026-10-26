//! The app's single model entity: the saved workspace, every running session's
//! live state, and the transient UI state (overlays, drags, toasts).
//!
//! As in muxy, views are plain functions over `&AppModel`; this file holds
//! every state transition.

pub mod bacnet;
pub mod ble;
pub mod dnp3;
pub mod editor;
pub mod enip;
pub mod licence;
pub mod live;
pub mod nats;
pub mod opcua;
pub mod palette;
pub mod publish;
pub mod s7;
pub mod serial;
pub mod terminal;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, IntoElement as _, Pixels, Point,
    SharedString, Subscription, Task, UniformListScrollHandle, Window,
};
use vt_app_core::{
    Appearance, Axis, Branch, ConnectionId, Pane, PaneId, PaneView, TabId, Workspace, store,
};
use vt_engine::{EngineHandle, templates};
use vt_protocol::{
    Command as EngineCommand, Event, ModbusSlaveSpec, MqttBrokerSpec, Protocol, Role, SessionId,
    SessionSpec,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::{self, Theme, ThemeCatalog};

use editor::{Editor, EditorKind};
use live::Live;
use palette::{Command, Mode, Palette};
use publish::PublishForm;
use serial::SerialConsole;

use crate::views::{self, cached::CachedView};

/// What `main` hands the model at startup.
pub struct Boot {
    pub workspace: Workspace,
    /// Where to save; `None` keeps the workspace in memory (demo runs).
    pub store: Option<PathBuf>,
    /// Shown once at startup, e.g. a workspace file that had to be set aside.
    pub notice: Option<String>,
}

pub enum Overlay {
    None,
    Palette(Palette),
    /// Boxed: the editor holds a field entity per input.
    Editor(Box<Editor>),
    /// The sidebar's "+" menu: which protocol to add, as a client or a
    /// simulator, shown where it was clicked.
    NewMenu {
        role: Role,
        at: Point<Pixels>,
    },
}

pub enum Drag {
    Sidebar {
        start_x: Pixels,
        start_width: f32,
    },
    Inspector {
        start_x: Pixels,
        start_width: f32,
    },
    Split {
        tab: TabId,
        path: Vec<Branch>,
        axis: Axis,
        start: Point<Pixels>,
        start_ratio: f32,
        extent: Pixels,
    },
}

pub struct Toast {
    pub message: SharedString,
    pub error: bool,
    id: u64,
}

pub type SplitBounds = Rc<RefCell<HashMap<(TabId, Vec<Branch>), Bounds<Pixels>>>>;
/// Each pane's width at the last layout, so its contents can fit the space.
pub type PaneWidths = Rc<RefCell<HashMap<PaneId, Pixels>>>;

/// Scroll positions of one pane's lists: two panes on one connection scroll
/// independently.
#[derive(Clone, Default)]
pub struct PaneScroll {
    pub points: UniformListScrollHandle,
    pub traffic: UniformListScrollHandle,
    pub log: UniformListScrollHandle,
}

/// The window's parts that are drawn by their own cached views.
pub struct Chrome {
    pub sidebar: Entity<CachedView>,
    pub tabs: Entity<CachedView>,
    pub inspector: Entity<CachedView>,
    pub status_bar: Entity<CachedView>,
    /// One per open pane, made as panes appear.
    pub panes: HashMap<PaneId, Entity<CachedView>>,
}

pub struct AppModel {
    pub workspace: Workspace,
    store: Option<PathBuf>,
    /// The session engine behind the licence gate (store builds lock it once
    /// the trial is over).
    engine: licence::GatedEngine,
    pub live: HashMap<ConnectionId, Live>,
    sessions: HashMap<SessionId, ConnectionId>,
    next_session: u64,
    pub catalog: Rc<ThemeCatalog>,
    pub focus: FocusHandle,
    pub overlay: Overlay,
    pub toast: Option<Toast>,
    toast_seq: u64,
    pub drag: Option<Drag>,
    pub split_bounds: SplitBounds,
    pub pane_widths: PaneWidths,
    /// Each wide pane header's name column at the last layout: its address
    /// line only truncates inside a known width.
    pub header_text_widths: PaneWidths,
    pane_scrolls: RefCell<HashMap<PaneId, PaneScroll>>,
    pub chrome: Chrome,
    pub write_input: Entity<TextInput>,
    /// The inspector's publish form (message protocols).
    pub publish: PublishForm,
    /// Each open serial pane's send bar.
    pub serial_consoles: HashMap<ConnectionId, SerialConsole>,
    /// The inspector's NATS publish/request form and JetStream browsers.
    pub nats: nats::NatsPanel,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
    /// OPC UA browsers and the inspector's typed inputs.
    pub opcua: opcua::OpcuaModel,
    /// The inspector's explicit message form (EtherNet/IP).
    pub cip: enip::CipForm,
    /// The App Store trial and unlock, and their sheets.
    pub licensing: licence::Licensing,
    /// BLE: the inspector's characteristic tools and the adapter new
    /// connections start on.
    pub ble: ble::BleState,
}

impl AppModel {
    pub fn new(
        boot: Boot,
        engine: EngineHandle,
        events: async_channel::Receiver<Event>,
        catalog: Rc<ThemeCatalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                // Drain what queued up meanwhile: one redraw per batch.
                let mut batch = vec![first];
                while batch.len() < 1024
                    && let Ok(event) = events.try_recv()
                {
                    batch.push(event);
                }
                let applied = this.update(cx, |model, cx| {
                    for event in batch {
                        model.apply_event(event);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        let write_input = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("New value")
                .kind(InputKind::Number)
                .mono()
        });
        let write_events = cx.subscribe_in(
            &write_input,
            window,
            |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => model.write_selected(window, cx),
                InputEvent::Cancelled => window.focus(&model.focus),
                InputEvent::Changed | InputEvent::Blurred => {}
            },
        );
        let appearance = cx.observe_window_appearance(window, |model, window, cx| {
            model.apply_theme(window, cx);
        });
        // ⌘Q, the menu and closing the window all end here: stop the
        // sessions and save the layout.
        let quit = cx.on_app_quit(|model, _| {
            model.shutdown();
            async {}
        });
        let subscriptions = vec![write_events, appearance, quit];
        let licensing = licence::Licensing::new(cx.focus_handle());
        let mut model = Self {
            workspace: boot.workspace,
            store: boot.store,
            engine: licence::GatedEngine::new(engine, licensing.gate.clone()),
            live: HashMap::new(),
            sessions: HashMap::new(),
            next_session: 0,
            catalog,
            focus: cx.focus_handle(),
            overlay: Overlay::None,
            toast: None,
            toast_seq: 0,
            drag: None,
            split_bounds: Rc::default(),
            pane_widths: Rc::default(),
            header_text_widths: Rc::default(),
            pane_scrolls: RefCell::default(),
            chrome: Chrome {
                sidebar: CachedView::new(
                    |model, _, cx| views::sidebar::render(model, cx).into_any_element(),
                    cx,
                ),
                tabs: CachedView::new(
                    |model, _, cx| views::tabs::render(model, cx).into_any_element(),
                    cx,
                ),
                inspector: CachedView::new(
                    |model, _, cx| views::inspector::render(model, cx).into_any_element(),
                    cx,
                ),
                status_bar: CachedView::new(
                    |model, _, cx| views::status_bar::render(model, cx).into_any_element(),
                    cx,
                ),
                panes: HashMap::new(),
            },
            write_input,
            publish: PublishForm::new(window, cx),
            serial_consoles: HashMap::new(),
            nats: nats::NatsPanel::new(window, cx),
            _tasks: vec![pump],
            _subscriptions: subscriptions,
            opcua: opcua::OpcuaModel::new(window, cx),
            cip: enip::CipForm::new(window, cx),
            licensing,
            ble: ble::BleState::new(window, cx),
        };
        window.focus(&model.focus);
        model.start_licensing(cx);
        if let Some(notice) = boot.notice {
            model.show_toast(notice, true, cx);
        }
        model
    }

    // ── Engine ────────────────────────────────────────────────────────────

    fn apply_event(&mut self, event: Event) {
        // The device picker's Who-Is runs under a request id, not a session.
        let event = match event {
            Event {
                session,
                kind: vt_protocol::EventKind::BacnetDevices(found),
            } => {
                self.bacnet_devices_found(session, found);
                return;
            }
            Event {
                session,
                kind: vt_protocol::EventKind::EnipDevices(found),
            } => {
                self.enip_devices_found(session, found);
                return;
            }
            // The port picker's list runs under a request id too.
            Event {
                session,
                kind: vt_protocol::EventKind::SerialPorts(found),
            } => {
                self.serial_ports_found(session, found);
                return;
            }
            event => event,
        };
        // OPC UA answers (and the endpoint picker's, under a request id).
        let event = match event {
            Event {
                session,
                kind: vt_protocol::EventKind::Opcua(answer),
            } => {
                self.opcua_answer(session, answer);
                return;
            }
            event => event,
        };
        // A NATS client's replies and JetStream answers go to the inspector.
        let event = match event {
            Event {
                session,
                kind: vt_protocol::EventKind::Nats(nats),
            } => {
                self.nats_event(session, nats);
                return;
            }
            event => event,
        };
        // So does the BLE editor's scan.
        let event = match event {
            Event {
                session,
                kind: vt_protocol::EventKind::BleDevices(found),
            } => {
                self.ble_devices_found(session, found);
                return;
            }
            event => event,
        };
        let Some(connection) = self.sessions.get(&event.session).copied() else {
            return;
        };
        let closed = matches!(
            &event.kind,
            vt_protocol::EventKind::State(state) if !state.is_running()
        );
        let connected = matches!(
            &event.kind,
            vt_protocol::EventKind::State(vt_protocol::SessionState::Connected)
        );
        if let Some(live) = self.live.get_mut(&connection) {
            live.apply_from(event.session, event.kind);
        }
        if closed {
            self.sessions.remove(&event.session);
        }
        if connected {
            self.opcua_connected(connection);
            self.nats_connected(connection);
        }
    }

    pub fn is_running(&self, connection: ConnectionId) -> bool {
        self.live
            .get(&connection)
            .is_some_and(|live| live.session.is_some())
    }

    pub fn connect(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(spec) = self
            .workspace
            .connection(connection)
            .map(|connection| connection.spec.clone())
        else {
            return;
        };
        if !self.licence_allows(cx) {
            return;
        }
        self.stop(connection);
        self.next_session += 1;
        let session = SessionId(self.next_session);
        self.sessions.insert(session, connection);
        self.live
            .entry(connection)
            .or_default()
            .restart(session, vt_engine::sample_interval_ms(&spec));
        self.engine.send(EngineCommand::Open { session, spec });
        cx.notify();
    }

    fn stop(&mut self, connection: ConnectionId) {
        if let Some(session) = self.live.get(&connection).and_then(|live| live.session) {
            self.engine.send(EngineCommand::Close { session });
        }
    }

    pub fn disconnect(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.stop(connection);
        cx.notify();
    }

    pub fn toggle_session(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if self.is_running(connection) {
            self.disconnect(connection, cx);
        } else {
            self.connect(connection, cx);
        }
    }

    pub fn shutdown(&mut self) {
        self.engine.send(EngineCommand::Shutdown);
        self.save_quietly();
    }

    // ── Workspace ─────────────────────────────────────────────────────────

    pub fn focused_pane(&self) -> Option<&Pane> {
        self.workspace
            .active_tab()
            .and_then(|tab| self.workspace.pane(tab.focused))
    }

    pub fn focused_connection(&self) -> Option<ConnectionId> {
        self.focused_pane().map(|pane| pane.connection)
    }

    /// The scroll handles of a pane's lists.
    pub fn pane_scroll(&self, pane: PaneId) -> PaneScroll {
        self.pane_scrolls
            .borrow_mut()
            .entry(pane)
            .or_default()
            .clone()
    }

    /// Give every open pane its cached view and forget closed panes.
    pub fn sync_pane_views(&mut self, cx: &mut Context<Self>) {
        let open: Vec<PaneId> = self.workspace.panes.iter().map(|pane| pane.id).collect();
        self.chrome.panes.retain(|pane, _| open.contains(pane));
        self.pane_scrolls
            .borrow_mut()
            .retain(|pane, _| open.contains(pane));
        self.pane_widths
            .borrow_mut()
            .retain(|pane, _| open.contains(pane));
        self.header_text_widths
            .borrow_mut()
            .retain(|pane, _| open.contains(pane));
        for pane in open {
            self.chrome.panes.entry(pane).or_insert_with(|| {
                CachedView::new(move |model, _, cx| views::pane::render(model, pane, cx), cx)
            });
        }
    }

    pub fn open_connection(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.workspace.open(connection);
        self.save(cx);
        cx.notify();
    }

    /// Open a connection and start its session if it isn't running.
    pub fn launch(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.workspace.open(connection);
        if !self.is_running(connection) {
            self.connect(connection, cx);
        }
        self.save(cx);
        cx.notify();
    }

    pub fn focus_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        self.workspace.focus(pane);
        cx.notify();
    }

    /// Split the focused pane, showing the same connection in the next view.
    pub fn split(&mut self, axis: Axis, cx: &mut Context<Self>) {
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let (connection, view) = (pane.connection, pane.view);
        let next = PaneView::ALL[(PaneView::ALL.iter().position(|v| *v == view).unwrap_or(0) + 1)
            % PaneView::ALL.len()];
        self.workspace.split(connection, axis, next);
        self.save(cx);
        cx.notify();
    }

    pub fn close_pane(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = self.workspace.active_tab().map(|tab| tab.focused) {
            self.workspace.close_pane(pane);
            self.save(cx);
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        self.workspace.close_tab(tab);
        self.save(cx);
        cx.notify();
    }

    pub fn set_view(&mut self, view: PaneView, cx: &mut Context<Self>) {
        if let Some(pane) = self.workspace.active_tab().map(|tab| tab.focused) {
            self.workspace.set_view(pane, view);
            self.save(cx);
            cx.notify();
        }
    }

    pub fn set_pane_view(&mut self, pane: PaneId, view: PaneView, cx: &mut Context<Self>) {
        self.workspace.focus(pane);
        self.workspace.set_view(pane, view);
        self.save(cx);
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.workspace.ui.sidebar_open = !self.workspace.ui.sidebar_open;
        self.save(cx);
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.workspace.ui.inspector_open = !self.workspace.ui.inspector_open;
        self.save(cx);
        cx.notify();
    }

    pub fn cycle_tab(&mut self, offset: isize, cx: &mut Context<Self>) {
        self.workspace.cycle_tab(offset);
        cx.notify();
    }

    pub fn select_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        self.workspace.select_tab(tab);
        cx.notify();
    }

    pub fn delete_connection(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop(connection);
        self.live.remove(&connection);
        self.workspace.remove_connection(connection);
        self.close_overlay(window, cx);
        self.save(cx);
    }

    /// A simulated SMA inverter and a client polling it, side by side.
    pub fn start_demo_plant(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(profile) = templates::find_modbus("sma-inverter") else {
            self.show_toast("The demo device template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_port(5020..5100) else {
            self.show_toast(
                "No free port between 5020 and 5099 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let simulator = self.workspace.add_connection(
            "Inverter simulator",
            SessionSpec::ModbusSlave(ModbusSlaveSpec {
                port,
                template: Some(profile.id.clone()),
                ..ModbusSlaveSpec::default()
            }),
        );
        let client = self.workspace.add_connection(
            format!("{} {}", profile.manufacturer, profile.model),
            SessionSpec::ModbusMaster(templates::master_for(profile, "127.0.0.1", port, 1)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// The SMA inverter template published by an MQTT broker simulator
    /// (plain topics, a JSON nameplate and a Sparkplug B node), and a client
    /// subscribed to all of it, side by side.
    pub fn start_mqtt_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(profile) = templates::find_modbus("sma-inverter") else {
            self.show_toast("The demo device template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_port(1883..1984) else {
            self.show_toast(
                "No free port between 1883 and 1983 for the broker",
                true,
                cx,
            );
            return None;
        };
        let broker = MqttBrokerSpec {
            port,
            template: Some(profile.id.clone()),
            sparkplug: true,
            ..MqttBrokerSpec::default()
        };
        let client = broker.client();
        let simulator = self
            .workspace
            .add_connection("Inverter broker", SessionSpec::MqttBroker(broker));
        let client = self.workspace.add_connection(
            format!("{} {} over MQTT", profile.manufacturer, profile.model),
            SessionSpec::MqttClient(client),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// Start a protocol's demo plant.
    pub fn start_demo(&mut self, protocol: Protocol, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        match protocol {
            Protocol::Modbus => {
                self.start_demo_plant(cx);
            }
            Protocol::Mqtt => {
                self.start_mqtt_demo(cx);
            }
            Protocol::Bacnet => {
                self.start_bacnet_demo(cx);
            }
            Protocol::Opcua => {
                self.start_opcua_demo(cx);
            }
            Protocol::Dnp3 => {
                self.start_dnp3_demo(cx);
            }
            Protocol::Enip => {
                self.start_enip_demo(cx);
            }
            Protocol::Serial => {
                self.start_serial_demo(cx);
            }
            Protocol::S7 => {
                self.start_s7_demo(cx);
            }
            Protocol::Nats => {
                self.start_nats_demo(cx);
            }
            Protocol::Ble => {
                self.start_ble_demo(cx);
            }
        }
    }

    /// Open a client with its simulator beside it, focus the client and
    /// start both.
    pub(super) fn show_side_by_side(
        &mut self,
        client: ConnectionId,
        simulator: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        self.workspace.open(client);
        self.workspace
            .split(simulator, Axis::Horizontal, PaneView::Points);
        if let Some(pane) = self
            .workspace
            .panes
            .iter()
            .find(|pane| pane.connection == client)
            .map(|pane| pane.id)
        {
            self.workspace.focus(pane);
        }
        self.connect(simulator, cx);
        self.connect(client, cx);
        self.save(cx);
    }

    /// A loopback port in `ports` that no saved simulator uses and nothing
    /// listens on.
    fn free_port(&self, mut ports: std::ops::Range<u16>) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::ModbusSlave(spec) => Some(spec.port),
                SessionSpec::MqttBroker(spec) => Some(spec.port),
                _ => None,
            })
            .collect();
        ports.find(|port| !taken.contains(port) && TcpListener::bind(("127.0.0.1", *port)).is_ok())
    }

    // ── Points ────────────────────────────────────────────────────────────

    pub fn select_point(&mut self, connection: ConnectionId, point: u32, cx: &mut Context<Self>) {
        let changed = self
            .live
            .get(&connection)
            .is_some_and(|live| live.selected != Some(point));
        if let Some(live) = self.live.get_mut(&connection) {
            if live.selected != Some(point) {
                live.write_result = None;
                self.write_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.opcua
                    .write_input
                    .update(cx, |input, cx| input.set_text("", cx));
                // Publishing from the inspector starts at the point's topic.
                if let Some(topic) = publish::topic_of(&self.workspace, connection, live, point) {
                    self.publish
                        .topic
                        .update(cx, |input, cx| input.set_text(topic, cx));
                }
                if let Some(subject) = nats::subject_of(&self.workspace, connection, live, point) {
                    self.nats
                        .subject
                        .update(cx, |input, cx| input.set_text(subject, cx));
                }
            }
            live.selected = Some(point);
        }
        // Explicit messages from the inspector start at the point's attribute.
        if changed {
            self.aim_cip_at(connection, point, cx);
        }
        self.sync_ble_inspector(connection, point, cx);
        cx.notify();
    }

    /// ↑↓ through the focused pane's points. Moving goes through
    /// `select_point`, so the inspector resets exactly as it does on a click.
    pub fn step_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(pane) = self.focused_pane().map(|pane| pane.id) else {
            return;
        };
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(next) = self
            .live
            .get(&connection)
            .and_then(|live| live.step_target(step))
        else {
            return;
        };
        let Ok(point) = u32::try_from(next) else {
            return;
        };
        self.pane_scroll(pane)
            .points
            .scroll_to_item(next, gpui::ScrollStrategy::Center);
        self.select_point(connection, point, cx);
    }

    pub fn toggle_trend(&mut self, connection: ConnectionId, point: u32, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.toggle_trend(point);
            cx.notify();
        }
    }

    pub fn set_trend_window(
        &mut self,
        connection: ConnectionId,
        window_ms: f64,
        cx: &mut Context<Self>,
    ) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.trend_window_ms = window_ms;
            cx.notify();
        }
    }

    /// Write the inspector's value to the selected point of the focused pane.
    pub fn write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let text = self.write_input.read(cx).value().trim().to_owned();
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let (Some(session), Some(point)) = (live.session, live.selected) else {
            return;
        };
        let value = match text.parse::<f64>() {
            Ok(value) if value.is_finite() => value,
            _ => {
                live.write_result = Some((point, Err(format!("“{text}” is not a number"))));
                cx.notify();
                return;
            }
        };
        // A simulator reports the point held once the write is applied.
        live.write_result = None;
        self.engine.send(EngineCommand::Write {
            session,
            point,
            value,
        });
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn toggle_lock(&mut self, connection: ConnectionId, point: u32, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection)
            && let Some(session) = live.session
        {
            let locked = !live.locked.contains(&point);
            if locked {
                live.locked.insert(point);
            } else {
                live.locked.remove(&point);
            }
            self.engine.send(EngineCommand::Lock {
                session,
                point,
                locked,
            });
            cx.notify();
        }
    }

    pub fn clear_traffic(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.traffic.clear();
            cx.notify();
        }
    }

    pub fn toggle_log_pause(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.log_paused = !live.log_paused;
            cx.notify();
        }
    }

    pub fn clear_log(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.log.clear();
            cx.notify();
        }
    }

    pub fn toggle_traffic_pause(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.traffic_paused = !live.traffic_paused;
            cx.notify();
        }
    }

    // ── Overlays ──────────────────────────────────────────────────────────

    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        match &mut self.overlay {
            Overlay::Editor(editor) => Some(editor.as_mut()),
            _ => None,
        }
    }

    pub fn palette_mut(&mut self) -> Option<&mut Palette> {
        match &mut self.overlay {
            Overlay::Palette(palette) => Some(palette),
            _ => None,
        }
    }

    pub fn open_editor(
        &mut self,
        kind: EditorKind,
        editing: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_previewed_theme(cx);
        let existing = editing.and_then(|id| {
            self.workspace
                .connection(id)
                .map(|connection| (id, connection.name.clone(), connection.spec.clone()))
        });
        let kind = existing
            .as_ref()
            .map_or(kind, |(_, _, spec)| EditorKind::for_spec(spec));
        let discover = kind == EditorKind::BacnetClient && existing.is_none();
        let discover_enip = kind == EditorKind::EnipClient && existing.is_none();
        let scan = kind == EditorKind::BleClient && existing.is_none();
        self.overlay = Overlay::Editor(Box::new(Editor::new(kind, existing, window, cx)));
        self.prepare_serial_editor(cx);
        // A new BACnet client starts by looking for devices.
        if discover {
            self.discover_bacnet(cx);
        }
        if discover_enip {
            self.discover_enip(cx);
        }
        // So does a new BLE connection, on the adapter used last.
        if scan {
            let adapter = self.ble.adapter;
            if let Some(editor) = self.editor_mut() {
                editor.ble.adapter = adapter;
            }
            self.scan_ble(cx);
        }
        cx.notify();
    }

    pub fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Overlay::Editor(editor) = &mut self.overlay else {
            return;
        };
        let (name, spec) = match editor.build(cx) {
            Ok(built) => built,
            Err(error) => {
                editor.error = Some(error);
                cx.notify();
                return;
            }
        };
        let editing = editor.editing;
        let add_client = editor.add_client;
        let saved = match editing {
            Some(id) => {
                self.workspace
                    .update_connection(id, name.clone(), spec.clone());
                if self.is_running(id) {
                    self.connect(id, cx);
                }
                id
            }
            None => {
                let id = self.workspace.add_connection(name.clone(), spec.clone());
                self.workspace.open(id);
                self.connect(id, cx);
                id
            }
        };
        if editing.is_none()
            && add_client
            && let SessionSpec::ModbusSlave(slave) = &spec
            && let Some(profile) = slave.template.as_deref().and_then(templates::find_modbus)
        {
            let host = if slave.bind == "0.0.0.0" {
                "127.0.0.1"
            } else {
                slave.bind.as_str()
            };
            let client = self.workspace.add_connection(
                format!("{name} client"),
                SessionSpec::ModbusMaster(templates::master_for(
                    profile,
                    host,
                    slave.port,
                    slave.unit_id,
                )),
            );
            self.workspace
                .split(client, Axis::Horizontal, PaneView::Points);
            self.connect(client, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::MqttBroker(broker) = &spec
            && broker.template.is_some()
        {
            let client = self.workspace.add_connection(
                format!("{name} client"),
                SessionSpec::MqttClient(broker.client()),
            );
            self.workspace
                .split(client, Axis::Horizontal, PaneView::Points);
            self.connect(client, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::BacnetDevice(device) = &spec
        {
            self.add_bacnet_client(&name, device, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::OpcuaServer(server) = &spec
        {
            self.add_opcua_client(&name, server, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::Dnp3Outstation(outstation) = &spec
        {
            self.add_dnp3_master(&name, outstation, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::EnipAdapter(adapter) = &spec
        {
            self.add_enip_client(&name, adapter, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::SerialDevice(device) = &spec
        {
            self.add_serial_monitor(&name, device, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::S7Server(server) = &spec
        {
            self.add_s7_client(&name, server, cx);
        }
        if editing.is_none()
            && add_client
            && let SessionSpec::NatsServer(server) = &spec
            && server.template.is_some()
        {
            self.add_nats_client(&name, server, cx);
        }
        self.workspace.focus(
            self.workspace
                .panes
                .iter()
                .find(|pane| pane.connection == saved)
                .map_or(PaneId(0), |pane| pane.id),
        );
        self.overlay = Overlay::None;
        window.focus(&self.focus);
        self.save(cx);
        cx.notify();
    }

    pub fn open_palette(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_previewed_theme(cx);
        let original = (mode == Mode::Themes).then(|| theme::theme(cx).name.clone());
        self.overlay = Overlay::Palette(Palette::new(mode, original, window, cx));
        cx.notify();
    }

    pub fn move_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Overlay::Palette(palette) = &self.overlay else {
            return;
        };
        let count = palette::entries(self, palette.mode, &palette.query(cx)).len();
        if let Overlay::Palette(palette) = &mut self.overlay
            && count > 0
        {
            let current = isize::try_from(palette.selected).unwrap_or(0);
            let last = isize::try_from(count - 1).unwrap_or(0);
            palette.selected = usize::try_from((current + delta).clamp(0, last)).unwrap_or(0);
            palette
                .scroll
                .scroll_to_item(palette.selected, gpui::ScrollStrategy::Center);
        }
        self.preview_selected_theme(cx);
        cx.notify();
    }

    pub fn confirm_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Overlay::Palette(palette) = &self.overlay else {
            return;
        };
        let entries = palette::entries(self, palette.mode, &palette.query(cx));
        let Some(entry) = entries.into_iter().nth(palette.selected) else {
            return;
        };
        if let Overlay::Palette(palette) = &mut self.overlay {
            // The chosen theme stays: nothing to restore.
            palette.original_theme = None;
        }
        self.overlay = Overlay::None;
        window.focus(&self.focus);
        self.run(entry.command, window, cx);
        cx.notify();
    }

    pub fn run_palette_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Overlay::Palette(palette) = &mut self.overlay {
            palette.selected = index;
        }
        self.confirm_palette(window, cx);
    }

    /// While browsing themes, show the highlighted one.
    pub fn preview_selected_theme(&mut self, cx: &mut Context<Self>) {
        let Overlay::Palette(palette) = &self.overlay else {
            return;
        };
        if palette.mode != Mode::Themes {
            return;
        }
        let entries = palette::entries(self, palette.mode, &palette.query(cx));
        if let Some(palette::Entry {
            command: Command::Theme(name),
            ..
        }) = entries.get(palette.selected)
            && let Some(theme) = self.catalog.get(name)
        {
            theme::set_theme(theme, cx);
        }
    }

    fn restore_previewed_theme(&mut self, cx: &mut Context<Self>) {
        if let Overlay::Palette(Palette {
            original_theme: Some(name),
            ..
        }) = &self.overlay
            && let Some(theme) = self.catalog.get(name)
        {
            theme::set_theme(theme, cx);
        }
    }

    /// Where the "+" menu opens from the keyboard: the top of the content,
    /// beside the sidebar.
    pub fn new_menu_anchor(&self) -> Point<Pixels> {
        let ui = &self.workspace.ui;
        let left = if ui.sidebar_open {
            gpui::px(ui.sidebar_width)
        } else {
            gpui::px(0.0)
        };
        gpui::point(
            left + theme::SPACE_4,
            theme::TITLE_BAR_HEIGHT + theme::SPACE_4,
        )
    }

    /// Show the menu of protocols to add a client or simulator for.
    pub fn open_new_menu(&mut self, role: Role, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.restore_previewed_theme(cx);
        self.overlay = Overlay::NewMenu { role, at };
        cx.notify();
    }

    /// Open the editor for a new connection of `protocol` in `role`.
    pub fn new_connection(
        &mut self,
        protocol: Protocol,
        role: Role,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match EditorKind::for_protocol(protocol, role) {
            Some(kind) => self.open_editor(kind, None, window, cx),
            None => self.close_overlay(window, cx),
        }
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_previewed_theme(cx);
        self.overlay = Overlay::None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Open(id) => self.open_connection(id, cx),
            Command::Toggle(id) => {
                self.workspace.open(id);
                self.toggle_session(id, cx);
            }
            Command::Edit(id) => self.open_editor(EditorKind::Master, Some(id), window, cx),
            Command::NewMaster => self.open_editor(EditorKind::Master, None, window, cx),
            Command::NewSlave => self.open_editor(EditorKind::Slave, None, window, cx),
            Command::QuickStart => {
                self.start_demo_plant(cx);
            }
            Command::NewMqttClient => self.open_editor(EditorKind::MqttClient, None, window, cx),
            Command::NewMqttBroker => self.open_editor(EditorKind::MqttBroker, None, window, cx),
            Command::MqttDemo => {
                self.start_mqtt_demo(cx);
            }
            Command::NewBacnetClient => {
                self.open_editor(EditorKind::BacnetClient, None, window, cx)
            }
            Command::NewBacnetDevice => {
                self.open_editor(EditorKind::BacnetDevice, None, window, cx)
            }
            Command::BacnetDemo => {
                self.start_bacnet_demo(cx);
            }
            Command::NewS7Client => self.open_editor(EditorKind::S7Client, None, window, cx),
            Command::NewS7Server => self.open_editor(EditorKind::S7Server, None, window, cx),
            Command::S7Demo => {
                self.start_s7_demo(cx);
            }
            Command::ReadS7Plc(connection) => self.read_s7_plc(connection, cx),
            Command::SplitRight => self.split(Axis::Horizontal, cx),
            Command::SplitDown => self.split(Axis::Vertical, cx),
            Command::ClosePane => self.close_pane(cx),
            Command::ToggleSidebar => self.toggle_sidebar(cx),
            Command::ToggleInspector => self.toggle_inspector(cx),
            Command::View(view) => self.set_view(view, cx),
            Command::ChooseTheme => self.open_palette(Mode::Themes, window, cx),
            Command::Theme(name) => self.choose_theme(&name, window, cx),
            Command::NewOpcuaClient => self.open_editor(EditorKind::OpcuaClient, None, window, cx),
            Command::NewOpcuaServer => self.open_editor(EditorKind::OpcuaServer, None, window, cx),
            Command::OpcuaDemo => {
                self.start_opcua_demo(cx);
            }
            Command::NewDnp3Master => self.open_editor(EditorKind::Dnp3Master, None, window, cx),
            Command::NewDnp3Outstation => {
                self.open_editor(EditorKind::Dnp3Outstation, None, window, cx)
            }
            Command::Dnp3Demo => {
                self.start_dnp3_demo(cx);
            }
            Command::Dnp3(connection, action) => self.dnp3_action(connection, action, cx),
            Command::NewEnipClient => self.open_editor(EditorKind::EnipClient, None, window, cx),
            Command::NewEnipAdapter => self.open_editor(EditorKind::EnipAdapter, None, window, cx),
            Command::EnipDemo => {
                self.start_enip_demo(cx);
            }
            Command::NewSerialMonitor => {
                self.open_editor(EditorKind::SerialMonitor, None, window, cx)
            }
            Command::NewSerialDevice => {
                self.open_editor(EditorKind::SerialDevice, None, window, cx)
            }
            Command::SerialDemo => {
                self.start_serial_demo(cx);
            }
            Command::SerialMacro(connection, index) => {
                self.run_serial_macro(connection, index, cx);
            }
            Command::NewNatsClient => self.open_editor(EditorKind::NatsClient, None, window, cx),
            Command::NewNatsServer => self.open_editor(EditorKind::NatsServer, None, window, cx),
            Command::NatsDemo => {
                self.start_nats_demo(cx);
            }
            Command::NatsRequest => self.nats_request(window, cx),
            Command::NatsJetStream => match self.focused_nats_client() {
                Some(connection) => {
                    self.workspace.ui.inspector_open = true;
                    if let Some(live) = self.live.get_mut(&connection) {
                        live.selected = None;
                    }
                    self.nats_refresh_streams(connection, cx);
                }
                None => {
                    self.show_toast("Focus a NATS connection to browse its JetStream", true, cx)
                }
            },
            Command::NewBleClient => self.open_editor(EditorKind::BleClient, None, window, cx),
            Command::BleDemo => {
                self.start_ble_demo(cx);
            }
        }
    }

    // ── Theme ─────────────────────────────────────────────────────────────

    fn wants_dark(&self, window: &Window) -> bool {
        match self.workspace.ui.appearance {
            Appearance::System => theme::is_dark(window.appearance()),
            Appearance::Dark => true,
            Appearance::Light => false,
        }
    }

    pub fn resolve_theme(&self, window: &Window) -> Theme {
        let dark = self.wants_dark(window);
        let ui = &self.workspace.ui;
        self.catalog.resolve(
            if dark {
                &ui.dark_theme
            } else {
                &ui.light_theme
            },
            dark,
        )
    }

    pub fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.resolve_theme(window);
        theme::set_theme(theme, cx);
    }

    /// Use `name` from now on: it fills the slot of its brightness, and the
    /// appearance follows it when the system's differs.
    pub fn choose_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chosen) = self.catalog.get(name) else {
            return;
        };
        let ui = &mut self.workspace.ui;
        if chosen.dark {
            ui.dark_theme = name.to_owned();
        } else {
            ui.light_theme = name.to_owned();
        }
        if chosen.dark != theme::is_dark(window.appearance()) {
            ui.appearance = if chosen.dark {
                Appearance::Dark
            } else {
                Appearance::Light
            };
        } else {
            ui.appearance = Appearance::System;
        }
        self.apply_theme(window, cx);
        self.save(cx);
    }

    // ── Drags ─────────────────────────────────────────────────────────────

    pub fn begin_drag(&mut self, drag: Drag) {
        self.drag = Some(drag);
    }

    pub fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else {
            return;
        };
        match drag {
            Drag::Sidebar {
                start_x,
                start_width,
            } => {
                let width = start_width + f32::from(position.x - *start_x);
                self.workspace.ui.sidebar_width = width.clamp(180.0, 480.0);
            }
            Drag::Inspector {
                start_x,
                start_width,
            } => {
                let width = start_width - f32::from(position.x - *start_x);
                self.workspace.ui.inspector_width = width.clamp(240.0, 560.0);
            }
            Drag::Split {
                tab,
                path,
                axis,
                start,
                start_ratio,
                extent,
            } => {
                let delta = match axis {
                    Axis::Horizontal => position.x - start.x,
                    Axis::Vertical => position.y - start.y,
                };
                let ratio = start_ratio + f32::from(delta) / f32::from(*extent).max(1.0);
                let (tab, path) = (*tab, path.clone());
                if let Some(tab) = self.workspace.tab_mut(tab) {
                    tab.layout.set_ratio(&path, ratio);
                }
            }
        }
        cx.notify();
    }

    pub fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.save(cx);
            cx.notify();
        }
    }

    // ── Persistence and toasts ────────────────────────────────────────────

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.store else {
            return;
        };
        if let Err(error) = store::save(path, &self.workspace) {
            self.show_toast(error.to_string(), true, cx);
        }
    }

    fn save_quietly(&self) {
        if let Some(path) = &self.store {
            let _ = store::save(path, &self.workspace);
        }
    }

    pub fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toast = Some(Toast {
            message: message.into(),
            error,
            id,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |model, cx| {
                if model.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    model.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
