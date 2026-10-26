//! The inspector's characteristic card for BLE sessions: where the selected
//! characteristic lives and what it supports, reading it now, its
//! notifications, how it decodes, and writing bytes or text to it.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, SharedString, div, px};
use vt_engine::ble::gatt;
use vt_protocol::BleFormat;
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, pill, section_label, segmented};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::ble::BleState;
use crate::model::live::Live;

pub fn card(live: &Live, point: u32, state: &BleState, cx: &Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(characteristic) = live.ble_characteristic(point) else {
        return div().into_any_element();
    };
    let running = live.session.is_some();
    let properties = characteristic.properties;
    let notifying = characteristic.notifying;
    let current = characteristic.decoder.format;
    let service = match gatt::name(&characteristic.service) {
        Some(name) => format!("{name} · {}", gatt::short(&characteristic.service)),
        None => characteristic.service.clone(),
    };
    let row = |label: &'static str, value: String| {
        div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(SPACE_5)
            .text_size(FONT_SMALL)
            .child(div().flex_none().text_color(theme.fg_muted).child(label))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .text_color(theme.fg)
                    .child(value),
            )
    };
    let mut supports = Vec::new();
    if properties.read {
        supports.push(pill("READ", theme.info));
    }
    if properties.write {
        supports.push(pill("WRITE", theme.warning));
    }
    if properties.write_without_response {
        supports.push(pill("WRITE NO RESP", theme.warning));
    }
    if properties.notify {
        supports.push(pill("NOTIFY", theme.success));
    }
    if properties.indicate {
        supports.push(pill("INDICATE", theme.success));
    }

    let ring = theme.accent;
    let chips = BleFormat::ALL
        .into_iter()
        .enumerate()
        .map(|(index, format)| {
            let selected = format == current;
            let hover = theme.hover;
            div()
                .id(ElementId::NamedInteger("ble-format".into(), index as u64))
                .flex()
                .items_center()
                .h(CONTROL_SM)
                .px(SPACE_4)
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    theme.accent_soft
                } else {
                    theme.surface
                })
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(if selected {
                    theme.accent
                } else {
                    theme.fg_muted
                })
                .child(format.label())
                .when(!running, |this| this.opacity(0.45))
                .when(running, |this| {
                    this.cursor_pointer()
                    // Tab reaches every format; Enter or Space picks it.
                    .tab_index(0)
                    .focus(move |style| style.border_color(ring))
                    .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                    .on_click(cx.listener(move |model, _, _, cx| {
                        model.decode_selected_ble(format, cx);
                    }))
                })
        });

    let mut content = div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label("Characteristic", cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(row("Service", service))
                .child(row("UUID", characteristic.uuid.clone()))
                .child(div().flex().flex_wrap().gap(SPACE_2).children(supports)),
        )
        .child(
            div()
                .flex()
                .gap(SPACE_3)
                .child(
                    Button::new("ble-read", "Read now")
                        .icon(Icon::RefreshCw)
                        .compact()
                        .disabled(!running || !properties.read)
                        .tooltip("Read the value from the peripheral")
                        .on_click(cx.listener(|model, _, _, cx| model.read_selected_ble(cx))),
                )
                .child(
                    Button::new(
                        "ble-notify",
                        if notifying {
                            "Stop notifications"
                        } else if properties.indicate && !properties.notify {
                            "Indicate"
                        } else {
                            "Notify"
                        },
                    )
                    .icon(if notifying {
                        Icon::Pause
                    } else {
                        Icon::Activity
                    })
                    .compact()
                    .disabled(!running || !properties.pushes())
                    .tooltip("Have the peripheral push every change")
                    .on_click(cx.listener(|model, _, _, cx| {
                        model.toggle_selected_ble_notify(cx);
                    })),
                )
                .when(notifying && running, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .child(pill("LIVE", theme.success)),
                    )
                }),
        )
        .child(section_label("Decode as", cx))
        .child(div().flex().flex_wrap().gap(SPACE_2).children(chips))
        // The fields keep their widths; the hint gets a row of its own so a
        // narrow inspector does not squeeze them.
        .child(
            div()
                .flex()
                .items_end()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_none()
                        .w(px(80.0))
                        .child(field("Offset", state.offset.clone(), cx)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(100.0))
                        .child(field("Scale", state.scale.clone(), cx)),
                ),
        )
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child("For number formats. Enter applies."),
        )
        .when_some(state.error.clone(), |this, error| {
            this.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                    .child(div().flex_1().child(error)),
            )
        });

    if properties.writable() {
        let text = state.text;
        content = content
            .child(section_label("Write bytes or text", cx))
            .child(
                div().flex().child(segmented(
                    "ble-write-mode",
                    vec![
                        Segment::new(
                            "Hex",
                            !text,
                            cx.listener(|model, _, _, cx| model.set_ble_text_mode(false, cx)),
                        )
                        .icon(Icon::Binary),
                        Segment::new(
                            "Text",
                            text,
                            cx.listener(|model, _, _, cx| model.set_ble_text_mode(true, cx)),
                        )
                        .icon(Icon::Terminal),
                    ],
                    cx,
                )),
            )
            .child(
                div()
                    .flex()
                    .gap(SPACE_3)
                    .child(div().flex_1().child(state.data.clone()))
                    .child(
                        Button::new("ble-write", "Write")
                            .icon(Icon::Send)
                            .primary()
                            .disabled(!running)
                            .on_click(cx.listener(|model, _, window, cx| {
                                model.write_ble_bytes(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(SharedString::from(if text {
                        "Sent as UTF-8. The result shows under Write to device."
                    } else {
                        "Hex bytes, e.g. 01 FF 3A or 0x01ff3a. The result shows under Write to device."
                    })),
            );
    }
    content.into_any_element()
}
