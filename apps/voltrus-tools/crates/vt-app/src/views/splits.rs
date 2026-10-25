//! The active tab's panes, laid out by its split tree.

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, MouseButton, MouseDownEvent, Window, canvas, div, px, relative,
};
use vt_app_core::{Axis, Branch, Layout, PaneId, TabId};
use vt_ui::theme::*;

use super::pane;
use crate::model::{AppModel, Drag};

pub fn render(model: &AppModel, window: &mut Window, cx: &mut Context<AppModel>) -> AnyElement {
    let Some(tab) = model.workspace.active_tab() else {
        return div().into_any_element();
    };
    let split = !matches!(tab.layout, Layout::Leaf(_));
    let layout = tab.layout.clone();
    node(
        model,
        tab.id,
        &layout,
        &mut Vec::new(),
        tab.focused,
        split,
        window,
        cx,
    )
}

#[allow(clippy::too_many_arguments)]
fn node(
    model: &AppModel,
    tab: TabId,
    layout: &Layout,
    path: &mut Vec<Branch>,
    focused: PaneId,
    split: bool,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    match layout {
        Layout::Leaf(pane) => pane::render(model, *pane, split && *pane == focused, window, cx),
        Layout::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let theme = theme(cx).clone();
            let horizontal = *axis == Axis::Horizontal;
            path.push(Branch::First);
            let first = node(model, tab, first, path, focused, split, window, cx);
            path.pop();
            path.push(Branch::Second);
            let second = node(model, tab, second, path, focused, split, window, cx);
            path.pop();

            let key = (tab, path.clone());
            let bounds = model.split_bounds.clone();
            let record_key = key.clone();
            let drag_path = path.clone();
            let axis = *axis;
            let ratio = *ratio;
            let divider_id = ElementId::Name(format!("split-{}-{:?}", tab.0, path).into());

            div()
                .relative()
                .size_full()
                .flex()
                .when(!horizontal, |this| this.flex_col())
                .child(
                    canvas(
                        move |area, _, _| {
                            bounds.borrow_mut().insert(record_key, area);
                        },
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(
                    div()
                        .flex_none()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .when(horizontal, |this| this.w(relative(ratio)).h_full())
                        .when(!horizontal, |this| this.h(relative(ratio)).w_full())
                        .child(first),
                )
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .bg(theme.border)
                        .when(horizontal, |this| this.w(px(1.0)).h_full())
                        .when(!horizontal, |this| this.h(px(1.0)).w_full())
                        .child(
                            div()
                                .id(divider_id)
                                .absolute()
                                .when(horizontal, |this| {
                                    this.top_0()
                                        .bottom_0()
                                        .left(px(-3.0))
                                        .w(px(7.0))
                                        .cursor(gpui::CursorStyle::ResizeLeftRight)
                                })
                                .when(!horizontal, |this| {
                                    this.left_0()
                                        .right_0()
                                        .top(px(-3.0))
                                        .h(px(7.0))
                                        .cursor(gpui::CursorStyle::ResizeUpDown)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |model, event: &MouseDownEvent, _, _| {
                                        let Some(area) =
                                            model.split_bounds.borrow().get(&key).copied()
                                        else {
                                            return;
                                        };
                                        model.begin_drag(Drag::Split {
                                            tab,
                                            path: drag_path.clone(),
                                            axis,
                                            start: event.position,
                                            start_ratio: ratio,
                                            extent: if axis == Axis::Horizontal {
                                                area.size.width
                                            } else {
                                                area.size.height
                                            },
                                        });
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(second),
                )
                .into_any_element()
        }
    }
}
