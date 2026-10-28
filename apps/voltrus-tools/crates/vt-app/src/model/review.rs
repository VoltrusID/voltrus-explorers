//! When to ask for an App Store rating (Mac App Store builds only).
//!
//! The ask follows the first successful device read of a session: a client
//! that read values, or a discovery that found a device. It waits until the
//! app has run [`MIN_UPTIME`] (never at launch), then [`SETTLE`] more, and is
//! dropped if an error arrives meanwhile or a sheet or modal is open when it
//! fires. It is requested at most once per session and once per app version
//! (the version is saved in the workspace file). Apple decides whether the
//! prompt shows.

use std::time::Duration;

use gpui::Context;
use vt_protocol::{EventKind, Quality, Role, SessionId};

use super::AppModel;

/// The app must have run this long before a read counts.
pub const MIN_UPTIME: Duration = Duration::from_secs(30);
/// Time from the read to the request, during which an error cancels it.
pub const SETTLE: Duration = Duration::from_secs(2);

/// The decision logic, with time passed in so it can be tested.
#[derive(Debug)]
pub struct ReviewGate {
    version: String,
    prompted: String,
    requested: bool,
    pending: Option<u64>,
    seq: u64,
}

impl ReviewGate {
    /// `version` is this app's; `prompted` the version last asked for.
    pub fn new(version: &str, prompted: &str) -> Self {
        Self {
            version: version.to_owned(),
            prompted: prompted.to_owned(),
            requested: false,
            pending: None,
            seq: 0,
        }
    }

    /// A device read or discovery succeeded `uptime` after launch. Returns a
    /// token when the caller should call [`Self::fire`] after [`SETTLE`].
    pub fn success(&mut self, uptime: Duration) -> Option<u64> {
        if self.requested
            || self.pending.is_some()
            || self.prompted == self.version
            || uptime < MIN_UPTIME
        {
            return None;
        }
        self.seq += 1;
        self.pending = Some(self.seq);
        self.pending
    }

    /// A read error arrived: nothing is asked for the read that was waiting.
    pub fn error(&mut self) {
        self.pending = None;
    }

