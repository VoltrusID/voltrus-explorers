//! The OPC UA editors: a client with its endpoint picker, sign-in and monitored variables, and
//! the plant server simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use vt_app_core::forms::OpcuaAuthKind;
use vt_protocol::{OpcuaSecurityMode, OpcuaSecurityPolicy};
use vt_ui::Icon;
use vt_ui::components::{
    Button, IconButton, Segment, checkbox, field, pill, section_label, segmented,
};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::editor::Editor;
use crate::model::opcua::{CertificateFile, Discovery};

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

/// The policies offered as buttons: the current ones, plus a deprecated one a saved connection
/// or a picked endpoint uses.
fn policies(current: OpcuaSecurityPolicy) -> Vec<OpcuaSecurityPolicy> {
    let mut shown: Vec<OpcuaSecurityPolicy> = OpcuaSecurityPolicy::ALL
        .into_iter()
        .filter(|policy| !policy.deprecated())
        .collect();
    if current.deprecated() {
        shown.push(current);
    }
    shown
}

fn policy_label(policy: OpcuaSecurityPolicy) -> &'static str {
    match policy {
        OpcuaSecurityPolicy::None => "None",
        OpcuaSecurityPolicy::Basic128Rsa15 => "Basic128Rsa15",
        OpcuaSecurityPolicy::Basic256 => "Basic256",
        OpcuaSecurityPolicy::Basic256Sha256 => "Basic256Sha256",
        OpcuaSecurityPolicy::Aes128Sha256RsaOaep => "Aes128 RsaOaep",
        OpcuaSecurityPolicy::Aes256Sha256RsaPss => "Aes256 RsaPss",
    }
}

