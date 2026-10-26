//! `--demo <scenario>`: a fixed, in-memory setup per screen, for screenshots
//! and trying the app without hardware. Nothing is saved.

use std::time::Duration;

use gpui::{Context, Window};
use vt_app_core::{Appearance, ConnectionId, PaneId, PaneView};

use crate::model::AppModel;
use crate::model::editor::EditorKind;
use crate::model::palette::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Welcome,
    Points,
    Trend,
    Traffic,
    Simulator,
    Editor,
    Palette,
    Light,
    MqttPoints,
    MqttTrend,
    MqttTraffic,
    MqttInspector,
    MqttEditor,
    BacnetPoints,
    BacnetTrend,
    BacnetTraffic,
    BacnetDiscover,
    BacnetCommand,
    OpcuaPoints,
    OpcuaTrend,
    OpcuaMethod,
    OpcuaEditor,
    Dnp3Points,
    Dnp3Trend,
    Dnp3Traffic,
    Dnp3Control,
    Dnp3Station,
    Dnp3Editor,
    EnipPoints,
    EnipTrend,
    EnipTraffic,
    EnipDiscover,
    EnipMessage,
    SerialTerminal,
    SerialHex,
    SerialTrend,
    SerialPoints,
    SerialEditor,
    SerialSimulator,
    S7Points,
    S7Trend,
    S7Traffic,
    S7Plc,
    S7Write,
    S7Editor,
    NatsPoints,
    NatsTrend,
    NatsTraffic,
    NatsInspector,
    NatsJetstream,
    NatsEditor,
    BlePoints,
    BleTrend,
    BleScan,
    BleInspector,
}

impl Scenario {
    pub const NAMES: &'static [&'static str] = &[
        "welcome",
        "points",
        "trend",
        "traffic",
        "simulator",
        "editor",
        "palette",
        "light",
        "mqtt-points",
        "mqtt-trend",
        "mqtt-traffic",
        "mqtt-inspector",
        "mqtt-editor",
        "bacnet-points",
        "bacnet-trend",
        "bacnet-traffic",
        "bacnet-discover",
        "bacnet-command",
        "opcua-points",
        "opcua-trend",
        "opcua-method",
        "opcua-editor",
        "dnp3-points",
        "dnp3-trend",
        "dnp3-traffic",
        "dnp3-control",
        "dnp3-station",
        "dnp3-editor",
        "enip-points",
        "enip-trend",
        "enip-traffic",
        "enip-discover",
        "enip-message",
        "serial-terminal",
        "serial-hex",
        "serial-trend",
        "serial-points",
        "serial-editor",
        "serial-simulator",
        "s7-points",
        "s7-trend",
        "s7-traffic",
        "s7-plc",
        "s7-write",
        "s7-editor",
        "nats-points",
        "nats-trend",
        "nats-traffic",
        "nats-inspector",
        "nats-jetstream",
        "nats-editor",
        "ble-points",
        "ble-trend",
        "ble-scan",
        "ble-inspector",
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "welcome" => Self::Welcome,
            "points" => Self::Points,
            "trend" => Self::Trend,
            "traffic" => Self::Traffic,
            "simulator" => Self::Simulator,
            "editor" => Self::Editor,
            "palette" => Self::Palette,
            "light" => Self::Light,
            "mqtt-points" => Self::MqttPoints,
            "mqtt-trend" => Self::MqttTrend,
            "mqtt-traffic" => Self::MqttTraffic,
            "mqtt-inspector" => Self::MqttInspector,
            "mqtt-editor" => Self::MqttEditor,
            "bacnet-points" => Self::BacnetPoints,
            "bacnet-trend" => Self::BacnetTrend,
            "bacnet-traffic" => Self::BacnetTraffic,
            "bacnet-discover" => Self::BacnetDiscover,
            "bacnet-command" => Self::BacnetCommand,
            "opcua-points" => Self::OpcuaPoints,
            "opcua-trend" => Self::OpcuaTrend,
            "opcua-method" => Self::OpcuaMethod,
            "opcua-editor" => Self::OpcuaEditor,
            "dnp3-points" => Self::Dnp3Points,
            "dnp3-trend" => Self::Dnp3Trend,
            "dnp3-traffic" => Self::Dnp3Traffic,
            "dnp3-control" => Self::Dnp3Control,
            "dnp3-station" => Self::Dnp3Station,
            "dnp3-editor" => Self::Dnp3Editor,
            "enip-points" => Self::EnipPoints,
            "enip-trend" => Self::EnipTrend,
            "enip-traffic" => Self::EnipTraffic,
            "enip-discover" => Self::EnipDiscover,
            "enip-message" => Self::EnipMessage,
            "serial-terminal" => Self::SerialTerminal,
            "serial-hex" => Self::SerialHex,
            "serial-trend" => Self::SerialTrend,
            "serial-points" => Self::SerialPoints,
            "serial-editor" => Self::SerialEditor,
            "serial-simulator" => Self::SerialSimulator,
            "s7-points" => Self::S7Points,
            "s7-trend" => Self::S7Trend,
            "s7-traffic" => Self::S7Traffic,
            "s7-plc" => Self::S7Plc,
            "s7-write" => Self::S7Write,
            "s7-editor" => Self::S7Editor,
            "nats-points" => Self::NatsPoints,
            "nats-trend" => Self::NatsTrend,
            "nats-traffic" => Self::NatsTraffic,
            "nats-inspector" => Self::NatsInspector,
            "nats-jetstream" => Self::NatsJetstream,
            "nats-editor" => Self::NatsEditor,
            "ble-points" => Self::BlePoints,
            "ble-trend" => Self::BleTrend,
            "ble-scan" => Self::BleScan,
            "ble-inspector" => Self::BleInspector,
            _ => return None,
        })
    }
}

