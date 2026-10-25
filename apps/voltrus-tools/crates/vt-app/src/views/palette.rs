//! The command palette and theme picker overlay.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, MouseButton, div, hsla, px, uniform_list};
use vt_ui::components::kbd;
use vt_ui::theme::*;

use crate::actions::PALETTE;
use crate::model::palette::{self, Command, Mode};
use crate::model::{AppModel, Overlay};

pub fn render(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let Overlay::Palette(state) = &model.overlay else {
        return div().into_any_element();
    };
    let mode = state.mode;
    let count = palette::entries(model, mode, &state.query(cx)).len();
    let title = match mode {
        Mode::Commands => "Commands",
        Mode::Themes => "Themes",
    };

    let list =
        uniform_list(
            "palette-entries",
            count,
            cx.processor(
                move |model: &mut AppModel, range: std::ops::Range<usize>, _, cx| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let Overlay::Palette(state) = &model.overlay else {
                        return Vec::new();
                    };
                    let selected = state.selected;
                    let entries = palette::entries(model, mode, &state.query(cx));
                    let catalog = model.catalog.clone();
                    entries
                        .into_iter()
                        .enumerate()
                        .skip(range.start)
                        .take(range.len())
                        .map(|(index, entry)| {
                            let active = index == selected;
                            let swatch = match &entry.command {
                                Command::Theme(name) => catalog.get(name),
                                _ => None,
                            };
                            div()
                                .id(("palette-entry", index))
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(36.0))
                                .w_full()
                                .px(SPACE_5)
                                .rounded(RADIUS_MD)
                                .cursor_pointer()
                                .when(active, |this| this.bg(theme.selected))
                                .when(!active, |this| this.hover(|style| style.bg(theme.hover)))
                                .on_click(cx.listener(move |model, _, window, cx| {
                                    model.run_palette_entry(index, window, cx);
                                }))
                                .map(|this| match swatch {
                                    Some(preview) => {
                                        this.child(
                                            div()
                                                .flex()
                                                .flex_none()
                                                .size(px(22.0))
                                                .rounded(RADIUS_SM)
                                                .overflow_hidden()
                                                .border_1()
                                                .border_color(theme.border_strong)
                                                .bg(preview.bg)
                                                .items_end()
                                                .children(preview.series.iter().take(4).map(
                                                    |color| div().flex_1().h(px(8.0)).bg(*color),
                                                )),
                                        )
                                    }
                                    None => {
                                        this.child(vt_ui::icon(entry.icon, ICON_MD, theme.fg_muted))
                                    }
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(entry.title.clone()),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .max_w(px(240.0))
                                        .truncate()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child(entry.detail.clone()),
                                )
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(state.scroll.clone())
        .px(SPACE_3)
        .size_full();

    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .justify_center()
        .pt(px(96.0))
        .bg(hsla(0.0, 0.0, 0.0, if theme.dark { 0.45 } else { 0.2 }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|model, _, window, cx| model.close_overlay(window, cx)),
        )
        .child(
            div()
                .id("palette")
                .key_context(PALETTE)
                .flex()
                .flex_col()
                .w(px(600.0))
                .h(px(470.0))
                .rounded(RADIUS_XL)
                .bg(theme.elevated)
                .border_1()
                .border_color(theme.border_strong)
                .shadow(theme.shadow(Elevation::Modal))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_4)
                        .p(SPACE_6)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(vt_ui::icon(vt_ui::Icon::Search, ICON_LG, theme.fg_dim))
                        .child(div().flex_1().child(state.input.clone()))
                        .child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(title),
                        ),
                )
                .child(div().flex_1().min_h_0().py(SPACE_3).child(list))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_6)
                        .px(SPACE_6)
                        .h(px(34.0))
                        .border_t_1()
                        .border_color(theme.border)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_2)
                                .child(kbd("↑↓", cx))
                                .child("Move"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_2)
                                .child(kbd("↵", cx))
                                .child(if mode == Mode::Themes {
                                    "Use theme"
                                } else {
                                    "Run"
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_2)
                                .child(kbd("esc", cx))
                                .child("Close"),
                        )
                        .child(div().flex_1())
                        .child(format!("{count} results")),
                ),
        )
        .into_any_element()
}
