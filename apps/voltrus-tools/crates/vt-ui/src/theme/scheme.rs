//! Ghostty-format colour schemes, as muxy reads them
//! (github.com/muxy-app/muxy, MIT).

use std::collections::HashMap;

use gpui::{Rgba, rgb};

#[derive(Debug, Clone, Default)]
pub struct ColorScheme {
    pub background: Option<Rgba>,
    pub foreground: Option<Rgba>,
    pub selection_background: Option<Rgba>,
    pub palette: HashMap<usize, Rgba>,
}

impl ColorScheme {
    /// Parse `key = value` lines; unknown keys and bad values are skipped.
    pub fn parse(source: &str) -> Self {
        let mut scheme = Self::default();
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "background" => scheme.background = parse_hex(value),
                "foreground" => scheme.foreground = parse_hex(value),
                "selection-background" => scheme.selection_background = parse_hex(value),
                "palette" => {
                    let Some((index, color)) = value.split_once('=') else {
                        continue;
                    };
                    if let (Ok(index), Some(color)) = (index.trim().parse(), parse_hex(color)) {
                        scheme.palette.insert(index, color);
                    }
                }
                _ => {}
            }
        }
        scheme
    }

    pub fn color(&self, index: usize) -> Option<Rgba> {
        self.palette.get(&index).copied()
    }

    /// A scheme is usable once it names both base colours.
    pub fn is_complete(&self) -> bool {
        self.background.is_some() && self.foreground.is_some()
    }
}

pub fn parse_hex(value: &str) -> Option<Rgba> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(rgb)
}

pub(crate) fn luminance(color: Rgba) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}