/// Points of the SMA inverter template worth looking at: DC power, AC power,
/// grid frequency and heatsink temperature.
const INTERESTING: [u32; 4] = [4, 11, 12, 15];

/// Window of the trend scenario: the screenshot script waits a little longer
/// than this so the lines span the whole chart.
const TREND_WINDOW_MS: f64 = 30_000.0;

/// Points of the AHU plant to trend: supply, return and outdoor air
/// temperatures and the supply temperature setpoint.
const BACNET_TREND: [u32; 4] = [0, 1, 2, 9];
/// The AHU's supply temperature setpoint, a commandable analog value.
const BACNET_SETPOINT: u32 = 9;

pub fn apply(
    model: &mut AppModel,
    scenario: Scenario,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) {
    // Screenshots must not depend on the machine's appearance setting.
    model.workspace.ui.appearance = if scenario == Scenario::Light {
        Appearance::Light
    } else {
        Appearance::Dark
    };
    model.apply_theme(window, cx);

    match scenario {
        Scenario::Welcome => {}
        Scenario::Editor => model.open_editor(EditorKind::Master, None, window, cx),
        Scenario::MqttEditor => model.open_editor(EditorKind::MqttClient, None, window, cx),
        Scenario::MqttPoints
        | Scenario::MqttTrend
        | Scenario::MqttTraffic
        | Scenario::MqttInspector => mqtt(model, scenario, cx),
        Scenario::BacnetPoints
        | Scenario::BacnetTrend
        | Scenario::BacnetTraffic
        | Scenario::BacnetDiscover
        | Scenario::BacnetCommand => bacnet(model, scenario, window, cx),
        Scenario::OpcuaPoints
        | Scenario::OpcuaTrend
        | Scenario::OpcuaMethod
        | Scenario::OpcuaEditor => opcua(model, scenario, window, cx),
        Scenario::Dnp3Points
        | Scenario::Dnp3Trend
        | Scenario::Dnp3Traffic
        | Scenario::Dnp3Control
        | Scenario::Dnp3Station
        | Scenario::Dnp3Editor => dnp3(model, scenario, window, cx),
        Scenario::EnipPoints
        | Scenario::EnipTrend
        | Scenario::EnipTraffic
        | Scenario::EnipDiscover
        | Scenario::EnipMessage => enip(model, scenario, window, cx),
        Scenario::SerialTerminal
        | Scenario::SerialHex
        | Scenario::SerialTrend
        | Scenario::SerialPoints
        | Scenario::SerialEditor
        | Scenario::SerialSimulator => serial(model, scenario, window, cx),
        Scenario::S7Points
        | Scenario::S7Trend
        | Scenario::S7Traffic
        | Scenario::S7Plc
        | Scenario::S7Write
        | Scenario::S7Editor => s7(model, scenario, window, cx),
        Scenario::NatsEditor => model.open_editor(EditorKind::NatsClient, None, window, cx),
        Scenario::NatsPoints
        | Scenario::NatsTrend
        | Scenario::NatsTraffic
        | Scenario::NatsInspector
        | Scenario::NatsJetstream => nats(model, scenario, cx),
        Scenario::BlePoints | Scenario::BleTrend | Scenario::BleScan | Scenario::BleInspector => {
            ble(model, scenario, window, cx)
        }
        Scenario::Palette => {
            model.start_demo_plant(cx);
            model.workspace.ui.inspector_open = false;
            model.open_palette(Mode::Commands, window, cx);
        }
        Scenario::Points
        | Scenario::Trend
        | Scenario::Traffic
        | Scenario::Simulator
        | Scenario::Light => {
            let Some((client, simulator)) = model.start_demo_plant(cx) else {
                return;
            };
            if let Some(live) = model.live.get_mut(&client) {
                live.selected = Some(INTERESTING[1]);
                live.trend = INTERESTING.into_iter().collect();
                live.trend_window_ms = TREND_WINDOW_MS;
            }
            let pane_of = |model: &AppModel, connection: ConnectionId| {
                model
                    .workspace
                    .panes
                    .iter()
                    .find(|pane| pane.connection == connection)
                    .map(|pane| pane.id)
            };
            match scenario {
                // The plant side by side: both panes get the full width.
                Scenario::Points => model.workspace.ui.inspector_open = false,
                Scenario::Trend => {
                    model.workspace.ui.inspector_open = false;
                    model.set_view(PaneView::Trend, cx);
                }
                Scenario::Traffic => {
                    model.workspace.ui.inspector_open = false;
                    model.set_view(PaneView::Traffic, cx);
                }
                // One connection with the inspector beside it.
                Scenario::Simulator => {
                    if let Some(pane) = pane_of(model, client) {
                        model.workspace.close_pane(pane);
                    }
                    if let Some(pane) = pane_of(model, simulator) {
                        model.focus_pane(pane, cx);
                    }
                    if let Some(live) = model.live.get_mut(&simulator) {
                        live.selected = Some(INTERESTING[3]);
                    }
                }
                Scenario::Light => {
                    if let Some(pane) = pane_of(model, simulator) {
                        model.workspace.close_pane(pane);
                    }
                    if let Some(pane) = pane_of(model, client) {
                        model.focus_pane(pane, cx);
                    }
                }
                _ => {}
            }
        }
    }
}

