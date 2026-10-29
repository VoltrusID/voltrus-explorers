//! MTConnect sessions over `mt-mtconnect`: a client that probes an agent
//! and streams its observations into points, and an agent simulator that
//! plays a machine over HTTP (and optionally SHDR).

pub(crate) mod agent;
pub(crate) mod client;

use std::collections::HashMap;

use mt_mtconnect::{Category, Condition, ConditionLevel, DataItem, Device, Devices, Value};
use vt_protocol::{PointInfo, Quality, Reading};

/// The names of a condition's levels, as point states 1 to 3.
const CONDITION_STATES: [&str; 3] = ["Normal", "Warning", "Fault"];

/// The type of the emergency stop data item: `TRIGGERED` is 1, `ARMED` 0.
pub(crate) const EMERGENCY_STOP: &str = "EMERGENCY_STOP";

/// What a point needs to turn an observation into a reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Kind {
    pub category: Category,
    pub data_type: String,
}

impl Kind {
    fn of(item: &DataItem) -> Self {
        Self {
            category: item.category,
            data_type: item.data_type.clone(),
        }
    }

    fn two_state(&self) -> bool {
        self.category == Category::Event
            && matches!(self.data_type.as_str(), EMERGENCY_STOP | "AVAILABILITY")
    }
}

/// The points of an agent's devices and where each data item's value goes.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub infos: Vec<PointInfo>,
    pub kinds: Vec<Kind>,
    pub ids: Vec<String>,
    pub index: HashMap<String, usize>,
}

impl Layout {
    /// One point per data item of every device, in probe order.
    pub(crate) fn from_devices(devices: &Devices) -> Self {
        let many = devices.devices.len() > 1;
        let mut layout = Self::default();
        for device in &devices.devices {
            layout.add_device(device, many, |_| false);
        }
        layout
    }

    /// The points of a simulated machine, with the data items the app can set
    /// marked writable.
    pub(crate) fn from_simulated(device: &Device) -> Self {
        let mut layout = Self::default();
        layout.add_device(device, false, writable_in_simulator);
        layout
    }

    fn add_device(&mut self, device: &Device, prefix: bool, writable: impl Fn(&DataItem) -> bool) {
        for (path, item) in device.flatten_data_items() {
            if self.index.contains_key(&item.id) {
                continue;
            }
            let id = self.infos.len();
            let mut name = item.name.clone().unwrap_or_else(|| item.id.clone());
            if !path.is_empty() {
                name = format!("{path}/{name}");
            }
            if prefix {
                name = format!("{}/{name}", device.name);
            }
            let kind = Kind::of(item);
            self.infos.push(PointInfo {
                id: u32::try_from(id).unwrap_or(u32::MAX),
                name,
                address: item.id.clone(),
                data_type: data_type_label(item),
                unit: unit_label(item.units.as_deref().unwrap_or_default()),
                writable: writable(item),
                boolean: kind.two_state(),
                states: if item.category == Category::Condition {
                    CONDITION_STATES
                        .iter()
                        .map(|state| (*state).into())
                        .collect()
                } else {
                    Vec::new()
                },
            });
            self.index.insert(item.id.clone(), id);
            self.ids.push(item.id.clone());
            self.kinds.push(kind);
        }
    }
}

/// Samples can be forced to a value, conditions raised and cleared, and the
/// emergency stop pressed; the rest of a simulated machine is its program's.
fn writable_in_simulator(item: &DataItem) -> bool {
    match item.category {
        Category::Sample | Category::Condition => true,
        Category::Event => item.data_type == EMERGENCY_STOP,
    }
}

fn data_type_label(item: &DataItem) -> String {
    match &item.sub_type {
        Some(sub) => format!("{}.{sub}", item.data_type),
        None => item.data_type.clone(),
    }
}

/// A short unit for what MTConnect names in full.
pub(crate) fn unit_label(units: &str) -> String {
    match units {
        "" | "COUNT" => String::new(),
        "MILLIMETER" => "mm".into(),
        "MILLIMETER/SECOND" => "mm/s".into(),
        "MILLIMETER/SECOND^2" => "mm/s²".into(),
        "MILLIMETER_3D" => "mm".into(),
        "REVOLUTION/MINUTE" => "rpm".into(),
        "REVOLUTION/SECOND" => "rev/s".into(),
        "PERCENT" => "%".into(),
        "CELSIUS" => "°C".into(),
        "DEGREE" => "°".into(),
        "DEGREE/SECOND" => "°/s".into(),
        "AMPERE" => "A".into(),
        "VOLT" => "V".into(),
        "WATT" => "W".into(),
        "HERTZ" => "Hz".into(),
        "NEWTON" => "N".into(),
        "PASCAL" => "Pa".into(),
        "SECOND" => "s".into(),
        other => other.to_lowercase().replace('_', " "),
    }
}

