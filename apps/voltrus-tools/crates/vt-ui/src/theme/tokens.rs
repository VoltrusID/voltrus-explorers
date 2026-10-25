use gpui::{BoxShadow, Hsla, Rgba, hsla, point, px, rgb};

use super::scheme::{ColorScheme, luminance};

/// Every colour the UI uses, derived from one colour scheme. Views never
/// hard-code colours: they read these tokens.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    /// Window background, behind content.
    pub bg: Hsla,
    /// Sidebar and inspector: a step away from the content area.
    pub chrome: Hsla,
    /// Cards, inputs and table headers.
    pub surface: Hsla,
    /// Menus, popovers and dialogs.
    pub elevated: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_dim: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_fg: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    /// Distinct line colours for charts and connection badges.
    pub series: [Hsla; 6],
}

const FALLBACK_BACKGROUND: u32 = 0x11_11_13;
const FALLBACK_FOREGROUND: u32 = 0xe4_e4_e7;
const FALLBACK_ACCENT: u32 = 0x22_c5_5e;

impl Theme {
    pub fn from_scheme(name: impl Into<String>, scheme: &ColorScheme) -> Self {
        let bg = scheme.background.unwrap_or(rgb(FALLBACK_BACKGROUND));
        let fg = scheme.foreground.unwrap_or(rgb(FALLBACK_FOREGROUND));
        let dark = luminance(bg) < 0.5;
        let accent = scheme.color(4).unwrap_or(rgb(FALLBACK_ACCENT));
        let color = |index: usize, fallback: u32| scheme.color(index).unwrap_or(rgb(fallback));
        let on_bg = |alpha: f32| -> Hsla { blend(with_alpha(fg, alpha), bg) };

        let chrome = if dark { on_bg(0.03) } else { on_bg(0.025) };
        Self {
            name: name.into(),
            dark,
            bg: bg.into(),
            chrome,
            surface: on_bg(if dark { 0.05 } else { 0.035 }),
            elevated: if dark {
                on_bg(0.07)
            } else {
                rgb(0xff_ff_ff).into()
            },
            fg: fg.into(),
            fg_muted: with_alpha(fg, 0.68).into(),
            fg_dim: with_alpha(fg, 0.42).into(),
            border: on_bg(if dark { 0.10 } else { 0.11 }),
            border_strong: on_bg(if dark { 0.18 } else { 0.2 }),
            hover: with_alpha(fg, 0.06).into(),
            selected: with_alpha(accent, if dark { 0.16 } else { 0.12 }).into(),
            accent: accent.into(),
            accent_soft: with_alpha(accent, 0.12).into(),
            accent_fg: if luminance(accent) > 0.6 {
                rgb(0x00_00_00).into()
            } else {
                rgb(0xff_ff_ff).into()
            },
            success: color(2, 0x22_c5_5e).into(),
            warning: color(3, 0xf5_9e_0b).into(),
            danger: color(1, 0xef_44_44).into(),
            info: color(6, 0x38_bd_f8).into(),
            series: [
                accent.into(),
                color(6, 0x38_bd_f8).into(),
                color(3, 0xf5_9e_0b).into(),
                color(5, 0xa7_8b_fa).into(),
                color(1, 0xf8_71_71).into(),
                color(12, 0x60_a5_fa).into(),
            ],
        }
    }

    pub fn series(&self, index: usize) -> Hsla {
        self.series[index % self.series.len()]
    }

    /// Shadow for things that float above the page.
    pub fn shadow(&self, elevation: Elevation) -> Vec<BoxShadow> {
        let strength = if self.dark { 1.0 } else { 0.55 };
        let layer = |offset: f32, blur: f32, opacity: f32| BoxShadow {
            color: hsla(0.0, 0.0, 0.0, opacity * strength),
            offset: point(px(0.0), px(offset)),
            blur_radius: px(blur),
            spread_radius: px(0.0),
        };
        match elevation {
            Elevation::Raised => vec![layer(1.0, 2.0, 0.18)],
            Elevation::Popover => vec![layer(2.0, 6.0, 0.22), layer(8.0, 24.0, 0.18)],
            Elevation::Modal => vec![layer(4.0, 12.0, 0.25), layer(16.0, 48.0, 0.3)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Raised,
    Popover,
    Modal,
}

fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

/// Composite a translucent colour over an opaque one.
fn blend(top: Rgba, bottom: Rgba) -> Hsla {
    let mix = |top_channel: f32, bottom_channel: f32| {
        top_channel * top.a + bottom_channel * (1.0 - top.a)
    };
    Rgba {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a: 1.0,
    }
    .into()
}