/// What the MQTT demo trends: the Sparkplug metrics of the inverter, which
/// carry their units.
const MQTT_TREND: [&str; 4] = [
    "› DC Power",
    "› AC Power",
    "› AC Frequency",
    "› Temperature",
];
/// The point the MQTT inspector scenario shows.
const MQTT_SELECTED: &str = "› AC Power";
/// Topics are points once their first messages arrive (Sparkplug metrics
/// after the client asks the node for its births): pick by name then.
const MQTT_SETTLE: Duration = Duration::from_secs(3);

fn mqtt(model: &mut AppModel, scenario: Scenario, cx: &mut Context<AppModel>) {
    let Some((client, simulator)) = model.start_mqtt_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    match scenario {
        Scenario::MqttTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::MqttTraffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
        }
        // One client with the inspector: the selected metric's message and
        // the publish form.
        Scenario::MqttInspector => {
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            model.publish.topic.update(cx, |input, cx| {
                input.set_text("plant/inverter/temperature/set", cx)
            });
            model
                .publish
                .payload
                .update(cx, |input, cx| input.set_text("48.5", cx));
        }
        _ => model.workspace.ui.inspector_open = false,
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(MQTT_SETTLE).await;
        let _ = this.update(cx, |model, cx| {
            let Some(live) = model.live.get(&client) else {
                return;
            };
            let find = |suffix: &str| {
                live.points
                    .iter()
                    .position(|point| point.name.ends_with(suffix))
                    .and_then(|index| u32::try_from(index).ok())
            };
            let trend = MQTT_TREND
                .iter()
                .filter_map(|suffix| find(suffix))
                .collect();
            let selected = find(MQTT_SELECTED);
            if let Some(live) = model.live.get_mut(&client) {
                live.trend = trend;
            }
            if scenario == Scenario::MqttInspector
                && let Some(point) = selected
            {
                model.select_point(client, point, cx);
            }
            cx.notify();
        });
    })
    .detach();
}

