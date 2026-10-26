//! The unlock sheet (start the trial, unlock, restore), the About sheet and
//! the licence item in the status bar.
//!
//! Before the trial starts the sheet says how long it lasts, what stops when
//! it ends and what the unlock costs, as App Review guideline 3.1.1 asks.

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, MouseButton, SharedString, div, hsla, px};
use vt_store::{Licence, TRIAL_DAYS};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, brand_mark, kbd, pill, section_label};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::licence::{
    Busy, LICENCE, LicenceConfirm, LicenceDismiss, Sheet, plural_days as days,
};

/// Everything the trial and the unlock cover.
const PROTOCOLS: [&str; 10] = [
    "Modbus",
    "MQTT",
    "BACnet",
    "OPC UA",
    "S7",
    "EtherNet/IP",
    "DNP3",
    "NATS",
    "Serial",
    "BLE",
];

/// The trial or read-only state in the status bar; nothing once unlocked.
pub fn status_item(model: &AppModel, cx: &mut Context<AppModel>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    let (glyph, text, color) = match model.licensing.licence {
        Licence::NotStarted => (Icon::Play, "Start free trial".to_owned(), theme.accent),
        Licence::Trial { days_left, .. } => (
            Icon::Clock,
            format!("Trial · {} left", days(days_left)),
            if days_left <= 3 {
                theme.warning
            } else {
                theme.accent
            },
        ),
        Licence::Expired => (Icon::Lock, "Read-only · Unlock".to_owned(), theme.warning),
        Licence::Unlocked | Licence::Unrestricted => return None,
    };
    let hover = theme.hover;
    Some(
        div()
            .id("licence-status")
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_3)
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .text_color(color)
            .font_weight(FontWeight::MEDIUM)
            .on_click(cx.listener(|model, _, _, cx| {
                model.open_licence_sheet(Sheet::Unlock, cx);
            }))
            .child(vt_ui::icon(glyph, ICON_SM, color))
            .child(text)
            .into_any_element(),
    )
}

/// The open sheet, or nothing.
pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let Some(sheet) = model.licensing.sheet else {
        return div().into_any_element();
    };
    let theme = theme(cx).clone();
    let licensing = &model.licensing;
    let (title, lead, body) = match sheet {
        Sheet::Unlock => unlock_text(model),
        Sheet::About => about_text(model),
    };

    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0.0, 0.0, 0.0, if theme.dark { 0.5 } else { 0.25 }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|model, _, window, cx| model.close_licence_sheet(window, cx)),
        )
        .child(
            div()
                .id("licence-sheet")
                .track_focus(&licensing.focus)
                .key_context(LICENCE)
                .on_action(cx.listener(|model, _: &LicenceConfirm, window, cx| {
                    model.licence_confirm(window, cx);
                }))
                .on_action(cx.listener(|model, _: &LicenceDismiss, window, cx| {
                    model.close_licence_sheet(window, cx);
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .w(px(560.0))
                .rounded(RADIUS_XL)
                .bg(theme.elevated)
                .border_1()
                .border_color(theme.border_strong)
                .shadow(theme.shadow(Elevation::Modal))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(SPACE_7)
                        .p(SPACE_8)
                        .child(brand_mark(px(48.0)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .gap(SPACE_3)
                                .child(
                                    div()
                                        .text_size(FONT_DISPLAY)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(FONT_EMPHASIS)
                                        .text_color(theme.fg_muted)
                                        .child(lead),
                                ),
                        )
                        .child(
                            IconButton::new("licence-close", Icon::X)
                                .tooltip("Close  esc")
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.close_licence_sheet(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_7)
                        .px(SPACE_8)
                        .pb(SPACE_8)
                        .children(body.into_iter().map(|part| part.render(&theme, cx))),
                )
                .when_some(licensing.notice.as_ref(), |this, (message, failed)| {
                    let color = if *failed { theme.danger } else { theme.success };
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_4)
                            .mx(SPACE_8)
                            .mb(SPACE_6)
                            .px(SPACE_6)
                            .py(SPACE_4)
                            .rounded(RADIUS_MD)
                            .bg(color.alpha(0.1))
                            .border_1()
                            .border_color(color.alpha(0.3))
                            .text_size(FONT_SMALL)
                            .text_color(color)
                            .child(vt_ui::icon(
                                if *failed {
                                    Icon::CircleAlert
                                } else {
                                    Icon::Info
                                },
                                ICON_MD,
                                color,
                            ))
                            .child(message.clone()),
                    )
                })
                .child(footer(model, sheet, cx)),
        )
        .into_any_element()
}

/// One block of a sheet's body.
enum Part {
    /// Each protocol as a pill: what the trial and the unlock include.
    Protocols,
    /// A highlighted note with an icon.
    Note(Icon, SharedString),
    /// A heading and paragraphs.
    Section(&'static str, Vec<SharedString>),
}

impl Part {
    fn render(self, theme: &Theme, cx: &App) -> AnyElement {
        match self {
            Part::Protocols => div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Every protocol, client and simulator", cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_3)
                        .children(PROTOCOLS.iter().map(|name| pill(*name, theme.accent))),
                )
                .into_any_element(),
            Part::Note(glyph, text) => div()
                .flex()
                .items_start()
                .gap(SPACE_5)
                .p(SPACE_6)
                .rounded(RADIUS_LG)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(vt_ui::icon(glyph, ICON_LG, theme.fg_muted))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_BODY)
                        .text_color(theme.fg_muted)
                        .child(text),
                )
                .into_any_element(),
            Part::Section(heading, paragraphs) => div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .child(section_label(heading, cx))
                .children(paragraphs.into_iter().map(|text| {
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(text)
                }))
                .into_any_element(),
        }
    }
}

