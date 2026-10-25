//! The bottom bar: what is running, and the theme.

use gpui::prelude::*;
use gpui::{Context, div};
use vt_ui::Icon;
use vt_ui::components::status_dot;
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::palette::Mode;

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let live = model
        .live
        .values()
        .filter(|live| live.state.is_live())
        .count();
    let trying = model
        .live
        .values()
        .filter(|live| live.state.is_running() && !live.state.is_live())
        .count();
    let frames: u64 = model.live.values().map(|live| live.stats.frames).sum();
    let hover = theme.hover;

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(STATUS_BAR_HEIGHT)
        .px(SPACE_6)
        .bg(theme.chrome)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_muted)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(status_dot(if live > 0 {
                    theme.success
                } else {
                    theme.fg_dim
                }))
                .child(format!(
                    "{live} live{}",
                    if trying > 0 {
                        format!(" · {trying} connecting")
                    } else {
                        String::new()
                    }
                )),
        )
        .child(format!("{} saved", model.workspace.connections.len()))
        .when(frames > 0, |this| {
            this.child(div().font_family(MONO).child(format!("{frames} frames")))
        })
        .child(div().flex_1())
        .child(
            div()
                .id("theme")
                .flex()
                .items_center()
                .gap(SPACE_2)
                .px(SPACE_3)
                .rounded(RADIUS_SM)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(|model, _, window, cx| {
                    model.open_palette(Mode::Themes, window, cx);
                }))
                .child(vt_ui::icon(Icon::Palette, ICON_SM, theme.fg_dim))
                .child(theme.name.clone()),
        )
        .child(concat!("Voltrus Tools ", env!("CARGO_PKG_VERSION")))
}
