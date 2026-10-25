//! A live multi-series trend of the chosen points.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, Window, div, px};
use vt_app_core::ConnectionId;
use vt_ui::Icon;
use vt_ui::chart::{Series, format_value, needs_lanes, trend_chart};
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

/// Below this pane width the footer hint is dropped.
const ROOMY: f32 = 560.0;

pub fn render(
    connection: ConnectionId,
    live: &Live,
    pane_width: f32,
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
        live.last_frame_ms as f64
    };

    let series: Vec<Series> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let (info, _) = live.point(*point)?;
            let label = if info.unit.is_empty() {
                info.name.clone()
            } else {
                format!("{} ({})", info.name, info.unit)
            };
            Some(Series {
                label: label.into(),
                color: theme.series(index),
                history: live.history(*point)?.clone(),
                stepped: info.boolean,
            })
        })
        .collect();

    // Lanes carry their own names and values; a shared axis needs a legend.
    let since = now - live.trend_window_ms;
    let ranges: Vec<(f64, f64)> = series
        .iter()
        .filter_map(|series| series.history.range(since))
        .collect();
    let lanes = needs_lanes(&ranges);

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
            .whitespace_nowrap()
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

    let count = series.len();
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
        .whitespace_nowrap()
        .child(div().min_w_0().truncate().child(if count == 1 {
            "1 point".to_owned()
        } else {
            format!("{count} points")
        }))
        .child(div().flex_1())
        .child(segmented(
            ElementId::NamedInteger("trend-window".into(), connection.0),
            windows,
            cx,
        ));

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(toolbar)
        .when(!lanes, |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .flex_wrap()
                    .gap(SPACE_3)
                    .px(SPACE_7)
                    .pt(SPACE_6)
                    .children(legend),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h_0()
                .px(SPACE_7)
                .py(SPACE_6)
                .child(trend_chart(series, now, live.trend_window_ms, cx)),
        )
        .when(pane_width >= ROOMY, |this| {
            this.child(
                div()
                    .flex_none()
                    .px(SPACE_7)
                    .pb(SPACE_5)
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child("Pick what is drawn with the chart button on a point."),
            )
        })
        .into_any_element()
}
