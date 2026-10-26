//! A serial session's terminal: what crossed the port as text lines or a
//! hex dump, the send bar with its history and macros, and logging.

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, MouseButton, SharedString, UniformListScrollHandle,
    div, px, uniform_list,
};
use vt_app_core::ConnectionId;
use vt_protocol::{Direction, LineEnding, SessionSpec};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, checkbox, empty_state, kbd, pill, segmented};
use vt_ui::theme::*;

use super::{clock, follow_end};
use crate::actions::*;
use crate::model::AppModel;
use crate::model::live::Live;
use crate::model::serial::{MACRO_KEYS, SerialConsole};

const ROW: f32 = 20.0;
/// Below this pane width the toolbar drops its byte counts.
const COMPACT: f32 = 640.0;
/// Width of a full hex row: 16 pairs and their spaces in the mono face.
const HEX_COLUMN: f32 = 340.0;

/// `1.2 kB`, `840 B`.
fn bytes(count: u64) -> String {
    match count {
        0..1_000 => format!("{count} B"),
        1_000..1_000_000 => format!("{:.1} kB", count as f64 / 1_000.0),
        _ => format!("{:.1} MB", count as f64 / 1_000_000.0),
    }
}

pub fn render(
    connection: ConnectionId,
    spec: &SessionSpec,
    live: &Live,
    console: Option<&SerialConsole>,
    pane_width: f32,
    scroll: UniformListScrollHandle,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let terminal = &live.terminal;
    let running = live.session.is_some();
    let count = terminal.rows().len();
    let (hex, timestamps, paused) = (terminal.hex, terminal.timestamps, terminal.paused);
    if !paused {
        follow_end(&scroll, count, ROW);
    }
    let compact = pane_width < COMPACT;
    // Text only truncates inside a known width: what a row leaves after its
    // padding (32), the time (96), the direction (22) and the gaps.
    let text_width = (pane_width - 32.0 - 30.0 - if timestamps { 106.0 } else { 0.0 }).max(60.0);
    let id = |name: &str| ElementId::NamedInteger(name.to_owned().into(), connection.0);

    let logging = terminal.logging.clone();
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
        .child(segmented(
            id("terminal-mode"),
            vec![
                Segment::new(
                    "Text",
                    !hex,
                    cx.listener(move |model, _, _, cx| {
                        model.set_terminal_hex(connection, false, cx)
                    }),
                )
                .icon(Icon::Terminal)
                .icon_only(compact),
                Segment::new(
                    "Hex",
                    hex,
                    cx.listener(move |model, _, _, cx| {
                        model.set_terminal_hex(connection, true, cx)
                    }),
                )
                .icon(Icon::Binary)
                .icon_only(compact),
            ],
            cx,
        ))
        .child(checkbox(
            id("terminal-time"),
            timestamps,
            "Time",
            cx.listener(move |model, _, _, cx| model.toggle_terminal_timestamps(connection, cx)),
            cx,
        ))
        .when(!compact, |this| {
            this.child(
                div()
                    .flex()
                    .gap(SPACE_4)
                    .font_family(MONO)
                    .child(
                        div()
                            .text_color(theme.success)
                            .child(format!("RX {}", bytes(terminal.rx_bytes))),
                    )
                    .child(
                        div()
                            .text_color(theme.info)
                            .child(format!("TX {}", bytes(terminal.tx_bytes))),
                    ),
            )
        })
        .when(paused, |this| this.child(pill("PAUSED", theme.warning)))
        .child(div().flex_1())
        .when_some(logging.clone(), |this, path| {
            let file = path.rsplit('/').next().unwrap_or(&path).to_owned();
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .min_w_0()
                    .child(vt_ui::components::status_dot(theme.danger))
                    .child(
                        div()
                            .max_w(px(180.0))
                            .truncate()
                            .font_family(MONO)
                            .text_color(theme.fg)
                            .child(file),
                    ),
            )
        })
        .map(|this| {
            let log_label = if logging.is_some() {
                "Stop logging"
            } else {
                "Log to file…"
            };
            let log_icon = if logging.is_some() {
                Icon::Square
            } else {
                Icon::Download
            };
            let pause_label = if paused { "Resume" } else { "Pause" };
            let pause_icon = if paused { Icon::Play } else { Icon::Pause };
            let log = cx.listener(move |model, _, _, cx| model.toggle_serial_log(connection, cx));
            let pause =
                cx.listener(move |model, _, _, cx| model.toggle_terminal_pause(connection, cx));
            let clear = cx.listener(move |model, _, _, cx| model.clear_terminal(connection, cx));
            let can_log = running || logging.is_some();
            // A narrow pane keeps the toolbar on one line with icons.
            if compact {
                this.child(
                    IconButton::new(id("terminal-log"), log_icon)
                        .size(CONTROL_MD)
                        .active(logging.is_some())
                        .disabled(!can_log)
                        .tooltip(log_label)
                        .on_click(log),
                )
                .child(
                    IconButton::new(id("terminal-pause"), pause_icon)
                        .size(CONTROL_MD)
                        .active(paused)
                        .tooltip(pause_label)
                        .on_click(pause),
                )
                .child(
                    IconButton::new(id("terminal-clear"), Icon::Trash)
                        .size(CONTROL_MD)
                        .tooltip("Clear")
                        .on_click(clear),
                )
            } else {
                this.child(
                    Button::new(id("terminal-log"), log_label)
                        .icon(log_icon)
                        .compact()
                        .disabled(!can_log)
                        .tooltip("Write every byte, in and out, to a CSV file")
                        .on_click(log),
                )
                .child(
                    Button::new(id("terminal-pause"), pause_label)
                        .icon(pause_icon)
                        .compact()
                        .on_click(pause),
                )
                .child(
                    Button::new(id("terminal-clear"), "Clear")
                        .icon(Icon::Trash)
                        .compact()
                        .ghost()
                        .on_click(clear),
                )
            }
        });

    let body: AnyElement = if count == 0 {
        empty_state(
            Icon::Terminal,
            if running {
                "Nothing received yet"
            } else {
                "Not connected"
            },
            if running {
                "Bytes from the port appear here as they arrive. Type below to send."
            } else {
                "Connect to open the port. What it sends appears here, and what you type goes out."
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        uniform_list(
            id("terminal"),
            count,
            cx.processor(
                move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let Some(live) = model.live.get(&connection) else {
                        return Vec::new();
                    };
                    let rows = live.terminal.rows();
                    range
                        .filter_map(|index| {
                            let row = rows.get(index)?;
                            let tx = row.direction == Direction::Tx;
                            let text: SharedString = row.text.clone().into();
                            Some(
                                div()
                                    .flex()
                                    .w_full()
                                    .items_center()
                                    .gap(SPACE_4)
                                    .h(px(ROW))
                                    .px(SPACE_7)
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .whitespace_nowrap()
                                    .when(tx, |this| this.bg(theme.info.alpha(0.06)))
                                    .when(timestamps, |this| {
                                        this.child(
                                            div()
                                                .w(px(96.0))
                                                .flex_none()
                                                .text_color(theme.fg_dim)
                                                .child(clock(row.at_ms)),
                                        )
                                    })
                                    .child(
                                        div()
                                            .w(px(22.0))
                                            .flex_none()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if tx { theme.info } else { theme.success })
                                            .child(if tx { "TX" } else { "RX" }),
                                    )
                                    .map(|this| {
                                        if hex {
                                            this.child(
                                                div()
                                                    .w(px(HEX_COLUMN.min(text_width)))
                                                    .flex_none()
                                                    .truncate()
                                                    .text_color(theme.fg)
                                                    .child(text),
                                            )
                                            // The ASCII column only where it fits.
                                            .when(text_width > HEX_COLUMN + 60.0, |this| {
                                                this.child(
                                                    div()
                                                        .flex_none()
                                                        .text_color(theme.fg_muted)
                                                        .child(row.ascii.clone()),
                                                )
                                            })
                                        } else {
                                            this.child(
                                                div()
                                                    .w(px(text_width))
                                                    .flex_none()
                                                    .truncate()
                                                    .text_color(if tx {
                                                        theme.info
                                                    } else {
                                                        theme.fg
                                                    })
                                                    .child(text),
                                            )
                                        }
                                    }),
                            )
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .flex_1()
        .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(toolbar)
        .child(
            div()
                .id(id("terminal-body"))
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                // Clicking the terminal types into the send bar, as in a terminal.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |model, _, window, cx| {
                        model.focus_serial_send(connection, window, cx);
                    }),
                )
                .child(body),
        )
        .child(send_bar(connection, spec, live, console, compact, cx))
        .into_any_element()
}

fn send_bar(
    connection: ConnectionId,
    spec: &SessionSpec,
    live: &Live,
    console: Option<&SerialConsole>,
    compact: bool,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let running = live.session.is_some();
    let id = |name: &str| ElementId::NamedInteger(name.to_owned().into(), connection.0);
    let hex = console.is_some_and(|console| console.hex);
    let (macros, ending) = match spec {
        SessionSpec::SerialMonitor(monitor) => (monitor.macros.clone(), Some(monitor.line_ending)),
        _ => (Vec::new(), None),
    };
    let history = console.map_or(0, SerialConsole::history_len);

    let input: AnyElement = match console {
        Some(console) => div()
            .flex_1()
            .min_w_0()
            .child(console.input.clone())
            .into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    let entry = div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .child(segmented(
            id("send-mode"),
            vec![
                Segment::new(
                    "Text",
                    !hex,
                    cx.listener(move |model, _, _, cx| {
                        model.set_serial_send_hex(connection, false, cx)
                    }),
                ),
                Segment::new(
                    "Hex",
                    hex,
                    cx.listener(move |model, _, _, cx| {
                        model.set_serial_send_hex(connection, true, cx)
                    }),
                ),
            ],
            cx,
        ))
        .child(input)
        .when_some(ending.filter(|_| !hex && !compact), |this, ending| {
            this.child(segmented(
                id("send-ending"),
                LineEnding::ALL
                    .into_iter()
                    .map(|option| {
                        Segment::new(
                            option.name(),
                            option == ending,
                            cx.listener(move |model, _, _, cx| {
                                model.set_serial_line_ending(connection, option, cx);
                            }),
                        )
                    })
                    .collect(),
                cx,
            ))
        })
        .child(
            Button::new(id("send"), "Send")
                .icon(Icon::Send)
                .primary()
                .compact()
                .disabled(!running)
                .tooltip("Send  ↵")
                .on_click(cx.listener(move |model, _, _, cx| model.send_serial(connection, cx))),
        );

    let status: Option<(Icon, gpui::Hsla, String)> = match &live.terminal.sent {
        Some(Err(error)) => Some((Icon::CircleAlert, theme.danger, error.clone())),
        _ => live
            .terminal
            .logging_error
            .clone()
            .map(|error| (Icon::CircleAlert, theme.danger, error)),
    };

    let chips = macros
        .iter()
        .enumerate()
        .map(|(index, item)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    Button::new(
                        ElementId::NamedInteger(format!("macro-{index}").into(), connection.0),
                        item.name.clone(),
                    )
                    .compact()
                    .disabled(!running)
                    .tooltip(if item.hex {
                        item.payload.clone()
                    } else {
                        format!("“{}”", item.payload)
                    })
                    .on_click(cx.listener(move |model, _, _, cx| {
                        model.run_serial_macro(connection, index, cx);
                    })),
                )
                .when(index < MACRO_KEYS && !compact, |this| {
                    this.child(kbd(format!("⌃{}", index + 1), cx))
                })
        })
        .collect::<Vec<_>>();

    div()
        .id(id("send-bar"))
        .key_context(SERIAL_SEND)
        .flex()
        .flex_none()
        .flex_col()
        .gap(SPACE_4)
        .px(SPACE_7)
        .py(SPACE_5)
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .on_action(cx.listener(move |model, _: &SendHistoryPrevious, _, cx| {
            model.serial_history(connection, -1, cx);
        }))
        .on_action(cx.listener(move |model, _: &SendHistoryNext, _, cx| {
            model.serial_history(connection, 1, cx);
        }))
        .on_action(cx.listener(move |model, _: &SendMacro1, _, cx| {
            model.run_serial_macro(connection, 0, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro2, _, cx| {
            model.run_serial_macro(connection, 1, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro3, _, cx| {
            model.run_serial_macro(connection, 2, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro4, _, cx| {
            model.run_serial_macro(connection, 3, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro5, _, cx| {
            model.run_serial_macro(connection, 4, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro6, _, cx| {
            model.run_serial_macro(connection, 5, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro7, _, cx| {
            model.run_serial_macro(connection, 6, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro8, _, cx| {
            model.run_serial_macro(connection, 7, cx)
        }))
        .on_action(cx.listener(move |model, _: &SendMacro9, _, cx| {
            model.run_serial_macro(connection, 8, cx)
        }))
        .child(entry)
        .when(!chips.is_empty() || status.is_some(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(SPACE_4)
                    .text_size(FONT_SMALL)
                    .children(chips)
                    .child(div().flex_1())
                    .when_some(status, |this, (glyph, color, text)| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_2)
                                .min_w_0()
                                .text_color(color)
                                .child(vt_ui::icon(glyph, ICON_SM, color))
                                .child(div().truncate().child(text)),
                        )
                    })
                    .when(history > 0 && !compact, |this| {
                        this.child(
                            div()
                                .text_color(theme.fg_dim)
                                .child(format!("↑↓ {history} sent")),
                        )
                    }),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::bytes;

    #[test]
    fn byte_counts_read_at_a_glance() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_234), "1.2 kB");
        assert_eq!(bytes(3_400_000), "3.4 MB");
    }
}
