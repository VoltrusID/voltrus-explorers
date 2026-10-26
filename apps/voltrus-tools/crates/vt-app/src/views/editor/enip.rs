//! The EtherNet/IP editors: a client with its device picker and points, and
//! the adapter simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use vt_engine::templates;
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, checkbox, field, section_label};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::{Editor, EnipDiscovery};

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

fn hint(text: impl Into<SharedString>, cx: &Context<AppModel>) -> impl IntoElement {
    div()
        .text_size(FONT_SMALL)
        .text_color(theme(cx).fg_dim)
        .child(text.into())
}

/// The device picker: the ListIdentity button and the devices that answered.
fn picker(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let discovery = &editor.enip.discovery;
    let searching = matches!(discovery, EnipDiscovery::Searching(_));
    let status: SharedString = match discovery {
        EnipDiscovery::Idle => {
            "ListIdentity on the port below and to simulators on this computer".into()
        }
        EnipDiscovery::Searching(_) => "Waiting for identities…".into(),
        EnipDiscovery::Found(devices) if devices.is_empty() => "No device answered".into(),
        EnipDiscovery::Found(devices) => format!(
            "{} device{} answered",
            devices.len(),
            if devices.len() == 1 { "" } else { "s" }
        )
        .into(),
        EnipDiscovery::Failed(_) => "ListIdentity could not be sent".into(),
    };

    let mut list = div().flex().flex_col().gap(SPACE_3);
    match discovery {
        EnipDiscovery::Found(devices) if devices.is_empty() => {
            list = list.child(
                div()
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(theme.border)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(
                        "Check that this computer is on the device's subnet (ListIdentity is a \
                         broadcast), or type its address below.",
                    ),
            );
        }
        EnipDiscovery::Found(devices) => {
            for (index, device) in devices.iter().enumerate() {
                let picked = editor.enip.picked(device, cx);
                let choose = device.clone();
                list = list.child(
                    div()
                        .id(ElementId::NamedInteger("enip-device".into(), index as u64))
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .px(SPACE_6)
                        .h(px(52.0))
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
                            model.pick_enip_device(&choose, cx);
                        }))
                        .child(vt_ui::icon(
                            Icon::Cpu,
                            ICON_LG,
                            if picked { theme.accent } else { theme.fg_muted },
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(
                                    if device.product.is_empty() {
                                        device.device_type.clone()
                                    } else {
                                        device.product.clone()
                                    },
                                ))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_muted)
                                        .child(format!(
                                            "{} · {} · rev {}",
                                            device.vendor, device.device_type, device.revision
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .items_end()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(
                                    div()
                                        .flex()
                                        .gap(SPACE_3)
                                        .text_color(theme.fg_muted)
                                        .child(format!("{}:{}", device.address, device.port))
                                        .when(!device.also.is_empty(), |this| {
                                            this.child(
                                                div()
                                                    .text_color(theme.fg_dim)
                                                    .child(format!("+{}", device.also.len())),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .text_color(theme.fg_dim)
                                        .child(format!("S/N {:08X}", device.serial)),
                                ),
                        )
                        .when(picked, |this| {
                            this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                        }),
                );
            }
        }
        EnipDiscovery::Failed(error) => {
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
        EnipDiscovery::Idle | EnipDiscovery::Searching(_) => {}
    }

    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
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
                        .child(section_label("Find devices", cx))
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(status),
                        ),
                )
                .child(
                    Button::new(
                        "enip-discover",
                        if searching {
                            "Searching…"
                        } else {
                            "List Identity"
                        },
                    )
                    .icon(if searching {
                        Icon::RefreshCw
                    } else {
                        Icon::RadioTower
                    })
                    .compact()
                    .disabled(searching)
                    .tooltip("Broadcast ListIdentity and list the devices that answer")
                    .on_click(cx.listener(|model, _, _, cx| model.discover_enip(cx))),
                ),
        )
        .child(list)
        .into_any_element()
}

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.enip;
    let rows = fields
        .points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            row()
                .child(
                    div()
                        .w(px(170.0))
                        .child(field("Name", point.name.clone(), cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .child(field("Tag or attribute", point.address.clone(), cx)),
                )
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("remove-enip-point".into(), index as u64),
                        Icon::Trash,
                    )
                    .size(CONTROL_MD)
                    .tooltip("Remove point")
                    .on_click(on_edit(cx, move |editor| {
                        if index < editor.enip.points.len() {
                            editor.enip.points.remove(index);
                        }
                    })),
                )
        })
        .collect::<Vec<_>>();
    let browse_all = rows.is_empty();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(picker(editor, cx))
        .child(
            group("Device", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(div().w(px(110.0)).child(field("Port", fields.port.clone(), cx)))
                        .child(div().w(px(130.0)).child(field("Poll (ms)", fields.poll.clone(), cx))),
                )
                .child(
                    row()
                        .child(
                            div()
                                .w(px(220.0))
                                .flex_none()
                                .child(field("Route path", fields.route.clone(), cx)),
                        )
                        .child(div().flex_1().min_w_0().pb(SPACE_3).child(hint(
                            "1,0 reaches a ControlLogix in slot 0 through its Ethernet module; \
                             leave empty for CompactLogix, Micro800 and adapters.",
                            cx,
                        ))),
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
                        .child(section_label("Points", cx))
                        .child(
                            Button::new("add-enip-point", "Add point")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_enip_point(window, cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .when(browse_all, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_start()
                            .gap(SPACE_4)
                            .px(SPACE_6)
                            .py(SPACE_5)
                            .rounded(RADIUS_MD)
                            .bg(theme.surface)
                            .border_1()
                            .border_color(theme.border)
                            .child(vt_ui::icon(Icon::List, ICON_MD, theme.fg_muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(
                                        "No points: every controller and program tag the device lists is read, \
                                         arrays by their first elements and structures by their members.",
                                    ),
                            ),
                    )
                })
                .children(rows)
                .when(!browse_all, |this| {
                    this.child(hint(
                        "A Logix tag by name (Program:Main.Speed, Recipe[3], Status.5), or a CIP attribute \
                         as @class/instance/attribute+offset:TYPE, e.g. @0x04/100/3+4:REAL for an assembly.",
                        cx,
                    ))
                }),
        )
        .into_any_element()
}

pub fn adapter(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.enip;
    let selected = fields.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let cards = templates::enip()
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let active = selected == template.id;
            let choose = template.id.clone();
            div()
                .id(("enip-plant", index))
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
                    editor.enip.template = choose.clone();
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
                                .child(template.name.clone()),
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
                        .w(px(130.0))
                        .flex_none()
                        .child(field("Port", fields.adapter_port.clone(), cx)),
                )
                .child(div().flex_1().min_w_0().pb(SPACE_3).child(hint(
                    "TCP sessions and UDP ListIdentity on every interface. 44818 is the EtherNet/IP port.",
                    cx,
                ))),
        )
        .child(group("Plant", cx).children(cards))
        .child(hint(
            "Serves its tags by name, the Identity object, input assembly 100 (every tag's value) and \
             output assembly 150, Forward_Open and class 1 I/O.",
            cx,
        ))
        .when(new, |this| {
            this.child(checkbox(
                "enip-add-client",
                add_client,
                "Also add a client that reads it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