fn pane_of(model: &AppModel, connection: ConnectionId) -> Option<PaneId> {
    model
        .workspace
        .panes
        .iter()
        .find(|pane| pane.connection == connection)
        .map(|pane| pane.id)
}

/// The BACnet demo: a simulated air handler and a client subscribed to it.
fn bacnet(
    model: &mut AppModel,
    scenario: Scenario,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) {
    let Some((client, simulator)) = model.start_bacnet_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend = BACNET_TREND.into_iter().collect();
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    match scenario {
        Scenario::BacnetPoints => {
            model.workspace.ui.inspector_open = false;
            if let Some(live) = model.live.get_mut(&client) {
                live.selected = Some(BACNET_SETPOINT);
            }
        }
        Scenario::BacnetTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::BacnetTraffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.set_view(pane, PaneView::Traffic);
            }
        }
        // The device picker, with the simulator answering its Who-Is. The
        // editor asks as it opens, maybe before the simulator listens: ask
        // again once it does.
        Scenario::BacnetDiscover => {
            model.workspace.ui.inspector_open = false;
            model.open_editor(EditorKind::BacnetClient, None, window, cx);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = this.update(cx, |model, cx| model.discover_bacnet(cx));
            })
            .detach();
        }
        // One client with a commandable point in the inspector.
        Scenario::BacnetCommand => {
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            if let Some(live) = model.live.get_mut(&client) {
                live.selected = Some(BACNET_SETPOINT);
            }
        }
        _ => {}
    }
}

/// The OPC UA demo: the plant server and a client browsing and monitoring it.
fn opcua(
    model: &mut AppModel,
    scenario: Scenario,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) {
    let Some((client, simulator)) = model.start_opcua_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend = (0..4).collect();
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    let plant = |key: &str| format!("ns={};s={key}", crate::model::opcua::PLANT_NAMESPACE_INDEX);
    match scenario {
        // The client alone: the browser open on the reactor with a variable picked, beside its
        // points.
        Scenario::OpcuaPoints => {
            model.workspace.ui.inspector_open = false;
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            let (parent, node) = (plant("Plant.Reactor"), plant("Plant.Reactor.AgitatorSpeed"));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = this.update(cx, |model, cx| model.select_node(client, parent, node, cx));
            })
            .detach();
        }
        Scenario::OpcuaTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        // One client, the StartBatch method in the inspector, called once.
        Scenario::OpcuaMethod => {
            // The inspector beside the browser and points: the sidebar makes room.
            model.workspace.ui.sidebar_open = false;
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            let (parent, node) = (plant("Plant.Reactor"), plant("Plant.Reactor.StartBatch"));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = this.update(cx, |model, cx| {
                    model.select_node(client, parent, node, cx);
                    for (input, text) in model.opcua.arguments.iter().zip(["Pale ale", "500"]) {
                        input.update(cx, |input, cx| input.set_text(text, cx));
                    }
                });
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, |model, cx| model.call_selected_method(cx));
            })
            .detach();
        }
        // A new client asking the server for its endpoints.
        Scenario::OpcuaEditor => {
            model.workspace.ui.inspector_open = false;
            let port = match model
                .workspace
                .connection(simulator)
                .map(|saved| &saved.spec)
            {
                Some(vt_protocol::SessionSpec::OpcuaServer(server)) => server.port,
                _ => vt_protocol::OPCUA_PORT,
            };
            model.open_editor(EditorKind::OpcuaClient, None, window, cx);
            if let Some(endpoint) = model
                .editor_mut()
                .map(|editor| editor.opcua.endpoint.clone())
            {
                let url = format!("opc.tcp://localhost:{port}");
                endpoint.update(cx, |input, cx| input.set_text(url, cx));
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = this.update(cx, |model, cx| model.discover_opcua(cx));
            })
            .detach();
        }
        _ => {}
    }
}

