//! The inspector's explicit message card for EtherNet/IP clients: any CIP
//! service to any object, with the reply as hex and the values it may hold.

use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div, px};
use vt_app_core::forms::{hex_rows, interpret_reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, section_label, segmented};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::enip::{CipForm, CipPreset};
use crate::model::live::Live;

/// Reply bytes per hex row: fits the narrowest inspector.
const HEX_ROW: usize = 8;
/// Rows shown before the dump scrolls.
const HEX_HEIGHT: f32 = 168.0;

pub fn card(live: &Live, form: &CipForm, cx: &Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let running = live.session.is_some();
    let preset = form.preset(cx);
    let narrow = |content: AnyElement| div().w(px(76.0)).child(content);

    let mut card = div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label("Explicit message", cx))
        .child(segmented(
            "cip-preset",
            CipPreset::ALL
                .into_iter()
                .map(|option| {
                    Segment::new(
                        option.label(),
                        preset == Some(option),
                        cx.listener(move |model, _, _, cx| model.set_cip_preset(option, cx)),
                    )
                })
                .collect(),
            cx,
        ))
        .child(
            div()
                .flex()
                .gap(SPACE_4)
                .child(narrow(
                    field("Service", form.service.clone(), cx).into_any_element(),
                ))
                .child(narrow(
                    field("Class", form.class.clone(), cx).into_any_element(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(field("Instance", form.instance.clone(), cx)),
                )
                .child(narrow(
                    field("Attribute", form.attribute.clone(), cx).into_any_element(),
                )),
        )
        .child(field("Data", form.data.clone(), cx))
        .child(
            div().flex().justify_end().child(
                Button::new(
                    "cip-send",
                    if live.cip_waiting {
                        "Sending…"
                    } else {
                        "Send"
                    },
                )
                .icon(Icon::Send)
                .primary()
                .disabled(!running || live.cip_waiting)
                .tooltip("Send the request  ↵")
                .on_click(cx.listener(|model, _, window, cx| {
                    model.send_cip_request(window, cx);
                })),
            ),
        );

    if !running {
        return card
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child("Connect to send explicit messages."),
            )
            .into_any_element();
    }
    match &live.cip_reply {
        None => {}
        Some(Err(error)) => {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(SharedString::from(error.clone())),
                    ),
            );
        }
        Some(Ok(reply)) => {
            let name =
                vt_engine::cip_service_name(reply.request.service, Some(reply.request.class));
            let readings = interpret_reply(&reply.data);
            card = card
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.success)
                        .child(vt_ui::icon(Icon::CircleCheck, ICON_MD, theme.success))
                        .child(div().flex_1().min_w_0().truncate().child(format!(
                            "{name} · {} byte{} · {:.1} ms",
                            reply.data.len(),
                            if reply.data.len() == 1 { "" } else { "s" },
                            f64::from(reply.latency_us) / 1000.0
                        ))),
                )
                .when(!reply.data.is_empty(), |this| {
                    this.child(
                        div()
                            .id("cip-reply")
                            .max_h(px(HEX_HEIGHT))
                            .overflow_y_scroll()
                            .px(SPACE_5)
                            .py(SPACE_4)
                            .rounded(RADIUS_MD)
                            .bg(theme.surface)
                            .border_1()
                            .border_color(theme.border)
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .children(hex_rows(&reply.data, HEX_ROW).into_iter().map(
                                |(offset, hex)| {
                                    div()
                                        .flex()
                                        .gap(SPACE_5)
                                        .child(
                                            div()
                                                .text_color(theme.fg_dim)
                                                .child(format!("{offset:04X}")),
                                        )
                                        .child(div().text_color(theme.fg).child(hex))
                                },
                            )),
                    )
                })
                .when(!readings.is_empty(), |this| {
                    this.child(div().flex().flex_col().gap(SPACE_2).children(
                        readings.into_iter().map(|(label, value)| {
                            div()
                                .flex()
                                .justify_between()
                                .gap(SPACE_4)
                                .text_size(FONT_SMALL)
                                .child(div().text_color(theme.fg_muted).child(label))
                                .child(div().min_w_0().truncate().font_family(MONO).child(value))
                        }),
                    ))
                });
        }
    }
    card.into_any_element()
}
