use serde::{Deserialize, Serialize};

/// One addressable value of a session, described once when the session opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointInfo {
    /// Index into every [`ValueFrame::readings`] of the session.
    pub id: u32,
    pub name: String,
    /// Address as engineers write it, e.g. `40001` or `HR 0`.
    pub address: String,
    /// Type label shown in the table, e.g. `Float32`.
    pub data_type: String,
    pub unit: String,
    pub writable: bool,
    /// Booleans render as ON/OFF and trend as steps.
    pub boolean: bool,
    /// Names of the values of a multi-state point, from 1 (BACnet State_Text):
    /// value `n` shows `states[n - 1]`. Empty for every other point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
}

impl PointInfo {
    /// The name of a multi-state point's `value`, if it has one.
    pub fn state_name(&self, value: f64) -> Option<&str> {
        if value.fract() != 0.0 || !(1.0..=self.states.len() as f64).contains(&value) {
            return None;
        }
        self.states.get(value as usize - 1).map(String::as_str)
    }
}

/// The latest value of every point, produced by one poll or tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValueFrame {
    /// Milliseconds since the Unix epoch when the values were read.
    pub at_ms: u64,
    /// Round trip of the poll that produced the frame.
    pub latency_us: u32,
    /// Aligned with the session's points: `readings[i]` belongs to point `i`.
    pub readings: Vec<Reading>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub value: f64,
    /// The words on the wire (big-endian order as received), for the hex view.
    pub raw: [u16; 4],
    pub raw_len: u8,
    pub quality: Quality,
}

impl Reading {
    pub const fn bad() -> Self {
        Self {
            value: f64::NAN,
            raw: [0; 4],
            raw_len: 0,
            quality: Quality::Bad,
        }
    }

    pub fn raw_words(&self) -> &[u16] {
        &self.raw[..usize::from(self.raw_len.min(4))]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Good,
    /// The device answered with an exception or the value failed to decode.
    Bad,
}

/// One frame of protocol traffic for the traffic monitor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrafficFrame {
    pub at_ms: u64,
    pub direction: Direction,
    /// Short protocol-level description, e.g. `FC03 Read Holding 0..9`.
    pub summary: String,
    /// Payload detail as received from the protocol layer.
    pub detail: String,
    pub bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Tx,
    Rx,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_state_values_name_their_state_from_one() {
        let point = PointInfo {
            id: 0,
            name: "Mode".into(),
            address: "MSV 1".into(),
            data_type: "Multi-state".into(),
            unit: String::new(),
            writable: true,
            boolean: false,
            states: vec!["Off".into(), "Heat".into(), "Cool".into()],
        };
        assert_eq!(point.state_name(1.0), Some("Off"));
        assert_eq!(point.state_name(3.0), Some("Cool"));
        // Outside the states, between them, or not a number: no name.
        assert_eq!(point.state_name(0.0), None);
        assert_eq!(point.state_name(4.0), None);
        assert_eq!(point.state_name(2.5), None);
        assert_eq!(point.state_name(f64::NAN), None);
        let plain = PointInfo {
            states: Vec::new(),
            ..point
        };
        assert_eq!(plain.state_name(1.0), None);
        // Saved without states, read back with none.
        let text = serde_json::to_string(&plain).unwrap();
        assert!(!text.contains("states"));
        assert_eq!(serde_json::from_str::<PointInfo>(&text).unwrap(), plain);
    }
}
