//! The Siemens S7 editors: a client with its CPU, connection and tags, and
//! the CPU simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px};
use vt_engine::templates;
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, checkbox, field, section_label, segmented};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::{CPU_PRESETS, Editor};

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.s7;
    let preset = fields.preset(cx);
    let cpu = segmented(
        "s7-cpu",
        CPU_PRESETS
            .iter()
            .enumerate()
            .map(|(index, (label, _, _))| {
                Segment::new(
                    *label,
                    preset == Some(index),
                    cx.listener(move |model, _, _, cx| {
                        if let Some(editor) = model.editor_mut() {
                            editor.set_s7_cpu(index, cx);
                        }
                        cx.notify();
                    }),
                )
            })
            .collect(),
        cx,
    );

    let tags = fields
        .tags
        .iter()
        .enumerate()
        .map(|(index, tag)| {
            let form = tag.form(cx);
            let types = form.types();
            let chosen = form.data_type;
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    row()
                        .child(div().flex_1().child(field("Name", tag.name.clone(), cx)))
                        .child(
                            div()
                                .w(px(200.0))
                                .child(field("Address", tag.address.clone(), cx)),
                        )
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("remove-s7-tag".into(), index as u64),
                                Icon::Trash,
                            )
                            .size(CONTROL_MD)
                            .tooltip("Remove tag")
                            .on_click(on_edit(cx, move |editor| {
                                if index < editor.s7.tags.len() {
                                    editor.s7.tags.remove(index);
                                }
                            })),
                        ),
                )
                .child(segmented(
                    ElementId::NamedInteger("s7-type".into(), index as u64),
                    types
                        .iter()
                        .map(|option| {
                            let option = *option;
                            Segment::new(
                                option.name(),
                                chosen == option,
                                on_edit(cx, move |editor| {
                                    if let Some(tag) = editor.s7.tags.get_mut(index) {
                                        tag.data_type = option;
                                    }
                                }),
                            )
                        })
                        .collect(),
                    cx,
                ))
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("PLC", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(
                            div()
                                .w(px(96.0))
                                .child(field("Port", fields.port.clone(), cx)),
                        ),
                )
                .child(
                    row()
                        .child(field("CPU", cpu, cx))
                        .child(
                            div()
                                .w(px(72.0))
                                .child(field("Rack", fields.rack.clone(), cx)),
                        )
                        .child(
                            div()
                                .w(px(72.0))
                                .child(field("Slot", fields.slot.clone(), cx)),
                        ),
                )
                .child(div().text_size(FONT_SMALL).text_color(theme.fg_dim).child(
                    "S7-1200 and S7-1500 need PUT/GET access allowed in the CPU, and data \
                             blocks without optimized block access.",
                )),
        )
        .child(
            group("Polling", cx).child(
                row()
                    .child(
                        div()
                            .w(px(120.0))
                            .child(field("Poll (ms)", fields.poll.clone(), cx)),
                    )
                    .child(div().w(px(120.0)).child(field(
                        "Timeout (ms)",
                        fields.timeout.clone(),
                        cx,
                    )))
                    .child(
                        div()
                            .w(px(120.0))
                            .child(field("PDU size", fields.pdu.clone(), cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .pb(SPACE_3)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child("240 to 960; the CPU may agree to less."),
                    ),
            ),
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
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SPACE_1)
                                .child(section_label("Tags", cx))
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child("DB1.DBD0  DB1.DBX4.0  MW20  I0.3  QB2  T5  C10"),
                                ),
                        )
                        .child(
                            Button::new("add-s7-tag", "Add tag")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_s7_tag(window, cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .children(tags),
        )
        .into_any_element()
}

pub fn server(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.s7;
    let selected = fields.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let cards = templates::s7()
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let active = selected == template.id;
            let choose = template.id.clone();
            div()
                .id(("s7-template", index))
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
                .when(!active, |this| this.hover(|style| style.bg(theme.hover)))
                .on_click(on_edit(cx, move |editor| {
                    editor.s7.template = choose.clone();
                }))
                .child(vt_ui::icon(
                    Icon::Cpu,
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
                                .flex()
                                .items_baseline()
                                .gap(SPACE_3)
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(template.name.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child(template.cpu.clone()),
                                ),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(format!(
                                    "{} · {} tags",
                                    template.description,
                                    template.points.len()
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
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .child(field("Listen on", fields.server_bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .child(field("TCP port", fields.server_port.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .child(field("PDU size", fields.server_pdu.clone(), cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .pb(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("0.0.0.0 opens it to the network. S7 clients expect port 102."),
                ),
        )
        .child(group("CPU", cx).children(cards))
        .when(new, |this| {
            this.child(checkbox(
                "s7-add-client",
                add_client,
                "Also add a client that reads it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
