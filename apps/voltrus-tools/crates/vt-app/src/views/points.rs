//! The live table of a session's points.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px, uniform_list};
use vt_app_core::{ConnectionId, PaneId};
use vt_protocol::{PointInfo, Quality, Reading};
use vt_ui::Icon;
use vt_ui::chart::{format_value, sparkline};
use vt_ui::components::{IconButton, pill};
use vt_ui::theme::*;

use super::clock;
use crate::model::AppModel;
use crate::model::live::Live;

const ADDRESS: f32 = 76.0;
const TYPE: f32 = 72.0;
const VALUE: f32 = 148.0;
const RAW: f32 = 132.0;
const SPARK: f32 = 96.0;
const ACTIONS: f32 = 52.0;

pub fn render(
    pane: PaneId,
    connection: ConnectionId,
    live: &Live,
    simulator: bool,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let count = live.points.len();
    let bad = live
        .readings
        .iter()
        .filter(|reading| reading.quality == Quality::Bad)
        .count();

    let summary = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_SMALL)
        .text_color(theme.fg_muted)
        .child(format!("{count} points"))
        .when(bad > 0, |this| {
            this.child(pill(format!("{bad} bad"), theme.danger))
        })
        .child(div().flex_1())
        .when(live.last_frame_ms > 0, |this| {
            this.child(
                div()
                    .font_family(MONO)
                    .text_color(theme.fg_dim)
                    .child(format!("updated {} UTC", clock(live.last_frame_ms))),
            )
        });

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(28.0))
        .px(SPACE_7)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(div().w(px(ADDRESS)).flex_none().child("ADDRESS"))
        .child(div().flex_1().min_w_0().child("NAME"))
        .child(div().w(px(TYPE)).flex_none().child("TYPE"))
        .child(div().w(px(VALUE)).flex_none().text_right().child("VALUE"))
        .child(div().w(px(RAW)).flex_none().child("RAW"))
        .child(div().w(px(SPARK)).flex_none().child("LAST MINUTE"))
        .child(div().w(px(ACTIONS)).flex_none());

    let list = uniform_list(
        ElementId::NamedInteger("points".into(), pane.0),
        count,
        cx.processor(
            move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                let theme = vt_ui::theme::theme(cx).clone();
                let Some(live) = model.live.get(&connection) else {
                    return Vec::new();
                };
                let trend = live.trend_points();
                range
                    .filter_map(|index| {
                        let point = u32::try_from(index).ok()?;
                        let (info, reading) = live.point(point)?;
                        let row_state = RowState {
                            point,
                            connection,
                            simulator,
                            trend: trend.iter().position(|candidate| *candidate == point),
                        };
                        Some(row(live, info, reading, row_state, &theme, cx))
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(live.points_scroll.clone())
    .flex_1();

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(summary)
        .child(header)
        .child(list)
        .into_any_element()
}

/// Where a row sits and how it relates to the trend.
struct RowState {
    point: u32,
    connection: ConnectionId,
    simulator: bool,
    /// Index among the trended points, if this one is trended.
    trend: Option<usize>,
}

fn row(
    live: &Live,
    info: &PointInfo,
    reading: Reading,
    state: RowState,
    theme: &Theme,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let RowState {
        point,
        connection,
        simulator,
        trend,
    } = state;
    let selected = live.selected == Some(point);
    let trended = trend.is_some();
    let locked = live.locked.contains(&point);
    let good = reading.quality == Quality::Good;
    let raw = reading
        .raw_words()
        .iter()
        .map(|word| format!("{word:04X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let trend_color = theme.series(trend.unwrap_or(0));
    let hover = theme.hover;

    div()
        .id(ElementId::NamedInteger("point".into(), u64::from(point)))
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(ROW_HEIGHT)
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |model, _, _, cx| model.select_point(connection, point, cx)))
        .child(
            div()
                .w(px(ADDRESS))
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(info.address.clone()),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_3)
                .child(div().truncate().child(info.name.clone()))
                .when(locked, |this| {
                    this.child(vt_ui::icon(Icon::Lock, ICON_SM, theme.warning))
                }),
        )
        .child(
            div()
                .w(px(TYPE))
                .flex_none()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(info.data_type.clone()),
        )
        .child(
            div()
                .w(px(VALUE))
                .flex_none()
                .flex()
                .justify_end()
                .items_center()
                .gap(SPACE_2)
                .child(value(info, reading, theme)),
        )
        .child(
            div()
                .w(px(RAW))
                .flex_none()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(if good { raw } else { String::new() }),
        )
        .child(div().w(px(SPARK)).h(px(18.0)).flex_none().when_some(
            live.history(point).cloned(),
            |this, history| {
                #[allow(clippy::cast_precision_loss)]
                let now = live.last_frame_ms as f64;
                this.child(sparkline(
                    history,
                    if good { theme.accent } else { theme.danger },
                    now,
                    60_000.0,
                ))
            },
        ))
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .justify_end()
                .when(simulator, |this| {
                    this.child(
                        IconButton::new(
                            ElementId::NamedInteger("lock".into(), u64::from(point)),
                            if locked { Icon::Lock } else { Icon::LockOpen },
                        )
                        .size(px(22.0))
                        .color(if locked { theme.warning } else { theme.fg_dim })
                        .tooltip(if locked {
                            "Release: let it drift"
                        } else {
                            "Hold this value"
                        })
                        .on_click(cx.listener(move |model, _, _, cx| {
                            cx.stop_propagation();
                            model.toggle_lock(connection, point, cx);
                        })),
                    )
                })
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("trend".into(), u64::from(point)),
                        Icon::ChartLine,
                    )
                    .size(px(22.0))
                    .color(if trended { trend_color } else { theme.fg_dim })
                    .tooltip(if trended {
                        "Remove from trend"
                    } else {
                        "Add to trend"
                    })
                    .on_click(cx.listener(move |model, _, _, cx| {
                        cx.stop_propagation();
                        model.toggle_trend(connection, point, cx);
                    })),
                ),
        )
        .into_any_element()
}

/// A reading as the table shows it: `ON`/`OFF` for bits, the number with its
/// unit otherwise, a dash when the read failed.
fn value(info: &PointInfo, reading: Reading, theme: &Theme) -> AnyElement {
    if reading.quality == Quality::Bad || !reading.value.is_finite() {
        return div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .text_color(theme.danger)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
            .child(div().font_family(MONO).child("—"))
            .into_any_element();
    }
    if info.boolean {
        let on = reading.value != 0.0;
        return pill(
            if on { "ON" } else { "OFF" },
            if on { theme.success } else { theme.fg_dim },
        )
        .into_any_element();
    }
    div()
        .flex()
        .items_baseline()
        .gap(SPACE_2)
        .child(
            div()
                .font_family(MONO)
                .font_weight(FontWeight::MEDIUM)
                .child(format_value(reading.value)),
        )
        .when(!info.unit.is_empty(), |this| {
            this.child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(info.unit.clone()),
            )
        })
        .into_any_element()
}
