//! The inspector's NATS cards: publish and request/reply (with the reply),
//! and the JetStream browser (streams, consumers, stored messages).

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, SharedString, div, px};
use vt_app_core::ConnectionId;
use vt_protocol::{NatsStoredMessage, NatsStreamDetail};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, field, section_label};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::live::Live;
use crate::model::nats::{Browser, Reply};

/// Stored messages a fetch or stream shows at most.
const SHOWN_MESSAGES: usize = 20;

fn note(icon: Icon, color: Hsla, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(vt_ui::icon(icon, ICON_MD, color))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// A box of monospaced lines, scrolling past `max_h`.
fn text_box(id: impl Into<ElementId>, text: &str, cx: &Context<AppModel>) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .id(id)
        .max_h(px(200.0))
        .overflow_y_scroll()
        .px(SPACE_5)
        .py(SPACE_4)
        .rounded(RADIUS_MD)
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .font_family(MONO)
        .text_size(FONT_SMALL)
        .text_color(theme.fg_muted)
        .children(
            text.lines()
                .map(|line| div().min_h(px(14.0)).child(line.to_owned())),
        )
}

/// `1.2 kB`, `3.4 MB`.
fn bytes(count: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = count as f64;
    match count {
        0..1_000 => format!("{count} B"),
        1_000..1_000_000 => format!("{:.1} kB", value / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", value / 1e6),
        _ => format!("{:.1} GB", value / 1e9),
    }
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Publish (any NATS session) and request/reply (clients) from the
/// inspector, with the last reply.
pub fn card(
    model: &AppModel,
    connection: ConnectionId,
    live: &Live,
    client: bool,
    cx: &Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let form = &model.nats;
    let running = live.session.is_some();
    let waiting = matches!(form.replies.get(&connection), Some(Reply::Waiting { .. }));

    let mut card = div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(
            if client {
                "Publish and request"
            } else {
                "Publish"
            },
            cx,
        ))
        .child(field("Subject", form.subject.clone(), cx))
        .child(field("Payload", form.payload.clone(), cx))
        .child(field("Headers", form.headers.clone(), cx))
        .child(
            div()
                .flex()
                .items_end()
                .gap(SPACE_3)
                .when(client, |this| {
                    this.child(div().w(px(96.0)).child(field(
                        "Timeout (ms)",
                        form.timeout.clone(),
                        cx,
                    )))
                })
                .child(div().flex_1())
                .child(
                    Button::new("nats-publish", "Publish")
                        .icon(Icon::Send)
                        .when(!client, Button::primary)
                        .disabled(!running)
                        .tooltip("Send without waiting for an answer")
                        .on_click(cx.listener(|model, _, window, cx| {
                            model.nats_publish(window, cx);
                        })),
                )
                .when(client, |this| {
                    this.child(
                        Button::new("nats-request", "Request")
                            .icon(Icon::ArrowUpDown)
                            .primary()
                            .disabled(!running || waiting)
                            .tooltip("Send and wait for one reply  ↩")
                            .on_click(cx.listener(|model, _, window, cx| {
                                model.nats_request(window, cx);
                            })),
                    )
                }),
        );

    if !running {
        card = card.child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(if client {
                    "Connect to publish and send requests."
                } else {
                    "Start the simulator to publish."
                }),
        );
    }
    if let Some((subject, result)) = live.publish_result.clone() {
        card = card.child(match result {
            Ok(()) => note(
                Icon::CircleCheck,
                theme.success,
                format!("Published to {subject}"),
            ),
            Err(error) => note(Icon::CircleAlert, theme.danger, error),
        });
    }
    match form.replies.get(&connection) {
        Some(Reply::Waiting { subject }) => {
            card = card.child(note(
                Icon::Clock,
                theme.fg_muted,
                format!("Waiting for a reply to {subject}…"),
            ));
        }
        Some(Reply::Done {
            subject,
            result: Ok(reply),
        }) => {
            let headers = reply
                .headers
                .iter()
                .map(|header| format!("{}: {}", header.name, header.value))
                .collect::<Vec<_>>()
                .join("\n");
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_3)
                    .child(note(
                        Icon::CircleCheck,
                        theme.success,
                        format!(
                            "Reply to {subject} in {:.1} ms · {}",
                            f64::from(reply.latency_us) / 1000.0,
                            bytes(u64::from(reply.bytes))
                        ),
                    ))
                    .when(!headers.is_empty(), |this| {
                        this.child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .children(headers.lines().map(|line| div().child(line.to_owned()))),
                        )
                    })
                    .child(text_box(
                        "nats-reply",
                        if reply.payload.is_empty() {
                            "(empty)"
                        } else {
                            reply.payload.as_str()
                        },
                        cx,
                    )),
            );
        }
        Some(Reply::Done {
            result: Err(error), ..
        }) => {
            card = card.child(note(Icon::CircleAlert, theme.danger, error.clone()));
        }
        None => {}
    }
    card.into_any_element()
}

