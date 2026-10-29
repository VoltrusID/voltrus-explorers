//! Views are functions over `&AppModel`; this is the window's root.
//!
//! The sidebar, tab strip, inspector, status bar and each pane are drawn by
//! their own [`cached::CachedView`]s, so a pane that animates (a live trend)
//! redraws itself and this thin shell, not the whole window.

mod ble;
pub mod cached;
mod cip;
mod cnc;
mod device;
mod dnp3;
mod editor;
pub mod inspector;
mod licence;
mod log;
mod nats;
mod new_menu;
mod opcua;
mod palette;
pub mod pane;
mod points;
mod publish;
mod s7;
pub mod sidebar;
mod splits;
pub mod status_bar;
pub mod tabs;
mod terminal;
mod traffic;
mod trend;
mod welcome;

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    AnyView, Context, CursorStyle, MouseButton, MouseMoveEvent, MouseUpEvent, Window, div, px,
};
use vt_app_core::{Axis, PaneView};
use vt_protocol::Role;
use vt_ui::Icon;
use vt_ui::theme::*;

use crate::actions::*;
use crate::model::palette::Mode;
use crate::model::{AppModel, Drag, Overlay};

impl Render for AppModel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_pane_views(cx);
        self.sync_licence_sheet(window, cx);
        self.sync_serial_consoles(window, cx);
        let theme = theme(cx).clone();
        let ui = self.workspace.ui.clone();
        let has_tabs = self.workspace.active_tab().is_some();
        let cursor = match &self.drag {
            None => None,
            Some(Drag::Split {
                axis: Axis::Vertical,
                ..
            }) => Some(CursorStyle::ResizeUpDown),
            Some(_) => Some(CursorStyle::ResizeLeftRight),
        };
        let chrome = &self.chrome;

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                AnyView::from(chrome.tabs.clone()).cached(
                    div()
                        .flex_none()
                        .w_full()
                        .h(TITLE_BAR_HEIGHT)
                        .style()
                        .clone(),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(has_tabs, |this| this.child(splits::render(self, cx)))
                            .when(!has_tabs, |this| this.child(welcome::render(self, cx))),
                    )
                    .when(ui.inspector_open && has_tabs, |this| {
                        this.child(
                            AnyView::from(chrome.inspector.clone()).cached(
                                div()
                                    .flex_none()
                                    .w(px(ui.inspector_width))
                                    .h_full()
                                    .style()
                                    .clone(),
                            ),
                        )
                    }),
            );
        div()
            .id("root")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(SANS)
            .text_size(FONT_BODY)
            .track_focus(&self.focus)
            .key_context(WORKSPACE)
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .on_action(cx.listener(|model, _: &ToggleCommandPalette, window, cx| {
                if matches!(model.overlay, Overlay::Palette(_)) {
                    model.close_overlay(window, cx);
                } else {
                    model.open_palette(Mode::Commands, window, cx);
                }
            }))
            .on_action(cx.listener(|model, _: &ChooseTheme, window, cx| {
                model.open_palette(Mode::Themes, window, cx);
            }))
            .on_action(cx.listener(|model, _: &NewConnection, _, cx| {
                model.open_new_menu(Role::Client, model.new_menu_anchor(), cx);
            }))
            .on_action(cx.listener(|model, _: &NewSimulator, _, cx| {
                model.open_new_menu(Role::Simulator, model.new_menu_anchor(), cx);
            }))
            .on_action(cx.listener(|model, _: &ToggleSidebar, _, cx| model.toggle_sidebar(cx)))
            .on_action(cx.listener(|model, _: &ToggleInspector, _, cx| model.toggle_inspector(cx)))
            .on_action(
                cx.listener(|model, _: &SplitRight, _, cx| model.split(Axis::Horizontal, cx)),
            )
            .on_action(cx.listener(|model, _: &SplitDown, _, cx| model.split(Axis::Vertical, cx)))
            .on_action(cx.listener(|model, _: &ClosePane, _, cx| model.close_pane(cx)))
            .on_action(cx.listener(|model, _: &NextTab, _, cx| model.cycle_tab(1, cx)))
            .on_action(cx.listener(|model, _: &PreviousTab, _, cx| model.cycle_tab(-1, cx)))
            .on_action(cx.listener(|model, _: &ToggleSession, _, cx| {
                if let Some(connection) = model.focused_connection() {
                    model.toggle_session(connection, cx);
                }
            }))
            .on_action(
                cx.listener(|model, _: &ShowPoints, _, cx| model.set_view(PaneView::Points, cx)),
            )
            .on_action(
                cx.listener(|model, _: &ShowTrend, _, cx| model.set_view(PaneView::Trend, cx)),
            )
            .on_action(
                cx.listener(|model, _: &ShowTraffic, _, cx| model.set_view(PaneView::Traffic, cx)),
            )
            .on_action(cx.listener(|model, _: &ShowLog, _, cx| model.set_view(PaneView::Log, cx)))
            .on_action(cx.listener(|model, _: &SelectNextPoint, _, cx| model.step_selection(1, cx)))
            .on_action(
                cx.listener(|model, _: &SelectPreviousPoint, _, cx| model.step_selection(-1, cx)),
            )
            .on_action(
                cx.listener(|model, _: &Dismiss, window, cx| model.close_overlay(window, cx)),
            )
            .on_action(cx.listener(|model, _: &PaletteUp, _, cx| model.move_palette(-1, cx)))
            .on_action(cx.listener(|model, _: &PaletteDown, _, cx| model.move_palette(1, cx)))
            .on_action(cx.listener(|model, _: &PaletteConfirm, window, cx| {
                model.confirm_palette(window, cx)
            }))
            .on_action(
                cx.listener(|model, _: &crate::model::opcua::FocusBrowser, window, cx| {
                    model.focus_browser(window, cx);
                }),
            )
            .on_action(cx.listener(|model, _: &FocusSend, window, cx| {
                if let Some(connection) = model.focused_connection() {
                    model.focus_serial_send(connection, window, cx);
                }
            }))
            .on_action(cx.listener(|model, _: &ShowDevice, _, cx| model.show_device_view(cx)))
            .on_action(cx.listener(|model, _: &RefreshDevice, _, cx| model.refresh_device(cx)))
            .on_mouse_move(cx.listener(|model, event: &MouseMoveEvent, _, cx| {
                if model.drag.is_some() {
                    if event.pressed_button == Some(MouseButton::Left) {
                        model.drag_to(event.position, cx);
                    } else {
                        model.end_drag(cx);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|model, _: &MouseUpEvent, _, cx| {
                    model.end_drag(cx);
                }),
            )
            .when_some(cursor, |this, cursor| this.cursor(cursor))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(ui.sidebar_open, |this| {
                        this.child(
                            AnyView::from(chrome.sidebar.clone()).cached(
                                div()
                                    .flex_none()
                                    .w(px(ui.sidebar_width))
                                    .h_full()
                                    .style()
                                    .clone(),
                            ),
                        )
                    })
                    .child(content),
            )
            .child(
                AnyView::from(chrome.status_bar.clone()).cached(
                    div()
                        .flex_none()
                        .w_full()
                        .h(STATUS_BAR_HEIGHT)
                        .style()
                        .clone(),
                ),
            )
            .map(|this| match &self.overlay {
                Overlay::None => this,
                Overlay::Palette(_) => this.child(palette::render(self, cx)),
                Overlay::Editor(_) => this.child(editor::render(self, cx)),
                Overlay::NewMenu { .. } => this.child(new_menu::render(self, cx)),
            })
            // Above every overlay: a refused command can come from any of them.
            .child(licence::render(self, cx))
            .when_some(self.toast.as_ref(), |this, toast| {
                let color = if toast.error {
                    theme.danger
                } else {
                    theme.success
                };
                this.child(
                    div()
                        .absolute()
                        .bottom(px(40.0))
                        .right(SPACE_7)
                        .max_w(px(420.0))
                        .flex()
                        .items_start()
                        .gap(SPACE_4)
                        .px(SPACE_6)
                        .py(SPACE_5)
                        .rounded(RADIUS_LG)
                        .bg(theme.elevated)
                        .border_1()
                        .border_color(color.alpha(0.4))
                        .shadow(theme.shadow(Elevation::Popover))
                        .child(vt_ui::icon(
                            if toast.error {
                                Icon::CircleAlert
                            } else {
                                Icon::CircleCheck
                            },
                            ICON_LG,
                            color,
                        ))
                        .child(div().text_size(FONT_BODY).child(toast.message.clone())),
                )
            })
    }
}

