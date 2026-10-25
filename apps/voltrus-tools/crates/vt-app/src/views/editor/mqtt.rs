//! The MQTT client and broker simulator sheets of the connection editor.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px};
use vt_engine::templates;
use vt_protocol::MqttQos;
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, checkbox, field, section_label, segmented};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::Editor;

fn hint(text: &'static str, cx: &Context<AppModel>) -> impl IntoElement {
    div()
        .text_size(FONT_SMALL)
        .text_color(theme(cx).fg_dim)
        .child(text)
}

pub(super) fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.mqtt;
    let tls = fields.tls;

    let subscriptions = fields
        .subscriptions
        .iter()
        .enumerate()
        .map(|(index, subscription)| {
            let qos = subscription.qos;
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
                        .child(div().flex_1().child(field(
                            "Topic filter",
                            subscription.filter.clone(),
                            cx,
                        )))
                        .child(div().w(px(200.0)).child(field(
                            "JSON path",
                            subscription.json_path.clone(),
                            cx,
                        )))
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("remove-subscription".into(), index as u64),
                                Icon::Trash,
                            )
                            .size(CONTROL_MD)
                            .tooltip("Remove subscription")
                            .on_click(on_edit(cx, move |editor| {
                                if index < editor.mqtt.subscriptions.len() {
                                    editor.mqtt.subscriptions.remove(index);
                                }
                            })),
                        ),
                )
                .child(segmented(
                    ElementId::NamedInteger("subscription-qos".into(), index as u64),
                    MqttQos::ALL
                        .into_iter()
                        .map(|option| {
                            Segment::new(
                                option.name(),
                                qos == option,
                                on_edit(cx, move |editor| {
                                    if let Some(row) = editor.mqtt.subscriptions.get_mut(index) {
                                        row.qos = option;
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
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(section_label("Broker", cx))
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(
                            div()
                                .w(px(110.0))
                                .child(field("Port", fields.port.clone(), cx)),
                        ),
                )
                .child(checkbox(
                    "mqtt-tls",
                    tls,
                    "TLS, verified against this computer's trusted certificates",
                    on_edit(cx, |editor| editor.mqtt.tls = !editor.mqtt.tls),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(section_label("Session", cx))
                .child(
                    row()
                        .child(div().flex_1().child(field(
                            "Client ID",
                            fields.client_id.clone(),
                            cx,
                        )))
                        .child(div().w(px(130.0)).child(field(
                            "Keep-alive (s)",
                            fields.keep_alive.clone(),
                            cx,
                        ))),
                )
                .child(
                    row()
                        .child(
                            div()
                                .flex_1()
                                .child(field("Username", fields.username.clone(), cx)),
                        )
                        .child(div().flex_1().child(field(
                            "Password",
                            fields.password.clone(),
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
                        .child(section_label("Subscriptions", cx))
                        .child(
                            Button::new("add-subscription", "Add subscription")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_subscription(window, cx);
                                        editor.error = None;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .children(subscriptions)
                .child(hint(
                    "Every topic received becomes a point: a number, true or false, each numeric \
                     field of a JSON payload (or only the one at the JSON path, e.g. phases[0].v), \
                     or each metric of a Sparkplug B payload.",
                    cx,
                )),
        )
        .into_any_element()
}

pub(super) fn broker(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.mqtt;
    let selected = fields.template.clone();
    let sparkplug = fields.sparkplug;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;

    let mut options = vec![(
        None,
        "Broker only".to_owned(),
        "Routes what clients publish; publishes nothing itself".to_owned(),
    )];
    options.extend(templates::modbus().iter().map(|profile| {
        (
            Some(profile.id.clone()),
            format!("{} {}", profile.manufacturer, profile.model),
            format!(
                "{} · {} topics",
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
                .id(("mqtt-template", index))
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
                    editor.mqtt.template = choose.clone();
                }))
                .child(vt_ui::icon(
                    if id.is_some() {
                        Icon::Factory
                    } else {
                        Icon::Waypoints
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
                .child(div().flex_1().child(field("Listen on", fields.bind.clone(), cx)))
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Port", fields.broker_port.clone(), cx)),
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
        .when(selected.is_some(), |this| {
            this.child(
                row()
                    .child(div().flex_1().child(field(
                        "Topic prefix",
                        fields.prefix.clone(),
                        cx,
                    )))
                    .child(div().w(px(150.0)).child(field(
                        "Publish every (ms)",
                        fields.publish_ms.clone(),
                        cx,
                    ))),
            )
            .child(hint(
                "Each value is a retained number at <prefix>/<value>, with a JSON nameplate at \
                 <prefix>/status. Publishing a number to a value's topic (or <topic>/set) holds it.",
                cx,
            ))
            .child(checkbox(
                "mqtt-sparkplug",
                sparkplug,
                "Also publish it as a Sparkplug B edge node (Voltrus/Simulator)",
                on_edit(cx, |editor| editor.mqtt.sparkplug = !editor.mqtt.sparkplug),
                cx,
            ))
        })
        .when(new && selected.is_some(), |this| {
            this.child(checkbox(
                "mqtt-add-client",
                add_client,
                "Also add a client that subscribes to it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