/// A point of the feeder plant the DNP3 demo plays, by key.
fn feeder_point(key: &str) -> Option<u32> {
    vt_engine::templates::find_dnp3("feeder")?
        .points
        .iter()
        .position(|point| point.key == key)
        .and_then(|index| u32::try_from(index).ok())
}

/// What the DNP3 demo trends: the three phase currents and active power.
const DNP3_TREND: [&str; 4] = ["i_a", "i_b", "i_c", "mw"];
/// Long enough for both sessions to run before the demo acts on them.
const DNP3_SETTLE: Duration = Duration::from_secs(2);

/// The DNP3 demo: a simulated distribution feeder and a master polling it.
fn dnp3(model: &mut AppModel, scenario: Scenario, window: &mut Window, cx: &mut Context<AppModel>) {
    let Some((client, simulator)) = model.start_dnp3_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend = DNP3_TREND
            .iter()
            .filter_map(|key| feeder_point(key))
            .collect();
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    let single = |model: &mut AppModel, cx: &mut Context<AppModel>| {
        if let Some(pane) = pane_of(model, simulator) {
            model.workspace.close_pane(pane);
        }
        if let Some(pane) = pane_of(model, client) {
            model.focus_pane(pane, cx);
        }
    };
    // What the demo does once the master is polling.
    let mut later: Option<vt_protocol::Dnp3Action> = None;
    match scenario {
        // Side by side, with a phase current the simulator takes offline:
        // the master reads it bad and the inspector shows its flags.
        Scenario::Dnp3Points => {
            if let Some(point) = feeder_point("i_b") {
                if let Some(live) = model.live.get_mut(&client) {
                    live.selected = Some(point);
                }
                later = Some(vt_protocol::Dnp3Action::SetOnline {
                    point,
                    online: false,
                });
            }
        }
        Scenario::Dnp3Trend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::Dnp3Traffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.set_view(pane, PaneView::Traffic);
            }
        }
        // The breaker's control in the inspector, tripped by select-before-operate.
        Scenario::Dnp3Control => {
            single(model, cx);
            if let Some(point) = feeder_point("breaker_cmd") {
                if let Some(live) = model.live.get_mut(&client) {
                    live.selected = Some(point);
                }
                later = Some(vt_protocol::Dnp3Action::Operate {
                    point,
                    operation: vt_protocol::Dnp3Operation::Crob(vt_protocol::Dnp3Crob::new(
                        vt_protocol::Dnp3CrobCode::Trip,
                    )),
                    mode: vt_protocol::Dnp3ControlMode::SelectBeforeOperate,
                });
            }
        }
        // The station card: indications, commands and a time sync's answer.
        Scenario::Dnp3Station => {
            single(model, cx);
            later = Some(vt_protocol::Dnp3Action::TimeSync);
        }
        Scenario::Dnp3Editor => {
            model.workspace.ui.inspector_open = false;
            model.open_editor(EditorKind::Dnp3Master, Some(client), window, cx);
        }
        _ => {}
    }
    if let Some(action) = later {
        let target = if matches!(action, vt_protocol::Dnp3Action::SetOnline { .. }) {
            simulator
        } else {
            client
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DNP3_SETTLE).await;
            let _ = this.update(cx, |model, cx| model.dnp3_action(target, action, cx));
        })
        .detach();
    }
}

