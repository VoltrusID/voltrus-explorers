//! The connection editor sheet.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, FontWeight, MouseButton, Window, div, hsla, px,
};
use vt_app_core::forms::Transport;
use vt_engine::templates;
use vt_protocol::{Area, DataType, Parity, WordOrder};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, field, section_label, segmented};
use vt_ui::theme::*;

use crate::actions::EDITOR;
use crate::model::editor::{Editor, EditorKind};
use crate::model::{AppModel, Overlay};

/// A click handler that changes the open editor.
pub(super) fn on_edit(
    cx: &Context<AppModel>,
    change: impl Fn(&mut Editor) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |model, _: &ClickEvent, _, cx| {
        if let Some(editor) = model.editor_mut() {
            change(editor);
            editor.error = None;
            cx.notify();
        }
    })
}

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let Overlay::Editor(editor) = &model.overlay else {
        return div().into_any_element();
    };
    let editing = editor.editing;
    let title = match (editor.kind, editing.is_some()) {
        (EditorKind::Master, false) => "New Modbus connection",
        (EditorKind::Master, true) => "Edit Modbus connection",
        (EditorKind::Slave, false) => "New Modbus simulator",
        (EditorKind::Slave, true) => "Edit Modbus simulator",
        (EditorKind::BacnetClient, false) => "New BACnet connection",
        (EditorKind::BacnetClient, true) => "Edit BACnet connection",
        (EditorKind::BacnetDevice, false) => "New BACnet device simulator",
        (EditorKind::BacnetDevice, true) => "Edit BACnet device simulator",
    };
    let subtitle = match editor.kind {
        EditorKind::Master => "Poll a device over Modbus TCP or RTU.",
        EditorKind::Slave => "Serve a device's registers on this computer for masters to poll.",
        EditorKind::BacnetClient => {
            "Find a device with Who-Is, read its objects, subscribe to changes and command them."
        }
        EditorKind::BacnetDevice => {
            "Play a BACnet/IP device on this computer for clients to read and command."
        }
    };
    let body = match editor.kind {
        EditorKind::Master => master(editor, cx),
        EditorKind::Slave => slave(editor, cx),
        EditorKind::BacnetClient => super::editor_bacnet::client(editor, cx),
        EditorKind::BacnetDevice => super::editor_bacnet::device(editor, cx),
    };
    let save_label = match (editor.kind, editing.is_some()) {
        (_, true) => "Save",
        (EditorKind::Master | EditorKind::BacnetClient, false) => "Save and connect",
        (EditorKind::Slave | EditorKind::BacnetDevice, false) => "Start simulator",
    };
    let confirm_delete = editor.confirm_delete;

    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0.0, 0.0, 0.0, if theme.dark { 0.5 } else { 0.25 }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|model, _, window, cx| model.close_overlay(window, cx)),
        )
        .child(
            div()
                .id("editor")
                // Its own context keeps pane shortcuts from acting behind it.
                .key_context(EDITOR)
                .flex()
                .flex_col()
                .w(px(680.0))
                .max_h(px(720.0))
                .rounded(RADIUS_XL)
                .bg(theme.elevated)
                .border_1()
                .border_color(theme.border_strong)
                .shadow(theme.shadow(Elevation::Modal))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_start()
                        .gap(SPACE_5)
                        .p(SPACE_8)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .gap(SPACE_2)
                                .child(
                                    div()
                                        .text_size(FONT_TITLE)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_muted)
                                        .child(subtitle),
                                ),
                        )
                        .child(
                            IconButton::new("editor-close", Icon::X)
                                .tooltip("Close  esc")
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.close_overlay(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .id("editor-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p(SPACE_8)
                        .child(body),
                )
                .when_some(editor.error.as_ref(), |this, error| {
                    this.child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(SPACE_4)
                            .mx(SPACE_8)
                            .mb(SPACE_4)
                            .px(SPACE_6)
                            .py(SPACE_4)
                            .rounded(RADIUS_MD)
                            .bg(theme.danger.alpha(0.1))
                            .border_1()
                            .border_color(theme.danger.alpha(0.3))
                            .text_size(FONT_SMALL)
                            .text_color(theme.danger)
                            .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                            .child(error.message.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_4)
                        .px(SPACE_8)
                        .py(SPACE_6)
                        .border_t_1()
                        .border_color(theme.border)
                        .when_some(editing, |this, id| {
                            this.child(
                                Button::new(
                                    "editor-delete",
                                    if confirm_delete {
                                        "Click again to delete"
                                    } else {
                                        "Delete"
                                    },
                                )
                                .icon(Icon::Trash)
                                .danger()
                                .on_click(cx.listener(
                                    move |model, _, window, cx| {
                                        let armed = model.editor_mut().is_some_and(|editor| {
                                            let armed = editor.confirm_delete;
                                            editor.confirm_delete = true;
                                            armed
                                        });
                                        if armed {
                                            model.delete_connection(id, window, cx);
                                        } else {
                                            cx.notify();
                                        }
                                    },
                                )),
                            )
                        })
                        .child(div().flex_1())
                        .child(Button::new("editor-cancel", "Cancel").ghost().on_click(
                            cx.listener(|model, _, window, cx| {
                                model.close_overlay(window, cx);
                            }),
                        ))
                        .child(
                            Button::new("editor-save", save_label)
                                .primary()
                                .icon(Icon::Check)
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.save_editor(window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}

pub(super) fn row() -> gpui::Div {
    div().flex().gap(SPACE_6).items_end()
}

fn master(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let tcp = editor.transport == Transport::Tcp;

    let transport = segmented(
        "transport",
        vec![
            Segment::new(
                "TCP",
                tcp,
                on_edit(cx, |editor| editor.transport = Transport::Tcp),
            )
            .icon(Icon::Network),
            Segment::new(
                "RTU (serial)",
                !tcp,
                on_edit(cx, |editor| editor.transport = Transport::Rtu),
            )
            .icon(Icon::Cable),
        ],
        cx,
    );

    let link: AnyElement = if tcp {
        row()
            .child(div().flex_1().child(field("Host", editor.host.clone(), cx)))
            .child(
                div()
                    .w(px(110.0))
                    .child(field("Port", editor.port.clone(), cx)),
            )
            .into_any_element()
    } else {
        let parity = editor.parity;
        let data_bits = editor.data_bits;
        let stop_bits = editor.stop_bits;
        div()
            .flex()
            .flex_col()
            .gap(SPACE_6)
            .child(
                row()
                    .child(div().flex_1().child(field(
                        "Serial port",
                        editor.serial_port.clone(),
                        cx,
                    )))
                    .child(
                        div()
                            .w(px(120.0))
                            .child(field("Baud", editor.baud.clone(), cx)),
                    ),
            )
            .child(
                row()
                    .child(field(
                        "Data bits",
                        segmented(
                            "data-bits",
                            [7u8, 8]
                                .into_iter()
                                .map(|bits| {
                                    Segment::new(
                                        bits.to_string(),
                                        data_bits == bits,
                                        on_edit(cx, move |editor| editor.data_bits = bits),
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
                            "parity",
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
                                        on_edit(cx, move |editor| editor.parity = option),
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
                            "stop-bits",
                            [1u8, 2]
                                .into_iter()
                                .map(|bits| {
                                    Segment::new(
                                        bits.to_string(),
                                        stop_bits == bits,
                                        on_edit(cx, move |editor| editor.stop_bits = bits),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                        cx,
                    )),
            )
            .into_any_element()
    };

    let word_order = editor.word_order;
    let blocks = editor
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let area = block.area;
            let data_type = block.data_type;
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    row()
                        .child(div().flex_1().child(field("Name", block.name.clone(), cx)))
                        .child(
                            div()
                                .w(px(96.0))
                                .child(field("Start", block.start.clone(), cx)),
                        )
                        .child(
                            div()
                                .w(px(84.0))
                                .child(field("Count", block.count.clone(), cx)),
                        )
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("remove-block".into(), index as u64),
                                Icon::Trash,
                            )
                            .size(CONTROL_MD)
                            .tooltip("Remove block")
                            .on_click(on_edit(cx, move |editor| {
                                if index < editor.blocks.len() {
                                    editor.blocks.remove(index);
                                }
                            })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_6)
                        .child(segmented(
                            ElementId::NamedInteger("area".into(), index as u64),
                            Area::ALL
                                .into_iter()
                                .map(|option| {
                                    Segment::new(
                                        option.short(),
                                        area == option,
                                        on_edit(cx, move |editor| {
                                            if let Some(block) = editor.blocks.get_mut(index) {
                                                block.area = option;
                                            }
                                        }),
                                    )
                                })
                                .collect(),
                            cx,
                        ))
                        .when(!area.is_bit(), |this| {
                            this.child(segmented(
                                ElementId::NamedInteger("type".into(), index as u64),
                                DataType::ALL
                                    .into_iter()
                                    .map(|option| {
                                        Segment::new(
                                            option.name(),
                                            data_type == option,
                                            on_edit(cx, move |editor| {
                                                if let Some(block) = editor.blocks.get_mut(index) {
                                                    block.data_type = option;
                                                }
                                            }),
                                        )
                                    })
                                    .collect(),
                                cx,
                            ))
                        }),
                )
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(section_label("Link", cx))
                .child(transport)
                .child(link),
        )
        .child(
            row()
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Unit ID", editor.unit.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .child(field("Poll (ms)", editor.poll.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .child(field("Timeout (ms)", editor.timeout.clone(), cx)),
                )
                .child(field(
                    "Word order",
                    segmented(
                        "word-order",
                        WordOrder::ALL
                            .into_iter()
                            .map(|option| {
                                Segment::new(
                                    option.name(),
                                    word_order == option,
                                    on_edit(cx, move |editor| editor.word_order = option),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                    cx,
                )),
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
                        .child(section_label("Blocks to read", cx))
                        .child(
                            Button::new("add-block", "Add block")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_block(window, cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .children(blocks),
        )
        .into_any_element()
}

fn slave(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let selected = editor.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;

    let mut options = vec![(
        None,
        "Plain register image".to_owned(),
        "Holding registers you set by hand".to_owned(),
    )];
    options.extend(templates::modbus().iter().map(|profile| {
        (
            Some(profile.id.clone()),
            format!("{} {}", profile.manufacturer, profile.model),
            format!(
                "{} · {} values",
                profile.description,
                profile.registers.len()
            ),
        )
    }));
    let cards = options
        .into_iter()
        .enumerate()
        .map(|(index, (id, title, detail))| {
            let active = selected == id;
            let choose = id.clone();
            div()
                .id(("template", index))
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
                .on_click(on_edit(cx, move |editor| editor.template = choose.clone()))
                .child(vt_ui::icon(
                    if id.is_some() {
                        Icon::Factory
                    } else {
                        Icon::Database
                    },
                    ICON_LG,
                    if active { theme.accent } else { theme.fg_muted },
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(detail),
                        ),
                )
        });

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .child(field("Listen on", editor.bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Port", editor.port.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Unit ID", editor.unit.clone(), cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Device", cx))
                .children(cards),
        )
        .when(selected.is_none(), |this| {
            this.child(
                div()
                    .w(px(160.0))
                    .child(field("Registers", editor.registers.clone(), cx)),
            )
        })
        .when(new && selected.is_some(), |this| {
            this.child(
                div()
                    .id("add-client")
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .cursor_pointer()
                    .on_click(on_edit(cx, |editor| editor.add_client = !editor.add_client))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(16.0))
                            .rounded(RADIUS_SM)
                            .border_1()
                            .border_color(if add_client {
                                theme.accent
                            } else {
                                theme.border_strong
                            })
                            .bg(if add_client { theme.accent } else { theme.bg })
                            .when(add_client, |this| {
                                this.child(vt_ui::icon(Icon::Check, ICON_SM, theme.accent_fg))
                            }),
                    )
                    .child(
                        div()
                            .text_size(FONT_BODY)
                            .child("Also add a client that polls it, side by side"),
                    ),
            )
        })
        .into_any_element()
}
