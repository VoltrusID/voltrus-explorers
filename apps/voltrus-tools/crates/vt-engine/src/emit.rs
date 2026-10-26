use std::time::{SystemTime, UNIX_EPOCH};

use vt_protocol::{
    Event, EventKind, LogEntry, LogLevel, PointInfo, PointText, SessionId, SessionState, Stats,
    TrafficFrame, ValueFrame,
};

/// Reports one session's events to the app.
#[derive(Clone, Debug)]
pub(crate) struct Emitter {
    session: SessionId,
    events: async_channel::Sender<Event>,
}

impl Emitter {
    pub(crate) fn new(session: SessionId, events: async_channel::Sender<Event>) -> Self {
        Self { session, events }
    }

    fn emit(&self, kind: EventKind) {
        // The channel is unbounded; it only fails once the app is gone.
        let _ = self.events.try_send(Event {
            session: self.session,
            kind,
        });
    }

    pub(crate) fn state(&self, state: SessionState) {
        self.emit(EventKind::State(state));
    }

    pub(crate) fn points(&self, points: Vec<PointInfo>) {
        self.emit(EventKind::Points(points));
    }

    pub(crate) fn values(&self, frame: ValueFrame) {
        self.emit(EventKind::Values(frame));
    }

    pub(crate) fn traffic(&self, frames: Vec<TrafficFrame>) {
        if !frames.is_empty() {
            self.emit(EventKind::Traffic(frames));
        }
    }

    pub(crate) fn stats(&self, stats: Stats) {
        self.emit(EventKind::Stats(stats));
    }

    pub(crate) fn written(&self, point: u32, result: Result<(), String>) {
        self.emit(EventKind::Written { point, result });
    }

    pub(crate) fn locked(&self, point: u32, locked: bool) {
        self.emit(EventKind::Locked { point, locked });
    }

    pub(crate) fn log(&self, level: LogLevel, message: impl Into<String>) {
        self.emit(EventKind::Log(LogEntry {
            at_ms: now_ms(),
            level,
            message: message.into(),
        }));
    }

    pub(crate) fn points_added(&self, points: Vec<PointInfo>) {
        self.emit(EventKind::PointsAdded(points));
    }

    pub(crate) fn payloads(&self, texts: Vec<PointText>) {
        self.emit(EventKind::Payloads(texts));
    }

    pub(crate) fn published(&self, topic: String, result: Result<(), String>) {
        self.emit(EventKind::Published { topic, result });
    }

    pub(crate) fn ble_gatt(&self, characteristics: Vec<vt_protocol::BleCharacteristicInfo>) {
        self.emit(EventKind::BleGatt(characteristics));
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