/// Points of the drive to trend: current, torque, temperature and frequency.
const ENIP_TREND: [u32; 4] = [2, 3, 6, 1];
/// The drive's speed reference, a setpoint the speed follows.
const ENIP_SETPOINT: u32 = 11;

/// The EtherNet/IP demo: a simulated motor drive and a client reading it.
fn enip(model: &mut AppModel, scenario: Scenario, window: &mut Window, cx: &mut Context<AppModel>) {
    let Some((client, simulator)) = model.start_enip_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend = ENIP_TREND.into_iter().collect();
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    match scenario {
        Scenario::EnipPoints => {
            model.workspace.ui.inspector_open = false;
            if let Some(live) = model.live.get_mut(&client) {
                live.selected = Some(ENIP_SETPOINT);
            }
        }
        Scenario::EnipTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::EnipTraffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.set_view(pane, PaneView::Traffic);
            }
        }
        // The device picker, with the adapter answering ListIdentity. The
        // editor asks as it opens, maybe before the adapter listens: ask
        // again once it does.
        Scenario::EnipDiscover => {
            model.workspace.ui.inspector_open = false;
            model.open_editor(EditorKind::EnipClient, None, window, cx);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = this.update(cx, |model, cx| model.discover_enip(cx));
            })
            .detach();
        }
        // One client with an explicit message answered in the inspector:
        // the adapter's input assembly, every tag's value as bytes.
        Scenario::EnipMessage => {
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            let request = [
                (model.cip.service.clone(), "0x0E"),
                (model.cip.class.clone(), "0x04"),
                (model.cip.instance.clone(), "100"),
                (model.cip.attribute.clone(), "3"),
            ];
            for (input, text) in request {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = this.update(cx, |model, cx| model.dispatch_cip_request(cx));
            })
            .detach();
        }
        _ => {}
    }
}
/// What the serial demo trends: the analyser's temperature, dissolved
/// oxygen, pH and turbidity, named as its lines name them.
const SERIAL_TREND: [&str; 4] = ["Temperature", "Dissolved_Oxygen", "pH", "Turbidity"];
/// Lines become points as the device prints them: pick by name then.
const SERIAL_SETTLE: Duration = Duration::from_millis(2500);

/// The serial demo: an ABB water analyser printing on a virtual port and a
/// monitor reading it, with commands typed at the device.
fn serial(
    model: &mut AppModel,
    scenario: Scenario,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) {
    let Some((client, simulator)) = model.start_serial_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    match scenario {
        Scenario::SerialTerminal | Scenario::SerialHex => {
            model.workspace.ui.inspector_open = false;
            if let Some(live) = model.live.get_mut(&client) {
                live.terminal.hex = scenario == Scenario::SerialHex;
            }
        }
        Scenario::SerialTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        // The monitor's points with its line settings and macros beside.
        Scenario::SerialPoints => {
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            model.set_view(PaneView::Points, cx);
        }
        // The port picker, listing the analyser's virtual port.
        Scenario::SerialEditor => {
            model.workspace.ui.inspector_open = false;
            model.open_editor(EditorKind::SerialMonitor, None, window, cx);
        }
        // The device's side: what it printed and the commands it took.
        Scenario::SerialSimulator => {
            if let Some(pane) = pane_of(model, client) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, simulator) {
                model.focus_pane(pane, cx);
            }
            model.set_view(PaneView::Traffic, cx);
        }
        _ => {}
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SERIAL_SETTLE).await;
        let _ = this.update(cx, |model, cx| {
            // Help, then hold the temperature: the device answers both.
            model.run_serial_macro(client, 1, cx);
            model.run_serial_macro(client, 2, cx);
            if let Some(console) = model.serial_consoles.get(&client) {
                console
                    .input
                    .update(cx, |input, cx| input.set_text("RATE 250", cx));
            }
            if let Some(live) = model.live.get_mut(&client) {
                live.trend = SERIAL_TREND
                    .iter()
                    .filter_map(|name| {
                        live.points
                            .iter()
                            .position(|point| point.name == *name)
                            .and_then(|index| u32::try_from(index).ok())
                    })
                    .collect();
            }
            cx.notify();
        });
    })
    .detach();
}

