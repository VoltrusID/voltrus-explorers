//! What the app knows about one connection's running session: the latest
//! values, their history for trends, traffic and log.

use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use vt_protocol::{
    EventKind, LogEntry, PointInfo, PointText, Quality, Reading, SessionId, SessionState, Stats,
    TrafficFrame,
};
use vt_ui::chart::History;

/// Trend history kept per point, in time: the longest trend window.
const HISTORY_SPAN_MS: u64 = 15 * 60 * 1000;
/// Samples kept per point at least (30 minutes at a 1 s poll) and at most
/// (15 minutes at a 20 ms poll). Buffers grow as samples arrive.
const HISTORY_MIN: usize = 1800;
const HISTORY_MAX: usize = 45_000;

/// Samples a point keeps so the longest trend window stays full when the
/// session reports every `sample_ms`.
pub fn history_capacity(sample_ms: u64) -> usize {
    let samples = HISTORY_SPAN_MS / sample_ms.max(1);
    usize::try_from(samples)
        .unwrap_or(HISTORY_MAX)
        .clamp(HISTORY_MIN, HISTORY_MAX)
}
const TRAFFIC: usize = 5000;
const LOG: usize = 2000;
/// Points shown on a trend before the user picks any.
const DEFAULT_TREND: usize = 4;

pub struct Live {
    pub session: Option<SessionId>,
    pub state: SessionState,
    pub points: Vec<PointInfo>,
    pub readings: Vec<Reading>,
    pub histories: Vec<Rc<History>>,
    pub traffic: VecDeque<TrafficFrame>,
    pub traffic_paused: bool,
    pub log: VecDeque<LogEntry>,
    pub stats: Stats,
    pub last_frame_ms: u64,
    pub selected: Option<u32>,
    /// Points the user put on the trend; empty means "the first few".
    pub trend: BTreeSet<u32>,
    pub trend_window_ms: f64,
    /// Simulator points the user pinned to a value.
    pub locked: BTreeSet<u32>,
    pub write_result: Option<(u32, Result<(), String>)>,
    /// The log keeps recording but stops following new entries.
    pub log_paused: bool,
    /// The message behind each point's latest value, for message protocols
    /// (MQTT); empty for the others.
    pub payloads: Vec<String>,
    /// Outcome of the last publish from the inspector.
    pub publish_result: Option<(String, Result<(), String>)>,
    /// Samples each point's history keeps; see [`history_capacity`].
    history_capacity: usize,
    /// DNP3: what the session last reported about its outstation.
    pub dnp3: super::dnp3::Dnp3Live,
    /// S7 clients: what the CPU last said about itself.
    pub s7_plc: Option<Result<vt_protocol::S7PlcReport, String>>,
    /// S7 clients: a read of the CPU is on its way.
    pub s7_plc_reading: bool,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            session: None,
            state: SessionState::Closed { error: None },
            points: Vec::new(),
            readings: Vec::new(),
            histories: Vec::new(),
            traffic: VecDeque::new(),
            traffic_paused: false,
            log: VecDeque::new(),
            stats: Stats::default(),
            last_frame_ms: 0,
            selected: None,
            trend: BTreeSet::new(),
            trend_window_ms: 60_000.0,
            locked: BTreeSet::new(),
            write_result: None,
            log_paused: false,
            payloads: Vec::new(),
            publish_result: None,
            history_capacity: HISTORY_MIN,
            dnp3: super::dnp3::Dnp3Live::default(),
            s7_plc: None,
            s7_plc_reading: false,
        }
    }
}

impl Live {
    /// Forget the previous run's data before a new session starts; keep what
    /// the user chose (selection, trend picks, trend window). The session
    /// reports values every `sample_ms`, which sizes the trend history.
    pub fn restart(&mut self, session: SessionId, sample_ms: u64) {
        self.session = Some(session);
        self.state = SessionState::Connecting;
        self.stats = Stats::default();
        self.write_result = None;
        self.publish_result = None;
        self.locked.clear();
        self.dnp3 = super::dnp3::Dnp3Live::default();
        self.s7_plc = None;
        // A connecting S7 client reads its CPU first.
        self.s7_plc_reading = true;
        let capacity = history_capacity(sample_ms);
        if capacity == self.history_capacity {
            for history in &self.histories {
                history.clear();
            }
        } else {
            // Rebuilt at the new size when the session reports its points.
            self.history_capacity = capacity;
            self.histories.clear();
        }
    }

    /// Apply an event from `session`. Events of a session this connection has
    /// since replaced (a reconnect, a restarted simulator) are dropped.
    pub fn apply_from(&mut self, session: SessionId, event: EventKind) {
        if self.session == Some(session) {
            self.apply(event);
        }
    }