    /// [`SETTLE`] has passed. `true` means: request the review now, and save
    /// [`Self::version`] as prompted. A sheet or modal open drops the ask; a
    /// later success may try again.
    pub fn fire(&mut self, token: u64, modal_open: bool) -> bool {
        if self.pending != Some(token) {
            return false;
        }
        self.pending = None;
        if modal_open {
            return false;
        }
        self.requested = true;
        self.prompted.clone_from(&self.version);
        true
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// What an event says about a read: a success, an error, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    Success,
    Error,
    Nothing,
}

fn classify(kind: &EventKind) -> Reading {
    match kind {
        EventKind::Values(frame) => {
            if frame.readings.iter().any(|r| r.quality != Quality::Good) {
                Reading::Error
            } else if frame.readings.is_empty() {
                Reading::Nothing
            } else {
                Reading::Success
            }
        }
        EventKind::State(vt_protocol::SessionState::Closed { error: Some(_) })
        | EventKind::State(vt_protocol::SessionState::Reconnecting { .. })
        | EventKind::Written { result: Err(_), .. }
        | EventKind::BacnetDevices(Err(_))
        | EventKind::EnipDevices(Err(_))
        | EventKind::BleDevices(Err(_))
        | EventKind::S7Plc(Err(_))
        | EventKind::CipReply(Err(_)) => Reading::Error,
        EventKind::BacnetDevices(Ok(found)) if !found.is_empty() => Reading::Success,
        EventKind::EnipDevices(Ok(found)) if !found.is_empty() => Reading::Success,
        EventKind::BleDevices(Ok(scan)) if !scan.devices.is_empty() => Reading::Success,
        _ => Reading::Nothing,
    }
}

impl AppModel {
    /// Called with every engine event, before it is applied. `session` is the
    /// connection's session for session events (`None` for discovery).
    pub(super) fn review_observe(
        &mut self,
        session: SessionId,
        kind: &EventKind,
        cx: &mut Context<Self>,
    ) {
        if !vt_store::store_build() || self.store.is_none() {
            return;
        }
        match classify(kind) {
            Reading::Nothing => {}
            Reading::Error => self.review.error(),
            Reading::Success => {
                // A simulator serving its own points is not a device read.
                let simulated = self
                    .sessions
                    .get(&session)
                    .and_then(|connection| self.workspace.connection(*connection))
                    .is_some_and(|connection| connection.spec.role() == Role::Simulator);
                if simulated {
                    return;
                }
                if let Some(token) = self.review.success(self.launched.elapsed()) {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SETTLE).await;
                        let _ = this.update(cx, |model, _| model.review_fire(token));
                    })
                    .detach();
                }
            }
        }
    }

    fn review_fire(&mut self, token: u64) {
        let modal = !matches!(self.overlay, super::Overlay::None) || self.licensing.sheet.is_some();
        if self.review.fire(token, modal) {
            self.workspace.review_prompted = self.review.version().to_owned();
            self.save_quietly();
            vt_store::app_store::request_review();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATE: Duration = Duration::from_secs(31);

    fn gate() -> ReviewGate {
        ReviewGate::new("1.0.1", "1.0.0")
    }

    #[test]
    fn a_read_at_launch_asks_for_nothing() {
        let mut gate = gate();
        assert_eq!(gate.success(Duration::from_secs(5)), None);
        assert_eq!(gate.success(Duration::from_millis(29_999)), None);
        assert!(gate.success(MIN_UPTIME).is_some());
    }

    #[test]
    fn a_read_after_thirty_seconds_fires_once() {
        let mut gate = gate();
        let token = gate.success(LATE).expect("scheduled");
        // More reads while waiting change nothing.
        assert_eq!(gate.success(LATE), None);
        assert!(gate.fire(token, false));
        assert_eq!(gate.success(LATE), None, "once per session");
        assert!(!gate.fire(token, false));
    }

    #[test]
    fn an_error_while_waiting_cancels_and_a_later_read_retries() {
        let mut gate = gate();
        let token = gate.success(LATE).expect("scheduled");
        gate.error();
        assert!(!gate.fire(token, false));
        let again = gate.success(LATE).expect("a later read may try again");
        assert_ne!(again, token);
        assert!(gate.fire(again, false));
    }

    #[test]
    fn a_version_already_asked_for_is_not_asked_again() {
        let mut gate = ReviewGate::new("1.0.1", "1.0.1");
        assert_eq!(gate.success(LATE), None);
        // A new version asks again.
        let mut gate = ReviewGate::new("1.0.2", "1.0.1");
        assert!(gate.success(LATE).is_some());
    }

    #[test]
    fn a_modal_at_fire_time_drops_the_ask_without_using_it_up() {
        let mut gate = gate();
        let token = gate.success(LATE).expect("scheduled");
        assert!(!gate.fire(token, true));
        let again = gate.success(LATE).expect("still eligible");
        assert!(gate.fire(again, false));
    }

    #[test]
    fn firing_records_the_version() {
        let mut gate = gate();
        let token = gate.success(LATE).expect("scheduled");
        assert!(gate.fire(token, false));
        assert_eq!(gate.version(), "1.0.1");
        assert_eq!(gate.prompted, "1.0.1");
    }

    #[test]
    fn events_are_classified() {
        use vt_protocol::{Reading as Value, SessionState};
        let good = Value {
            value: 1.0,
            raw: [0; 4],
            raw_len: 1,
            quality: Quality::Good,
        };
        assert_eq!(
            classify(&EventKind::Values(vt_protocol::ValueFrame {
                at_ms: 0,
                latency_us: 0,
                readings: vec![good]
            })),
            Reading::Success
        );
        assert_eq!(
            classify(&EventKind::Values(vt_protocol::ValueFrame {
                at_ms: 0,
                latency_us: 0,
                readings: vec![good, Value::bad()]
            })),
            Reading::Error
        );
        assert_eq!(
            classify(&EventKind::BacnetDevices(Ok(Vec::new()))),
            Reading::Nothing
        );
        assert_eq!(
            classify(&EventKind::BacnetDevices(Err("x".into()))),
            Reading::Error
        );
        assert_eq!(
            classify(&EventKind::State(SessionState::Closed {
                error: Some("x".into())
            })),
            Reading::Error
        );
        assert_eq!(
            classify(&EventKind::State(SessionState::Connected)),
            Reading::Nothing
        );
    }
}
