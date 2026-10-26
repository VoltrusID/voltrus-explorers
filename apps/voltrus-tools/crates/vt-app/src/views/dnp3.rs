//! The DNP3 parts of the inspector: a master's station card (internal
//! indications, polls, time sync, restarts, control mode), the control card
//! of a selected output, and a simulator's online switch.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, SharedString, div};
use vt_app_core::ConnectionId;
use vt_protocol::{
    Dnp3Action, Dnp3ControlMode, Dnp3Crob, Dnp3CrobCode, Dnp3Operation, Dnp3PointKind, PointInfo,
    Quality, Reading, SessionSpec,
};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, pill, section_label, segmented};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::dnp3::{Dnp3Live, STATION};
use crate::model::live::Live;

/// The outcome of the last station command, as a line under the buttons.
fn last_result(state: &Dnp3Live, cx: &Context<AppModel>) -> Option<AnyElement> {
    result_line(state.last.as_ref(), cx)
}

/// A command's outcome: a check and its answer, or an alert and the error.
fn result_line(
    last: Option<&(Dnp3Action, Result<String, String>)>,
    cx: &Context<AppModel>,
) -> Option<AnyElement> {
    let theme = theme(cx);
    let (action, result) = last?;
    let (glyph, color, text): (Icon, gpui::Hsla, String) = match result {
        Ok(text) => (
            Icon::CircleCheck,
            theme.success,
            format!("{}: {text}", action.label()),
        ),
        Err(error) => (
            Icon::CircleAlert,
            theme.danger,
            format!("{}: {error}", action.label()),
        ),
    };
    Some(
        div()
            .flex()
            .items_start()
            .gap(SPACE_3)
            .text_size(FONT_SMALL)
            .text_color(color)
            .child(vt_ui::icon(glyph, ICON_MD, color))
            .child(div().flex_1().child(text))
            .into_any_element(),
    )
}

/// How the master commands outputs; changing it saves the connection.
fn mode_switch(
    connection: ConnectionId,
    mode: Dnp3ControlMode,
    cx: &Context<AppModel>,
) -> AnyElement {
    segmented(
        "dnp3-inspector-mode",
        Dnp3ControlMode::ALL
            .into_iter()
            .map(|option| {
                Segment::new(
                    option.name(),
                    mode == option,
                    cx.listener(move |model, _, _, cx| {
                        model.set_dnp3_control_mode(connection, option, cx);
                    }),
                )
            })
            .collect(),
        cx,
    )
}

/// The station card of a DNP3 master's session summary.
pub fn station_card(
    model: &AppModel,
    connection: ConnectionId,
    cx: &mut Context<AppModel>,
) -> Option<AnyElement> {
    let Some(SessionSpec::Dnp3Master(spec)) = model
        .workspace
        .connection(connection)
        .map(|saved| &saved.spec)
    else {
        return None;
    };
    let theme = theme(cx).clone();
    let live = model.live.get(&connection);
    let running = live.is_some_and(|live| live.session.is_some());
    let state = live.map(|live| &live.dnp3);

    let indications: AnyElement = match state.and_then(|state| state.iin) {
        None => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child(if running {
                "Waiting for the first response"
            } else {
                "Connect to read the outstation's indications"
            })
            .into_any_element(),
        Some(iin) if iin.set().is_empty() => div()
            .flex()
            .child(pill("NO INDICATIONS", theme.success))
            .into_any_element(),
        Some(iin) => div()
            .flex()
            .flex_wrap()
            .gap(SPACE_3)
            .children(iin.set().into_iter().map(|bit| {
                pill(
                    bit.name,
                    if bit.problem {
                        theme.warning
                    } else {
                        theme.info
                    },
                )
            }))
            .into_any_element(),
    };

    let buttons = STATION
        .iter()
        .enumerate()
        .map(|(index, (action, detail, icon))| {
            let action = *action;
            let busy = state.is_some_and(|state| state.is_busy(&action));
            let label: SharedString = match action {
                Dnp3Action::IntegrityPoll => "Integrity".into(),
                Dnp3Action::ClassPoll { class } => format!("Class {class}").into(),
                Dnp3Action::TimeSync => "Time sync".into(),
                Dnp3Action::FreezeCounters => "Freeze".into(),
                Dnp3Action::WarmRestart => "Warm restart".into(),
                Dnp3Action::ColdRestart => "Cold restart".into(),
                _ => action.label().into(),
            };
            let mut button = Button::new(
                ElementId::NamedInteger("dnp3-station".into(), index as u64),
                label,
            )
            .icon(if busy { Icon::RefreshCw } else { *icon })
            .compact()
            .disabled(!running || busy)
            .tooltip(*detail)
            .on_click(cx.listener(move |model, _, _, cx| {
                model.dnp3_action(connection, action, cx);
            }));
            if matches!(action, Dnp3Action::ColdRestart) {
                button = button.danger();
            }
            button
        });

    Some(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_5)
            .child(section_label(
                format!(
                    "Outstation {} · internal indications",
                    spec.outstation_address
                ),
                cx,
            ))
            .child(indications)
            .child(section_label("Station", cx))
            .child(div().flex().flex_wrap().gap(SPACE_3).children(buttons))
            .children(state.and_then(|state| last_result(state, cx)))
            .child(section_label("Controls", cx))
            .child(
                div()
                    .flex()
                    .child(mode_switch(connection, spec.control_mode, cx)),
            )
            .into_any_element(),
    )
}

