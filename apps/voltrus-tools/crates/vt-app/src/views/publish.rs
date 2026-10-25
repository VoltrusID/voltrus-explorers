//! The inspector's publish card for MQTT sessions: any topic, any payload,
//! with QoS and retain.

use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div};
use vt_protocol::MqttQos;
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, checkbox, field, section_label, segmented};
use vt_ui::theme::*;

use crate::model::AppModel;
use crate::model::live::Live;
use crate::model::publish::PublishForm;

pub fn card(live: &Live, form: &PublishForm, cx: &Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    let running = live.session.is_some();
    let (qos, retain) = (form.qos, form.retain);
    let result = live.publish_result.clone();

    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label("Publish", cx))
        .child(field("Topic", form.topic.clone(), cx))
        .child(field("Payload", form.payload.clone(), cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .child(segmented(
                    "publish-qos",
                    MqttQos::ALL
                        .into_iter()
                        .map(|option| {
                            Segment::new(
                                option.name(),
                                qos == option,
                                cx.listener(move |model, _, _, cx| {
                                    model.set_publish_qos(option, cx)
                                }),
                            )
                        })
                        .collect(),
                    cx,
                ))
                .child(checkbox(
                    "publish-retain",
                    retain,
                    "Retain",
                    cx.listener(|model, _, _, cx| model.toggle_publish_retain(cx)),
                    cx,
                )),
        )
        .child(
            div().flex().justify_end().child(
                Button::new("publish", "Publish")
                    .icon(Icon::Send)
                    .primary()
                    .disabled(!running)
                    .on_click(cx.listener(|model, _, window, cx| {
                        model.publish_message(window, cx);
                    })),
            ),
        )
        .when(!running, |this| {
            this.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child("Start the session to publish."),
            )
        })
        .when_some(result, |this, (topic, result)| {
            let (glyph, color, text): (Icon, gpui::Hsla, SharedString) = match result {
                Ok(()) => (
                    Icon::CircleCheck,
                    theme.success,
                    format!("Published to {topic}").into(),
                ),
                Err(error) => (Icon::CircleAlert, theme.danger, error.into()),
            };
            this.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(color)
                    .child(vt_ui::icon(glyph, ICON_MD, color))
                    .child(div().flex_1().min_w_0().child(text)),
            )
        })
        .into_any_element()
}
