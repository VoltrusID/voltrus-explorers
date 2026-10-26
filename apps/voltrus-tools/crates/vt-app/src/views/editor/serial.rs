//! The serial sheets of the connection editor: a monitor with its port
//! picker (shared with the Modbus RTU master), line parser and macros, and
//! the virtual device simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use vt_app_core::forms::{BAUD_PRESETS, ParserKind};
use vt_engine::templates;
use vt_protocol::{FlowControl, LineEnding, Parity, SerialFormat, short_port};
use vt_ui::Icon;
use vt_ui::components::{
    Button, IconButton, Segment, checkbox, field, pill, section_label, segmented,
};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::{Editor, EditorKind};

fn hint(text: impl Into<SharedString>, cx: &Context<AppModel>) -> impl IntoElement {
    div()
        .text_size(FONT_SMALL)
        .text_color(theme(cx).fg_dim)
        .child(text.into())
}

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

/// The ports the system offers (and this app's virtual devices), refreshed
/// every few seconds while the editor is open; a click fills the port field.
pub(super) fn port_picker(
    editor: &Editor,
    current: String,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.serial;
    let mut list = div().flex().flex_col().gap(SPACE_2);
    let status: SharedString = match &fields.ports {
        None => "Looking for ports…".into(),
        Some(Ok(ports)) if ports.is_empty() => {
            "No ports yet: plug in a USB serial adapter and it appears here".into()
        }
        Some(Ok(ports)) => format!(
            "{} port{} · updates as devices come and go",
            ports.len(),
            if ports.len() == 1 { "" } else { "s" }
        )
        .into(),
        Some(Err(_)) => "The ports could not be listed".into(),
    };
    match &fields.ports {
        Some(Ok(ports)) => {
            for (index, port) in ports.iter().enumerate() {
                let picked = current.trim() == port.path;
                let choose = port.path.clone();
                let info = port.clone();
                let glyph = match port.kind.as_str() {
                    "USB" => Icon::Cable,
                    "Bluetooth" => Icon::Bluetooth,
                    "Simulator" => Icon::Server,
                    _ => Icon::Cpu,
                };
                list = list.child(
                    div()
                        .id(ElementId::NamedInteger("serial-port".into(), index as u64))
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .px(SPACE_6)
                        .h(px(40.0))
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(if picked { theme.accent } else { theme.border })
                        .bg(if picked {
                            theme.accent_soft
                        } else {
                            theme.surface
                        })
                        .cursor_pointer()
                        .when(!picked, |this| this.hover(|style| style.bg(theme.hover)))
                        .on_click(cx.listener(move |model, _, _, cx| {
                            model.pick_serial_port(&info, cx);
                        }))
                        .child(vt_ui::icon(
                            glyph,
                            ICON_LG,
                            if picked { theme.accent } else { theme.fg_muted },
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_baseline()
                                .gap(SPACE_4)
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(MONO)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(short_port(&choose).to_owned()),
                                )
                                .when(!port.description.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .truncate()
                                            .text_size(FONT_SMALL)
                                            .text_color(theme.fg_muted)
                                            .child(port.description.clone()),
                                    )
                                }),
                        )
                        .when(!port.kind.is_empty(), |this| {
                            this.child(pill(port.kind.clone(), theme.fg_muted))
                        })
                        .when(picked, |this| {
                            this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                        }),
                );
            }
        }
        Some(Err(error)) => {
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                    .child(div().flex_1().child(error.clone())),
            );
        }
        None => {}
    }
    let refreshing = fields.request.is_some();
    div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(SPACE_6)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .min_w_0()
                        .child(section_label("Ports", cx))
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(status),
                        ),
                )
                .child(
                    IconButton::new("serial-refresh", Icon::RefreshCw)
                        .size(CONTROL_MD)
                        .disabled(refreshing)
                        .tooltip("List the ports again")
                        .on_click(cx.listener(|model, _, _, cx| model.list_serial_ports(cx))),
                ),
        )
        .child(list)
        .into_any_element()
}