/// "$49.00" from the App Store, or nothing while it's unknown (offline).
fn price(model: &AppModel) -> Option<String> {
    model.licensing.prices.unlock.clone()
}

/// "for $49.00, once" / "once" when the price is unknown.
fn for_price(model: &AppModel) -> String {
    match price(model) {
        Some(price) => format!("for {price}, once"),
        None => "with a one-time purchase".to_owned(),
    }
}

fn unlock_text(model: &AppModel) -> (SharedString, SharedString, Vec<Part>) {
    let read_only = "Your saved connections, layouts and the data you recorded stay open to read.";
    match model.licensing.licence {
        Licence::NotStarted => (
            format!("Try Voltrus Tools free for {TRIAL_DAYS} days").into(),
            "Connect, simulate and write with every tool. Starting the trial costs nothing."
                .into(),
            vec![
                Part::Protocols,
                Part::Note(
                    Icon::Clock,
                    format!(
                        "After {TRIAL_DAYS} days, connecting, simulating and writing stop until you \
                         unlock Voltrus Tools {}. {read_only}",
                        for_price(model)
                    )
                    .into(),
                ),
            ],
        ),
        Licence::Trial { days_left, .. } => (
            if days_left == 1 {
                "Last day of your free trial".into()
            } else {
                format!("{} left in your free trial", days(days_left)).into()
            },
            format!(
                "Unlock Voltrus Tools {} to keep connecting, simulating and writing after the trial.",
                for_price(model)
            )
            .into(),
            vec![
                Part::Protocols,
                Part::Note(
                    Icon::Info,
                    format!("When the trial ends the app turns read-only. {read_only}").into(),
                ),
            ],
        ),
        Licence::Expired => (
            "Your free trial has ended".into(),
            format!(
                "Unlock Voltrus Tools {} to connect, simulate and write again.",
                for_price(model)
            )
            .into(),
            vec![
                Part::Note(
                    Icon::Lock,
                    format!("Voltrus Tools is read-only now. {read_only}").into(),
                ),
                Part::Protocols,
            ],
        ),
        Licence::Unlocked => (
            "Voltrus Tools is unlocked".into(),
            "Every tool, for good. Thank you for buying Voltrus Tools.".into(),
            vec![Part::Protocols],
        ),
        Licence::Unrestricted => (
            "Every tool is unlocked".into(),
            "This copy of Voltrus Tools was built from source, so nothing in it is sold.".into(),
            vec![Part::Protocols],
        ),
    }
}

fn about_text(model: &AppModel) -> (SharedString, SharedString, Vec<Part>) {
    let licensing = &model.licensing;
    let edition = if licensing.sells() {
        "Mac App Store"
    } else {
        "built from source"
    };
    let state = match licensing.licence {
        Licence::Unrestricted => "Every tool is unlocked in this build.".to_owned(),
        Licence::NotStarted => format!("The {TRIAL_DAYS}-day free trial hasn't started."),
        Licence::Trial { days_left, .. } => {
            format!("Free trial: {} left.", days(days_left))
        }
        Licence::Expired => {
            "The free trial has ended: Voltrus Tools is read-only until you unlock it.".to_owned()
        }
        Licence::Unlocked => "Unlocked. Thank you for buying Voltrus Tools.".to_owned(),
    };
    let mut software: Vec<SharedString> = vec![
        "Voltrus Tools is free software: you can redistribute and modify it under the GNU \
         General Public License, version 3 or later."
            .into(),
        "Third-party notices and licences are in Voltrus Tools.app › Contents › Resources › \
         Licenses."
            .into(),
    ];
    // Store builds point nowhere else (App Review 3.1.1).
    if !licensing.sells() {
        software.push("Source: github.com/VoltrusID/voltrus-explorers".into());
    }
    software.push("© 2026 Joyo Digitama".into());
    (
        "Voltrus Tools".into(),
        format!("Version {} · {edition}", env!("CARGO_PKG_VERSION")).into(),
        vec![
            Part::Section("Licence", vec![state.into()]),
            Part::Section("Software licence", software),
        ],
    )
}

