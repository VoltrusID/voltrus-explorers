//! The MTConnect and FANUC editors: a client and a simulator of each.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, div, px};
use vt_protocol::{FocasProfile, MtconnectTemplate};
use vt_ui::Icon;
use vt_ui::components::{Segment, checkbox, field, pill, section_label, segmented};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::cnc::BETA_NOTE;
use crate::model::editor::Editor;

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

fn hint(text: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    let theme = theme(cx);
    div()
        .text_size(FONT_SMALL)
        .text_color(theme.fg_dim)
        .child(text)
}

/// A choice between the machines or controllers a simulator plays.
fn card(
    id: &'static str,
    index: usize,
    active: bool,
    title: &'static str,
    description: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    cx: &Context<AppModel>,
) -> impl IntoElement {
    let theme = theme(cx).clone();
    div()
        .id((id, index))
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
        .on_click(on_click)
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
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(description),
                ),
        )
}

pub fn mtconnect_client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.cnc;
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Agent", cx)
                .child(field("Agent address", fields.url.clone(), cx))
                .child(
                    row()
                        .child(
                            div()
                                .flex_1()
                                .child(field("Device", fields.device.clone(), cx)),
                        )
                        .child(div().flex_1().pb(SPACE_3).child(hint(
                            "Leave empty to watch every device the agent serves.",
                            cx,
                        ))),
                ),
        )
        .child(
            group("Streaming", cx).child(
                row()
                    .child(div().w(px(150.0)).child(field(
                        "Interval (ms)",
                        fields.interval.clone(),
                        cx,
                    )))
                    .child(div().w(px(150.0)).child(field(
                        "Timeout (ms)",
                        fields.timeout.clone(),
                        cx,
                    )))
                    .child(div().flex_1().pb(SPACE_3).child(hint(
                        "The agent's multipart stream, polling /sample when it does not stream.",
                        cx,
                    ))),
            ),
        )
        .into_any_element()
}

pub fn mtconnect_agent(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.cnc;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let chosen = fields.agent_template;
    let cards = MtconnectTemplate::ALL
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let template = *template;
            card(
                "mtconnect-template",
                index,
                chosen == template,
                template.name(),
                template.description(),
                on_edit(cx, move |editor| editor.cnc.agent_template = template),
                cx,
            )
        })
        .collect::<Vec<_>>();
    let shdr = fields.shdr;
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
                        .child(field("Listen on", fields.agent_bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .child(field("HTTP port", fields.agent_port.clone(), cx)),
                )
                .child(div().flex_1().pb(SPACE_3).child(hint(
                    "0.0.0.0 opens it to the network. Agents usually use 5000, which macOS's AirPlay Receiver holds.",
                    cx,
                ))),
        )
        .child(group("Machine", cx).children(cards))
        .child(
            group("SHDR adapter", cx)
                .child(checkbox(
                    "mtconnect-shdr",
                    shdr,
                    "Also serve the machine's observations as an SHDR adapter",
                    on_edit(cx, |editor| editor.cnc.shdr = !editor.cnc.shdr),
                    cx,
                ))
                .when(shdr, |this| {
                    this.child(
                        row().child(div().w(px(120.0)).child(field(
                            "SHDR port",
                            fields.shdr_port.clone(),
                            cx,
                        ))),
                    )
                }),
        )
        .when(new, |this| {
            this.child(checkbox(
                "mtconnect-add-client",
                add_client,
                "Also add a client that streams it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}

/// The Beta label and what it means.
fn beta(cx: &Context<AppModel>) -> gpui::Div {
    let theme = theme(cx).clone();
    div()
        .flex()
        .items_start()
        .gap(SPACE_4)
        .p(SPACE_5)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(pill("Beta", theme.accent))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(BETA_NOTE),
        )
}

pub fn focas_client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.cnc;
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(beta(cx))
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Controller", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(
                            div()
                                .w(px(96.0))
                                .child(field("Port", fields.port.clone(), cx)),
                        ),
                )
                .child(hint(
                    "The controller's built-in Ethernet (FOCAS2/Ethernet, port 8193) must be enabled.",
                    cx,
                )),
        )
        .child(
            group("Polling", cx).child(
                row()
                    .child(
                        div()
                            .w(px(150.0))
                            .child(field("Poll (ms)", fields.poll.clone(), cx)),
                    )
                    .child(div().w(px(150.0)).child(field(
                        "Timeout (ms)",
                        fields.focas_timeout.clone(),
                        cx,
                    ))),
            ),
        )
        .child(
            group("Macro variables", cx)
                .child(field("Numbers", fields.macros.clone(), cx))
                .child(hint(
                    "Common macro variables to read, and to write from the inspector: 500, 501 or 500-503.",
                    cx,
                )),
        )
        .into_any_element()
}

pub fn focas_simulator(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.cnc;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let chosen = fields.profile;
    let run = fields.run;
    let profiles = segmented(
        "focas-profile",
        FocasProfile::ALL
            .iter()
            .map(|profile| {
                let profile = *profile;
                Segment::new(
                    profile.name(),
                    chosen == profile,
                    on_edit(cx, move |editor| editor.cnc.profile = profile),
                )
            })
            .collect(),
        cx,
    );
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
                        .child(field("Listen on", fields.sim_bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .child(field("TCP port", fields.sim_port.clone(), cx)),
                )
                .child(div().flex_1().pb(SPACE_3).child(hint(
                    "0.0.0.0 opens it to the network. FOCAS/Ethernet uses port 8193.",
                    cx,
                ))),
        )
        .child(
            group("Controller", cx)
                .child(field("Series", profiles, cx))
                .child(hint(
                    "A three-axis mill running a demo program: status, positions, feed, spindle, alarms and macros. The 0i-F differs only in its identity.",
                    cx,
                )),
        )
        .child(checkbox(
            "focas-run",
            run,
            "Run the demo program as soon as it starts",
            on_edit(cx, |editor| editor.cnc.run = !editor.cnc.run),
            cx,
        ))
        .when(new, |this| {
            this.child(checkbox(
                "focas-add-client",
                add_client,
                "Also add a client that polls it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
