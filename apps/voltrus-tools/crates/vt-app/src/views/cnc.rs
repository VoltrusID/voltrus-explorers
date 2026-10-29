//! The Beta label of the FANUC CNC connection.

use gpui::prelude::*;
use gpui::{ElementId, div};
use vt_ui::components::{Tooltip, pill};
use vt_ui::theme::theme;

use crate::model::cnc::BETA_NOTE;

/// "Beta", with what it means as its tooltip.
pub fn beta_badge(id: impl Into<ElementId>, cx: &gpui::App) -> impl IntoElement {
    let accent = theme(cx).accent;
    div()
        .id(id)
        .flex_none()
        .child(pill("Beta", accent))
        .tooltip(|_, cx| Tooltip::view(BETA_NOTE.into(), cx))
}
