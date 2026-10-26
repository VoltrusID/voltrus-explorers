//! The Device view: a protocol's own view of the device, for the protocols
//! that have one (an S7 CPU's identity, mode, diagnostics and blocks).

use gpui::prelude::*;
use gpui::{AnyElement, Context};
use vt_app_core::ConnectionId;
use vt_protocol::SessionSpec;
use vt_ui::Icon;
use vt_ui::components::empty_state;

use super::s7;
use crate::model::AppModel;
use crate::model::live::Live;

/// Whether connections of `spec` offer the Device view.
pub fn available(spec: &SessionSpec) -> bool {
    crate::model::s7::has_device_view(spec)
}

/// The view's name in the pane's switcher.
pub fn label(spec: &SessionSpec) -> &'static str {
    match spec {
        SessionSpec::S7Client(_) => "PLC",
        _ => "Device",
    }
}

pub fn render(
    connection: ConnectionId,
    spec: &SessionSpec,
    live: &Live,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    match spec {
        SessionSpec::S7Client(_) => s7::plc(connection, live, cx),
        _ => empty_state(
            Icon::Cpu,
            "No device view",
            "This protocol shows its device in the points, trend, traffic and log views.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
    }
}
