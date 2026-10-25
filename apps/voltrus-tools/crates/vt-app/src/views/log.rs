//! The session's event log.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, UniformListScrollHandle, div, px, uniform_list};
use vt_app_core::ConnectionId;
use vt_protocol::LogLevel;
use vt_ui::Icon;
use vt_ui::components::{Button, empty_state, pill};
use vt_ui::theme::*;

use super::{clock, follow_end};
use crate::model::AppModel;
use crate::model::live::Live;

const ROW: f32 = 26.0;

pub fn render(
    connection: ConnectionId,
    live: &Live,
    pane_width: f32,
    scroll: UniformListScrollHandle,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    // Text only truncates inside a known width: what a row leaves after its
    // padding (32), time (96), icon (12) and gaps (20).
    let message_width = (pane_width - 160.0).max(40.0);
    let count = live.log.len();
    let paused = live.log_paused;
    if !paused {
        follow_end(&scroll, count, ROW);
    }

    let toolbar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(36.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_SMALL)
        .text_color(theme.fg_muted)
        .child(if count == 1 {
            "1 entry".to_owned()
        } else {
            format!("{count} entries")
        })
        .when(paused, |this| this.child(pill("PAUSED", theme.warning)))
        .child(div().flex_1())
        .child(
            Button::new(
                ElementId::NamedInteger("log-pause".into(), connection.0),
                if paused { "Resume" } else { "Pause" },
            )
            .icon(if paused { Icon::Play } else { Icon::Pause })
            .compact()
            .tooltip(if paused {
                "Follow new entries again"
            } else {
                "Stop following new entries; they are still recorded"
            })
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_log_pause(connection, cx))),
        )
        .child(
            Button::new(
                ElementId::NamedInteger("log-clear".into(), connection.0),
                "Clear",
            )
            .icon(Icon::Trash)
            .compact()
            .ghost()
            .on_click(cx.listener(move |model, _, _, cx| model.clear_log(connection, cx))),
        );

    if count == 0 {
        return div()
            .flex()
            .flex_col()
            .size_full()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(empty_state(
                Icon::ScrollText,
                "Nothing logged",
                "Connections, failures and warnings are recorded here.",
                Vec::new(),
                cx,
            )))
            .into_any_element();
    }
    let list = uniform_list(
        ElementId::NamedInteger("log".into(), connection.0),
        count,
        cx.processor(
            move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                let theme = vt_ui::theme::theme(cx).clone();
                let Some(live) = model.live.get(&connection) else {
                    return Vec::new();
                };
                range
                    .filter_map(|index| {
                        let entry = live.log.get(index)?;
                        let (glyph, color) = match entry.level {
                            LogLevel::Info => (Icon::Info, theme.info),
                            LogLevel::Warn => (Icon::CircleAlert, theme.warning),
                            LogLevel::Error => (Icon::CircleX, theme.danger),
                        };
                        Some(
                            div()
                                .flex()
                                .w_full()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(ROW))
                                .px(SPACE_7)
                                .text_size(FONT_SMALL)
                                .child(
                                    div()
                                        .w(px(96.0))
                                        .flex_none()
                                        .font_family(MONO)
                                        .text_color(theme.fg_dim)
                                        .child(clock(entry.at_ms)),
                                )
                                .child(vt_ui::icon(glyph, ICON_SM, color))
                                .child(
                                    div()
                                        .w(px(message_width))
                                        .flex_none()
                                        .truncate()
                                        .child(entry.message.clone()),
                                ),
                        )
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .flex_1();

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(toolbar)
        .child(list)
        .into_any_element()
}
