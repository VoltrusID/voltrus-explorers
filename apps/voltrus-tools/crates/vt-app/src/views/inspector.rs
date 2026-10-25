//! The right-hand inspector: the selected point in detail, or the focused
//! session's summary.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, div, px};
use vt_protocol::{Quality, Role};
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, divider, pill, section_label};
use vt_ui::theme::*;

use super::{now_ms, resize_handle, state_color};
use crate::model::editor::EditorKind;
use crate::model::{AppModel, Drag};

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let width = model.workspace.ui.inspector_width;
    let body = match model.focused_connection() {
        Some(connection) => match model
            .live
            .get(&connection)
            .and_then(|live| live.selected.map(|point| (live, point)))
        {
            Some((live, point)) => point_detail(model, connection, live, point, cx),
            None => session_summary(model, connection, cx),
        },
        None => div().into_any_element(),
    };

    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .bg(theme.chrome)
        .border_l_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(px(48.0))
                .px(SPACE_7)
                .border_b_1()
                .border_color(theme.border)
                .child(section_label("Inspector", cx)),
        )
        .child(
            div()
                .id("inspector-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(body),
        )
        .child(
            resize_handle(
                "inspector-resize",
                cx.listener(move |model, event: &gpui::MouseDownEvent, _, _| {
                    model.begin_drag(Drag::Inspector {
                        start_x: event.position.x,
                        start_width: width,
                    });
                }),
            )
            .left(px(-3.0)),
        )
}

fn point_detail(
    model: &AppModel,
    connection: vt_app_core::ConnectionId,
    live: &crate::model::live::Live,
    point: u32,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some((info, reading)) = live.point(point) else {
        return div().into_any_element();
    };
    let simulator = model
        .workspace
        .connection(connection)
        .is_some_and(|connection| connection.spec.role() == Role::Simulator);
    let running = live.session.is_some();
    let good = reading.quality == Quality::Good && reading.value.is_finite();
    let locked = live.locked.contains(&point);
    let trended = live.trend_points().contains(&point);
    let summary = live
        .history(point)
        .and_then(|history| history.summary(now_ms() - 60_000.0));

    let big_value: SharedString = if !good {
        "—".into()
    } else if info.boolean {
        SharedString::from(if reading.value != 0.0 { "ON" } else { "OFF" })
    } else {
        format_value(reading.value).into()
    };
    let words = reading.raw_words();

    let mut content = div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .p(SPACE_7)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(info.name.clone()),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(format!("{} · {}", info.address, info.data_type)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .p(SPACE_7)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(SPACE_3)
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_VALUE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if good { theme.fg } else { theme.danger })
                                .child(big_value),
                        )
                        .when(!info.unit.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_size(FONT_EMPHASIS)
                                    .text_color(theme.fg_muted)
                                    .child(info.unit.clone()),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .child(pill(
                            if good { "GOOD" } else { "BAD" },
                            if good { theme.success } else { theme.danger },
                        ))
                        .when(locked, |this| this.child(pill("HELD", theme.warning))),
                ),
        );

    if !words.is_empty() && good {
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Registers", cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_3)
                        .children(words.iter().map(|word| {
                            div()
                                .px(SPACE_4)
                                .py(SPACE_2)
                                .rounded(RADIUS_SM)
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(format!("0x{word:04X}"))
                        })),
                )
                .when(words.len() == 1, |this| {
                    let bits = format!("{:016b}", words[0]);
                    let grouped = bits
                        .as_bytes()
                        .chunks(4)
                        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
                        .collect::<Vec<_>>()
                        .join(" ");
                    this.child(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(grouped),
                    )
                }),
        );
    }

    if let Some(summary) = summary {
        let stat = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .flex_1()
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(label),
                )
                .child(div().font_family(MONO).text_size(FONT_BODY).child(value))
        };
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Last minute", cx))
                .child(
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(stat("MIN", format_value(summary.min)))
                        .child(stat("MAX", format_value(summary.max)))
                        .child(stat("MEAN", format_value(summary.mean)))
                        .child(stat("SAMPLES", summary.count.to_string())),
                ),
        );
    }

    content = content.child(divider(cx));

    if info.writable {
        let result = live
            .write_result
            .as_ref()
            .filter(|(written, _)| *written == point)
            .map(|(_, result)| result.clone());
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label(
                    if simulator {
                        "Set value"
                    } else {
                        "Write to device"
                    },
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .child(div().flex_1().child(model.write_input.clone()))
                        .child(
                            Button::new("write", if simulator { "Set" } else { "Write" })
                                .icon(Icon::Send)
                                .primary()
                                .disabled(!running)
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.write_selected(window, cx);
                                })),
                        ),
                )
                .when(!running, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child("Connect to write."),
                    )
                })
                .when(info.boolean, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child("Write 1 for ON, 0 for OFF."),
                    )
                })
                .when_some(result, |this, result| {
                    let (glyph, color, text): (Icon, gpui::Hsla, SharedString) = match result {
                        Ok(()) => (
                            Icon::CircleCheck,
                            theme.success,
                            if simulator {
                                "Value set and held"
                            } else {
                                "Written"
                            }
                            .into(),
                        ),
                        Err(error) => (Icon::CircleAlert, theme.danger, error.into()),
                    };
                    this.child(
                        div()
                            .flex()
                            .items_start()
                            .gap(SPACE_3)
                            .text_size(FONT_SMALL)
                            .text_color(color)
                            .child(vt_ui::icon(glyph, ICON_MD, color))
                            .child(div().flex_1().child(text)),
                    )
                }),
        );
    }

    content = content.child(
        div()
            .flex()
            .gap(SPACE_3)
            .child(
                Button::new(
                    "inspector-trend",
                    if trended { "On trend" } else { "Add to trend" },
                )
                .icon(Icon::ChartLine)
                .compact()
                .on_click(
                    cx.listener(move |model, _, _, cx| model.toggle_trend(connection, point, cx)),
                ),
            )
            .when(simulator && running, |this| {
                this.child(
                    Button::new(
                        "inspector-hold",
                        if locked { "Release" } else { "Hold value" },
                    )
                    .icon(if locked { Icon::LockOpen } else { Icon::Lock })
                    .compact()
                    .on_click(
                        cx.listener(move |model, _, _, cx| {
                            model.toggle_lock(connection, point, cx)
                        }),
                    ),
                )
            }),
    );
    content.into_any_element()
}

