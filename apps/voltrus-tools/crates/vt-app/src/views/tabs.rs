//! The tab strip, sharing the title bar row.

use gpui::prelude::*;
use gpui::{Context, ElementId, FontWeight, SharedString, div, px};
use vt_app_core::Axis;
use vt_ui::Icon;
use vt_ui::components::{IconButton, status_dot};
use vt_ui::theme::*;

use super::state_color;
use crate::model::AppModel;
use crate::model::palette::Mode;

/// Tabs share one title width, like a browser's: text only truncates inside a
/// known width.
const TAB_TITLE: f32 = 150.0;

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let sidebar_open = model.workspace.ui.sidebar_open;
    let inspector_open = model.workspace.ui.inspector_open;
    let left = if cfg!(target_os = "macos") && !sidebar_open {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_4
    };
    let active = model.workspace.active_tab;

    let tabs = model.workspace.tabs.iter().map(|tab| {
        let selected = Some(tab.id) == active;
        // The focused pane names the tab; a badge counts the other panes.
        let panes = tab.layout.panes();
        let title: SharedString = model
            .workspace
            .pane(tab.focused)
            .and_then(|pane| model.workspace.connection(pane.connection))
            .map(|connection| connection.name.clone())
            .unwrap_or_default()
            .into();
        let others = panes.len().saturating_sub(1);
        let color = model
            .workspace
            .pane(tab.focused)
            .and_then(|pane| model.live.get(&pane.connection))
            .map_or(theme.fg_dim, |live| state_color(&live.state, &theme));
        let id = tab.id;
        let group: SharedString = format!("tab-{}", id.0).into();
        div()
            .id(ElementId::NamedInteger("tab".into(), id.0))
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_3)
            .h(px(28.0))
            .pl(SPACE_5)
            .pr(SPACE_2)
            .rounded(RADIUS_MD)
            .text_size(FONT_SMALL)
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(theme.bg)
                    .border_1()
                    .border_color(theme.border_strong)
                    .text_color(theme.fg)
            })
            .when(!selected, |this| {
                this.text_color(theme.fg_muted)
                    .hover(|style| style.bg(theme.hover))
            })
            .on_click(cx.listener(move |model, _, _, cx| model.select_tab(id, cx)))
            .child(status_dot(color))
            .child(div().flex_none().w(px(TAB_TITLE)).truncate().child(title))
            .when(others > 0, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px(SPACE_2)
                        .rounded(RADIUS_SM)
                        .bg(theme.hover)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_muted)
                        .child(format!("+{others}")),
                )
            })
            .child(
                div()
                    .flex_none()
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .when(selected, |this| this.visible())
                    .child(
                        IconButton::new(ElementId::NamedInteger("close-tab".into(), id.0), Icon::X)
                            .size(px(18.0))
                            .tooltip("Close tab")
                            .on_click(cx.listener(move |model, _, _, cx| {
                                cx.stop_propagation();
                                model.close_tab(id, cx);
                            })),
                    ),
            )
    });

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_2)
        .h(TITLE_BAR_HEIGHT)
        .pl(left)
        .pr(SPACE_4)
        .bg(theme.chrome)
        .border_b_1()
        .border_color(theme.border)
        .when(!sidebar_open, |this| {
            this.child(
                IconButton::new("show-sidebar", Icon::PanelLeft)
                    .tooltip("Show sidebar  ⌘B")
                    .on_click(cx.listener(|model, _, _, cx| model.toggle_sidebar(cx))),
            )
        })
        .child(
            div()
                .id("tab-strip")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_2)
                .overflow_x_scroll()
                .children(tabs)
                .child(
                    IconButton::new("open", Icon::Plus)
                        .tooltip("Open a connection  ⌘K")
                        .on_click(cx.listener(|model, _, window, cx| {
                            model.open_palette(Mode::Commands, window, cx);
                        })),
                ),
        )
        .when(active.is_some(), |this| {
            this.child(
                IconButton::new("split-right", Icon::Columns)
                    .tooltip("Split right  ⌘D")
                    .on_click(cx.listener(|model, _, _, cx| model.split(Axis::Horizontal, cx))),
            )
            .child(
                IconButton::new("split-down", Icon::Rows)
                    .tooltip("Split down  ⇧⌘D")
                    .on_click(cx.listener(|model, _, _, cx| model.split(Axis::Vertical, cx))),
            )
            .child(
                IconButton::new("toggle-inspector", Icon::PanelRight)
                    .active(inspector_open)
                    .tooltip("Inspector  ⌘I")
                    .on_click(cx.listener(|model, _, _, cx| model.toggle_inspector(cx))),
            )
        })
}