/// Wall-clock now in milliseconds, the time base of every chart.
pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// `hh:mm:ss.mmm` of a Unix-epoch millisecond time, in UTC.
pub(crate) fn clock(at_ms: u64) -> String {
    let millis = at_ms % 1000;
    let seconds = at_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{millis:03}",
        (seconds / 3600) % 24,
        (seconds / 60) % 60,
        seconds % 60
    )
}

/// Resize handle for the edge of a side panel; the caller places it.
pub(crate) fn resize_handle(
    id: &'static str,
    on_down: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(6.0))
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(MouseButton::Left, on_down)
}

/// Keep a growing list on its newest row, but only while the user is already
/// there: scrolling back to read stays put until they return to the end.
pub(crate) fn follow_end(scroll: &gpui::UniformListScrollHandle, count: usize, row: f32) {
    if count == 0 {
        return;
    }
    let (offset, max) = {
        let state = scroll.0.borrow();
        (
            f32::from(state.base_handle.offset().y),
            f32::from(state.base_handle.max_offset().height),
        )
    };
    if at_end(offset, max, row) {
        scroll.scroll_to_item(count - 1, gpui::ScrollStrategy::Bottom);
    }
}

/// Whether a list scrolled to `offset` (zero or negative, gpui's sign) out of
/// `max` is within two rows of its end. A list that doesn't scroll yet is.
fn at_end(offset: f32, max: f32, row: f32) -> bool {
    let slack = 2.0 * row;
    max <= slack || -offset >= max - slack
}

/// The colour of a session state: live, trying, failed or idle.
pub(crate) fn state_color(state: &vt_protocol::SessionState, theme: &Theme) -> gpui::Hsla {
    use vt_protocol::SessionState;
    match state {
        SessionState::Connected | SessionState::Listening { .. } => theme.success,
        SessionState::Connecting | SessionState::Reconnecting { .. } => theme.warning,
        SessionState::Closed { error: Some(_) } => theme.danger,
        SessionState::Closed { error: None } => theme.fg_dim,
    }
}

#[cfg(test)]
mod tests {
    use super::at_end;

    #[test]
    fn lists_follow_only_from_their_end() {
        // Nothing to scroll yet.
        assert!(at_end(0.0, 0.0, 24.0));
        // At the end, or a row short of it after new rows arrived.
        assert!(at_end(-480.0, 480.0, 24.0));
        assert!(at_end(-456.0, 480.0, 24.0));
        // Scrolled back to read: stay put.
        assert!(!at_end(-100.0, 480.0, 24.0));
        assert!(!at_end(0.0, 480.0, 24.0));
    }
}