/// The JetStream browser of a NATS client: streams, and the opened one's
/// consumers and newest messages.
pub fn jetstream(model: &AppModel, connection: ConnectionId, cx: &Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let running = model
        .live
        .get(&connection)
        .is_some_and(|live| live.session.is_some());
    let browser = model.nats.browsers.get(&connection);
    let loading = browser.is_some_and(|browser| browser.loading);

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(section_label("JetStream", cx))
        .child(
            IconButton::new("nats-js-refresh", Icon::RefreshCw)
                .size(CONTROL_SM)
                .disabled(!running || loading)
                .tooltip("Refresh streams")
                .on_click(cx.listener(move |model, _, _, cx| {
                    model.nats_refresh_streams(connection, cx);
                })),
        );

    let body: AnyElement = match browser.and_then(|browser| browser.streams.as_ref()) {
        _ if !running && browser.is_none() => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child("Connect to browse the server's streams.")
            .into_any_element(),
        None => note(Icon::Clock, theme.fg_muted, "Loading streams…").into_any_element(),
        Some(Err(error)) => note(Icon::CircleAlert, theme.danger, error.clone()).into_any_element(),
        Some(Ok(streams)) if streams.is_empty() => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child("No streams on this server.")
            .into_any_element(),
        Some(Ok(streams)) => {
            let open = browser.and_then(|browser| browser.open.as_deref());
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .children(streams.iter().enumerate().map(|(index, stream)| {
                    let is_open = open == Some(stream.name.as_str());
                    let name = stream.name.clone();
                    let row = div()
                        .id(("nats-stream", index))
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .px(SPACE_5)
                        .py(SPACE_4)
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(if is_open { theme.accent } else { theme.border })
                        .bg(if is_open {
                            theme.accent_soft
                        } else {
                            theme.surface
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.hover))
                        .on_click(cx.listener(move |model, _, _, cx| {
                            model.nats_toggle_stream(connection, name.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .child(vt_ui::icon(
                                    if is_open {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    },
                                    ICON_SM,
                                    theme.fg_muted,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(MONO)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(stream.name.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_muted)
                                        .child(plural(stream.messages, "msg", "msgs")),
                                ),
                        )
                        .child(
                            div()
                                .pl(SPACE_7)
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(format!(
                                    "{} · {} · {}",
                                    stream.subjects.join(" "),
                                    bytes(stream.bytes),
                                    plural(stream.consumers, "consumer", "consumers")
                                )),
                        );
                    let detail = is_open.then(|| {
                        stream_detail(
                            connection,
                            browser.and_then(|browser| browser.detail.as_ref()),
                            browser,
                            cx,
                        )
                    });
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_4)
                        .child(row)
                        .children(detail)
                }))
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(header)
        .child(body)
        .into_any_element()
}

fn stream_detail(
    connection: ConnectionId,
    detail: Option<&Result<NatsStreamDetail, String>>,
    browser: Option<&Browser>,
    cx: &Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let detail = match detail {
        None => return note(Icon::Clock, theme.fg_muted, "Loading…").into_any_element(),
        Some(Err(error)) => {
            return note(Icon::CircleAlert, theme.danger, error.clone()).into_any_element();
        }
        Some(Ok(detail)) => detail,
    };
    let fetching = browser.and_then(|browser| browser.fetching.as_deref());
    let stream = detail.stream.name.clone();

    let consumers = detail
        .consumers
        .iter()
        .enumerate()
        .map(|(index, consumer)| {
            let (stream, name) = (stream.clone(), consumer.name.clone());
            let busy = fetching == Some(consumer.name.as_str());
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_family(MONO)
                                .truncate()
                                .child(consumer.name.clone()),
                        )
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .truncate()
                                .child(format!(
                                    "{}{} · {} pending · {} unacked",
                                    consumer.kind,
                                    if consumer.durable { ", durable" } else { "" },
                                    consumer.pending,
                                    consumer.ack_pending
                                )),
                        ),
                )
                .when(consumer.kind == "pull", |this| {
                    this.child(
                        Button::new(
                            ("nats-fetch", index),
                            if busy { "Fetching…" } else { "Fetch 10" },
                        )
                        .icon(Icon::Download)
                        .compact()
                        .disabled(busy)
                        .tooltip("Pull the next messages and acknowledge them")
                        .on_click(cx.listener(move |model, _, _, cx| {
                            model.nats_fetch(connection, stream.clone(), name.clone(), cx);
                        })),
                    )
                })
        });

    let mut panel = div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .pl(SPACE_5)
        .ml(SPACE_3)
        .border_l_1()
        .border_color(theme.border)
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(format!(
                    "seq {}–{} · {} · {}",
                    detail.stream.first_seq,
                    detail.stream.last_seq,
                    if detail.stream.storage.is_empty() {
                        "storage ?"
                    } else {
                        detail.stream.storage.as_str()
                    },
                    plural(detail.stream.messages, "message", "messages"),
                )),
        );
    if !detail.consumers.is_empty() {
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Consumers", cx))
                .children(consumers),
        );
    }
    if let Some((consumer, result)) = browser.and_then(|browser| browser.fetched.as_ref()) {
        panel = panel.child(match result {
            Ok(messages) if messages.is_empty() => {
                div().flex().flex_col().gap(SPACE_3).child(note(
                    Icon::Inbox,
                    theme.fg_muted,
                    format!("{consumer} has nothing new"),
                ))
            }
            Ok(messages) => div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .child(note(
                    Icon::CircleCheck,
                    theme.success,
                    format!(
                        "Fetched {} from {consumer}, acknowledged",
                        plural(messages.len() as u64, "message", "messages")
                    ),
                ))
                .child(message_list("nats-fetched", messages, false, cx)),
            Err(error) => note(Icon::CircleAlert, theme.danger, error.clone()),
        });
    }
    panel = panel.child(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .child(section_label(
                format!(
                    "Newest {}",
                    plural(detail.messages.len() as u64, "message", "messages")
                ),
                cx,
            ))
            .child(if detail.messages.is_empty() {
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child("The stream is empty.")
                    .into_any_element()
            } else {
                message_list("nats-stored", &detail.messages, true, cx).into_any_element()
            }),
    );
    panel.into_any_element()
}

