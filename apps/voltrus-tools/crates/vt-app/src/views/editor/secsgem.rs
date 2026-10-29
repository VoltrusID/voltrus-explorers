//! The SECS/GEM editors: a host and an equipment simulator.

use gpui::prelude::*;
use gpui::{AnyElement, Context, div, px};
use vt_ui::components::{checkbox, field};
use vt_ui::theme::*;

use super::cnc::{beta, group, hint};
use super::{on_edit, row};
use crate::model::AppModel;
use crate::model::cnc::SECSGEM_BETA_NOTE;
use crate::model::editor::Editor;

pub fn client(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.secsgem;
    let alarms = fields.alarms;
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(beta(SECSGEM_BETA_NOTE, cx))
        .child(field("Name", editor.name.clone(), cx))
        .child(
            group("Equipment", cx)
                .child(
                    row()
                        .child(div().flex_1().child(field("Host", fields.host.clone(), cx)))
                        .child(
                            div()
                                .w(px(96.0))
                                .child(field("Port", fields.port.clone(), cx)),
                        )
                        .child(
                            div()
                                .w(px(110.0))
                                .child(field("Device ID", fields.device_id.clone(), cx)),
                        ),
                )
                .child(hint(
                    "The equipment listens for an active HSMS connection (SEMI E37). The device ID is the one it is configured with.",
                    cx,
                )),
        )
        .child(
            group("Polling", cx).child(
                row()
                    .child(
                        div()
                            .w(px(150.0))
                            .child(field("Poll (ms)", fields.poll.clone(), cx)),
                    )
                    .child(div().w(px(150.0)).child(field(
                        "Reply timeout (ms)",
                        fields.timeout.clone(),
                        cx,
                    ))),
            ),
        )
        .child(
            group("Variables", cx)
                .child(field("Status variables", fields.svids.clone(), cx))
                .child(field("Equipment constants", fields.ecids.clone(), cx))
                .child(hint(
                    "Ids to read, like 10, 11 or 1001-1005. Empty reads every one the equipment names. Equipment constants can be written from the inspector.",
                    cx,
                )),
        )
        .child(
            group("Events and alarms", cx)
                .child(field("Collection events", fields.events.clone(), cx))
                .child(hint(
                    "Event ids to subscribe to, like 100-104, 300, 301. A report of the status variables is linked to each; the reports appear in the log and the traffic view.",
                    cx,
                ))
                .child(checkbox(
                    "secsgem-alarms",
                    alarms,
                    "Enable alarm reports",
                    on_edit(cx, |editor| editor.secsgem.alarms = !editor.secsgem.alarms),
                    cx,
                )),
        )
        .into_any_element()
}

pub fn simulator(editor: &Editor, cx: &mut Context<AppModel>) -> AnyElement {
    let fields = &editor.secsgem;
    let new = editor.editing.is_none();
    let add_client = editor.add_client;
    let run = fields.run;
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(field("Name", editor.name.clone(), cx))
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .child(field("Listen on", fields.sim_bind.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .child(field("TCP port", fields.sim_port.clone(), cx)),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .child(field("Device ID", fields.sim_device_id.clone(), cx)),
                ),
        )
        .child(hint(
            "0.0.0.0 opens it to the network. The usual HSMS port is 5000, which macOS's AirPlay Receiver holds.",
            cx,
        ))
        .child(
            group("Chamber", cx).child(hint(
                "A wafer processing chamber: control and process state, pressure, temperature, wafer count, five equipment constants, three alarms and eleven collection events, for any GEM host.",
                cx,
            )),
        )
        .child(checkbox(
            "secsgem-run",
            run,
            "Keep processing wafers: start a process whenever the chamber is idle",
            on_edit(cx, |editor| editor.secsgem.run = !editor.secsgem.run),
            cx,
        ))
        .when(new, |this| {
            this.child(checkbox(
                "secsgem-add-client",
                add_client,
                "Also add a host that reads it, side by side",
                on_edit(cx, |editor| editor.add_client = !editor.add_client),
                cx,
            ))
        })
        .into_any_element()
}