/// A value read as a point: the number and the text to show beside it.
/// Numbers carry no text; a value with no number (an `UNAVAILABLE` item, a
/// text event, a data set) is a good reading with no number, whose text the
/// app shows instead.
pub(crate) fn reading(
    kind: &Kind,
    value: &Value,
    condition: Option<&Condition>,
) -> (Reading, String) {
    let good = |value: f64| Reading {
        value,
        raw: [0; 4],
        raw_len: 0,
        quality: Quality::Good,
    };
    if kind.category == Category::Condition {
        return match condition {
            Some(condition) => match condition.level {
                ConditionLevel::Normal => (good(1.0), String::new()),
                ConditionLevel::Warning => (good(2.0), condition_text(condition)),
                ConditionLevel::Fault => (good(3.0), condition_text(condition)),
                ConditionLevel::Unavailable => (good(f64::NAN), "UNAVAILABLE".into()),
            },
            None if value.is_unavailable() => (good(f64::NAN), "UNAVAILABLE".into()),
            None => (Reading::bad(), String::new()),
        };
    }
    if kind.two_state()
        && let Value::Text(text) = value
    {
        let on = match (kind.data_type.as_str(), text.as_str()) {
            (EMERGENCY_STOP, "TRIGGERED") | ("AVAILABILITY", "AVAILABLE") => Some(1.0),
            (EMERGENCY_STOP, "ARMED") | ("AVAILABILITY", "UNAVAILABLE") => Some(0.0),
            _ => None,
        };
        if let Some(on) = on {
            return (good(on), String::new());
        }
    }
    match value {
        Value::Number(number) => (good(*number), String::new()),
        Value::Text(text) => match text.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => (good(number), String::new()),
            _ => (good(f64::NAN), text.clone()),
        },
        other => (good(f64::NAN), other.to_text()),
    }
}

fn condition_text(condition: &Condition) -> String {
    let message = condition.message.as_deref().unwrap_or_default().trim();
    match condition.native_code.as_deref().map(str::trim) {
        Some(code) if !code.is_empty() && message.is_empty() => code.to_owned(),
        Some(code) if !code.is_empty() => format!("{code}: {message}"),
        _ => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use mt_mtconnect::{DeviceTemplate, SimModel};

    use super::*;

    fn kind(category: Category, data_type: &str) -> Kind {
        Kind {
            category,
            data_type: data_type.into(),
        }
    }

    #[test]
    fn numbers_and_numeric_text_are_values_and_other_text_is_shown() {
        let sample = kind(Category::Sample, "POSITION");
        let (read, text) = reading(&sample, &Value::Number(12.5), None);
        assert_eq!((read.value, text.as_str()), (12.5, ""));
        let count = kind(Category::Event, "PART_COUNT");
        let (read, _) = reading(&count, &Value::Text("42".into()), None);
        assert_eq!(read.value, 42.0);
        let exec = kind(Category::Event, "EXECUTION");
        let (read, text) = reading(&exec, &Value::Text("ACTIVE".into()), None);
        assert!(read.value.is_nan());
        assert_eq!((read.quality, text.as_str()), (Quality::Good, "ACTIVE"));
    }

    #[test]
    fn unavailable_is_good_and_says_so() {
        let sample = kind(Category::Sample, "POSITION");
        let (read, text) = reading(&sample, &Value::Unavailable, None);
        assert_eq!(read.quality, Quality::Good);
        assert!(read.value.is_nan());
        assert_eq!(text, "UNAVAILABLE");
    }

    #[test]
    fn conditions_are_a_level_with_the_controllers_message() {
        let system = kind(Category::Condition, "SYSTEM");
        let fault = Condition {
            level: ConditionLevel::Fault,
            native_code: Some("1001".into()),
            native_severity: None,
            qualifier: None,
            message: Some("Servo overload".into()),
            condition_id: None,
        };
        let (read, text) = reading(&system, &Value::Text("Servo overload".into()), Some(&fault));
        assert_eq!((read.value, text.as_str()), (3.0, "1001: Servo overload"));
        let normal = Condition {
            level: ConditionLevel::Normal,
            message: None,
            native_code: None,
            ..fault
        };
        let (read, text) = reading(&system, &Value::Text(String::new()), Some(&normal));
        assert_eq!((read.value, text.as_str()), (1.0, ""));
    }

    #[test]
    fn the_emergency_stop_is_on_or_off() {
        let estop = kind(Category::Event, EMERGENCY_STOP);
        assert_eq!(
            reading(&estop, &Value::Text("TRIGGERED".into()), None)
                .0
                .value,
            1.0
        );
        assert_eq!(
            reading(&estop, &Value::Text("ARMED".into()), None).0.value,
            0.0
        );
    }

    #[test]
    fn a_template_machine_lays_out_one_point_per_data_item() {
        let device = DeviceTemplate::Vmc3Axis.build();
        let layout = Layout::from_simulated(&device);
        assert_eq!(layout.infos.len(), device.flatten_data_items().len());
        let at = |id: &str| layout.index[id];
        assert_eq!(layout.infos[at("Xpos")].unit, "mm");
        assert!(layout.infos[at("Xpos")].writable);
        assert!(layout.infos[at("estop")].boolean);
        assert!(!layout.infos[at("exec")].writable);
        assert_eq!(layout.infos[at("sys_cond")].states.len(), 3);
    }

    #[test]
    fn the_simulators_model_reads_as_points() {
        let model = SimModel::new(DeviceTemplate::Lathe2Axis, 64);
        let layout = Layout::from_simulated(&model.device());
        assert!(layout.index.contains_key("Zpos"));
        assert!(!layout.index.contains_key("Ypos"));
    }
}
