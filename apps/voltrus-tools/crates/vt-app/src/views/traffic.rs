//! The protocol traffic monitor.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, ScrollStrategy, div, px, uniform_list};
use vt_app_core::ConnectionId;
use vt_protocol::Direction;
use vt_ui::Icon;
use vt_ui::components::{Button, empty_state, pill};
use vt_ui::theme::*;

use super::clock;
use crate::model::AppModel;
use crate::model::live::Live;

pub fn render(connection: ConnectionId, live: &Live, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let count = live.traffic.len();
    let paused = live.traffic_paused;
    if count > 0 && !paused {
        live.traffic_scroll
            .scroll_to_item(count - 1, ScrollStrategy::Bottom);
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
        .child(format!("{count} frames"))
        .when(paused, |this| this.child(pill("PAUSED", theme.warning)))
        .child(div().flex_1())
        .child(
            Button::new(
                ElementId::NamedInteger("traffic-pause".into(), connection.0),
                if paused { "Resume" } else { "Pause" },
            )
            .icon(if paused { Icon::Play } else { Icon::Pause })
            .compact()
            .on_click(
                cx.listener(move |model, _, _, cx| model.toggle_traffic_pause(connection, cx)),
            ),
        )
        .child(
            Button::new(
                ElementId::NamedInteger("traffic-clear".into(), connection.0),
                "Clear",
            )
            .icon(Icon::Trash)
            .compact()
            .ghost()
            .on_click(cx.listener(move |model, _, _, cx| model.clear_traffic(connection, cx))),
        );

    let body: AnyElement = if count == 0 {
        empty_state(
            Icon::ArrowUpDown,
            "No traffic yet",
            "Requests and responses appear here as the session talks to the device.",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        uniform_list(
            ElementId::NamedInteger("traffic".into(), connection.0),
            count,
            cx.processor(
                move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let Some(live) = model.live.get(&connection) else {
                        return Vec::new();
                    };
                    range
                        .filter_map(|index| {
                            let frame = live.traffic.get(index)?;
                            let tx = frame.direction == Direction::Tx;
                            Some(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_5)
                                    .h(px(24.0))
                                    .px(SPACE_7)
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .child(
                                        div()
                                            .w(px(96.0))
                                            .flex_none()
                                            .text_color(theme.fg_dim)
                                            .child(clock(frame.at_ms)),
                                    )
                                    .child(
                                        div()
                                            .w(px(34.0))
                                            .flex_none()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if tx { theme.info } else { theme.success })
                                            .child(if tx { "TX" } else { "RX" }),
                                    )
                                    .child(
                                        div()
                                            .w(px(260.0))
                                            .flex_none()
                                            .truncate()
                                            .text_color(theme.fg)
                                            .child(frame.summary.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(theme.fg_muted)
                                            .child(frame.detail.clone()),
                                    ),
                            )
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(live.traffic_scroll.clone())
        .flex_1()
        .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(toolbar)
        .child(body)
        .into_any_element()
}
