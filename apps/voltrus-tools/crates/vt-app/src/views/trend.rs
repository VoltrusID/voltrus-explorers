//! A live multi-series trend of the chosen points.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, Window, div, px};
use vt_app_core::ConnectionId;
use vt_ui::Icon;
use vt_ui::chart::{Series, format_value, trend_chart};
use vt_ui::components::{Segment, empty_state, segmented};
use vt_ui::theme::*;

use super::now_ms;
use crate::model::AppModel;
use crate::model::live::Live;

const WINDOWS: [(&str, f64); 4] = [
    ("30 s", 30_000.0),
    ("1 min", 60_000.0),
    ("5 min", 300_000.0),
    ("15 min", 900_000.0),
];

pub fn render(
    connection: ConnectionId,
    live: &Live,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let points = live.trend_points();
    if points.is_empty() {
        return empty_state(
            Icon::ChartLine,
            "Nothing to trend yet",
            "Values appear here once the session reports its points.",
            Vec::new(),
            cx,
        )
        .into_any_element();
    }

    // Live sessions scroll every frame; stopped ones freeze at their last sample.
    let live_now = live.state.is_live();
    let now = if live_now {
        window.request_animation_frame();
        now_ms()
    } else {
        #[allow(clippy::cast_precision_loss)]
        let last = live.last_frame_ms as f64;
        last
    };

    let series: Vec<Series> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let (info, _) = live.point(*point)?;
            Some(Series {
                label: info.name.clone().into(),
                color: theme.series(index),
                history: live.history(*point)?.clone(),
                stepped: info.boolean,
            })
        })
        .collect();

    let legend = series.iter().map(|series| {
        let latest = series
            .history
            .latest()
            .map(|(_, value)| format_value(value))
            .unwrap_or_else(|| "—".into());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_3)
            .px(SPACE_4)
            .h(px(24.0))
            .rounded(RADIUS_MD)
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(div().size(px(8.0)).rounded_full().bg(series.color))
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(series.label.clone()),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .font_weight(FontWeight::MEDIUM)
                    .child(SharedString::from(latest)),
            )
    });

    let windows = WINDOWS
        .into_iter()
        .map(|(label, span)| {
            Segment::new(
                label,
                (live.trend_window_ms - span).abs() < 1.0,
                cx.listener(move |model, _, _, cx| model.set_trend_window(connection, span, cx)),
            )
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .px(SPACE_7)
                .py(SPACE_5)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .id(ElementId::NamedInteger("legend".into(), connection.0))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .gap(SPACE_3)
                        .overflow_x_scroll()
                        .children(legend),
                )
                .child(segmented(
                    ElementId::NamedInteger("trend-window".into(), connection.0),
                    windows,
                    cx,
                )),
        )
        .child(div().flex_1().min_h_0().p(SPACE_7).child(trend_chart(
            series,
            now,
            live.trend_window_ms,
            cx,
        )))
        .child(
            div()
                .flex_none()
                .px(SPACE_7)
                .pb(SPACE_5)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child("Use the chart button on a point to choose what is drawn."),
        )
        .into_any_element()
}