pub(super) fn monitor(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.serial;
    let (data_bits, parity, stop_bits) = (fields.data_bits, fields.parity, fields.stop_bits);
    let (flow, ending, parser) = (fields.flow, fields.line_ending, fields.parser);
    let current_port = fields.port.read(cx).value().to_owned();
    let baud_text = fields.baud.read(cx).value().trim().to_owned();

    let presets = segmented(
        "serial-baud-presets",
        BAUD_PRESETS
            .into_iter()
            .map(|baud| {
                Segment::new(
                    baud.to_string(),
                    baud_text == baud.to_string(),
                    cx.listener(move |model, _, _, cx| {
                        if let Some(editor) = model.editor_mut() {
                            editor.error = None;
                            let field = editor.serial.baud.clone();
                            field.update(cx, |input, cx| input.set_text(baud.to_string(), cx));
                            cx.notify();
                        }
                    }),
                )
            })
            .collect(),
        cx,
    );

    let parser_settings: AnyElement = match parser {
        ParserKind::None => hint(
            "Bytes show in the terminal only; nothing becomes a point.",
            cx,
        )
        .into_any_element(),
        ParserKind::KeyValue => hint(
            "Lines like T=21.4C RH=45%: each name becomes a point, a unit after the number is kept.",
            cx,
        )
        .into_any_element(),
        ParserKind::Csv => div()
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .child(
                row()
                    .child(
                        div()
                            .w(px(110.0))
                            .child(field("Delimiter", fields.delimiter.clone(), cx)),
                    )
                    .child(div().flex_1().child(field(
                        "Column names, in order",
                        fields.columns.clone(),
                        cx,
                    ))),
            )
            .child(hint(
                "One point per column of numbers. A line of names (a header) names them too; \\t is a tab.",
                cx,
            ))
            .into_any_element(),
        ParserKind::Regex => div()
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .child(field("Pattern", fields.pattern.clone(), cx))
            .child(hint(
                "Each capture group around a number is a point, named after the group: (?P<temp>\\d+\\.\\d+).",
                cx,
            ))
            .into_any_element(),
    };

    let macros = fields
        .macros
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let hex = item.hex;
            row()
                .child(
                    div()
                        .w(px(150.0))
                        .child(field("Name", item.name.clone(), cx)),
                )
                .child(div().flex_1().child(field(
                    if hex { "Bytes (hex)" } else { "Text" },
                    item.payload.clone(),
                    cx,
                )))
                .child(segmented(
                    ElementId::NamedInteger("macro-kind".into(), index as u64),
                    vec![
                        Segment::new(
                            "Text",
                            !hex,
                            on_edit(cx, move |editor| {
                                if let Some(row) = editor.serial.macros.get_mut(index) {
                                    row.hex = false;
                                }
                            }),
                        ),
                        Segment::new(
                            "Hex",
                            hex,
                            on_edit(cx, move |editor| {
                                if let Some(row) = editor.serial.macros.get_mut(index) {
                                    row.hex = true;
                                }
                            }),
                        ),
                    ],
                    cx,
                ))
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("remove-macro".into(), index as u64),
                        Icon::Trash,
                    )
                    .size(CONTROL_MD)
                    .tooltip("Remove macro")
                    .on_click(on_edit(cx, move |editor| {
                        if index < editor.serial.macros.len() {
                            editor.serial.macros.remove(index);
                        }
                    })),
                )
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(port_picker(editor, current_port, cx))
        .child(
            group("Line", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Port", fields.port.clone(), cx)))
                        .child(
                            div()
                                .w(px(120.0))
                                .child(field("Baud", fields.baud.clone(), cx)),
                        ),
                )
                .child(presets)
                .child(
                    row()
                        .child(field(
                            "Data bits",
                            segmented(
                                "serial-data-bits",
                                [5u8, 6, 7, 8]
                                    .into_iter()
                                    .map(|bits| {
                                        Segment::new(
                                            bits.to_string(),
                                            data_bits == bits,
                                            on_edit(cx, move |editor| editor.serial.data_bits = bits),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ),
                            cx,
                        ))
                        .child(field(
                            "Parity",
                            segmented(
                                "serial-parity",
                                Parity::ALL
                                    .into_iter()
                                    .map(|option| {
                                        Segment::new(
                                            match option {
                                                Parity::None => "None",
                                                Parity::Even => "Even",
                                                Parity::Odd => "Odd",
                                            },
                                            parity == option,
                                            on_edit(cx, move |editor| editor.serial.parity = option),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ),
                            cx,
                        ))
                        .child(field(
                            "Stop bits",
                            segmented(
                                "serial-stop-bits",
                                [1u8, 2]
                                    .into_iter()
                                    .map(|bits| {
                                        Segment::new(
                                            bits.to_string(),
                                            stop_bits == bits,
                                            on_edit(cx, move |editor| editor.serial.stop_bits = bits),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ),
                            cx,
                        )),
                )
                .child(
                    row()
                        .child(field(
                            "Flow control",
                            segmented(
                                "serial-flow",
                                FlowControl::ALL
                                    .into_iter()
                                    .map(|option| {
                                        Segment::new(
                                            option.name(),
                                            flow == option,
                                            on_edit(cx, move |editor| editor.serial.flow = option),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ),
                            cx,
                        ))
                        .child(field(
                            "Sends end with",
                            segmented(
                                "serial-ending",
                                LineEnding::ALL
                                    .into_iter()
                                    .map(|option| {
                                        Segment::new(
                                            option.name(),
                                            ending == option,
                                            on_edit(cx, move |editor| {
                                                editor.serial.line_ending = option;
                                            }),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ),
                            cx,
                        )),
                ),
        )
        .child(
            group("Lines into points", cx)
                .child(segmented(
                    "serial-parser",
                    ParserKind::ALL
                        .into_iter()
                        .map(|option| {
                            Segment::new(
                                option.name(),
                                parser == option,
                                on_edit(cx, move |editor| editor.serial.parser = option),
                            )
                        })
                        .collect(),
                    cx,
                ))
                .child(parser_settings),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(section_label("Macros", cx))
                        .child(
                            Button::new("add-macro", "Add macro")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_macro(window, cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .children(macros)
                .child(hint(
                    "Text macros end with the line ending; ⌃1 to ⌃9 send the first nine from the send bar.",
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .border_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Info, ICON_MD, theme.fg_muted))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child("Talking Modbus RTU on this port? A Modbus connection polls its registers."),
                )
                .child(
                    Button::new("serial-to-modbus", "Poll as Modbus RTU")
                        .icon(Icon::Table)
                        .compact()
                        .on_click(cx.listener(|model, _, window, cx| {
                            model.serial_to_modbus(window, cx);
                        })),
                ),
        )
        .into_any_element()
}

pub(super) fn device(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.serial;
    let selected = fields.template.clone();
    let format = fields.format;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let link = vt_protocol::virtual_port_path(fields.alias.read(cx).value().trim());

    let cards = templates::modbus()
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let active = selected == profile.id;
            let choose = profile.id.clone();
            div()
                .id(("serial-template", index))
                .flex()
                .items_center()
                .gap(SPACE_5)
                .p(SPACE_5)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(if active { theme.accent } else { theme.border })
                .bg(if active {
                    theme.accent_soft
                } else {
                    theme.surface
                })
                .cursor_pointer()
                .on_click(on_edit(cx, move |editor| {
                    editor.serial.template = choose.clone()
                }))
                .child(vt_ui::icon(
                    Icon::Factory,
                    ICON_LG,
                    if active { theme.accent } else { theme.fg_muted },
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("{} {}", profile.manufacturer, profile.model)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(format!(
                                    "{} · {} values",
                                    profile.description,
                                    profile.registers.len()
                                )),
                        ),
                )
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(group("Instrument", cx).children(cards))
        .child(
            row()
                .child(field(
                    "Prints",
                    segmented(
                        "serial-format",
                        SerialFormat::ALL
                            .into_iter()
                            .map(|option| {
                                Segment::new(
                                    option.name(),
                                    format == option,
                                    on_edit(cx, move |editor| editor.serial.format = option),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                    cx,
                ))
                .child(
                    div()
                        .w(px(130.0))
                        .child(field("Every (ms)", fields.interval.clone(), cx)),
                )
                .child(div().flex_1().child(field("Port name", fields.alias.clone(), cx))),
        )
        .child(hint(
            format!("Monitors open it at {link}; the pane shows the system's own /dev/ttys path once it runs."),
            cx,
        ))
        .when(new && editor.kind == EditorKind::SerialDevice, |this| {
            this.child(checkbox(
                "serial-add-monitor",
                add_client,
                "Also add a monitor of it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
