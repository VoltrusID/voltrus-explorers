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
