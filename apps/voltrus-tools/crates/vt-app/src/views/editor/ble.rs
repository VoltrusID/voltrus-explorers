//! The BLE editor: the adapter, a scan of advertising peripherals with their
//! signal and advertisement, and how to read the one picked.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, SharedString, div, px};
use vt_engine::ble::{decode, gatt};
use vt_protocol::{BleAdapter, BleDeviceInfo};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, checkbox, field, section_label, segmented};
use vt_ui::theme::*;

use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::ble::device_label;
use crate::model::editor::Editor;

fn group(title: &'static str, cx: &Context<AppModel>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(title, cx))
}

/// Bars lit for a signal strength, out of four.
fn bars(rssi: Option<i16>) -> usize {
    match rssi {
        Some(rssi) if rssi >= -60 => 4,
        Some(rssi) if rssi >= -70 => 3,
        Some(rssi) if rssi >= -80 => 2,
        Some(rssi) if rssi >= -90 => 1,
        _ => 0,
    }
}

/// A four-bar signal meter.
fn signal(rssi: Option<i16>, lit: Hsla, unlit: Hsla) -> impl IntoElement {
    let on = bars(rssi);
    div()
        .flex()
        .items_end()
        .gap(px(2.0))
        .h(px(14.0))
        .children((0..4).map(move |bar| {
            div()
                .w(px(3.0))
                .h(px(5.0 + 3.0 * bar as f32))
                .rounded(px(1.0))
                .bg(if bar < on { lit } else { unlit })
        }))
}

