//! What the MTConnect and FANUC sessions share: text points that are only
//! reported when they change, traffic summaries, and reconnecting.

use std::time::Duration;

use tokio::sync::mpsc;
use vt_protocol::{Direction, LogLevel, PointText, SessionState, TrafficFrame};

use crate::Control;
use crate::emit::{Emitter, now_ms};

const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// The text last reported for each point, so a text is only sent when it
/// changes. Points start empty, as the app's do.
#[derive(Debug, Default)]
pub(crate) struct Texts {
    last: Vec<String>,
}

impl Texts {
    pub(crate) fn new(points: usize) -> Self {
        Self {
            last: vec![String::new(); points],
        }
    }

    /// The points whose text in `now` differs from the one reported before.
    pub(crate) fn changed(&mut self, now: &[String]) -> Vec<PointText> {
        let mut changed = Vec::new();
        for (index, text) in now.iter().enumerate() {
            if self.last.get(index) != Some(text)
                && let Some(slot) = self.last.get_mut(index)
            {
                slot.clone_from(text);
                changed.push(PointText {
                    point: u32::try_from(index).unwrap_or(u32::MAX),
                    text: text.clone(),
                });
            }
        }
        changed
    }
}

/// One line of the traffic view. These protocols' crates report no wire
/// frames, so the sessions describe each exchange instead of its bytes.
pub(crate) fn frame(
    direction: Direction,
    summary: impl Into<String>,
    detail: impl Into<String>,
) -> TrafficFrame {
    let detail = detail.into();
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary: summary.into(),
        bytes: u32::try_from(detail.len()).unwrap_or(u32::MAX),
        detail,
    }
}

pub(crate) fn fail(out: &Emitter, message: String) {
    out.log(LogLevel::Error, message.clone());
    out.state(SessionState::Closed {
        error: Some(message),
    });
}

pub(crate) fn backoff(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF)
}

/// Report a lost link and wait before the next attempt. Returns `false` when
/// the app closed the session while waiting.
pub(crate) async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let wait = backoff(attempt);
    out.state(SessionState::Reconnecting {
        attempt,
        retry_in_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        reason: reason.to_owned(),
    });
    tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = until_closed(control, out, "Not connected") => false,
    }
}

/// Wait for the app to close the session while there is no link, refusing
/// writes with `refusal`.
pub(crate) async fn until_closed(
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
    refusal: &str,
) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. }) => out.written(point, Err(refusal.to_owned())),
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_are_reported_once_until_they_change() {
        let mut texts = Texts::new(2);
        assert!(texts.changed(&[String::new(), String::new()]).is_empty());
        let first = texts.changed(&["MEM".into(), String::new()]);
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].point, first[0].text.as_str()), (0, "MEM"));
        assert!(texts.changed(&["MEM".into(), String::new()]).is_empty());
        let cleared = texts.changed(&[String::new(), String::new()]);
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].text, "");
    }

    #[test]
    fn backoff_doubles_up_to_ten_seconds() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(9), MAX_BACKOFF);
    }
}
