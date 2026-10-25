//! `--demo <scenario>`: a fixed, in-memory setup per screen, for screenshots
//! and trying the app without hardware. Nothing is saved.

use gpui::{Context, Window};
use vt_app_core::{Appearance, PaneView};

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
}

impl Scenario {
    pub const NAMES: [&'static str; 8] = [
        "welcome",
        "points",
        "trend",
        "traffic",
        "simulator",
        "editor",
        "palette",
        "light",
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
            _ => return None,
        })
    }
}

/// Points of the SMA inverter template worth looking at: DC power, AC power,
/// grid frequency and heatsink temperature.
const INTERESTING: [u32; 4] = [4, 11, 12, 15];

pub fn apply(
    model: &mut AppModel,
    scenario: Scenario,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) {
    match scenario {
        Scenario::Welcome => {}
        Scenario::Editor => model.open_editor(EditorKind::Master, None, window, cx),
        Scenario::Palette => {
            model.start_demo_plant(cx);
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
            }
            match scenario {
                Scenario::Trend => model.set_view(PaneView::Trend, cx),
                Scenario::Traffic => model.set_view(PaneView::Traffic, cx),
                Scenario::Simulator => {
                    if let Some(pane) = model
                        .workspace
                        .panes
                        .iter()
                        .find(|pane| pane.connection == simulator)
                        .map(|pane| pane.id)
                    {
                        model.focus_pane(pane, cx);
                    }
                    if let Some(live) = model.live.get_mut(&simulator) {
                        live.selected = Some(INTERESTING[3]);
                    }
                }
                Scenario::Light => {
                    model.workspace.ui.appearance = Appearance::Light;
                    model.apply_theme(window, cx);
                }
                _ => {}
            }
        }
    }
}