/// What a peripheral advertises, in a line: its services, the company
/// behind its manufacturer data, and its transmit power.
fn advertisement(device: &BleDeviceInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !device.services.is_empty() {
        parts.push(
            device
                .services
                .iter()
                .map(|uuid| gatt::name(uuid).map_or_else(|| gatt::short(uuid), str::to_owned))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    for data in &device.manufacturer {
        let company = gatt::company(data.company)
            .map_or_else(|| format!("Company 0x{:04X}", data.company), str::to_owned);
        parts.push(format!("{company} {}", decode::hex(&data.data)));
    }
    for data in &device.service_data {
        parts.push(format!(
            "{} data {}",
            gatt::short(&data.service),
            decode::hex(&data.data)
        ));
    }
    if let Some(power) = device.tx_power {
        parts.push(format!("Tx {power} dBm"));
    }
    if parts.is_empty() {
        "No advertisement data".into()
    } else {
        parts.join(" · ")
    }
}

/// The scan: its status, the button, and the peripherals seen.
fn picker(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let scan = &editor.ble.scan;
    let scanning = scan.request.is_some();
    let count = scan.devices.len();
    let found = format!("{count} peripheral{}", if count == 1 { "" } else { "s" });
    let status: SharedString = match (&scan.error, scanning, scan.done) {
        (Some(_), _, _) => "The scan could not start".into(),
        (None, true, _) => format!("Scanning… {found} so far").into(),
        (None, false, true) => format!("{found} advertising").into(),
        (None, false, false) => "Find peripherals that are advertising nearby".into(),
    };

    let mut list = div()
        .id("ble-devices")
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .max_h(px(300.0))
        .overflow_y_scroll();
    if let Some(error) = &scan.error {
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
    } else if scan.done && scan.devices.is_empty() {
        list = list.child(
            div()
                .px(SPACE_6)
                .py(SPACE_5)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(match editor.ble.adapter {
                    BleAdapter::System => {
                        "Nothing is advertising. Check that Bluetooth is on, that Voltrus Tools may use it \
                         (System Settings › Privacy & Security › Bluetooth), and that the peripheral is \
                         awake and not connected to another device."
                    }
                    BleAdapter::Simulated => "The simulated peripherals did not answer.",
                }),
        );
    }
    for (index, device) in scan.devices.iter().enumerate() {
        let picked = editor.ble.picked(device, cx);
        let choose = device.clone();
        let named = !device.name.is_empty();
        let ring = theme.accent;
        list = list.child(
            div()
                .id(ElementId::NamedInteger("ble-device".into(), index as u64))
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_5)
                .px(SPACE_6)
                .py(SPACE_4)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(if picked { theme.accent } else { theme.border })
                .bg(if picked {
                    theme.accent_soft
                } else {
                    theme.surface
                })
                .cursor_pointer()
                // Tab reaches every row; Enter or Space picks it.
                .tab_index(0)
                .focus(move |style| style.border_color(ring))
                .when(!picked, |this| this.hover(|style| style.bg(theme.hover)))
                .on_click(cx.listener(move |model, _, _, cx| {
                    model.pick_ble_device(&choose, cx);
                }))
                .child(vt_ui::icon(
                    Icon::Bluetooth,
                    ICON_LG,
                    if picked { theme.accent } else { theme.fg_muted },
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(SPACE_1)
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(SPACE_4)
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_none()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(if named { theme.fg } else { theme.fg_muted })
                                        .child(if named {
                                            device.name.clone()
                                        } else {
                                            "Unnamed".to_owned()
                                        }),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child(device.address.clone()),
                                ),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(advertisement(device)),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(
                            div()
                                .w(px(58.0))
                                .flex()
                                .justify_end()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(match device.rssi {
                                    Some(rssi) => format!("{rssi} dBm"),
                                    None => "—".to_owned(),
                                }),
                        )
                        .child(signal(device.rssi, theme.accent, theme.border_strong)),
                )
                .when(picked, |this| {
                    this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                }),
        );
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
                        .child(section_label("Peripherals", cx))
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(status),
                        ),
                )
                .child(
                    Button::new("ble-scan", if scanning { "Scanning…" } else { "Scan" })
                        .icon(if scanning {
                            Icon::RefreshCw
                        } else {
                            Icon::Radio
                        })
                        .compact()
                        .disabled(scanning)
                        .tooltip("Listen for advertising peripherals for a few seconds")
                        .on_click(cx.listener(|model, _, _, cx| model.scan_ble(cx))),
                ),
        )
        .when(
            !scan.devices.is_empty() || scan.error.is_some() || scan.done,
            |this| this.child(list),
        )
        .into_any_element()
}

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.ble;
    let adapter = fields.adapter;
    let notify = fields.notify;
    let picked_name = editor
        .ble
        .scan
        .devices
        .iter()
        .find(|device| fields.picked(device, cx))
        .map(|device| device_label(device).to_owned());
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Adapter", cx).child(
                div().flex().child(segmented(
                    "ble-adapter",
                    [BleAdapter::System, BleAdapter::Simulated]
                        .into_iter()
                        .map(|option| {
                            Segment::new(
                                match option {
                                    BleAdapter::System => "This Mac's Bluetooth",
                                    BleAdapter::Simulated => "Simulated peripherals",
                                },
                                adapter == option,
                                cx.listener(move |model, _, _, cx| {
                                    model.set_ble_adapter(option, cx);
                                }),
                            )
                            .icon(match option {
                                BleAdapter::System => Icon::Bluetooth,
                                BleAdapter::Simulated => Icon::Cpu,
                            })
                        })
                        .collect(),
                    cx,
                )),
            ),
        )
        .child(picker(editor, cx))
        .child(group("Peripheral", cx).child(field(
            match picked_name {
                Some(name) => SharedString::from(format!("Identifier · {name}")),
                None => SharedString::from("Identifier"),
            },
            fields.peripheral.clone(),
            cx,
        )))
        .child(
            group("Reading", cx)
                .child(row().child(div().w(px(150.0)).child(field(
                    "Re-read every (ms)",
                    fields.poll.clone(),
                    cx,
                ))))
                .child(checkbox(
                    "ble-notify",
                    notify,
                    "Subscribe to every characteristic that notifies or indicates",
                    on_edit(cx, |editor| editor.ble.notify = !editor.ble.notify),
                    cx,
                )),
        )
        .into_any_element()
}