fn session_summary(
    model: &AppModel,
    connection: vt_app_core::ConnectionId,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(saved) = model.workspace.connection(connection) else {
        return div().into_any_element();
    };
    let live = model.live.get(&connection);
    let state = live.map(|live| live.state.clone());
    let stats = live.map(|live| live.stats).unwrap_or_default();
    let row = |label: &'static str, value: String| {
        div()
            .flex()
            .justify_between()
            .text_size(FONT_SMALL)
            .child(div().text_color(theme.fg_muted).child(label))
            .child(div().font_family(MONO).child(value))
    };
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_7)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(saved.name.clone()),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(saved.spec.endpoint()),
                ),
        )
        .when_some(state, |this, state| {
            this.child(
                div()
                    .flex()
                    .child(pill(state.label(), state_color(&state, &theme))),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(row("Polls", stats.polls.to_string()))
                .child(row("Errors", stats.errors.to_string()))
                .child(row("Frames", stats.frames.to_string()))
                .child(row(
                    "Latency",
                    format!("{:.1} ms", f64::from(stats.last_latency_us) / 1000.0),
                )),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child("Select a point to inspect, trend or write it. ↑ ↓ move through points."),
        )
        .child(
            div().flex().child(
                Button::new("inspector-edit", "Edit connection")
                    .icon(Icon::Pencil)
                    .compact()
                    .on_click(cx.listener(move |model, _, window, cx| {
                        model.open_editor(EditorKind::Master, Some(connection), window, cx);
                    })),
            ),
        )
        .into_any_element()
}
