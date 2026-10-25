//! The BACnet editors: a client with its device picker, and the device
//! simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use vt_engine::templates;
use vt_ui::Icon;
use vt_ui::components::{Button, checkbox, field, section_label};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::bacnet::Discovery;
use crate::model::editor::Editor;

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

/// The device picker: the Who-Is button and the devices that answered.
fn picker(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let searching = matches!(editor.bacnet.discovery, Discovery::Searching(_));
    let status: SharedString = match &editor.bacnet.discovery {
        Discovery::Idle => "Who-Is on the BACnet port and to simulators on this computer".into(),
        Discovery::Searching(_) => "Waiting for I-Am answers…".into(),
        Discovery::Found(devices) if devices.is_empty() => "No device answered".into(),
        Discovery::Found(devices) => format!(
            "{} device{} answered",
            devices.len(),
            if devices.len() == 1 { "" } else { "s" }
        )
        .into(),
        Discovery::Failed(_) => "The Who-Is could not be sent".into(),
    };

    let mut list = div().flex().flex_col().gap(SPACE_3);
    match &editor.bacnet.discovery {
        Discovery::Found(devices) if devices.is_empty() => {
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
                        "Check the port and that this computer is on the device's network, \
                         or type its address below.",
                    ),
            );
        }
        Discovery::Found(devices) => {
            for (index, device) in devices.iter().enumerate() {
                let picked = editor.bacnet.picked(device, cx);
                let choose = device.clone();
                list = list.child(
                    div()
                        .id(ElementId::NamedInteger(
                            "bacnet-device".into(),
                            index as u64,
                        ))
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .px(SPACE_6)
                        .h(px(44.0))
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
                            model.pick_bacnet_device(&choose, cx);
                        }))
                        .child(vt_ui::icon(
                            Icon::Cpu,
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
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Device"),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(MONO)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(device.instance.to_string()),
                                )
                                .when(!device.vendor.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .truncate()
                                            .text_size(FONT_SMALL)
                                            .text_color(theme.fg_muted)
                                            .child(device.vendor.clone()),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_baseline()
                                .gap(SPACE_3)
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
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
                        .when(picked, |this| {
                            this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                        }),
                );
            }
        }
        Discovery::Failed(error) => {
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
        Discovery::Idle | Discovery::Searching(_) => {}
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
                        "bacnet-discover",
                        if searching { "Searching…" } else { "Who-Is" },
                    )
                    .icon(if searching {
                        Icon::RefreshCw
                    } else {
                        Icon::RadioTower
                    })
                    .compact()
                    .disabled(searching)
                    .tooltip("Broadcast a Who-Is and list the devices that answer")
                    .on_click(cx.listener(|model, _, _, cx| model.discover_bacnet(cx))),
                ),
        )
        .child(list)
        .into_any_element()
}

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.bacnet;
    let cov = fields.cov;
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(picker(editor, cx))
        .child(
            group("Device", cx).child(
                row()
                    .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                    .child(
                        div()
                            .w(px(110.0))
                            .child(field("Port", fields.port.clone(), cx)),
                    )
                    .child(div().w(px(170.0)).child(field(
                        "Device instance",
                        fields.instance.clone(),
                        cx,
                    ))),
            ),
        )
        .child(
            group("This computer", cx).child(
                row()
                    .child(
                        div()
                            .flex_1()
                            .child(field("Local address", fields.bind.clone(), cx)),
                    )
                    .child(div().w(px(170.0)).child(field(
                        "Local port",
                        fields.local_port.clone(),
                        cx,
                    ))),
            ),
        )
        .child(
            group("Reading and writing", cx)
                .child(
                    row()
                        .child(div().w(px(130.0)).child(field(
                            "Poll (ms)",
                            fields.poll.clone(),
                            cx,
                        )))
                        .child(div().w(px(130.0)).child(field(
                            "Write priority",
                            fields.priority.clone(),
                            cx,
                        ))),
                )
                .child(checkbox(
                    "bacnet-cov",
                    cov,
                    "Subscribe to changes (COV); objects the device takes no subscription for are polled",
                    on_edit(cx, |editor| editor.bacnet.cov = !editor.bacnet.cov),
                    cx,
                )),
        )
        .into_any_element()
}

pub fn device(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.bacnet;
    let selected = fields.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let cards = templates::bacnet()
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let active = selected == template.id;
            let choose = template.id.clone();
            div()
                .id(("bacnet-plant", index))
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
                    editor.bacnet.template = choose.clone();
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
                                    "{} · {} objects",
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
                        .child(field("UDP port", fields.device_port.clone(), cx)),
                )
                .child(div().w(px(170.0)).child(field(
                    "Device instance",
                    fields.device_instance.clone(),
                    cx,
                )))
                .child(
                    div()
                        .flex_1()
                        .pb(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("Answers on every interface. 47808 is the BACnet port."),
                ),
        )
        .child(group("Plant", cx).children(cards))
        .when(new, |this| {
            this.child(checkbox(
                "bacnet-add-client",
                add_client,
                "Also add a client that subscribes to it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