/// The DNP3 card of a selected point: CROB operations for a master's binary
/// output, and the online switch of a simulator's point.
pub fn point_card(
    model: &AppModel,
    connection: ConnectionId,
    live: &Live,
    info: &PointInfo,
    reading: Reading,
    cx: &mut Context<AppModel>,
) -> Option<AnyElement> {
    let spec = &model.workspace.connection(connection)?.spec;
    let running = live.session.is_some();
    let theme = theme(cx).clone();
    match spec {
        SessionSpec::Dnp3Master(master) => {
            let kind = kind_of(info)?;
            if !kind.writable() {
                return None;
            }
            let mode = master.control_mode;
            if kind == Dnp3PointKind::AnalogOutput {
                // The write field above sends the value; here only how.
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_4)
                        .child(div().flex().child(mode_switch(connection, mode, cx)))
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child("Sent as a float32 analog output block (g41v3)."),
                        )
                        .into_any_element(),
                );
            }
            let operations = Dnp3CrobCode::ALL.into_iter().enumerate().map(|(index, code)| {
                let busy = live.dnp3.busy.is_some_and(|busy| {
                    matches!(busy, Dnp3Action::Operate { operation: Dnp3Operation::Crob(crob), .. } if crob.code == code)
                });
                Button::new(
                    ElementId::NamedInteger("dnp3-crob".into(), index as u64),
                    code.name(),
                )
                .icon(match code {
                    Dnp3CrobCode::LatchOn | Dnp3CrobCode::Close => Icon::Play,
                    Dnp3CrobCode::LatchOff | Dnp3CrobCode::Trip => Icon::Square,
                    Dnp3CrobCode::PulseOn | Dnp3CrobCode::PulseOff => Icon::Activity,
                })
                .compact()
                .disabled(!running || busy)
                .tooltip(if code.is_pulse() {
                    "One 1 s pulse"
                } else {
                    "Hold the output until told otherwise"
                })
                .on_click(cx.listener(move |model, _, _, cx| {
                    model.dnp3_operate_selected(Dnp3Operation::Crob(Dnp3Crob::new(code)), cx);
                }))
            });
            // The outcome of the last control of this point.
            let outcome = live.dnp3.last.as_ref().filter(|(action, _)| {
                matches!(action, Dnp3Action::Operate { point, .. } if *point == info.id)
            });
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_4)
                    .child(section_label(format!("Command · {}", mode.name()), cx))
                    .child(div().flex().child(mode_switch(connection, mode, cx)))
                    .child(div().flex().flex_wrap().gap(SPACE_3).children(operations))
                    .children(result_line(outcome, cx))
                    .when(!running, |this| {
                        this.child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child("Connect to command."),
                        )
                    })
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child("CROB (g12v1). The new state comes back as an event."),
                    )
                    .into_any_element(),
            )
        }
        SessionSpec::Dnp3Outstation(_) => {
            let offline = reading.quality == Quality::Bad && reading.value.is_finite();
            let busy = live
                .dnp3
                .busy
                .is_some_and(|busy| matches!(busy, Dnp3Action::SetOnline { .. }));
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_4)
                    .child(section_label("Point flags", cx))
                    .child(
                        div().flex().child(
                            Button::new(
                                "dnp3-online",
                                if offline { "Bring online" } else { "Take offline" },
                            )
                            .icon(if offline { Icon::Plug } else { Icon::Unplug })
                            .compact()
                            .disabled(!running || busy)
                            .tooltip(if offline {
                                "Report the point ONLINE again"
                            } else {
                                "Report COMM_LOST, as a device that lost its sensor: masters read it bad"
                            })
                            .on_click(cx.listener(move |model, _, _, cx| {
                                model.dnp3_set_online_selected(offline, cx);
                            })),
                        ),
                    )
                    .children(last_result(&live.dnp3, cx))
                    .into_any_element(),
            )
        }
        _ => None,
    }
}

/// The point's DNP3 type, from its address (`BO 0`).
fn kind_of(info: &PointInfo) -> Option<Dnp3PointKind> {
    let short = info.address.split_whitespace().next()?;
    Dnp3PointKind::ALL
        .into_iter()
        .find(|kind| kind.short() == short)
}