    pub fn apply(&mut self, event: EventKind) {
        match event {
            EventKind::State(state) => {
                if !state.is_running() {
                    self.session = None;
                }
                self.state = state;
            }
            EventKind::Points(points) => {
                let count = points.len();
                let same_shape = self.points.len() == count && self.histories.len() == count;
                self.points = points;
                self.readings = vec![Reading::bad(); count];
                self.payloads = vec![String::new(); count];
                if !same_shape {
                    let capacity = self.history_capacity;
                    self.histories = (0..count).map(|_| History::new(capacity)).collect();
                }
                let limit = u32::try_from(count).unwrap_or(u32::MAX);
                self.trend.retain(|point| *point < limit);
                self.locked.retain(|point| *point < limit);
                if self.selected.is_some_and(|point| point >= limit) {
                    self.selected = None;
                }
            }
            EventKind::Values(frame) => {
                #[allow(clippy::cast_precision_loss)]
                let at = frame.at_ms as f64;
                for (history, reading) in self.histories.iter().zip(&frame.readings) {
                    history.push(
                        at,
                        if reading.quality == Quality::Good {
                            reading.value
                        } else {
                            f64::NAN
                        },
                    );
                }
                self.readings = frame.readings;
                self.last_frame_ms = frame.at_ms;
            }
            EventKind::Traffic(frames) => {
                if self.traffic_paused {
                    return;
                }
                self.traffic.extend(frames);
                while self.traffic.len() > TRAFFIC {
                    self.traffic.pop_front();
                }
            }
            EventKind::Log(entry) => {
                self.log.push_back(entry);
                while self.log.len() > LOG {
                    self.log.pop_front();
                }
            }
            EventKind::Stats(stats) => self.stats = stats,
            EventKind::Written { point, result } => self.write_result = Some((point, result)),
            EventKind::Locked { point, locked } => {
                if locked {
                    self.locked.insert(point);
                } else {
                    self.locked.remove(&point);
                }
            }
            EventKind::PointsAdded(points) => {
                self.points.extend(points);
                let count = self.points.len();
                self.readings.resize(count, Reading::bad());
                self.payloads.resize(count, String::new());
                let capacity = self.history_capacity;
                while self.histories.len() < count {
                    self.histories.push(History::new(capacity));
                }
            }
            EventKind::Payloads(texts) => {
                for PointText { point, text } in texts {
                    if let Some(slot) = usize::try_from(point)
                        .ok()
                        .and_then(|index| self.payloads.get_mut(index))
                    {
                        *slot = text;
                    }
                }
            }
            EventKind::Published { topic, result } => {
                self.publish_result = Some((topic, result));
            }
            // The device picker's answer: the editor takes it, not a session.
            EventKind::BacnetDevices(_) => {}
            EventKind::Dnp3Iin(iin) => self.dnp3.iin = Some(iin),
            EventKind::Dnp3Done { action, result } => {
                if self.dnp3.busy == Some(action) {
                    self.dnp3.busy = None;
                }
                self.dnp3.last = Some((action, result));
            }
            EventKind::S7Plc(report) => {
                self.s7_plc = Some(report);
                self.s7_plc_reading = false;
            }
        }
    }

    /// The points drawn on the trend.
    pub fn trend_points(&self) -> Vec<u32> {
        if self.trend.is_empty() {
            let count = self.points.len().min(DEFAULT_TREND);
            (0..u32::try_from(count).unwrap_or(0)).collect()
        } else {
            self.trend.iter().copied().collect()
        }
    }

    pub fn toggle_trend(&mut self, point: u32) {
        if self.trend.is_empty() {
            // Start from what is drawn now, so toggling one doesn't drop the rest.
            self.trend = self.trend_points().into_iter().collect();
        }
        if !self.trend.remove(&point) {
            self.trend.insert(point);
        }
    }

    pub fn point(&self, point: u32) -> Option<(&PointInfo, Reading)> {
        let index = usize::try_from(point).ok()?;
        let info = self.points.get(index)?;
        Some((
            info,
            self.readings.get(index).copied().unwrap_or(Reading::bad()),
        ))
    }

    pub fn history(&self, point: u32) -> Option<&Rc<History>> {
        self.histories.get(usize::try_from(point).ok()?)
    }

    /// The message behind a point's latest value, if the protocol has one.
    pub fn payload(&self, point: u32) -> Option<&str> {
        self.payloads
            .get(usize::try_from(point).ok()?)
            .map(String::as_str)
            .filter(|text| !text.is_empty())
    }

