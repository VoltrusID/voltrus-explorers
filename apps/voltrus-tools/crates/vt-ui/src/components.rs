//! Small stateless building blocks. Every colour comes from the active theme,
//! except the Voltrus logo, which keeps its brand colours in every theme.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, Pixels, SharedString, Window, div,
    px, rgb, transparent_black,
};

use crate::icon::{Icon, icon};
use crate::theme::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A text button, optionally with a leading icon.
#[derive(IntoElement)]
#[must_use]
pub struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    kind: ButtonKind,
    compact: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            kind: ButtonKind::Secondary,
            compact: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(ButtonKind::Primary)
    }

    pub fn ghost(self) -> Self {
        self.kind(ButtonKind::Ghost)
    }

    pub fn danger(self) -> Self {
        self.kind(ButtonKind::Danger)
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let (background, foreground, border, hover) = match self.kind {
            ButtonKind::Primary => (
                theme.accent,
                theme.accent_fg,
                theme.accent,
                theme.accent.opacity(0.88),
            ),
            ButtonKind::Secondary => (theme.surface, theme.fg, theme.border_strong, theme.hover),
            ButtonKind::Ghost => (
                transparent_black(),
                theme.fg_muted,
                transparent_black(),
                theme.hover,
            ),
            ButtonKind::Danger => (
                theme.danger.alpha(0.1),
                theme.danger,
                theme.danger.alpha(0.35),
                theme.danger.alpha(0.18),
            ),
        };
        let height = if self.compact { CONTROL_SM } else { CONTROL_MD };
        // The keyboard focus ring: Tab reaches every enabled button, and
        // Enter or Space clicks the focused one.
        let ring = if self.kind == ButtonKind::Primary {
            theme.fg
        } else {
            theme.accent
        };
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(SPACE_3)
            .h(height)
            .px(if self.compact { SPACE_4 } else { SPACE_6 })
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(if self.compact { FONT_SMALL } else { FONT_BODY })
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when_some(self.icon, |this, glyph| {
                this.child(icon(glyph, ICON_MD, foreground))
            })
            .child(self.label)
            .when(self.disabled, |this| this.opacity(0.45))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .tab_index(0)
                    .focus(move |style| style.border_color(ring))
                    .hover(move |style| style.bg(hover))
                    .when_some(self.on_click, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
            .when_some(self.tooltip, |this, text| {
                this.tooltip(move |_, cx| Tooltip::view(text.clone(), cx))
            })
    }
}

/// A square icon-only button.
#[derive(IntoElement)]
#[must_use]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    size: Pixels,
    color: Option<Hsla>,
    active: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            icon,
            size: CONTROL_SM,
            color: None,
            active: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx);
        let color = self.color.unwrap_or(if self.active {
            theme.fg
        } else {
            theme.fg_muted
        });
        let hover = theme.hover;
        let ring = theme.accent;
        let glyph = if self.size > CONTROL_SM {
            ICON_LG
        } else {
            ICON_MD
        };
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(self.size)
            .rounded(RADIUS_MD)
            // Transparent until focused, so the ring does not move the icon.
            .border_1()
            .border_color(transparent_black())
            .when(self.active, |this| this.bg(theme.selected))
            .child(icon(self.icon, glyph, color))
            .when(self.disabled, |this| this.opacity(0.4))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .tab_index(0)
                    .focus(move |style| style.border_color(ring))
                    .hover(move |style| style.bg(hover))
                    .when_some(self.on_click, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
            .when_some(self.tooltip, |this, text| {
                this.tooltip(move |_, cx| Tooltip::view(text.clone(), cx))
            })
    }
}

/// The floating label of a hovered control.
#[derive(Debug)]
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn view(text: SharedString, cx: &mut App) -> gpui::AnyView {
        cx.new(|_| Self { text }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .max_w(px(320.0))
            .px(SPACE_4)
            .py(SPACE_2)
            .rounded(RADIUS_MD)
            .bg(theme.elevated)
            .border_1()
            .border_color(theme.border_strong)
            .shadow(theme.shadow(Elevation::Popover))
            .text_size(FONT_SMALL)
            .text_color(theme.fg)
            .child(self.text.clone())
    }
}

/// A small filled circle, e.g. a session state.
pub fn status_dot(color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(8.0))
        .rounded_full()
        .bg(color)
        .border_2()
        .border_color(color.alpha(0.25))
}

/// A rounded tinted label.
pub fn pill(text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .px(SPACE_3)
        .py(px(1.0))
        .rounded(RADIUS_SM)
        .bg(color.alpha(0.14))
        .text_color(color)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .child(text.into())
}

/// A coloured square with two letters, identifying a protocol or connection.
pub fn monogram(letters: impl Into<SharedString>, color: Hsla, size: Pixels) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(RADIUS_MD)
        .bg(color.alpha(0.16))
        .border_1()
        .border_color(color.alpha(0.3))
        .text_color(color)
        .text_size(if size > px(24.0) {
            FONT_BODY
        } else {
            FONT_CAPTION
        })
        .font_weight(FontWeight::BOLD)
        .child(letters.into())
}

