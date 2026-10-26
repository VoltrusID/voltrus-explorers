//! The S7 simulator's built-in CPUs (from the S7 Explorer app's templates,
//! with inputs, outputs, markers, a timer, a counter and strings added): the
//! memory each serves and how its values move.

use std::sync::LazyLock;

use serde::Deserialize;
use vt_protocol::{S7Address, S7ClientSpec, S7TagSpec, S7Type};

const S7: &str = include_str!("../../templates/s7.json");

/// A CPU the simulator plays.
#[derive(Debug, Clone, Deserialize)]
pub struct S7Template {
    pub id: String,
    pub name: String,
    /// The family it stands for, e.g. `S7-1500`.
    pub cpu: String,
    /// The slot a client of that family uses.
    pub slot: u8,
    pub description: String,
    pub points: Vec<S7SimPoint>,
}

/// One tag of a simulated CPU. Numbers swing around `base` by `variance`
/// over `period_s` with `noise`, or count by `step` per second between `min`
/// and `max`; booleans hold `base`, flipping every `toggle_s` if set.
#[derive(Debug, Clone, Deserialize)]
pub struct S7SimPoint {
    pub label: String,
    pub address: S7Address,
    pub data_type: S7Type,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub base: f64,
    #[serde(default)]
    pub variance: f64,
    #[serde(default)]
    pub noise: f64,
    #[serde(default)]
    pub period_s: f64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub toggle_s: f64,
    #[serde(default)]
    pub step: f64,
    /// The text of a string tag.
    #[serde(default)]
    pub text: String,
}

impl S7SimPoint {
    /// The value `seconds` after the simulator started.
    pub fn value_at(&self, seconds: f64, index: usize) -> f64 {
        match self.data_type {
            S7Type::String => f64::NAN,
            S7Type::Bool => {
                let on = self.base != 0.0;
                if self.toggle_s > 0.0 && (seconds / self.toggle_s) as u64 % 2 == 1 {
                    f64::from(u8::from(!on))
                } else {
                    f64::from(u8::from(on))
                }
            }
            data_type => {
                let min = self.min.unwrap_or(0.0_f64.min(self.base));
                let max = self.max.unwrap_or(f64::MAX);
                let value = if self.step != 0.0 {
                    let span = max - min;
                    let moved = self.base - min + self.step * seconds;
                    if span > 0.0 && span.is_finite() {
                        min + moved.rem_euclid(span + if is_whole(data_type) { 1.0 } else { 0.0 })
                    } else {
                        self.base + self.step * seconds
                    }
                } else {
                    let wave = if self.period_s > 0.0 {
                        (seconds / self.period_s * std::f64::consts::TAU).sin() * self.variance
                    } else {
                        0.0
                    };
                    self.base + wave + noise(seconds, index) * self.noise
                };
                let value = value.clamp(min, max);
                if is_whole(data_type) {
                    value.floor()
                } else {
                    value
                }
            }
        }
    }
}

fn is_whole(data_type: S7Type) -> bool {
    !matches!(data_type, S7Type::Real)
}

/// Pseudo noise in [-0.5, 0.5) from the time and the point, without a random
/// number generator.
fn noise(seconds: f64, index: usize) -> f64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    hash ^= (seconds * 1000.0) as u64;
    hash = hash.wrapping_mul(0x0100_0000_01b3);
    hash ^= index as u64;
    hash = hash.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (hash >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

static TEMPLATES: LazyLock<Vec<S7Template>> = LazyLock::new(|| match serde_json::from_str(S7) {
    Ok(templates) => templates,
    Err(error) => {
        tracing::error!("built-in S7 templates are invalid: {error}");
        Vec::new()
    }
});

/// Every built-in S7 CPU.
pub fn s7() -> &'static [S7Template] {
    &TEMPLATES
}

pub fn find_s7(id: &str) -> Option<&'static S7Template> {
    s7().iter().find(|template| template.id == id)
}

/// A client of a simulated CPU that reads all of its tags.
pub fn s7_client_for(template: &S7Template, host: &str, port: u16) -> S7ClientSpec {
    S7ClientSpec {
        host: host.into(),
        port,
        rack: 0,
        slot: template.slot,
        tags: template
            .points
            .iter()
            .map(|point| S7TagSpec {
                name: point.label.clone(),
                address: point.address,
                data_type: point.data_type,
                unit: point.unit.clone(),
            })
            .collect(),
        ..S7ClientSpec::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_point_has_a_type_its_address_can_hold() {
        let templates = s7();
        assert!(templates.len() >= 3, "templates: {}", templates.len());
        for template in templates {
            assert!(!template.points.is_empty(), "{}", template.id);
            for point in &template.points {
                assert!(
                    point.address.types().contains(&point.data_type),
                    "{} {}: {:?}",
                    template.id,
                    point.address,
                    point.data_type
                );
                assert!(
                    point.data_type != S7Type::String || !point.text.is_empty(),
                    "{} {} has no text",
                    template.id,
                    point.address
                );
            }
            let client = s7_client_for(template, "127.0.0.1", 1102);
            assert_eq!(client.tags.len(), template.points.len());
        }
    }

    #[test]
    fn simulated_values_stay_in_range_and_count_around() {
        let template = find_s7("line").expect("the packaging line");
        for second in 0..600 {
            let seconds = f64::from(second) * 1.3;
            for (index, point) in template.points.iter().enumerate() {
                let value = point.value_at(seconds, index);
                if point.data_type == S7Type::String {
                    assert!(value.is_nan());
                    continue;
                }
                assert!(
                    point.data_type.check(value).is_ok()
                        || point.data_type == S7Type::Timer
                        || point.data_type == S7Type::Counter,
                    "{} = {value}",
                    point.label
                );
                if let Some(min) = point.min {
                    assert!(value >= min, "{} = {value}", point.label);
                }
                if let Some(max) = point.max {
                    assert!(value <= max, "{} = {value}", point.label);
                }
            }
        }
        // A counter counts up from its base and wraps after its maximum.
        let rejects = template
            .points
            .iter()
            .find(|point| point.data_type == S7Type::Counter)
            .expect("a counter");
        assert_eq!(rejects.value_at(0.0, 0), 12.0);
        assert_eq!(rejects.value_at(100.0, 0), 22.0);
        assert_eq!(rejects.value_at(9_880.0, 0), 0.0, "999 + 1 wraps to 0");
        // A gate that toggles every 7 s.
        let gate = template
            .points
            .iter()
            .find(|point| point.toggle_s > 0.0)
            .expect("a toggling output");
        assert_eq!(gate.value_at(1.0, 0), 0.0);
        assert_eq!(gate.value_at(8.0, 0), 1.0);
    }
}
