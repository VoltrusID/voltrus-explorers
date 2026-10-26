//! An S7 client's PLC view: what the CPU says about itself, its operating
//! mode, diagnostic buffer (newest first) and blocks.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Hsla, SharedString, div, px};
use vt_app_core::ConnectionId;
use vt_protocol::{S7Block, S7DiagEntry, S7PlcReport};
use vt_ui::Icon;
use vt_ui::components::{Button, empty_state, pill, section_label};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::live::Live;

/// Widest the view grows; wider panes centre it.
const MAX_WIDTH: f32 = 980.0;
/// Width of one identity cell.
const CELL_WIDTH: f32 = 220.0;
/// Width of the diagnostic buffer's time column.
const TIME_WIDTH: f32 = 176.0;
const EVENT_WIDTH: f32 = 64.0;
/// Width of a block row's type column.
const KIND_WIDTH: f32 = 44.0;

pub fn plc(connection: ConnectionId, live: &Live, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let connected = live.state.is_live();
    let reading = live.s7_plc_reading && live.session.is_some();
    let refresh = Button::new(
        "s7-read-plc",
        if reading { "Reading…" } else { "Read again" },
    )
    .icon(Icon::RefreshCw)
    .compact()
    .disabled(reading || !connected)
    .tooltip("Read the CPU's identity, mode, diagnostic buffer and blocks again")
    .on_click(cx.listener(move |model, _, _, cx| model.read_s7_plc(connection, cx)));

    let report = match &live.s7_plc {
        None => {
            return empty_state(
                Icon::Cpu,
                if reading {
                    "Reading the CPU…"
                } else {
                    "No CPU report yet"
                },
                "Its identity, operating mode, diagnostic buffer and blocks appear here.",
                Vec::new(),
                cx,
            )
            .into_any_element();
        }
        Some(Err(error)) => {
            return empty_state(
                Icon::CircleAlert,
                "The CPU could not be asked",
                error.clone(),
                vec![refresh.into_any_element()],
                cx,
            )
            .into_any_element();
        }
        Some(Ok(report)) => report,
    };

    div()
        .id("s7-plc")
        .size_full()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_8)
                .p(SPACE_8)
                .w_full()
                .max_w(px(MAX_WIDTH))
                .mx_auto()
                .child(header(report, refresh, cx))
                .child(identity(report, cx))
                .child(diagnostics(&report.diagnostics, cx))
                .child(blocks(&report.blocks, cx))
                .when(!connected, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child("Shown as last read; the CPU is not connected now."),
                    )
                }),
        )
        .into_any_element()
}

fn mode_color(mode: &str, theme: &Theme) -> Hsla {
    match mode {
        "RUN" => theme.success,
        "STOP" | "DEFECT" => theme.danger,
        "STARTUP" | "HOLD" => theme.warning,
        _ => theme.fg_muted,
    }
}

fn header(report: &S7PlcReport, refresh: Button, cx: &Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let title: SharedString = match &report.cpu {
        Ok(cpu) if !cpu.module_type.is_empty() => cpu.module_type.clone().into(),
        _ => "S7 CPU".into(),
    };
    let order = report
        .cpu
        .as_ref()
        .map(|cpu| cpu.order_code.clone())
        .unwrap_or_default();
    div()
        .flex()
        .items_center()
        .gap(SPACE_6)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .rounded(RADIUS_LG)
                .bg(theme.accent_soft)
                .child(vt_ui::icon(Icon::Cpu, px(20.0), theme.accent)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(SPACE_1)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .map(|this| match &report.mode {
                            Ok(mode) => this.child(pill(mode.clone(), mode_color(mode, &theme))),
                            Err(_) => this.child(pill("MODE ?", theme.fg_muted)),
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(if order.is_empty() {
                            format!("PDU {} bytes", report.pdu_size)
                        } else {
                            format!("{order} · PDU {} bytes", report.pdu_size)
                        }),
                ),
        )
        .child(refresh)
}

/// A part of the report the CPU refused, said plainly.
fn refused(error: &str, cx: &Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(theme.danger)
        .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
        .child(div().flex_1().child(error.to_owned()))
}

fn card(cx: &Context<AppModel>) -> gpui::Div {
    let theme = theme(cx);
    div()
        .rounded(RADIUS_LG)
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
}

fn identity(report: &S7PlcReport, cx: &Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .child(section_label("Identity", cx));
    let cpu = match &report.cpu {
        Ok(cpu) => cpu,
        Err(error) => return section.child(refused(error, cx)),
    };
    let cells = [
        ("Module", cpu.module_type.as_str()),
        ("Order number", cpu.order_code.as_str()),
        ("Firmware", cpu.firmware.as_str()),
        ("Serial number", cpu.serial_number.as_str()),
        ("Station", cpu.station.as_str()),
        ("Module name", cpu.module_name.as_str()),
    ];
    section.child(
        card(cx)
            .flex()
            .flex_wrap()
            .gap_y(SPACE_6)
            .p(SPACE_7)
            .children(cells.into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .w(px(CELL_WIDTH))
                    .pr(SPACE_6)
                    .child(
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(label),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(FONT_BODY)
                            .text_color(if value.is_empty() {
                                theme.fg_dim
                            } else {
                                theme.fg
                            })
                            .child(if value.is_empty() {
                                "—".to_owned()
                            } else {
                                value.to_owned()
                            }),
                    )
            })),
    )
}