    /// Move the selection by `step` rows; returns the newly selected row.
    pub fn select_step(&mut self, step: isize) -> Option<usize> {
        let count = self.points.len();
        if count == 0 {
            return None;
        }
        let next = match self.selected {
            None => 0,
            Some(current) => {
                let current = isize::try_from(current).unwrap_or(0);
                let last = isize::try_from(count - 1).unwrap_or(0);
                (current + step).clamp(0, last)
            }
        };
        let next = usize::try_from(next).unwrap_or(0);
        self.selected = u32::try_from(next).ok();
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use vt_protocol::{EventKind, PointInfo, SessionId, SessionState};

    use super::{Live, history_capacity};

    fn point(id: u32) -> PointInfo {
        PointInfo {
            id,
            name: format!("P{id}"),
            address: (40_001 + id).to_string(),
            data_type: "UInt16".into(),
            unit: String::new(),
            writable: true,
            boolean: false,
            states: Vec::new(),
        }
    }

    #[test]
    fn a_restart_keeps_choices_and_drops_the_old_run() {
        let mut live = Live::default();
        live.restart(SessionId(1), 1000);
        live.apply(EventKind::Points((0..3).map(point).collect()));
        live.selected = Some(2);
        live.apply(EventKind::Locked {
            point: 1,
            locked: true,
        });
        assert!(live.locked.contains(&1));
        live.restart(SessionId(2), 1000);
        assert_eq!(live.session, Some(SessionId(2)));
        assert!(live.locked.is_empty(), "holds belong to the old run");
        assert_eq!(live.selected, Some(2), "the selection survives");
        // The new run reports fewer points: choices past the end go.
        live.apply(EventKind::Points(vec![point(0)]));
        assert_eq!(live.selected, None);
    }

    #[test]
    fn a_stopped_session_detaches_and_steps_stay_in_range() {
        let mut live = Live::default();
        live.restart(SessionId(7), 1000);
        live.apply(EventKind::Points((0..3).map(point).collect()));
        assert_eq!(live.select_step(1), Some(0));
        assert_eq!(live.select_step(5), Some(2));
        assert_eq!(live.select_step(-9), Some(0));
        live.apply(EventKind::State(SessionState::Closed { error: None }));
        assert_eq!(live.session, None);
    }

    #[test]
    fn events_of_a_replaced_session_are_ignored() {
        let mut live = Live::default();
        live.restart(SessionId(1), 1000);
        live.restart(SessionId(2), 1000);
        live.apply_from(SessionId(1), EventKind::Points((0..3).map(point).collect()));
        live.apply_from(
            SessionId(1),
            EventKind::State(SessionState::Closed { error: None }),
        );
        assert!(live.points.is_empty(), "the old run's schema was dropped");
        assert_eq!(live.session, Some(SessionId(2)), "the old run's close too");
        live.apply_from(SessionId(2), EventKind::Points(vec![point(0)]));
        assert_eq!(live.points.len(), 1);
    }

    #[test]
    fn history_holds_the_longest_trend_window() {
        // 15 minutes at the rate the session reports, within the bounds.
        assert_eq!(history_capacity(1000), 1800);
        assert_eq!(history_capacity(250), 3600);
        assert_eq!(history_capacity(100), 9000);
        assert_eq!(history_capacity(20), 45_000);
        assert_eq!(history_capacity(1), 45_000);
        assert_eq!(history_capacity(0), 45_000);
        assert_eq!(history_capacity(60_000), 1800);
    }

    #[test]
    fn a_faster_session_rebuilds_its_histories() {
        let mut live = Live::default();
        live.restart(SessionId(1), 1000);
        live.apply(EventKind::Points((0..2).map(point).collect()));
        let before = live.histories[0].clone();
        live.restart(SessionId(2), 100);
        assert!(live.histories.is_empty());
        live.apply_from(SessionId(2), EventKind::Points((0..2).map(point).collect()));
        assert_eq!(live.histories.len(), 2);
        assert!(!std::rc::Rc::ptr_eq(&before, &live.histories[0]));
    }

    #[test]
    fn topics_joining_later_keep_what_came_before() {
        use vt_protocol::{PointText, Quality, Reading, ValueFrame};

        let reading = |value: f64| Reading {
            value,
            raw: [0; 4],
            raw_len: 0,
            quality: Quality::Good,
        };
        let mut live = Live::default();
        live.restart(SessionId(1), 250);
        live.apply(EventKind::Points(Vec::new()));
        live.apply(EventKind::PointsAdded(vec![point(0)]));
        live.apply(EventKind::Values(ValueFrame {
            at_ms: 1_000,
            latency_us: 0,
            readings: vec![reading(1.0)],
        }));
        live.trend.insert(0);
        live.selected = Some(0);
        let first = live.histories[0].clone();

        live.apply(EventKind::PointsAdded(vec![point(1), point(2)]));
        assert_eq!(live.points.len(), 3);
        assert!(
            std::rc::Rc::ptr_eq(&first, &live.histories[0]),
            "history kept"
        );
        assert_eq!(live.histories[0].len(), 1);
        assert_eq!(live.histories.len(), 3);
        assert_eq!(live.readings[0].value, 1.0, "the value so far stays");
        assert_eq!(live.readings[2].quality, Quality::Bad, "nothing yet");
        assert_eq!(live.selected, Some(0));
        assert!(live.trend.contains(&0));

        live.apply(EventKind::Payloads(vec![
            PointText {
                point: 2,
                text: "{\"v\": 3}".into(),
            },
            PointText {
                point: 9,
                text: "past the end".into(),
            },
        ]));
        assert_eq!(live.payload(2), Some("{\"v\": 3}"));
        assert_eq!(live.payload(1), None);
        assert_eq!(live.payload(9), None);
    }
}
