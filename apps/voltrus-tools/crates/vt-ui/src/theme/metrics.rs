//! One spacing, type and size scale for the whole app, after muxy's metrics
//! (github.com/muxy-app/muxy, MIT).

use gpui::{Pixels, px};

pub const FONT_CAPTION: Pixels = px(10.5);
pub const FONT_SMALL: Pixels = px(11.5);
pub const FONT_BODY: Pixels = px(12.5);
pub const FONT_EMPHASIS: Pixels = px(13.0);
pub const FONT_TITLE: Pixels = px(15.0);
pub const FONT_DISPLAY: Pixels = px(22.0);
/// Big live values in cards.
pub const FONT_VALUE: Pixels = px(26.0);

pub const SPACE_1: Pixels = px(2.0);
pub const SPACE_2: Pixels = px(4.0);
pub const SPACE_3: Pixels = px(6.0);
pub const SPACE_4: Pixels = px(8.0);
pub const SPACE_5: Pixels = px(10.0);
pub const SPACE_6: Pixels = px(12.0);
pub const SPACE_7: Pixels = px(16.0);
pub const SPACE_8: Pixels = px(20.0);
pub const SPACE_9: Pixels = px(24.0);
pub const SPACE_10: Pixels = px(32.0);

pub const ICON_SM: Pixels = px(12.0);
pub const ICON_MD: Pixels = px(14.0);
pub const ICON_LG: Pixels = px(16.0);

pub const CONTROL_SM: Pixels = px(22.0);
pub const CONTROL_MD: Pixels = px(26.0);
pub const CONTROL_LG: Pixels = px(32.0);

pub const RADIUS_SM: Pixels = px(4.0);
pub const RADIUS_MD: Pixels = px(6.0);
pub const RADIUS_LG: Pixels = px(8.0);
pub const RADIUS_XL: Pixels = px(12.0);

pub const TITLE_BAR_HEIGHT: Pixels = px(38.0);
pub const STATUS_BAR_HEIGHT: Pixels = px(26.0);
pub const ROW_HEIGHT: Pixels = px(28.0);
pub const TRAFFIC_LIGHT_WIDTH: Pixels = px(78.0);

/// The bundled monospace face, for values, addresses and hex.
pub const MONO: &str = "JetBrains Mono";
/// The platform UI face (San Francisco on macOS).
pub const SANS: &str = ".SystemUIFont";