/// The endpoint picker: GetEndpoints and the endpoints the server offers.
fn picker(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.opcua;
    let searching = matches!(fields.discovery, Discovery::Searching(_));
    let status: SharedString = match &fields.discovery {
        Discovery::Idle => "Ask the server which security and sign-in it offers".into(),
        Discovery::Searching(_) => "Asking the server…".into(),
        Discovery::Found(endpoints) if endpoints.is_empty() => {
            "The server offers no endpoint".into()
        }
        Discovery::Found(endpoints) => format!(
            "{} endpoint{} · pick one to use its security",
            endpoints.len(),
            if endpoints.len() == 1 { "" } else { "s" }
        )
        .into(),
        Discovery::Failed(_) => "The server did not answer".into(),
    };

    let mut list = div().flex().flex_col().gap(SPACE_3);
    match &fields.discovery {
        Discovery::Found(endpoints) => {
            for (index, endpoint) in endpoints.iter().enumerate() {
                let picked = fields.picked(endpoint);
                let usable = endpoint.policy.is_some();
                let choose = endpoint.clone();
                let secure = endpoint.mode != OpcuaSecurityMode::None;
                let policy = endpoint.policy.map_or_else(
                    || {
                        endpoint
                            .policy_uri
                            .rsplit('#')
                            .next()
                            .unwrap_or_default()
                            .to_owned()
                    },
                    |policy| policy.name().to_owned(),
                );
                list = list.child(
                    div()
                        .id(ElementId::NamedInteger(
                            "opcua-endpoint".into(),
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
                        .when(usable, |this| {
                            this.cursor_pointer()
                                .when(!picked, |this| this.hover(|style| style.bg(theme.hover)))
                                .on_click(cx.listener(move |model, _, _, cx| {
                                    model.pick_opcua_endpoint(&choose, cx);
                                }))
                        })
                        .child(vt_ui::icon(
                            if secure { Icon::Lock } else { Icon::LockOpen },
                            ICON_LG,
                            if picked {
                                theme.accent
                            } else if secure {
                                theme.success
                            } else {
                                theme.fg_muted
                            },
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
                                        .child(policy),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_muted)
                                        .child(endpoint.mode.name()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child(endpoint.user_tokens.join(" · ")),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(format!("level {}", endpoint.security_level)),
                        )
                        .when(!usable, |this| {
                            this.child(pill("UNSUPPORTED", theme.warning))
                        })
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
            row()
                .child(
                    div()
                        .flex_1()
                        .child(field("Endpoint", fields.endpoint.clone(), cx)),
                )
                .child(
                    Button::new(
                        "opcua-discover",
                        if searching {
                            "Asking…"
                        } else {
                            "Get endpoints"
                        },
                    )
                    .icon(if searching {
                        Icon::RefreshCw
                    } else {
                        Icon::Search
                    })
                    .disabled(searching)
                    .tooltip(
                        "GetEndpoints: the security policies, modes and sign-in the server offers",
                    )
                    .on_click(cx.listener(|model, _, _, cx| model.discover_opcua(cx))),
                ),
        )
        .child(
            div()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(status),
        )
        .child(list)
        .into_any_element()
}

fn choose(which: CertificateFile, cx: &mut Context<AppModel>) -> impl IntoElement {
    Button::new(
        match which {
            CertificateFile::Certificate => "opcua-choose-certificate",
            CertificateFile::PrivateKey => "opcua-choose-key",
        },
        "Choose…",
    )
    .icon(Icon::File)
    .on_click(cx.listener(move |model, _, _, cx| model.choose_opcua_file(which, cx)))
}

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.opcua;
    let (policy, mode, auth) = (fields.policy, fields.mode, fields.auth);

    let policy_buttons = segmented(
        "opcua-policy",
        policies(policy)
            .into_iter()
            .map(|option| {
                Segment::new(
                    policy_label(option),
                    option == policy,
                    on_edit(cx, move |editor| {
                        editor.opcua.policy = option;
                        // Keep the pair valid: None goes with None, a policy with a signature.
                        if option == OpcuaSecurityPolicy::None {
                            editor.opcua.mode = OpcuaSecurityMode::None;
                        } else if editor.opcua.mode == OpcuaSecurityMode::None {
                            editor.opcua.mode = OpcuaSecurityMode::SignAndEncrypt;
                        }
                    }),
                )
            })
            .collect(),
        cx,
    );
    let mode_buttons = segmented(
        "opcua-mode",
        OpcuaSecurityMode::ALL
            .into_iter()
            .map(|option| {
                Segment::new(
                    option.name(),
                    option == mode,
                    on_edit(cx, move |editor| {
                        editor.opcua.mode = option;
                        if option == OpcuaSecurityMode::None {
                            editor.opcua.policy = OpcuaSecurityPolicy::None;
                        } else if editor.opcua.policy == OpcuaSecurityPolicy::None {
                            editor.opcua.policy = OpcuaSecurityPolicy::Basic256Sha256;
                        }
                    }),
                )
                .icon(match option {
                    OpcuaSecurityMode::None => Icon::LockOpen,
                    OpcuaSecurityMode::Sign | OpcuaSecurityMode::SignAndEncrypt => Icon::Lock,
                })
            })
            .collect(),
        cx,
    );
    let auth_buttons = segmented(
        "opcua-auth",
        OpcuaAuthKind::ALL
            .into_iter()
            .map(|option| {
                Segment::new(
                    option.name(),
                    option == auth,
                    on_edit(cx, move |editor| editor.opcua.auth = option),
                )
            })
            .collect(),
        cx,
    );
    let credentials: AnyElement = match auth {
        OpcuaAuthKind::Anonymous => hint(
            "No user: the server decides what anonymous clients may read and write.",
            cx,
        )
        .into_any_element(),
        OpcuaAuthKind::UserName => row()
            .child(
                div()
                    .flex_1()
                    .child(field("User name", fields.username.clone(), cx)),
            )
            .child(
                div()
                    .flex_1()
                    .child(field("Password", fields.password.clone(), cx)),
            )
            .into_any_element(),
        OpcuaAuthKind::Certificate => div()
            .flex()
            .flex_col()
            .gap(SPACE_5)
            .child(
                row()
                    .child(div().flex_1().child(field(
                        "User certificate",
                        fields.certificate.clone(),
                        cx,
                    )))
                    .child(choose(CertificateFile::Certificate, cx)),
            )
            .child(
                row()
                    .child(div().flex_1().child(field(
                        "Private key",
                        fields.private_key.clone(),
                        cx,
                    )))
                    .child(choose(CertificateFile::PrivateKey, cx)),
            )
            .into_any_element(),
    };

    let nodes = fields
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .h(px(32.0))
                .px(SPACE_6)
                .rounded(RADIUS_MD)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Gauge, ICON_MD, theme.fg_muted))
                .child(div().flex_none().max_w(px(220.0)).truncate().child(
                    if node.name.is_empty() {
                        "Display name".to_owned()
                    } else {
                        node.name.clone()
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(node.node_id.clone()),
                )
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("opcua-remove-node".into(), index as u64),
                        Icon::Trash,
                    )
                    .tooltip("Stop monitoring it")
                    .on_click(on_edit(cx, move |editor| {
                        if index < editor.opcua.nodes.len() {
                            editor.opcua.nodes.remove(index);
                        }
                    })),
                )
        })
        .collect::<Vec<_>>();
    let count = nodes.len();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(group("Server", cx).child(picker(editor, cx)))
        .child(
            group("Security", cx)
                .child(policy_buttons)
                .child(mode_buttons)
                .when(policy.deprecated(), |this| {
                    this.child(hint(
                        "This policy uses SHA-1 and is deprecated: prefer Basic256Sha256 or an Aes policy.",
                        cx,
                    ))
                }),
        )
        .child(group("Sign in", cx).child(auth_buttons).child(credentials))
        .child(
            group("Subscription", cx)
                .child(
                    row()
                        .child(div().w(px(150.0)).flex_none().child(field(
                            "Publishing (ms)",
                            fields.publishing.clone(),
                            cx,
                        )))
                        .child(div().w(px(150.0)).flex_none().child(field(
                            "Sampling (ms)",
                            fields.sampling.clone(),
                            cx,
                        )))
                        .child(div().flex_1().min_w_0().pb(SPACE_3).child(hint(
                            "The server samples each variable this often and sends changes every publishing interval. 0 samples as fast as it can.",
                            cx,
                        ))),
                ),
        )
        .child(
            group("Monitored variables", cx)
                .children(nodes)
                .when(count == 0, |this| {
                    this.child(hint(
                        "None yet. Connect, then add variables from the address-space browser: select one and press Enter.",
                        cx,
                    ))
                }),
        )
        .into_any_element()
}

pub fn server(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let fields = &editor.opcua;
    let anonymous = fields.anonymous;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
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
                        .w(px(130.0))
                        .flex_none()
                        .child(field("TCP port", fields.server_port.clone(), cx)),
                ),
        )
        .child(hint(
            "127.0.0.1 keeps the simulator on this computer; 0.0.0.0 or :: opens it to the network. 4840 is the OPC UA port. Endpoints: None, Basic256Sha256 (Sign, Sign & Encrypt) and Aes256 RsaPss.",
            cx,
        ))
        .child(
            group("Sign in", cx)
                .child(checkbox(
                    "opcua-anonymous",
                    anonymous,
                    "Allow anonymous clients",
                    on_edit(cx, |editor| editor.opcua.anonymous = !editor.opcua.anonymous),
                    cx,
                ))
                .child(
                    row()
                        .child(div().flex_1().child(field(
                            "User name",
                            fields.server_user.clone(),
                            cx,
                        )))
                        .child(div().flex_1().child(field(
                            "Password",
                            fields.server_password.clone(),
                            cx,
                        ))),
                )
                .child(hint(
                    "Clients can also sign in with the user certificate the server creates in its certificate folder; the log names it.",
                    cx,
                )),
        )
        .child(
            group("Address space", cx).child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_5)
                    .p(SPACE_5)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(theme.accent)
                    .bg(theme.accent_soft)
                    .child(vt_ui::icon(Icon::Factory, ICON_LG, theme.accent))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Process plant"))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(
                                        "Batch reactor, steam boiler and utilities · 17 variables with units and ranges · StartBatch and ResetAlarms methods",
                                    ),
                            ),
                    ),
            ),
        )
        .when(new, |this| {
            this.child(checkbox(
                "opcua-add-client",
                add_client,
                "Also add a client that browses and monitors it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