/// Voltrus brand green (the logo's stroke).
pub const BRAND_GREEN: u32 = 0x00e676;
/// The logo tile's background, as on the app icon.
pub const BRAND_TILE: u32 = 0x0b0b0d;

/// The Voltrus logo: the green mark on its dark tile, `size` square. Brand
/// colours are fixed so the logo reads the same in light and dark themes.
pub fn brand_mark(size: Pixels) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(size * 0.22)
        .bg(rgb(BRAND_TILE))
        .border_1()
        .border_color(gpui::white().alpha(0.08))
        .child(icon(Icon::Voltrus, size * 0.72, rgb(BRAND_GREEN).into()))
}

/// A small uppercase heading over a group of controls.
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(text.into().to_uppercase())
}

/// A keyboard shortcut hint, e.g. `⌘K`.
pub fn kbd(keys: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .flex_none()
        .px(SPACE_2)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_muted)
        .font_family(MONO)
        .child(keys.into())
}

/// A labelled form row: the label above, the control below.
pub fn field(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .min_w_0()
        .child(
            div()
                .text_size(FONT_SMALL)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.fg_muted)
                .child(label.into()),
        )
        .child(control)
}

/// One option of a [`segmented`] control.
pub struct Segment {
    pub label: SharedString,
    pub icon: Option<Icon>,
    pub selected: bool,
    /// Show only the icon, with the label as a tooltip (narrow spaces).
    pub icon_only: bool,
    pub on_click: ClickHandler,
}

impl Segment {
    pub fn new(
        label: impl Into<SharedString>,
        selected: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon: None,
            selected,
            icon_only: false,
            on_click: Box::new(on_click),
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Drop the label when `icon_only` holds and the segment has an icon.
    #[must_use]
    pub fn icon_only(mut self, icon_only: bool) -> Self {
        self.icon_only = icon_only;
        self
    }
}

/// A row of mutually exclusive options.
pub fn segmented(id: impl Into<ElementId>, segments: Vec<Segment>, cx: &App) -> AnyElement {
    let theme = theme(cx);
    let id: ElementId = id.into();
    let (selected_bg, selected_fg, fg, hover) =
        (theme.elevated, theme.fg, theme.fg_muted, theme.hover);
    let border = theme.border;
    let ring = theme.accent;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(RADIUS_MD)
        .bg(theme.surface)
        .border_1()
        .border_color(border)
        .children(
            segments
                .into_iter()
                .enumerate()
                .map(move |(index, segment)| {
                    let color = if segment.selected { selected_fg } else { fg };
                    let handler = segment.on_click;
                    let icon_only = segment.icon_only && segment.icon.is_some();
                    let label = segment.label;
                    div()
                        .id(ElementId::NamedInteger(
                            SharedString::from(format!("{id}-segment")),
                            u64::try_from(index).unwrap_or(0),
                        ))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(SPACE_2)
                        .h(px(20.0))
                        .when(icon_only, |this| this.w(px(26.0)))
                        .when(!icon_only, |this| this.px(SPACE_4))
                        .rounded(RADIUS_SM)
                        .text_size(FONT_SMALL)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .tab_index(0)
                        // Every segment carries a border, transparent unless
                        // selected, so the focus ring does not shift the row.
                        .border_1()
                        .border_color(transparent_black())
                        .focus(move |style| style.border_color(ring))
                        .when(segment.selected, |this| {
                            this.bg(selected_bg).border_color(border)
                        })
                        .when(!segment.selected, |this| {
                            this.hover(move |style| style.bg(hover))
                        })
                        .when_some(segment.icon, |this, glyph| {
                            this.child(icon(glyph, ICON_SM, color))
                        })
                        .map(|this| {
                            if icon_only {
                                this.tooltip(move |_, cx| Tooltip::view(label.clone(), cx))
                            } else {
                                this.child(label)
                            }
                        })
                        .on_click(move |event, window, cx| handler(event, window, cx))
                }),
        )
        .into_any_element()
}

/// A centred message for an empty area, with optional actions below.
pub fn empty_state(
    glyph: Icon,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    actions: Vec<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_4)
        .p(SPACE_10)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(44.0))
                .rounded(RADIUS_XL)
                .bg(theme.accent_soft)
                .child(icon(glyph, px(22.0), theme.accent)),
        )
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg)
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(420.0))
                .text_center()
                .text_size(FONT_BODY)
                .text_color(theme.fg_muted)
                .child(body.into()),
        )
        .when(!actions.is_empty(), |this| {
            this.child(div().flex().gap(SPACE_4).pt(SPACE_4).children(actions))
        })
}

/// A 1px rule.
pub fn divider(cx: &App) -> impl IntoElement {
    div().flex_none().h(px(1.0)).w_full().bg(theme(cx).border)
}
