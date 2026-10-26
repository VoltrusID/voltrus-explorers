//! The NATS client and server simulator sheets of the connection editor.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px};
use vt_app_core::forms::NatsAuthKind;
use vt_engine::templates;
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

fn group(label: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(label, cx))
}

pub(super) fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.nats;
    let auth = fields.auth;

    let subscriptions = fields
        .subscriptions
        .iter()
        .enumerate()
        .map(|(index, subscription)| {
            div()
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    row()
                        .child(div().flex_1().child(field(
                            "Subject",
                            subscription.subject.clone(),
                            cx,
                        )))
                        .child(div().w(px(120.0)).child(field(
                            "Queue group",
                            subscription.queue.clone(),
                            cx,
                        )))
                        .child(div().w(px(150.0)).child(field(
                            "JSON path",
                            subscription.json_path.clone(),
                            cx,
                        )))
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("remove-nats-subject".into(), index as u64),
                                Icon::Trash,
                            )
                            .size(CONTROL_MD)
                            .tooltip("Remove subscription")
                            .on_click(on_edit(cx, move |editor| {
                                if index < editor.nats.subscriptions.len() {
                                    editor.nats.subscriptions.remove(index);
                                }
                            })),
                        ),
                )
        })
        .collect::<Vec<_>>();

    let credential: AnyElement = match auth {
        NatsAuthKind::None => hint(
            "Connect without logging in: servers that allow anonymous clients.",
            cx,
        )
        .into_any_element(),
        NatsAuthKind::Token => field("Token", fields.token.clone(), cx).into_any_element(),
        NatsAuthKind::UserPassword => row()
            .child(
                div()
                    .flex_1()
                    .child(field("Username", fields.username.clone(), cx)),
            )
            .child(
                div()
                    .flex_1()
                    .child(field("Password", fields.password.clone(), cx)),
            )
            .into_any_element(),
        NatsAuthKind::Nkey => field("User NKey seed", fields.seed.clone(), cx).into_any_element(),
        NatsAuthKind::Credentials => {
            let chosen = !fields.creds.is_empty();
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .p(SPACE_5)
                .rounded(RADIUS_MD)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(vt_ui::icon(
                    Icon::File,
                    ICON_LG,
                    if chosen { theme.accent } else { theme.fg_muted },
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().font_family(MONO).truncate().child(if chosen {
                            if fields.creds_file.is_empty() {
                                "credentials".to_owned()
                            } else {
                                fields.creds_file.clone()
                            }
                        } else {
                            "No file chosen".to_owned()
                        }))
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child("A user JWT and NKey seed, from nsc or your NATS operator"),
                        ),
                )
                .child(
                    Button::new("nats-creds", if chosen { "Change…" } else { "Choose…" })
                        .icon(Icon::Folder)
                        .compact()
                        .on_click(cx.listener(|model, _, _, cx| model.choose_nats_creds(cx))),
                )
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Server", cx)
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
                    "nats-tls",
                    fields.tls,
                    "TLS, verified against the built-in web trust roots",
                    on_edit(cx, |editor| editor.nats.tls = !editor.nats.tls),
                    cx,
                )),
        )
        .child(
            group("Login", cx)
                .child(segmented(
                    "nats-auth",
                    NatsAuthKind::ALL
                        .into_iter()
                        .map(|option| {
                            Segment::new(
                                option.name(),
                                auth == option,
                                on_edit(cx, move |editor| editor.nats.auth = option),
                            )
                        })
                        .collect(),
                    cx,
                ))
                .child(credential)
                .child(field("Connection name", fields.client_name.clone(), cx)),
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
                            Button::new("add-nats-subject", "Add subscription")
                                .icon(Icon::Plus)
                                .compact()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    if let Some(editor) = model.editor_mut() {
                                        editor.add_nats_subscription(window, cx);
                                        editor.error = None;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .children(subscriptions)
                .child(hint(
                    "Every subject received becomes a point: a number, true or false, or each \
                     numeric field of a JSON message (only the one at the JSON path, e.g. \
                     phases[0].v). * matches one token, > the rest. With no subscription the \
                     connection still publishes, requests and browses JetStream.",
                    cx,
                )),
        )
        .into_any_element()
}

pub(super) fn server(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.nats;
    let selected = fields.template.clone();
    let new = editor.editing.is_none();
    let add_client = editor.add_client;

    let mut options = vec![(
        None,
        "Server only".to_owned(),
        "Routes what clients publish; publishes nothing itself".to_owned(),
    )];
    options.extend(templates::modbus().iter().map(|profile| {
        (
            Some(profile.id.clone()),
            format!("{} {}", profile.manufacturer, profile.model),
            format!(
                "{} · {} subjects",
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
                .id(("nats-template", index))
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
                    editor.nats.template = choose.clone();
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
                .child(
                    div()
                        .flex_1()
                        .child(field("Listen on", fields.bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Port", fields.server_port.clone(), cx)),
                ),
        )
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .child(field("Username", fields.server_username.clone(), cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .child(field("Password", fields.server_password.clone(), cx)),
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
                    .child(
                        div()
                            .flex_1()
                            .child(field("Subject prefix", fields.prefix.clone(), cx)),
                    )
                    .child(div().w(px(150.0)).child(field(
                        "Publish every (ms)",
                        fields.publish_ms.clone(),
                        cx,
                    ))),
            )
            .child(hint(
                "Each value is a number at <prefix>.<value>, with a JSON nameplate at \
                 <prefix>.status. A number sent to <prefix>.<value>.set is held; a request to \
                 <prefix>.get answers with every value.",
                cx,
            ))
            .child(checkbox(
                "nats-jetstream",
                fields.jetstream,
                "Keep the device's messages in a JetStream stream with a durable consumer",
                on_edit(cx, |editor| editor.nats.jetstream = !editor.nats.jetstream),
                cx,
            ))
        })
        .when(new && selected.is_some(), |this| {
            this.child(checkbox(
                "nats-add-client",
                add_client,
                "Also add a client that subscribes to it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
