//! The DNP3 editors: a master with its link, polling, controls and point
//! names, and the outstation simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px};
use vt_engine::templates;
use vt_protocol::{Dnp3ControlMode, Dnp3PointKind};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, checkbox, field, section_label, segmented};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::Editor;

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

fn hint(text: &'static str, cx: &Context<AppModel>) -> impl IntoElement {
    div()
        .text_size(FONT_SMALL)
        .text_color(theme(cx).fg_dim)
        .child(text)
}

/// The point-name rows: type (click to change), index, name.
fn point_rows(editor: &Editor, cx: &mut Context<AppModel>) -> Vec<AnyElement> {
    let theme = theme(cx).clone();
    editor
        .dnp3
        .points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let kind: Dnp3PointKind = point.kind;
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .id(ElementId::NamedInteger(
                            "dnp3-point-kind".into(),
                            index as u64,
                        ))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w(px(64.0))
                        .h(CONTROL_MD)
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.hover))
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .font_weight(FontWeight::MEDIUM)
                        .tooltip(move |_, cx| {
                            vt_ui::components::Tooltip::view(
                                format!("{}: click for the next type", kind.label()).into(),
                                cx,
                            )
                        })
                        .on_click(on_edit(cx, move |editor| {
                            editor.cycle_dnp3_point_kind(index)
                        }))
                        .child(kind.short()),
                )
                .child(div().w(px(84.0)).child(point.index.clone()))
                .child(div().flex_1().min_w_0().child(point.name.clone()))
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("remove-dnp3-point".into(), index as u64),
                        Icon::Trash,
                    )
                    .size(CONTROL_MD)
                    .tooltip("Remove the name")
                    .on_click(on_edit(cx, move |editor| {
                        if index < editor.dnp3.points.len() {
                            editor.dnp3.points.remove(index);
                        }
                    })),
                )
                .into_any_element()
        })
        .collect()
}

pub fn master(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.dnp3;
    let mode = fields.control_mode;
    let time_sync = fields.time_sync;
    let rows = point_rows(editor, cx);
    let named = !rows.is_empty();
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Outstation", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(
                            div()
                                .w(px(110.0))
                                .child(field("Port", fields.port.clone(), cx)),
                        ),
                )
                .child(
                    row()
                        .child(div().w(px(150.0)).child(field(
                            "Outstation address",
                            fields.outstation_address.clone(),
                            cx,
                        )))
                        .child(div().w(px(150.0)).child(field(
                            "Master address",
                            fields.master_address.clone(),
                            cx,
                        )))
                        .child(div().flex_1().pb(SPACE_3).child(hint(
                            "Link addresses, 0 to 65519. 20000 is the DNP3 port.",
                            cx,
                        ))),
                ),
        )
        .child(
            group("Polling", cx)
                .child(
                    row()
                        .child(div().w(px(150.0)).child(field(
                            "Integrity every (ms)",
                            fields.integrity.clone(),
                            cx,
                        )))
                        .child(div().w(px(150.0)).child(field(
                            "Events every (ms)",
                            fields.events.clone(),
                            cx,
                        )))
                        .child(div().w(px(130.0)).child(field(
                            "Timeout (ms)",
                            fields.timeout.clone(),
                            cx,
                        ))),
                )
                .child(checkbox(
                    "dnp3-time-sync",
                    time_sync,
                    "Write the time whenever the outstation asks for it (IIN1.4)",
                    on_edit(cx, |editor| editor.dnp3.time_sync = !editor.dnp3.time_sync),
                    cx,
                )),
        )
        .child(
            group("Controls", cx)
                .child(
                    div().flex().child(segmented(
                        "dnp3-control-mode",
                        Dnp3ControlMode::ALL
                            .into_iter()
                            .map(|option| {
                                Segment::new(
                                    option.name(),
                                    mode == option,
                                    on_edit(cx, move |editor| editor.dnp3.control_mode = option),
                                )
                            })
                            .collect(),
                        cx,
                    )),
                )
                .child(hint(
                    "How outputs are commanded: SELECT then OPERATE, as breakers usually require, \
                     or one DIRECT OPERATE. The inspector can change it any time.",
                    cx,
                )),
        )
        .child(
            group("Point names", cx)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(SPACE_6)
                        .child(hint(
                            "Every point the outstation reports is listed. Names here replace \
                             “Analog input 3”, and named points come first.",
                            cx,
                        ))
                        .child(
                            Button::new("add-dnp3-point", "Add name")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_dnp3_point(window, cx);
                                        editor.error = None;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .when(named, |this| {
                    this.child(
                        div()
                            .flex()
                            .gap(SPACE_4)
                            .text_size(FONT_CAPTION)
                            .text_color(theme(cx).fg_dim)
                            .child(div().w(px(64.0)).child("TYPE"))
                            .child(div().w(px(84.0)).child("INDEX"))
                            .child(div().flex_1().child("NAME")),
                    )
                    .children(rows)
                }),
        )
        .into_any_element()
}

pub fn outstation(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.dnp3;
    let selected = fields.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let cards = templates::dnp3()
        .iter()
        .enumerate()
        .map(|(index, plant)| {
            let active = selected == plant.id;
            let choose = plant.id.clone();
            let outputs = plant
                .points
                .iter()
                .filter(|point| point.kind.writable())
                .count();
            div()
                .id(("dnp3-plant", index))
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
                    editor.dnp3.template = choose.clone();
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
                                .child(plant.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(format!(
                                    "{} · {} points, {outputs} outputs",
                                    plant.description,
                                    plant.points.len()
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
                        .child(field("Listen on", fields.bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Port", fields.listen_port.clone(), cx)),
                ),
        )
        .child(
            row()
                .child(div().w(px(150.0)).child(field(
                    "Outstation address",
                    fields.own_address.clone(),
                    cx,
                )))
                .child(div().w(px(150.0)).child(field(
                    "Master address",
                    fields.peer_address.clone(),
                    cx,
                )))
                .child(div().flex_1().pb(SPACE_3).child(hint(
                    "127.0.0.1 serves this computer only; 0.0.0.0 serves the network.",
                    cx,
                ))),
        )
        .child(group("Plant", cx).children(cards))
        .when(new, |this| {
            this.child(checkbox(
                "dnp3-add-master",
                add_client,
                "Also add a master that polls it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
