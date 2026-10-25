mod metrics;
mod scheme;
mod tokens;

pub use metrics::*;
pub use scheme::{ColorScheme, parse_hex};
pub use tokens::{Elevation, Theme};

use gpui::{App, Global, WindowAppearance};

use crate::assets::Assets;

pub const DEFAULT_DARK: &str = "Voltrus";
pub const DEFAULT_LIGHT: &str = "Voltrus Light";

/// Every bundled colour scheme, house themes first, the rest by name.
#[derive(Debug)]
pub struct ThemeCatalog {
    entries: Vec<(String, ColorScheme)>,
}

impl ThemeCatalog {
    pub fn bundled() -> Self {
        let mut entries: Vec<(String, ColorScheme)> = Assets::themes()
            .map(|(name, source)| (name.to_owned(), ColorScheme::parse(source)))
            .filter(|(_, scheme)| scheme.is_complete())
            .collect();
        entries.sort_by_key(|(name, _)| {
            (
                !name.starts_with("Voltrus"),
                name.to_lowercase(),
                name.clone(),
            )
        });
        Self { entries }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(name, _)| name.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<Theme> {
        self.entries
            .iter()
            .find(|(entry, _)| entry == name)
            .map(|(name, scheme)| Theme::from_scheme(name.clone(), scheme))
    }

    /// The named theme, or the house theme of the same brightness.
    pub fn resolve(&self, name: &str, dark: bool) -> Theme {
        self.get(name)
            .or_else(|| self.get(if dark { DEFAULT_DARK } else { DEFAULT_LIGHT }))
            .unwrap_or_else(|| Theme::from_scheme(DEFAULT_DARK, &ColorScheme::default()))
    }
}

/// The theme every component reads.
#[derive(Debug)]
pub struct ActiveTheme(pub Theme);

impl Global for ActiveTheme {}

pub fn theme(cx: &App) -> &Theme {
    &cx.global::<ActiveTheme>().0
}

pub fn set_theme(theme: Theme, cx: &mut App) {
    cx.set_global(ActiveTheme(theme));
    cx.refresh_windows();
}

pub fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}
