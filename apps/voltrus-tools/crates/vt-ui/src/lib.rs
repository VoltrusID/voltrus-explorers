//! The Voltrus design system for GPUI.
//!
//! Structure after muxy-ui (github.com/muxy-app/muxy, MIT): theme tokens
//! derived from a colour scheme, one metrics scale, and stateless components
//! that read the active theme. No app state lives here.

mod assets;
pub mod chart;
pub mod components;
pub mod icon;
pub mod text_input;
pub mod theme;

pub use assets::Assets;
pub use icon::{Icon, icon};

use std::borrow::Cow;

use gpui::App;

/// Register fonts and key bindings and install `initial` as the active theme.
/// Call once at startup, before opening windows.
pub fn init(initial: theme::Theme, cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = Assets::fonts().map(Cow::Borrowed).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        // Values fall back to the system monospace face.
        eprintln!("voltrus-tools: could not load bundled fonts: {error}");
    }
    cx.bind_keys(text_input::key_bindings());
    cx.set_global(theme::ActiveTheme(initial));
}
