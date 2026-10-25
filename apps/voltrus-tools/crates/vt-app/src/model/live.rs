//! What the app knows about one connection's running session: the latest
//! values, their history for trends, traffic and log.

use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use vt_protocol::{
    EventKind, LogEntry, PointInfo, Quality, Reading, SessionId, SessionState, Stats, TrafficFrame,
};
use vt_ui::chart::History;

/// Samples kept per point: 30 minutes at a 1 s poll.
const HISTORY: usize = 1800;
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
        }
    }
}

impl Live {
    /// Forget the previous run's data before a new session starts; keep what
    /// the user chose (selection, trend picks, trend window).
    pub fn restart(&mut self, session: SessionId) {
        self.session = Some(session);
        self.state = SessionState::Connecting;
        self.stats = Stats::default();
        self.write_result = None;
        self.locked.clear();
        for history in &self.histories {
            history.clear();
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
                let same_shape = self.points.len() == count;
                self.points = points;
                self.readings = vec![Reading::bad(); count];
                if !same_shape {
                    self.histories = (0..count).map(|_| History::new(HISTORY)).collect();
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

    use super::Live;

    fn point(id: u32) -> PointInfo {
        PointInfo {
            id,
            name: format!("P{id}"),
            address: (40_001 + id).to_string(),
            data_type: "UInt16".into(),
            unit: String::new(),
            writable: true,
            boolean: false,
        }
    }

    #[test]
    fn a_restart_keeps_choices_and_drops_the_old_run() {
        let mut live = Live::default();
        live.restart(SessionId(1));
        live.apply(EventKind::Points((0..3).map(point).collect()));
        live.selected = Some(2);
        live.apply(EventKind::Locked {
            point: 1,
            locked: true,
        });
        assert!(live.locked.contains(&1));
        live.restart(SessionId(2));
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
        live.restart(SessionId(7));
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
        live.restart(SessionId(1));
        live.restart(SessionId(2));
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
}