/// Stored messages, one per row: sequence, subject and payload.
fn message_list(
    id: &'static str,
    messages: &[NatsStoredMessage],
    newest_first: bool,
    cx: &Context<AppModel>,
) -> impl IntoElement {
    let theme = theme(cx).clone();
    let rows: Vec<&NatsStoredMessage> = if newest_first {
        messages.iter().rev().take(SHOWN_MESSAGES).collect()
    } else {
        messages.iter().take(SHOWN_MESSAGES).collect()
    };
    div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(px(260.0))
        .overflow_y_scroll()
        .rounded(RADIUS_MD)
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .children(rows.into_iter().enumerate().map(|(index, message)| {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .px(SPACE_5)
                .py(SPACE_3)
                .when(index > 0, |this| {
                    this.border_t_1().border_color(theme.border)
                })
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.fg_dim)
                                .child(format!("#{}", message.sequence)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.fg_muted)
                                .child(message.subject.clone()),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(
                            message
                                .payload
                                .lines()
                                .next()
                                .unwrap_or_default()
                                .to_owned(),
                        ),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::{bytes, plural};

    #[test]
    fn sizes_read_at_a_glance() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_000), "1.0 kB");
        assert_eq!(bytes(9_600), "9.6 kB");
        assert_eq!(bytes(2_500_000), "2.5 MB");
        assert_eq!(bytes(3_000_000_000), "3.0 GB");
        assert_eq!(plural(1, "msg", "msgs"), "1 msg");
        assert_eq!(plural(0, "msg", "msgs"), "0 msgs");
    }
}
