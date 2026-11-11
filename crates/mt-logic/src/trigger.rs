//! Triggers: an edge on one channel, or a pattern over several (each channel 0, 1 or don't care)
//! that becomes true. Scanned chunk by chunk as samples stream in; the state carries the previous
//! sample across chunk boundaries.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Rising,
    Falling,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Trigger {
    /// Start capturing at once.
    #[default]
    None,
    Edge { channel: u8, edge: Edge },
    /// Fires when `(sample & mask) == value` becomes true.
    Pattern { mask: u8, value: u8 },
}

#[derive(Debug, Clone)]
pub struct TriggerState {
    spec: Trigger,
    prev: Option<u8>,
}

impl TriggerState {
    pub fn new(spec: Trigger) -> Self {
        Self { spec, prev: None }
    }

    pub fn is_immediate(&self) -> bool {
        self.spec == Trigger::None
    }

    fn hit(&self, prev: u8, s: u8) -> bool {
        match self.spec {
            Trigger::None => true,
            Trigger::Edge { channel, edge } => {
                let (a, b) = (prev >> channel & 1, s >> channel & 1);
                match edge {
                    Edge::Rising => a == 0 && b == 1,
                    Edge::Falling => a == 1 && b == 0,
                    Edge::Any => a != b,
                }
            }
            Trigger::Pattern { mask, value } => s & mask == value & mask && prev & mask != value & mask,
        }
    }

    /// Index in `chunk` of the first sample where the trigger fires.
    pub fn scan(&mut self, chunk: &[u8]) -> Option<usize> {
        if self.is_immediate() {
            return Some(0);
        }
        // A condition already true in the very first sample only counts once it becomes true.
        let mut prev = match self.prev {
            Some(p) => p,
            None => *chunk.first()?,
        };
        for (i, &s) in chunk.iter().enumerate() {
            if self.hit(prev, s) {
                self.prev = Some(s);
                return Some(i);
            }
            prev = s;
        }
        self.prev = Some(prev);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_across_chunks() {
        let mut t = TriggerState::new(Trigger::Edge { channel: 2, edge: Edge::Rising });
        assert_eq!(t.scan(&[0, 0, 0]), None);
        assert_eq!(t.scan(&[4, 4]), Some(0));
        let mut t = TriggerState::new(Trigger::Edge { channel: 0, edge: Edge::Falling });
        assert_eq!(t.scan(&[1, 1, 1, 0]), Some(3));
        let mut t = TriggerState::new(Trigger::Edge { channel: 0, edge: Edge::Any });
        assert_eq!(t.scan(&[1, 1]), None);
        assert_eq!(t.scan(&[1, 0]), Some(1));
    }

    #[test]
    fn pattern_must_become_true() {
        // D0 = 1, D1 = 0, the rest don't care.
        let mut t = TriggerState::new(Trigger::Pattern { mask: 0b11, value: 0b01 });
        assert_eq!(t.scan(&[0b01, 0b01, 0b11]), None, "already true at the start: wait for it to become true");
        assert_eq!(t.scan(&[0b1101]), Some(0));
        let mut t = TriggerState::new(Trigger::Pattern { mask: 0b11, value: 0b01 });
        assert_eq!(t.scan(&[0b10, 0b00, 0b1001]), Some(2));
    }

    #[test]
    fn immediate() {
        assert_eq!(TriggerState::new(Trigger::None).scan(&[7]), Some(0));
    }
}