/// Tags of the motor drive to trend: speed, current, winding temperature
/// and power.
const S7_TREND: [u32; 4] = [0, 1, 2, 4];
/// The motor drive's speed setpoint, a writable Real in DB1.
const S7_SETPOINT: u32 = 9;

/// The S7 demo: a simulated S7-1500 motor drive and a client reading it.
fn s7(model: &mut AppModel, scenario: Scenario, window: &mut Window, cx: &mut Context<AppModel>) {
    let Some((client, simulator)) = model.start_s7_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend = S7_TREND.into_iter().collect();
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    let alone = |model: &mut AppModel, cx: &mut Context<AppModel>| {
        if let Some(pane) = pane_of(model, simulator) {
            model.workspace.close_pane(pane);
        }
        if let Some(pane) = pane_of(model, client) {
            model.focus_pane(pane, cx);
        }
    };
    match scenario {
        Scenario::S7Points => model.workspace.ui.inspector_open = false,
        Scenario::S7Trend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::S7Traffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.set_view(pane, PaneView::Traffic);
            }
        }
        // The CPU's identity, mode, diagnostic buffer and blocks, full width.
        Scenario::S7Plc => {
            model.workspace.ui.inspector_open = false;
            alone(model, cx);
            model.set_view(PaneView::Device, cx);
        }
        // One client with its setpoint in the inspector, a value typed in.
        Scenario::S7Write => {
            alone(model, cx);
            model.select_point(client, S7_SETPOINT, cx);
            model
                .write_input
                .update(cx, |input, cx| input.set_text("1480", cx));
        }
        Scenario::S7Editor => {
            model.workspace.ui.inspector_open = false;
            model.open_editor(EditorKind::S7Client, Some(client), window, cx);
        }
        _ => {}
    }
}

/// What the NATS demo trends: subjects of the inverter, which carry their
/// units in a header.
const NATS_TREND: [&str; 4] = [
    "plant.inverter.dc_power",
    "plant.inverter.ac_power",
    "plant.inverter.ac_frequency",
    "plant.inverter.temperature",
];
/// The point the NATS inspector scenario shows.
const NATS_SELECTED: &str = "plant.inverter.ac_power";
/// Subjects are points once their first messages arrive; the client's
/// JetStream streams come right after it connects.
const NATS_SETTLE: Duration = Duration::from_secs(2);

/// The NATS demo: an inverter on a simulated NATS server with JetStream and
/// a client subscribed to it.
fn nats(model: &mut AppModel, scenario: Scenario, cx: &mut Context<AppModel>) {
    let Some((client, simulator)) = model.start_nats_demo(cx) else {
        return;
    };
    if let Some(live) = model.live.get_mut(&client) {
        live.trend_window_ms = TREND_WINDOW_MS;
    }
    match scenario {
        Scenario::NatsTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
        }
        Scenario::NatsTraffic => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Traffic, cx);
        }
        // One client with the inspector: a request and its reply, or the
        // JetStream browser.
        Scenario::NatsInspector | Scenario::NatsJetstream => {
            if let Some(pane) = pane_of(model, simulator) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, client) {
                model.focus_pane(pane, cx);
            }
            model.workspace.ui.inspector_open = true;
        }
        _ => model.workspace.ui.inspector_open = false,
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(NATS_SETTLE).await;
        let _ = this.update(cx, |model, cx| {
            let Some(live) = model.live.get(&client) else {
                return;
            };
            let find = |name: &str| {
                live.points
                    .iter()
                    .position(|point| point.name == name)
                    .and_then(|index| u32::try_from(index).ok())
            };
            let trend = NATS_TREND.iter().filter_map(|name| find(name)).collect();
            let selected = find(NATS_SELECTED);
            if let Some(live) = model.live.get_mut(&client) {
                live.trend = trend;
            }
            match scenario {
                Scenario::NatsInspector => {
                    if let Some(point) = selected {
                        model.select_point(client, point, cx);
                    }
                    model
                        .nats
                        .subject
                        .update(cx, |input, cx| input.set_text("plant.inverter.get", cx));
                    model.send_nats_request(cx);
                }
                Scenario::NatsJetstream => {
                    let stream = vt_protocol::NatsServerSpec::default().stream_name();
                    model.nats_toggle_stream(client, stream.clone(), cx);
                    model.nats_fetch(
                        client,
                        stream,
                        vt_protocol::NatsServerSpec::CONSUMER.to_owned(),
                        cx,
                    );
                }
                _ => {}
            }
            cx.notify();
        });
    })
    .detach();
}

