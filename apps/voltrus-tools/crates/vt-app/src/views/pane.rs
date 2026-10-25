//! One pane: a connection's header and the view it shows.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, MouseButton, Window, div, px};
use vt_app_core::{PaneId, PaneView};
use vt_protocol::{Role, SessionState};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, empty_state, monogram, pill, segmented};
use vt_ui::theme::*;

use super::{log, points, state_color, traffic, trend};
use crate::model::AppModel;

pub fn render(
    model: &AppModel,
    pane_id: PaneId,
    highlight: bool,
    window: &mut Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
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
        .map(|live| format!("{:.1} ms", f64::from(live.stats.last_latency_us) / 1000.0));

    let segments = PaneView::ALL
        .into_iter()
        .map(|option| {
            Segment::new(
                option.name(),
                option == view,
                cx.listener(move |model, _, _, cx| model.set_pane_view(pane_id, option, cx)),
            )
            .icon(match option {
                PaneView::Points => Icon::Table,
                PaneView::Trend => Icon::ChartLine,
                PaneView::Traffic => Icon::ArrowUpDown,
                PaneView::Log => Icon::ScrollText,
            })
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
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_shrink()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_EMPHASIS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(connection.name.clone()),
                        )
                        .child(pill(state.label(), color)),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(if detail.is_some() {
                            color
                        } else {
                            theme.fg_dim
                        })
                        .child(detail.unwrap_or_else(|| connection.spec.endpoint())),
                ),
        )
        .child(div().flex_1())
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
        .child(
            Button::new(
                ElementId::NamedInteger("session".into(), pane_id.0),
                match (running, simulator) {
                    (true, true) => "Stop",
                    (true, false) => "Disconnect",
                    (false, true) => "Start",
                    (false, false) => "Connect",
                },
            )
            .icon(if running { Icon::Square } else { Icon::Play })
            .kind(if running {
                vt_ui::components::ButtonKind::Secondary
            } else {
                vt_ui::components::ButtonKind::Primary
            })
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_session(connection_id, cx))),
        );

    let body: AnyElement = match live {
        Some(live) if !live.points.is_empty() || running => match view {
            PaneView::Points => points::render(pane_id, connection_id, live, simulator, cx),
            PaneView::Trend => trend::render(connection_id, live, window, cx),
            PaneView::Traffic => traffic::render(connection_id, live, cx),
            PaneView::Log => log::render(connection_id, live, cx),
        },
        _ => empty_state(
            if simulator { Icon::Server } else { Icon::Plug },
            if simulator { "Simulator stopped" } else { "Not connected" },
            if simulator {
                "Start it to serve its registers on this computer. Any Modbus master can poll it, including this app."
            } else {
                "Connect to start polling. Values, trends and traffic appear here as they arrive."
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

    div()
        .id(ElementId::NamedInteger("pane".into(), pane_id.0))
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
        .child(header)
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}
