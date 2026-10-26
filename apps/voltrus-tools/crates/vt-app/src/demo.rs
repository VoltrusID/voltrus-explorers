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
        // The browser open on the reactor, a variable picked, beside the server.
        Scenario::OpcuaPoints => {
            model.workspace.ui.inspector_open = false;
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
