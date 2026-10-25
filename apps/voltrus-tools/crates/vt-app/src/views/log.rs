//! The session's event log.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, ScrollStrategy, div, px, uniform_list};
use vt_app_core::ConnectionId;
use vt_protocol::LogLevel;
use vt_ui::Icon;
use vt_ui::components::empty_state;
use vt_ui::theme::*;

use super::clock;
use crate::model::AppModel;
use crate::model::live::Live;

pub fn render(connection: ConnectionId, live: &Live, cx: &mut Context<AppModel>) -> AnyElement {
    let count = live.log.len();
    if count == 0 {
        return empty_state(
            Icon::ScrollText,
            "Nothing logged",
            "Connections, failures and warnings are recorded here.",
            Vec::new(),
            cx,
        )
        .into_any_element();
    }
    live.log_scroll
        .scroll_to_item(count - 1, ScrollStrategy::Bottom);
    uniform_list(
        ElementId::NamedInteger("log".into(), connection.0),
        count,
        cx.processor(
            move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                let theme = theme(cx).clone();
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
                                .h(px(26.0))
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
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(entry.message.clone()),
                                ),
                        )
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(live.log_scroll.clone())
    .size_full()
    .into_any_element()
}