fn footer(model: &AppModel, sheet: Sheet, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let licensing = &model.licensing;
    let busy = licensing.busy;
    let waiting = |item: Busy, label: String| -> String {
        if busy == Some(item) {
            "Waiting for the App Store…".to_owned()
        } else {
            label
        }
    };
    let unlock_label = waiting(
        Busy::Unlock,
        match price(model) {
            Some(price) => format!("Unlock · {price}"),
            None => "Unlock".to_owned(),
        },
    );
    let unlock = |primary: bool| {
        let button = Button::new("licence-unlock", unlock_label.clone())
            .icon(Icon::LockOpen)
            .disabled(busy.is_some())
            .on_click(cx.listener(|model, _, _, cx| model.buy(Busy::Unlock, cx)));
        if primary { button.primary() } else { button }
    };
    let restore = Button::new(
        "licence-restore",
        waiting(Busy::Restore, "Restore purchases".to_owned()),
    )
    .ghost()
    .icon(Icon::RefreshCw)
    .disabled(busy.is_some())
    .on_click(cx.listener(|model, _, _, cx| model.restore_purchases(cx)));
    let done = Button::new("licence-done", "Done")
        .primary()
        .on_click(cx.listener(|model, _, window, cx| model.close_licence_sheet(window, cx)));
    let sells = licensing.sells();
    let licence = licensing.licence;

    let mut buttons: Vec<AnyElement> = Vec::new();
    let mut fine_print = None;
    match (sheet, licence) {
        (Sheet::Unlock, Licence::NotStarted) => {
            buttons.push(restore.into_any_element());
            buttons.push(div().flex_1().into_any_element());
            buttons.push(unlock(false).into_any_element());
            buttons.push(
                Button::new(
                    "licence-trial",
                    waiting(Busy::Trial, "Start free trial".to_owned()),
                )
                .primary()
                .icon(Icon::Play)
                .disabled(busy.is_some())
                .on_click(cx.listener(|model, _, _, cx| model.buy(Busy::Trial, cx)))
                .into_any_element(),
            );
            fine_print = Some(
                "The trial is a free in-app purchase. The unlock is a one-time purchase, not a \
                 subscription.",
            );
        }
        (Sheet::Unlock, Licence::Trial { .. } | Licence::Expired) => {
            buttons.push(restore.into_any_element());
            buttons.push(div().flex_1().into_any_element());
            buttons.push(unlock(true).into_any_element());
            fine_print =
                Some("A one-time purchase through your Apple Account, not a subscription.");
        }
        (Sheet::About, Licence::NotStarted | Licence::Trial { .. } | Licence::Expired) => {
            buttons.push(restore.into_any_element());
            buttons.push(div().flex_1().into_any_element());
            buttons.push(
                Button::new("licence-open-unlock", "Unlock…")
                    .icon(Icon::LockOpen)
                    .on_click(cx.listener(|model, _, _, cx| {
                        model.open_licence_sheet(Sheet::Unlock, cx);
                    }))
                    .into_any_element(),
            );
            buttons.push(done.into_any_element());
        }
        (Sheet::About, Licence::Unlocked) if sells => {
            buttons.push(restore.into_any_element());
            buttons.push(div().flex_1().into_any_element());
            buttons.push(done.into_any_element());
        }
        _ => {
            buttons.push(div().flex_1().into_any_element());
            buttons.push(done.into_any_element());
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .px(SPACE_8)
        .py(SPACE_6)
        .border_t_1()
        .border_color(theme.border)
        .child(div().flex().items_center().gap(SPACE_4).children(buttons))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .when_some(fine_print, |this, text| {
                    this.child(div().flex_1().child(text))
                })
                .when(fine_print.is_none(), |this| this.child(div().flex_1()))
                .child(hint("↵", primary_hint(sheet, licence), &theme, cx))
                .child(hint("esc", "close", &theme, cx)),
        )
        .into_any_element()
}

/// What Enter does on this sheet.
fn primary_hint(sheet: Sheet, licence: Licence) -> &'static str {
    match (sheet, licence) {
        (Sheet::Unlock, Licence::NotStarted) => "start trial",
        (Sheet::Unlock, Licence::Trial { .. } | Licence::Expired) => "unlock",
        _ => "done",
    }
}

fn hint(keys: &'static str, label: &'static str, theme: &Theme, cx: &App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_2)
        .text_color(theme.fg_dim)
        .child(kbd(keys, cx))
        .child(label)
        .into_any_element()
}
