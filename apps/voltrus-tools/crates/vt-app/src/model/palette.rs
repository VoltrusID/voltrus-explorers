//! The command palette (⌘K) and the theme picker, which is the palette
//! listing themes.

use gpui::{
    AppContext as _, Context, Entity, SharedString, Subscription, UniformListScrollHandle, Window,
};
use vt_app_core::{ConnectionId, PaneView, search};
use vt_protocol::Role;
use vt_ui::Icon;
use vt_ui::text_input::{InputEvent, TextInput};

use super::AppModel;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Open(ConnectionId),
    Toggle(ConnectionId),
    Edit(ConnectionId),
    NewMaster,
    NewSlave,
    QuickStart,
    NewMqttClient,
    NewMqttBroker,
    MqttDemo,
    SplitRight,
    SplitDown,
    ClosePane,
    ToggleSidebar,
    ToggleInspector,
    View(PaneView),
    ChooseTheme,
    Theme(String),
    NewBacnetClient,
    NewBacnetDevice,
    BacnetDemo,
    NewDnp3Master,
    NewDnp3Outstation,
    Dnp3Demo,
    /// A DNP3 station command for a connection's session.
    Dnp3(ConnectionId, vt_protocol::Dnp3Action),
    NewEnipClient,
    NewEnipAdapter,
    EnipDemo,
    NewSerialMonitor,
    NewSerialDevice,
    SerialDemo,
    /// Send a serial connection's macro, by index.
    SerialMacro(ConnectionId, usize),
    NewS7Client,
    NewS7Server,
    S7Demo,
    /// Read an S7 client's CPU identity, mode, diagnostics and blocks again.
    ReadS7Plc(ConnectionId),
    NewNatsClient,
    NewNatsServer,
    NatsDemo,
    /// Send the inspector's NATS request from the focused client.
    NatsRequest,
    /// Show the focused NATS client's JetStream streams.
    NatsJetStream,
    NewBleClient,
    BleDemo,
}

pub struct Entry {
    pub title: SharedString,
    pub detail: SharedString,
    pub icon: Icon,
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Themes,
}

