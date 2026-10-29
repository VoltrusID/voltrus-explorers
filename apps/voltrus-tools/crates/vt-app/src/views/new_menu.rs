//! The "+" menu: pick the protocol of a new client or simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, MouseButton, anchored, div, point, px};
use vt_protocol::{Protocol, Role};
use vt_ui::components::{monogram, section_label};
use vt_ui::theme::*;

use super::cnc::beta_badge;
use crate::model::cnc::is_beta;
use crate::model::editor::EditorKind;
use crate::model::{AppModel, Overlay};

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let Overlay::NewMenu { role, at } = model.overlay else {
        return div().into_any_element();
    };
    let items = Protocol::ALL
        .iter()
        .copied()
        .filter(|protocol| EditorKind::for_protocol(*protocol, role).is_some())
        .enumerate()
        .map(|(index, protocol)| {
            div()
                .id(ElementId::NamedInteger("new-menu".into(), index as u64))
                .flex()
                .items_center()
                .gap(SPACE_5)
                .px(SPACE_4)
                .py(SPACE_3)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(move |model, _, window, cx| {
                    model.new_connection(protocol, role, window, cx);
                }))
                .child(monogram(protocol.monogram(), theme.accent, px(28.0)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .child(div().font_weight(FontWeight::MEDIUM).child(match role {
                                    Role::Client => format!("{} connection", protocol.name()),
                                    Role::Simulator => format!("{} simulator", protocol.name()),
                                }))
                                .when(role == Role::Client && is_beta(protocol), |this| {
                                    this.child(beta_badge(("new-menu-beta", index), protocol, cx))
                                }),
                        )
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(protocol.summary()),
                        ),
                )
        })
        .collect::<Vec<_>>();

    div()
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|model, _, window, cx| model.close_overlay(window, cx)),
        )
        .child(
            anchored()
                .position(at)
                .offset(point(px(0.0), SPACE_3))
                .snap_to_window_with_margin(SPACE_4)
                .child(
                    div()
                        .id("new-menu")
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .w(px(340.0))
                        .p(SPACE_3)
                        .rounded(RADIUS_LG)
                        .bg(theme.elevated)
                        .border_1()
                        .border_color(theme.border_strong)
                        .shadow(theme.shadow(Elevation::Popover))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().px(SPACE_4).py(SPACE_2).child(section_label(
                            match role {
                                Role::Client => "New connection",
                                Role::Simulator => "New simulator",
                            },
                            cx,
                        )))
                        .children(items),
                ),
        )
        .into_any_element()
}