fn diagnostics(
    entries: &Result<Vec<S7DiagEntry>, String>,
    cx: &Context<AppModel>,
) -> impl IntoElement {
    let theme = theme(cx).clone();
    let title = match entries {
        Ok(entries) => format!("Diagnostic buffer · {} events, newest first", entries.len()),
        Err(_) => "Diagnostic buffer".to_owned(),
    };
    let section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .child(section_label(title, cx));
    let entries = match entries {
        Ok(entries) if entries.is_empty() => {
            return section.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("The buffer is empty."),
            );
        }
        Ok(entries) => entries,
        Err(error) => return section.child(refused(error, cx)),
    };
    section.child(
        card(cx)
            .flex()
            .flex_col()
            .children(entries.iter().enumerate().map(|(index, entry)| {
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_6)
                    .min_h(ROW_HEIGHT)
                    .px(SPACE_7)
                    .py(SPACE_2)
                    .when(index > 0, |this| {
                        this.border_t_1().border_color(theme.border)
                    })
                    .child(
                        div()
                            .flex_none()
                            .w(px(TIME_WIDTH))
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(entry.timestamp.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(EVENT_WIDTH))
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(format!("0x{:04X}", entry.event_id)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(FONT_BODY)
                            .child(entry.text.clone()),
                    )
            })),
    )
}

/// The blocks by type, in the order the CPU lists them.
fn grouped(blocks: &[S7Block]) -> Vec<(&str, Vec<&S7Block>)> {
    let mut groups: Vec<(&str, Vec<&S7Block>)> = Vec::new();
    for block in blocks {
        match groups.iter_mut().find(|(kind, _)| *kind == block.kind) {
            Some((_, members)) => members.push(block),
            None => groups.push((block.kind.as_str(), vec![block])),
        }
    }
    for (_, members) in &mut groups {
        members.sort_by_key(|block| block.number);
    }
    groups
}

fn blocks(blocks: &Result<Vec<S7Block>, String>, cx: &Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let title = match blocks {
        Ok(blocks) => format!("Blocks · {}", blocks.len()),
        Err(_) => "Blocks".to_owned(),
    };
    let section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .child(section_label(title, cx));
    let blocks = match blocks {
        Ok(blocks) if blocks.is_empty() => {
            return section.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("The CPU lists no blocks."),
            );
        }
        Ok(blocks) => blocks,
        Err(error) => return section.child(refused(error, cx)),
    };
    section.child(
        card(cx)
            .flex()
            .flex_col()
            .children(
                grouped(blocks)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (kind, members))| {
                        div()
                            .flex()
                            .items_start()
                            .gap(SPACE_6)
                            .px(SPACE_7)
                            .py(SPACE_5)
                            .when(index > 0, |this| {
                                this.border_t_1().border_color(theme.border)
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(KIND_WIDTH))
                                    .pt(SPACE_1)
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.fg_muted)
                                    .child(format!("{kind} {}", members.len())),
                            )
                            .child(div().flex().flex_1().flex_wrap().gap(SPACE_3).children(
                                members.into_iter().map(|block| {
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(SPACE_2)
                                        .px(SPACE_4)
                                        .py(SPACE_1)
                                        .rounded(RADIUS_SM)
                                        .bg(theme.bg)
                                        .border_1()
                                        .border_color(theme.border)
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .child(format!("{}{}", block.kind, block.number))
                                        .when(!block.language.is_empty(), |this| {
                                            this.child(
                                                div()
                                                    .text_size(FONT_CAPTION)
                                                    .text_color(theme.fg_dim)
                                                    .child(block.language.clone()),
                                            )
                                        })
                                }),
                            ))
                    }),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(kind: &str, number: u16) -> S7Block {
        S7Block {
            kind: kind.into(),
            number,
            language: String::new(),
        }
    }

    #[test]
    fn blocks_group_by_type_in_listed_order_and_sort_by_number() {
        let blocks = [
            block("OB", 100),
            block("DB", 5),
            block("OB", 1),
            block("DB", 2),
        ];
        let groups = grouped(&blocks);
        let summary: Vec<(&str, Vec<u16>)> = groups
            .iter()
            .map(|(kind, members)| (*kind, members.iter().map(|block| block.number).collect()))
            .collect();
        assert_eq!(summary, [("OB", vec![1, 100]), ("DB", vec![2, 5])]);
    }
}
