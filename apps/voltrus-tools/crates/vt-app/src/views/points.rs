//! The live table of a session's points.

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, UniformListScrollHandle, div, px, uniform_list,
};
use vt_app_core::{ConnectionId, PaneId};
use vt_protocol::{PointInfo, Quality, Reading};
use vt_ui::Icon;
use vt_ui::chart::{format_value, sparkline};
use vt_ui::components::{IconButton, pill};
use vt_ui::theme::*;

use super::clock;
use crate::model::AppModel;
use crate::model::live::Live;

const ADDRESS: f32 = 64.0;
const NAME_MIN: f32 = 120.0;
const TYPE: f32 = 64.0;
const VALUE: f32 = 120.0;
const RAW: f32 = 112.0;
const SPARK: f32 = 96.0;
const ACTIONS: f32 = 52.0;

/// Which optional columns fit the pane. They drop in order: raw words, the
/// sparkline, the type, then the row actions (the inspector has them too).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Columns {
    raw: bool,
    spark: bool,
    data_type: bool,
    actions: bool,
    /// Width left for the name: text only truncates inside a known width.
    name: f32,
}

impl Columns {
    fn fitting(pane_width: f32) -> Self {
        let gap = f32::from(SPACE_5);
        let mut room =
            pane_width - 2.0 * f32::from(SPACE_7) - ADDRESS - NAME_MIN - VALUE - 2.0 * gap;
        let mut take = |width: f32| {
            let fits = room >= width + gap;
            if fits {
                room -= width + gap;
            }
            fits
        };
        // Claimed in the reverse of the drop order.
        let actions = take(ACTIONS);
        let data_type = actions && take(TYPE);
        let spark = data_type && take(SPARK);
        let raw = spark && take(RAW);
        Self {
            raw,
            spark,
            data_type,
            actions,
            name: NAME_MIN + room.max(0.0),
        }
    }
}

/// `holds`: the session can pin values (a template simulator), so rows get a
/// hold button.
pub fn render(
    pane: PaneId,
    connection: ConnectionId,
    live: &Live,
    holds: bool,
    pane_width: f32,
    scroll: UniformListScrollHandle,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let columns = Columns::fitting(pane_width);
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
        .whitespace_nowrap()
        .child(format!("{count} points"))
        .when(bad > 0, |this| {
            this.child(pill(format!("{bad} bad"), theme.danger))
        })
        .child(div().flex_1())
        .when(live.last_frame_ms > 0, |this| {
            this.child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .text_color(theme.fg_dim)
                    .child(if columns.data_type {
                        format!("updated {} UTC", clock(live.last_frame_ms))
                    } else {
                        clock(live.last_frame_ms)
                    }),
            )
        });

    let heading = |width: Option<f32>, text: &'static str| {
        div()
            .whitespace_nowrap()
            .overflow_hidden()
            .map(|this| match width {
                Some(width) => this.w(px(width)).flex_none(),
                None => this.flex_1().min_w_0(),
            })
            .child(text)
    };
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
        .child(heading(Some(ADDRESS), "ADDRESS"))
        .child(heading(None, "NAME"))
        .when(columns.data_type, |this| {
            this.child(heading(Some(TYPE), "TYPE"))
        })
        .child(heading(Some(VALUE), "VALUE").text_right())
        .when(columns.raw, |this| this.child(heading(Some(RAW), "RAW")))
        .when(columns.spark, |this| {
            this.child(heading(Some(SPARK), "LAST MINUTE"))
        })
        .when(columns.actions, |this| {
            this.child(div().w(px(ACTIONS)).flex_none())
        });

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
                            holds,
                            columns,
                            trend: trend.iter().position(|candidate| *candidate == point),
                        };
                        Some(row(live, info, reading, row_state, &theme, cx))
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
        .child(summary)
        .child(header)
        .child(list)
        .into_any_element()
}

/// Where a row sits and how it relates to the trend.
struct RowState {
    point: u32,
    connection: ConnectionId,
    holds: bool,
    columns: Columns,
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
        holds,
        columns,
        trend,
    } = state;
    let selected = live.selected == Some(point);
    let trended = trend.is_some();
    let locked = live.locked.contains(&point);
    let good = reading.quality == Quality::Good;
    // Message protocols show what arrived; the others the words on the wire.
    let raw = match live.payload(point) {
        Some(text) => text.split_whitespace().collect::<Vec<_>>().join(" "),
        None => reading
            .raw_words()
            .iter()
            .map(|word| format!("{word:04X}"))
            .collect::<Vec<_>>()
            .join(" "),
    };
    let trend_color = theme.series(trend.unwrap_or(0));
    let hover = theme.hover;

    div()
        .id(ElementId::NamedInteger("point".into(), u64::from(point)))
        .flex()
        .w_full()
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
                .whitespace_nowrap()
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
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .w(px(if locked {
                            columns.name - f32::from(ICON_SM + SPACE_3)
                        } else {
                            columns.name
                        }))
                        .truncate()
                        .child(info.name.clone()),
                )
                .when(locked, |this| {
                    this.child(vt_ui::icon(Icon::Lock, ICON_SM, theme.warning))
                }),
        )
        .when(columns.data_type, |this| {
            this.child(
                div()
                    .w(px(TYPE))
                    .flex_none()
                    .whitespace_nowrap()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(info.data_type.clone()),
            )
        })
        .child(
            div()
                .w(px(VALUE))
                .flex_none()
                .flex()
                .justify_end()
                .items_center()
                .gap(SPACE_2)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(value(info, reading, live.payload(point), theme)),
        )
        .when(columns.raw, |this| {
            this.child(
                div()
                    .w(px(RAW))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(if good { raw } else { String::new() }),
            )
        })
        .when(columns.spark, |this| {
            this.child(div().w(px(SPARK)).h(px(18.0)).flex_none().when_some(
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
        })
        .when(columns.actions, |this| {
            this.child(
                div()
                    .w(px(ACTIONS))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .when(holds, |this| {
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
                            .on_click(cx.listener(
                                move |model, _, _, cx| {
                                    cx.stop_propagation();
                                    model.toggle_lock(connection, point, cx);
                                },
                            )),
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
        })
        .into_any_element()
}

/// A reading as the table shows it: `ON`/`OFF` for bits, the number with its
/// unit otherwise, the message for text values (MQTT), a dash when the read
/// failed.
fn value(info: &PointInfo, reading: Reading, text: Option<&str>, theme: &Theme) -> AnyElement {
    if let Some(text) = text.filter(|_| reading.quality == Quality::Good && reading.value.is_nan())
    {
        return div()
            .min_w_0()
            .truncate()
            .font_family(MONO)
            .text_color(theme.fg_muted)
            .child(text.lines().next().unwrap_or_default().to_owned())
            .into_any_element();
    }
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
    if let Some(name) = info.state_name(reading.value) {
        return div()
            .flex()
            .min_w_0()
            .items_baseline()
            .gap(SPACE_2)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name.to_owned()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(format_value(reading.value)),
            )
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
