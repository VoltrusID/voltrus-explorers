//! Saved connections, grouped into devices and simulators.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use vt_app_core::Connection;
use vt_protocol::Role;
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, kbd, monogram, section_label, status_dot};
use vt_ui::theme::*;

use super::{resize_handle, state_color};
use crate::model::editor::EditorKind;
use crate::model::palette::Mode;
use crate::model::{AppModel, Drag};

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let width = model.workspace.ui.sidebar_width;
    let top_pad = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_4
    };
    let devices: Vec<&Connection> = model
        .workspace
        .connections
        .iter()
        .filter(|connection| connection.spec.role() == Role::Client)
        .collect();
    let simulators: Vec<&Connection> = model
        .workspace
        .connections
        .iter()
        .filter(|connection| connection.spec.role() == Role::Simulator)
        .collect();
    let empty = model.workspace.connections.is_empty();

    let device_rows: Vec<AnyElement> = devices.iter().map(|c| row(model, c, cx)).collect();
    let simulator_rows: Vec<AnyElement> = simulators.iter().map(|c| row(model, c, cx)).collect();

    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .bg(theme.chrome)
        .border_r_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .h(TITLE_BAR_HEIGHT)
                .pl(top_pad)
                .pr(SPACE_4)
                .child(
                    IconButton::new("hide-sidebar", Icon::PanelLeft)
                        .tooltip("Hide sidebar  ⌘B")
                        .on_click(cx.listener(|model, _, _, cx| model.toggle_sidebar(cx))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .px(SPACE_6)
                .pb(SPACE_6)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(28.0))
                        .rounded(RADIUS_LG)
                        .bg(theme.accent_soft)
                        .child(vt_ui::icon(Icon::Activity, ICON_LG, theme.accent)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(FONT_EMPHASIS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Voltrus Tools"),
                        )
                        .child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child("Industrial protocol suite"),
                        ),
                ),
        )
        .child(
            div()
                .id("connections")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(SPACE_2)
                .child(section(
                    "Devices",
                    "new-device",
                    "New connection  ⌘N",
                    EditorKind::Master,
                    cx,
                ))
                .children(device_rows)
                .when(devices.is_empty() && !empty, |this| {
                    this.child(hint("No devices yet. ⌘N adds one.", cx))
                })
                .child(div().h(SPACE_6))
                .child(section(
                    "Simulators",
                    "new-simulator",
                    "New simulator  ⇧⌘N",
                    EditorKind::Slave,
                    cx,
                ))
                .children(simulator_rows)
                .when(simulators.is_empty() && !empty, |this| {
                    this.child(hint("Simulators serve a device on this computer.", cx))
                })
                .when(empty, |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_4)
                            .m(SPACE_6)
                            .p(SPACE_6)
                            .rounded(RADIUS_LG)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.surface)
                            .child(
                                div()
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child("Nothing saved yet. Try a simulated plant first."),
                            )
                            .child(
                                Button::new("sidebar-demo", "Start demo plant")
                                    .icon(Icon::Zap)
                                    .primary()
                                    .compact()
                                    .on_click(cx.listener(|model, _, _, cx| {
                                        model.start_demo_plant(cx);
                                    })),
                            ),
                    )
                }),
        )
        .child(
            div()
                .id("palette-hint")
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .mx(SPACE_4)
                .mb(SPACE_4)
                .px(SPACE_5)
                .h(CONTROL_LG)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(|model, _, window, cx| {
                    model.open_palette(Mode::Commands, window, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
                        .child("Search and commands"),
                )
                .child(kbd(
                    if cfg!(target_os = "macos") {
                        "⌘K"
                    } else {
                        "Ctrl K"
                    },
                    cx,
                )),
        )
        .child(
            resize_handle(
                "sidebar-resize",
                cx.listener(move |model, event: &gpui::MouseDownEvent, _, _| {
                    model.begin_drag(Drag::Sidebar {
                        start_x: event.position.x,
                        start_width: width,
                    });
                }),
            )
            .right(px(-3.0)),
        )
}

fn section(
    title: &'static str,
    id: &'static str,
    tooltip: &'static str,
    kind: EditorKind,
    cx: &mut Context<AppModel>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(SPACE_6)
        .pr(SPACE_4)
        .h(px(24.0))
        .child(section_label(title, cx))
        .child(
            IconButton::new(id, Icon::Plus)
                .size(px(20.0))
                .tooltip(tooltip)
                .on_click(cx.listener(move |model, _, window, cx| {
                    model.open_editor(kind, None, window, cx);
                })),
        )
}

fn hint(text: &'static str, cx: &Context<AppModel>) -> impl IntoElement {
    div()
        .px(SPACE_6)
        .py(SPACE_2)
        .text_size(FONT_SMALL)
        .text_color(theme(cx).fg_dim)
        .child(text)
}

fn row(model: &AppModel, connection: &Connection, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let id = connection.id;
    let live = model.live.get(&id);
    let running = live.is_some_and(|live| live.session.is_some());
    let focused = model.focused_connection() == Some(id);
    let color = theme.series(usize::try_from(id.0).unwrap_or(0));
    let simulator = connection.spec.role() == Role::Simulator;
    let group: SharedString = format!("connection-{}", id.0).into();
    let state = live.map(|live| (state_color(&live.state, &theme), live.state.label()));

    div()
        .id(ElementId::NamedInteger("connection".into(), id.0))
        .group(group.clone())
        .flex()
        .items_center()
        .gap(SPACE_4)
        .mx(SPACE_3)
        .px(SPACE_3)
        .h(px(44.0))
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .when(focused, |this| this.bg(theme.selected))
        .when(!focused, |this| this.hover(|style| style.bg(theme.hover)))
        .on_click(cx.listener(move |model, _, _, cx| model.open_connection(id, cx)))
        .child(monogram(
            connection.spec.protocol().monogram(),
            color,
            px(28.0),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_BODY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.fg)
                        .child(connection.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(connection.spec.address()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_2)
                .child(
                    div()
                        .flex()
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("edit".into(), id.0),
                                Icon::Pencil,
                            )
                            .size(px(20.0))
                            .tooltip("Edit")
                            .on_click(cx.listener(
                                move |model, _, window, cx| {
                                    cx.stop_propagation();
                                    model.open_editor(EditorKind::Master, Some(id), window, cx);
                                },
                            )),
                        )
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("toggle".into(), id.0),
                                if running { Icon::Square } else { Icon::Play },
                            )
                            .size(px(20.0))
                            .tooltip(match (running, simulator) {
                                (true, true) => "Stop simulator",
                                (true, false) => "Disconnect",
                                (false, true) => "Start simulator",
                                (false, false) => "Connect",
                            })
                            .on_click(cx.listener(
                                move |model, _, _, cx| {
                                    cx.stop_propagation();
                                    if model.is_running(id) {
                                        model.disconnect(id, cx);
                                    } else {
                                        model.launch(id, cx);
                                    }
                                },
                            )),
                        ),
                )
                .when_some(state.filter(|_| running), |this, (color, _)| {
                    this.child(status_dot(color))
                }),
        )
        .into_any_element()
}