/// What the BLE demo trends and selects, by point name.
const BLE_MONITOR_TREND: [&str; 2] = ["Heart Rate Measurement", "Battery Level"];
const BLE_SENSOR_TREND: [&str; 2] = ["Temperature", "Humidity"];
const BLE_SELECTED: &str = "Setpoint";
/// Points exist once the simulated peripherals are connected and browsed.
const BLE_SETTLE: Duration = Duration::from_secs(2);

/// The BLE demo: a simulated heart rate monitor and environment sensor, or
/// the editor scanning the simulated adapter.
fn ble(model: &mut AppModel, scenario: Scenario, window: &mut Window, cx: &mut Context<AppModel>) {
    if scenario == Scenario::BleScan {
        model.workspace.ui.inspector_open = false;
        model.ble.adapter = vt_protocol::BleAdapter::Simulated;
        model.open_editor(EditorKind::BleClient, None, window, cx);
        return;
    }
    let Some((monitor, sensor)) = model.start_ble_demo(cx) else {
        return;
    };
    for connection in [monitor, sensor] {
        if let Some(live) = model.live.get_mut(&connection) {
            live.trend_window_ms = TREND_WINDOW_MS;
        }
    }
    match scenario {
        Scenario::BleTrend => {
            model.workspace.ui.inspector_open = false;
            model.set_view(PaneView::Trend, cx);
            if let Some(pane) = pane_of(model, sensor) {
                model.workspace.set_view(pane, PaneView::Trend);
            }
        }
        // The sensor alone, its setpoint in the inspector.
        Scenario::BleInspector => {
            if let Some(pane) = pane_of(model, monitor) {
                model.workspace.close_pane(pane);
            }
            if let Some(pane) = pane_of(model, sensor) {
                model.focus_pane(pane, cx);
            }
        }
        _ => model.workspace.ui.inspector_open = false,
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(BLE_SETTLE).await;
        let _ = this.update(cx, |model, cx| {
            let find = |model: &AppModel, connection: ConnectionId, name: &str| {
                model.live.get(&connection).and_then(|live| {
                    live.points
                        .iter()
                        .position(|point| point.name == name)
                        .and_then(|index| u32::try_from(index).ok())
                })
            };
            let monitor_trend = BLE_MONITOR_TREND
                .into_iter()
                .filter_map(|name| find(model, monitor, name))
                .collect();
            let sensor_trend = BLE_SENSOR_TREND
                .into_iter()
                .filter_map(|name| find(model, sensor, name))
                .collect();
            let heart_rate = find(model, monitor, BLE_MONITOR_TREND[0]);
            let setpoint = find(model, sensor, BLE_SELECTED);
            if let Some(live) = model.live.get_mut(&monitor) {
                live.trend = monitor_trend;
                live.selected = heart_rate;
            }
            if let Some(live) = model.live.get_mut(&sensor) {
                live.trend = sensor_trend;
            }
            if scenario == Scenario::BleInspector
                && let Some(point) = setpoint
            {
                model.select_point(sensor, point, cx);
                model
                    .write_input
                    .update(cx, |input, cx| input.set_text("23.5", cx));
                model
                    .ble
                    .data
                    .update(cx, |input, cx| input.set_text("00 00 BC 41", cx));
            }
            cx.notify();
        });
    })
    .detach();
}
