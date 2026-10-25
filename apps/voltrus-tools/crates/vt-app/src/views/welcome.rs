//! What the window shows with no tab open.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, div, px};
use vt_protocol::{Protocol, Role};
use vt_ui::Icon;
use vt_ui::components::{Button, brand_mark, kbd, monogram};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::editor::EditorKind;

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let has_connections = !model.workspace.connections.is_empty();
    let shortcut = |keys: &str| -> SharedString {
        if cfg!(target_os = "macos") {
            keys.replace("Mod", "⌘").into()
        } else {
            keys.replace("Mod", "Ctrl ").into()
        }
    };

    let card = |icon: Icon, title: &'static str, body: &'static str| {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .flex_1()
            .min_w_0()
            .p(SPACE_7)
            .rounded(RADIUS_LG)
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(vt_ui::icon(icon, ICON_LG, theme.accent))
            .child(
                div()
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(body),
            )
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .px(SPACE_10)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_9)
                .w_full()
                .max_w(px(640.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_4)
                        .child(div().mb(SPACE_4).child(brand_mark(px(56.0))))
                        .child(
                            div()
                                .text_size(FONT_DISPLAY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if has_connections {
                                    "Pick up where you left off"
                                } else {
                                    "Talk to your plant"
                                }),
                        )
                        .child(
                            div()
                                .text_size(FONT_EMPHASIS)
                                .text_color(theme.fg_muted)
                                .child(
                                    "Poll devices, watch live trends, read the raw traffic and \
                                     simulate equipment, all in one window.",
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(
                            Button::new("welcome-demo", "Start demo plant")
                                .icon(Icon::Zap)
                                .primary()
                                .on_click(cx.listener(|model, _, _, cx| {
                                    model.start_demo_plant(cx);
                                })),
                        )
                        .child(
                            Button::new("welcome-new", "New connection")
                                .icon(Icon::Plus)
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.open_editor(EditorKind::Master, None, window, cx);
                                })),
                        )
                        .child(
                            Button::new("welcome-sim", "New simulator")
                                .icon(Icon::Server)
                                .ghost()
                                .on_click(cx.listener(|model, _, window, cx| {
                                    model.open_editor(EditorKind::Slave, None, window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(card(
                            Icon::Table,
                            "Live points",
                            "Every register with its raw words, quality and a sparkline of the last minute.",
                        ))
                        .child(card(
                            Icon::ChartLine,
                            "GPU trends",
                            "Multi-series charts that scroll at your display's refresh rate.",
                        ))
                        .child(card(
                            Icon::Columns,
                            "Side by side",
                            "Split panes to watch a device and its simulator, or two views of one.",
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child("PROTOCOLS"),
                        )
                        .child(div().flex().gap(SPACE_4).children(Protocol::ALL.iter().map(
                            |protocol| {
                                let protocol = *protocol;
                                div()
                                    .id(SharedString::from(format!("welcome-{}", protocol.name())))
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap(SPACE_4)
                                    .p(SPACE_4)
                                    .rounded(RADIUS_MD)
                                    .cursor_pointer()
                                    .hover(|style| style.bg(theme.hover))
                                    .on_click(cx.listener(move |model, _, window, cx| {
                                        if let Some(kind) =
                                            EditorKind::for_protocol(protocol, Role::Client)
                                        {
                                            model.open_editor(kind, None, window, cx);
                                        }
                                    }))
                                    .child(monogram(protocol.monogram(), theme.accent, px(26.0)))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(protocol.name()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(FONT_SMALL)
                                                    .text_color(theme.fg_muted)
                                                    .child(protocol.summary()),
                                            ),
                                    )
                            },
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_6)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(div().flex().items_center().gap(SPACE_3).child(kbd(shortcut("ModK"), cx)).child("Commands"))
                        .child(div().flex().items_center().gap(SPACE_3).child(kbd(shortcut("ModN"), cx)).child("New connection"))
                        .child(div().flex().items_center().gap(SPACE_3).child(kbd(shortcut("ModD"), cx)).child("Split")),
                ),
        )
        .into_any_element()
}
