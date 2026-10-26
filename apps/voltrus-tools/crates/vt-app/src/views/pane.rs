//! One pane: a connection's header and the view it shows.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, MouseButton, canvas, div, px};
use vt_app_core::{Layout, PaneId, PaneView};
use vt_protocol::{Role, SessionSpec, SessionState};
use vt_ui::Icon;
use vt_ui::components::{
    Button, ButtonKind, IconButton, Segment, empty_state, monogram, pill, segmented, status_dot,
};
use vt_ui::theme::*;

use super::{log, points, state_color, terminal, traffic, trend};
use crate::model::AppModel;

/// Below this width the view switcher shows icons only and the latency hides.
const COMPACT: f32 = 720.0;
/// Below this width the state moves under the name and the session button
/// becomes an icon.
const NARROW: f32 = 520.0;
/// Width a narrow header spends on everything but the name: padding (32),
/// monogram (28), four gaps (40), icon-only switcher (116), session icon (22),
/// plus a little slack.
const NARROW_CHROME: f32 = 246.0;
/// Assumed before the first layout measured the pane.
const UNMEASURED: f32 = 1200.0;

pub fn render(model: &AppModel, pane_id: PaneId, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    // The focused pane of a split tab carries an accent edge.
    let highlight = model
        .workspace
        .tab_of(pane_id)
        .is_some_and(|tab| tab.focused == pane_id && !matches!(tab.layout, Layout::Leaf(_)));
    let scroll = model.pane_scroll(pane_id);
    let width = model
        .pane_widths
        .borrow()
        .get(&pane_id)
        .map_or(UNMEASURED, |width| f32::from(*width));
    let compact = width < COMPACT;
    let narrow = width < NARROW;
    let Some(pane) = model.workspace.pane(pane_id) else {
        return div().into_any_element();
    };
    let Some(connection) = model.workspace.connection(pane.connection) else {
        return div().into_any_element();
    };
    let connection_id = connection.id;
    let view = pane.view;
    let live = model.live.get(&connection_id);
    let running = live.is_some_and(|live| live.session.is_some());
    let simulator = connection.spec.role() == Role::Simulator;
    let state = live.map_or(SessionState::Closed { error: None }, |live| {
        live.state.clone()
    });
    let color = state_color(&state, &theme);

    let detail = match &state {
        SessionState::Reconnecting {
            retry_in_ms,
            reason,
            ..
        } => Some(format!(
            "{reason} · retrying in {:.1} s",
            *retry_in_ms as f64 / 1000.0
        )),
        SessionState::Closed { error: Some(error) } => Some(error.clone()),
        _ => None,
    };
    let latency = live
        .filter(|live| live.state == SessionState::Connected && live.stats.polls > 0)
        .filter(|_| !compact)
        // Serial lines arrive when the device prints them: no round trip.
        .filter(|_| connection.spec.protocol() != vt_protocol::Protocol::Serial)
        .map(|live| format!("{:.1} ms", f64::from(live.stats.last_latency_us) / 1000.0));
    let session_label = match (running, simulator) {
        (true, true) => "Stop",
        (true, false) => "Disconnect",
        (false, true) => "Start",
        (false, false) => "Connect",
    };
    let session_id = ElementId::NamedInteger("session".into(), pane_id.0);
    let session_button = if narrow {
        IconButton::new(session_id, if running { Icon::Unplug } else { Icon::Play })
            .color(if running {
                theme.fg_muted
            } else {
                theme.accent
            })
            .tooltip(session_label)
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_session(connection_id, cx)))
            .into_any_element()
    } else {
        Button::new(session_id, session_label)
            .icon(if running { Icon::Unplug } else { Icon::Play })
            .kind(if running {
                ButtonKind::Secondary
            } else {
                ButtonKind::Primary
            })
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_session(connection_id, cx)))
            .into_any_element()
    };
    // Narrow headers hold the name over the state (or the failure) and drop
    // the address. Text only truncates inside a known width, so give it one:
    // what the header leaves after padding, monogram, gaps, the icon-only
    // view switcher and the session icon.
    let narrow_text = (width - NARROW_CHROME).max(48.0);
    let narrow_status = detail.clone().unwrap_or_else(|| state.label().to_string());
    let has_detail = detail.is_some();
    let subtitle = detail.unwrap_or_else(|| connection.spec.endpoint());
    // Serial sessions show their bytes in a terminal where others show traffic.
    let serial = connection.spec.protocol() == vt_protocol::Protocol::Serial;

    let segments = PaneView::ALL
        .into_iter()
        .map(|option| {
            Segment::new(
                if serial && option == PaneView::Traffic {
                    "Terminal"
                } else {
                    option.name()
                },
                option == view,
                cx.listener(move |model, _, _, cx| model.set_pane_view(pane_id, option, cx)),
            )
            .icon(match option {
                PaneView::Points => Icon::Table,
                PaneView::Trend => Icon::ChartLine,
                PaneView::Traffic if serial => Icon::Terminal,
                PaneView::Traffic => Icon::ArrowUpDown,
                PaneView::Log => Icon::ScrollText,
            })
            .icon_only(compact)
        })
        .collect();

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(48.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .child(monogram(
            connection.spec.protocol().monogram(),
            theme.series(usize::try_from(connection_id.0).unwrap_or(0)),
            px(28.0),
        ))
        .map(|this| {
            if narrow {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .w(px(narrow_text))
                        .child(
                            div()
                                .w(px(narrow_text))
                                .truncate()
                                .text_size(FONT_EMPHASIS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(connection.name.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .child(status_dot(color))
                                .child(
                                    div()
                                        .w(px((narrow_text - 14.0).max(24.0)))
                                        .truncate()
                                        .text_size(FONT_CAPTION)
                                        .text_color(color)
                                        .child(narrow_status),
                                ),
                        ),
                )
                .child(div().flex_1())
            } else {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_4)
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(FONT_EMPHASIS)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(connection.name.clone()),
                                )
                                .child(pill(state.label(), color)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_CAPTION)
                                .text_color(if has_detail { color } else { theme.fg_dim })
                                .child(subtitle),
                        ),
                )
            }
        })
        .when_some(latency, |this, latency| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_2)
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(vt_ui::icon(Icon::Clock, ICON_SM, theme.fg_dim))
                    .child(latency),
            )
        })
        .child(segmented(
            ElementId::NamedInteger("views".into(), pane_id.0),
            segments,
            cx,
        ))
        .child(session_button);

    // Only template simulators hold values; a plain register image has
    // nothing that drifts.
    let holds = match &connection.spec {
        SessionSpec::ModbusSlave(spec) => spec.template.is_some(),
        SessionSpec::MqttBroker(spec) => spec.template.is_some(),
        SessionSpec::BacnetDevice(_) => true,
        SessionSpec::Dnp3Outstation(_) => true,
        SessionSpec::EnipAdapter(_) => true,
        SessionSpec::SerialDevice(_) => true,
        SessionSpec::NatsServer(spec) => spec.template.is_some(),
        _ => false,
    };
    let body: AnyElement = match live {
        Some(live) if !live.points.is_empty() || running || !live.terminal.rows().is_empty() => {
            match view {
                PaneView::Points => {
                    points::render(pane_id, connection_id, live, holds, width, scroll.points, cx)
                }
                PaneView::Trend => trend::render(connection_id, live, width, cx),
                PaneView::Traffic if serial => terminal::render(
                    connection_id,
                    &connection.spec,
                    live,
                    model.serial_consoles.get(&connection_id),
                    width,
                    scroll.traffic,
                    cx,
                ),
                PaneView::Traffic => {
                    traffic::render(connection_id, live, width, scroll.traffic, cx)
                }
                PaneView::Log => log::render(connection_id, live, width, scroll.log, cx),
            }
        }
        _ => empty_state(
            if simulator { Icon::Server } else { Icon::Plug },
            if simulator { "Simulator stopped" } else { "Not connected" },
            match (&connection.spec, simulator) {
                (SessionSpec::MqttBroker(_), _) => {
                    "Start it to run a broker on this computer. Any MQTT client can connect, including this app."
                }
                (SessionSpec::MqttClient(_), _) => {
                    "Connect to subscribe. Topics become points as their messages arrive."
                }
                (SessionSpec::BacnetDevice(_), _) => {
                    "Start it to play the device on this computer. Any BACnet/IP client can read and command it, including this app."
                }
                (SessionSpec::BacnetClient(_), _) => {
                    "Connect to read the device's objects. Values arrive as changes (COV) or polls."
                }
                (SessionSpec::Dnp3Outstation(_), _) => {
                    "Start it to play the outstation on this computer. Any DNP3 master can poll and command it, including this app."
                }
                (SessionSpec::Dnp3Master(_), _) => {
                    "Connect to poll the outstation. Every point it reports appears here, with its flags and events."
                }
                (SessionSpec::EnipAdapter(_), _) => {
                    "Start it to play the adapter on this computer. Any EtherNet/IP client can browse, read and write its tags, including this app."
                }
                (SessionSpec::EnipClient(_), _) => {
                    "Connect to read the controller's tags and attributes. The inspector sends explicit messages."
                }
                (SessionSpec::SerialDevice(_), _) => {
                    "Start it to print an instrument's values on a virtual serial port. Any serial monitor can open it, including this app."
                }
                (SessionSpec::SerialMonitor(_), _) => {
                    "Connect to open the port. Its bytes appear in the terminal; lines with values become points to trend."
                }
                (SessionSpec::NatsServer(_), _) => {
                    "Start it to run a NATS server with JetStream on this computer. Any NATS client can connect, including this app."
                }
                (SessionSpec::NatsClient(_), _) => {
                    "Connect to subscribe. Subjects become points as their messages arrive; publish, request and browse JetStream from the inspector."
                }
                (_, true) => {
                    "Start it to serve its registers on this computer. Any Modbus master can poll it, including this app."
                }
                (_, false) => {
                    "Connect to start polling. Values, trends and traffic appear here as they arrive."
                }
            },
            vec![
                Button::new(
                    ElementId::NamedInteger("empty-connect".into(), pane_id.0),
                    if simulator { "Start simulator" } else { "Connect" },
                )
                .icon(Icon::Play)
                .primary()
                .on_click(cx.listener(move |model, _, _, cx| model.connect(connection_id, cx)))
                .into_any_element(),
            ],
            cx,
        )
        .into_any_element(),
    };

    let widths = model.pane_widths.clone();
    div()
        .id(ElementId::NamedInteger("pane".into(), pane_id.0))
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.bg)
        .when(highlight, |this| {
            this.border_t_2().border_color(theme.accent)
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |model, _, _, cx| {
                if model.workspace.active_tab().map(|tab| tab.focused) != Some(pane_id) {
                    model.focus_pane(pane_id, cx);
                }
            }),
        )
        // Measure the pane; when its width changes, draw once more so the
        // header and columns fit the new space.
        .child(
            canvas(
                move |area, window, _| {
                    let previous = widths.borrow_mut().insert(pane_id, area.size.width);
                    if previous != Some(area.size.width) {
                        window.request_animation_frame();
                    }
                },
                |_, (), _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(header)
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}
