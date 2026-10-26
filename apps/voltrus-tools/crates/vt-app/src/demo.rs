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
    EnipPoints,
    EnipTrend,
    EnipTraffic,
    EnipDiscover,
    EnipMessage,
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
        "enip-points",
        "enip-trend",
        "enip-traffic",
        "enip-discover",
        "enip-message",
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
            "enip-points" => Self::EnipPoints,
            "enip-trend" => Self::EnipTrend,
            "enip-traffic" => Self::EnipTraffic,
            "enip-discover" => Self::EnipDiscover,
            "enip-message" => Self::EnipMessage,
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
        Scenario::EnipPoints
        | Scenario::EnipTrend
        | Scenario::EnipTraffic
        | Scenario::EnipDiscover
        | Scenario::EnipMessage => enip(model, scenario, window, cx),
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
