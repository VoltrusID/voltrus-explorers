//! The Beta label of the FANUC CNC and SECS/GEM connections.

use gpui::prelude::*;
use gpui::{ElementId, div};
use vt_protocol::Protocol;
use vt_ui::components::{Tooltip, pill};
use vt_ui::theme::theme;

use crate::model::cnc::beta_note;

/// "Beta", with what it means for `protocol` as its tooltip.
pub fn beta_badge(
    id: impl Into<ElementId>,
    protocol: Protocol,
    cx: &gpui::App,
) -> impl IntoElement {
    let accent = theme(cx).accent;
    div()
        .id(id)
        .flex_none()
        .child(pill("Beta", accent))
        .tooltip(move |_, cx| Tooltip::view(beta_note(protocol).into(), cx))
}
