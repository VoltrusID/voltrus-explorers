use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Icons, themes and fonts compiled into the binary.
#[derive(Debug, Clone, Copy)]
pub struct Assets;

impl Assets {
    /// `(name, source)` of every bundled colour scheme.
    pub fn themes() -> impl Iterator<Item = (&'static str, &'static str)> {
        ASSETS.iter().filter_map(|(path, bytes)| {
            Some((
                path.strip_prefix("themes/")?,
                std::str::from_utf8(bytes).ok()?,
            ))
        })
    }

    pub fn fonts() -> impl Iterator<Item = &'static [u8]> {
        ASSETS
            .iter()
            .filter(|(path, _)| path.starts_with("fonts/") && path.ends_with(".ttf"))
            .map(|(_, bytes)| *bytes)
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .binary_search_by(|(key, _)| (*key).cmp(path))
            .ok()
            .map(|index| Cow::Borrowed(ASSETS[index].1)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(key, _)| key.starts_with(path))
            .map(|(key, _)| (*key).into())
            .collect())
    }
}