pub struct Palette {
    pub mode: Mode,
    pub input: Entity<TextInput>,
    pub selected: usize,
    /// Theme to restore when the theme picker is dismissed.
    pub original_theme: Option<String>,
    pub scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl Palette {
    pub fn new(
        mode: Mode,
        original_theme: Option<String>,
        window: &mut Window,
        cx: &mut Context<AppModel>,
    ) -> Self {
        let placeholder = match mode {
            Mode::Commands => "Search connections and commands…",
            Mode::Themes => "Search themes…",
        };
        let input = cx.new(|cx| TextInput::new(cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => {
                    if let Some(palette) = model.palette_mut() {
                        palette.selected = 0;
                    }
                    model.preview_selected_theme(cx);
                    cx.notify();
                }
                InputEvent::Submitted => model.confirm_palette(window, cx),
                InputEvent::Cancelled => model.close_overlay(window, cx),
                InputEvent::Blurred => {}
            },
        );
        input.update(cx, |input, _| input.focus(window));
        Self {
            mode,
            input,
            selected: 0,
            original_theme,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        }
    }

    pub fn query(&self, cx: &gpui::App) -> String {
        self.input.read(cx).value().to_owned()
    }
}

/// Palette entries matching `query`, best first.
pub fn entries(model: &AppModel, mode: Mode, query: &str) -> Vec<Entry> {
    let mut entries = match mode {
        Mode::Commands => commands(model),
        Mode::Themes => model
            .catalog
            .names()
            .map(|name| Entry {
                title: name.to_owned().into(),
                detail: SharedString::default(),
                icon: Icon::Palette,
                command: Command::Theme(name.to_owned()),
            })
            .collect(),
    };
    if query.trim().is_empty() {
        return entries;
    }
    let mut scored: Vec<(i32, Entry)> = entries
        .drain(..)
        .filter_map(|entry| {
            let haystack = format!("{} {}", entry.title, entry.detail);
            search::score(query, &haystack).map(|score| (score, entry))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, entry)| entry).collect()
}

fn commands(model: &AppModel) -> Vec<Entry> {
    let mut entries = Vec::new();
    for connection in &model.workspace.connections {
        let running = model
            .live
            .get(&connection.id)
            .is_some_and(|live| live.state.is_running());
        let simulator = connection.spec.role() == Role::Simulator;
        entries.push(Entry {
            title: connection.name.clone().into(),
            detail: format!("Open · {}", connection.spec.endpoint()).into(),
            icon: if simulator { Icon::Server } else { Icon::Cpu },
            command: Command::Open(connection.id),
        });
        entries.push(Entry {
            title: format!(
                "{} {}",
                match (running, simulator) {
                    (true, true) => "Stop",
                    (true, false) => "Disconnect",
                    (false, true) => "Start",
                    (false, false) => "Connect",
                },
                connection.name
            )
            .into(),
            detail: connection.spec.endpoint().into(),
            icon: if running { Icon::Unplug } else { Icon::Play },
            command: Command::Toggle(connection.id),
        });
        entries.push(Entry {
            title: format!("Edit {}", connection.name).into(),
            detail: "Connection settings".into(),
            icon: Icon::Pencil,
            command: Command::Edit(connection.id),
        });
    }
    let fixed = [
        (
            "New Modbus connection…",
            "Poll a device over TCP or RTU",
            Icon::Plus,
            Command::NewMaster,
        ),
        (
            "New Modbus simulator…",
            "Serve a device template on this computer",
            Icon::Server,
            Command::NewSlave,
        ),
        (
            "Start the demo plant",
            "A simulated inverter and a client polling it",
            Icon::Zap,
            Command::QuickStart,
        ),
        (
            "New MQTT connection…",
            "Subscribe to a broker's topics and publish to it",
            Icon::Plus,
            Command::NewMqttClient,
        ),
        (
            "New MQTT broker simulator…",
            "A broker on this computer publishing a device template",
            Icon::Server,
            Command::NewMqttBroker,
        ),
        (
            "Start the MQTT demo plant",
            "An inverter on a simulated broker and a client subscribed to it",
            Icon::Zap,
            Command::MqttDemo,
        ),
        (
            "New BACnet connection…",
            "Discover a device with Who-Is, read its objects, command them",
            Icon::Plus,
            Command::NewBacnetClient,
        ),
        (
            "New BACnet device simulator…",
            "Play an air handler, chiller or VAV box on this computer",
            Icon::Server,
            Command::NewBacnetDevice,
        ),
        (
            "Start the BACnet demo",
            "A simulated air handler and a client subscribed to it",
            Icon::Zap,
            Command::BacnetDemo,
        ),
        (
            "New DNP3 connection…",
            "Poll an outstation: integrity and class polls, controls, time sync",
            Icon::Plus,
            Command::NewDnp3Master,
        ),
        (
            "New DNP3 outstation simulator…",
            "Play a feeder, transformer bay or solar plant on this computer",
            Icon::Server,
            Command::NewDnp3Outstation,
        ),
        (
            "Start the DNP3 demo",
            "A simulated distribution feeder and a master polling it",
            Icon::Zap,
            Command::Dnp3Demo,
        ),
        (
            "New EtherNet/IP connection…",
            "Find a device with ListIdentity, read Logix tags and assemblies, send CIP messages",
            Icon::Plus,
            Command::NewEnipClient,
        ),
        (
            "New EtherNet/IP adapter simulator…",
            "Play a motor drive, tank farm or packaging line on this computer",
            Icon::Server,
            Command::NewEnipAdapter,
        ),
        (
            "Start the EtherNet/IP demo",
            "A simulated motor drive and a client reading its tags",
            Icon::Zap,
            Command::EnipDemo,
        ),
        (
            "Split right",
            "Show another connection beside this one",
            Icon::Columns,
            Command::SplitRight,
        ),
        (
            "Split down",
            "Show another connection below this one",
            Icon::Rows,
            Command::SplitDown,
        ),
        ("Close pane", "", Icon::X, Command::ClosePane),
        (
            "Toggle sidebar",
            "",
            Icon::PanelLeft,
            Command::ToggleSidebar,
        ),
        (
            "Toggle inspector",
            "",
            Icon::PanelRight,
            Command::ToggleInspector,
        ),
        (
            "Show points",
            "",
            Icon::Table,
            Command::View(PaneView::Points),
        ),
        (
            "Show trend",
            "",
            Icon::ChartLine,
            Command::View(PaneView::Trend),
        ),
        (
            "Show traffic",
            "",
            Icon::ArrowUpDown,
            Command::View(PaneView::Traffic),
        ),
        (
            "Show log",
            "",
            Icon::ScrollText,
            Command::View(PaneView::Log),
        ),
        (
            "Choose theme…",
            "Colour schemes, previewed live",
            Icon::Palette,
            Command::ChooseTheme,
        ),
        (
            "New serial connection…",
            "Open a port: terminal, sends, macros, logging, lines into trends",
            Icon::Plus,
            Command::NewSerialMonitor,
        ),
        (
            "New serial device simulator…",
            "An instrument printing its values on a virtual port",
            Icon::Server,
            Command::NewSerialDevice,
        ),
        (
            "Start the serial demo",
            "A water analyser on a virtual port and a monitor trending its lines",
            Icon::Zap,
            Command::SerialDemo,
        ),
        (
            "New NATS connection…",
            "Subscribe to subjects, publish, request and browse JetStream",
            Icon::Plus,
            Command::NewNatsClient,
        ),
        (
            "New NATS server simulator…",
            "A NATS server with JetStream on this computer publishing a device",
            Icon::Server,
            Command::NewNatsServer,
        ),
        (
            "Start the NATS demo",
            "An inverter on a simulated NATS server and a client subscribed to it",
            Icon::Zap,
            Command::NatsDemo,
        ),
        (
            "Send NATS request",
            "The inspector's subject and payload, from the focused connection",
            Icon::Send,
            Command::NatsRequest,
        ),
        (
            "Browse JetStream",
            "Streams, consumers and stored messages of the focused connection",
            Icon::Database,
            Command::NatsJetStream,
        ),
        (
            "New Bluetooth LE connection…",
            "Scan for a peripheral, browse its GATT services, read, write and subscribe",
            Icon::Bluetooth,
            Command::NewBleClient,
        ),
        (
            "Start the Bluetooth LE demo",
            "A simulated heart rate monitor and environment sensor",
            Icon::Zap,
            Command::BleDemo,
        ),
    ];
    entries.extend(
        fixed
            .into_iter()
            .map(|(title, detail, icon, command)| Entry {
                title: title.into(),
                detail: detail.into(),
                icon,
                command,
            }),
    );
    entries.extend(super::dnp3::palette_entries(model));
    // The focused serial connection's macros, so they are a ⌘K away.
    if let Some(connection) = model.focused_connection()
        && let Some(saved) = model.workspace.connection(connection)
        && let vt_protocol::SessionSpec::SerialMonitor(monitor) = &saved.spec
    {
        entries.extend(monitor.macros.iter().enumerate().map(|(index, item)| {
            Entry {
                title: format!("Send {}", item.name).into(),
                detail: format!(
                    "{} · {}",
                    saved.name,
                    if item.hex {
                        item.payload.clone()
                    } else {
                        format!("“{}”", item.payload)
                    }
                )
                .into(),
                icon: Icon::Send,
                command: Command::SerialMacro(connection, index),
            }
        }));
    }
    entries.extend(super::s7::palette_entries(model));
    entries
}
