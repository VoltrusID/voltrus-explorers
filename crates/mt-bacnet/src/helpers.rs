use std::collections::HashMap;
use std::sync::OnceLock;

use bacnet_rs::object::{ObjectType, PropertyIdentifier};

/// Lower-case a name and drop `_`, `-` and spaces, so `analogInput`, `analog_input` and
/// `Analog-Input` compare equal.
fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Every standard name of an enumeration, keyed by its [`normalize_name`]d form.
fn name_table<T: std::fmt::Display>(ids: std::ops::Range<u32>, make: impl Fn(u32) -> T) -> HashMap<String, u32> {
    ids.filter_map(|id| {
        let name = make(id).to_string();
        // Ids without a standard name display as `Custom(n)` / `Reserved(n)`.
        (!name.ends_with(')')).then(|| (normalize_name(&name), id))
    })
    .collect()
}

/// Convert a name to a BACnet [`ObjectType`]. Accepts the standard type names in camelCase, snake_case
/// or any capitalisation (`analogInput`, `multi_state_value`), the usual short forms (`AI`, `AO`, `AV`,
/// `BI`, `BO`, `BV`, `MSI`/`MI`, `MSO`/`MO`, `MSV`/`MV`) and a plain number (`8` = device).
pub fn object_type_from_str(s: &str) -> Option<ObjectType> {
    static NAMES: OnceLock<HashMap<String, u32>> = OnceLock::new();
    let key = normalize_name(s.trim());
    if let Ok(n) = key.parse::<u32>() {
        return Some(ObjectType::from(n));
    }
    let short = match key.as_str() {
        "ai" => Some(ObjectType::AnalogInput),
        "ao" => Some(ObjectType::AnalogOutput),
        "av" => Some(ObjectType::AnalogValue),
        "bi" => Some(ObjectType::BinaryInput),
        "bo" => Some(ObjectType::BinaryOutput),
        "bv" => Some(ObjectType::BinaryValue),
        "mi" | "msi" => Some(ObjectType::MultiStateInput),
        "mo" | "mso" => Some(ObjectType::MultiStateOutput),
        "mv" | "msv" => Some(ObjectType::MultiStateValue),
        _ => None,
    };
    short.or_else(|| {
        NAMES
            .get_or_init(|| name_table(0..128, ObjectType::from))
            .get(&key)
            .map(|&id| ObjectType::from(id))
    })
}

/// Convert a BACnet [`ObjectType`] to a camelCase string.
pub fn object_type_to_string(ot: &ObjectType) -> String {
    match ot {
        ObjectType::AnalogInput => "analogInput".to_string(),
        ObjectType::AnalogOutput => "analogOutput".to_string(),
        ObjectType::AnalogValue => "analogValue".to_string(),
        ObjectType::BinaryInput => "binaryInput".to_string(),
        ObjectType::BinaryOutput => "binaryOutput".to_string(),
        ObjectType::BinaryValue => "binaryValue".to_string(),
        ObjectType::MultiStateInput => "multiStateInput".to_string(),
        ObjectType::MultiStateOutput => "multiStateOutput".to_string(),
        ObjectType::MultiStateValue => "multiStateValue".to_string(),
        ObjectType::Device => "device".to_string(),
        ObjectType::TrendLog => "trendLog".to_string(),
        ObjectType::TrendLogMultiple => "trendLogMultiple".to_string(),
        ObjectType::Schedule => "schedule".to_string(),
        ObjectType::Calendar => "calendar".to_string(),
        ObjectType::NotificationClass => "notificationClass".to_string(),
        // Named types keep their variant name (`Loop`), which `object_type_from_str` resolves; vendor
        // types (and ids beyond the standard) are written as their number.
        ObjectType::Custom(_) | ObjectType::Reserved(_) => u32::from(*ot).to_string(),
        _ => format!("{ot:?}"),
    }
}

/// Convert a name to a BACnet [`PropertyIdentifier`]. Accepts every standard property name in camelCase,
/// snake_case or any capitalisation (`presentValue`, `object_name`) and a plain number (`85` = present-value,
/// also proprietary ids). `None` for anything else: guessing (for example falling back to present-value)
/// would read, or worse write, a different property than the one asked for.
pub fn property_id_from_str(s: &str) -> Option<PropertyIdentifier> {
    static NAMES: OnceLock<HashMap<String, u32>> = OnceLock::new();
    let key = normalize_name(s.trim());
    // `property-85` is how the property inspector labels properties it has no name for.
    if let Ok(n) = key.strip_prefix("property").unwrap_or(&key).parse::<u32>() {
        return Some(PropertyIdentifier::from(n));
    }
    NAMES
        .get_or_init(|| name_table(0..2048, PropertyIdentifier::from))
        .get(&key)
        .map(|&id| PropertyIdentifier::from(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_types_resolve_in_every_spelling() {
        assert_eq!(object_type_from_str("analogInput"), Some(ObjectType::AnalogInput));
        assert_eq!(object_type_from_str("ANALOG_INPUT"), Some(ObjectType::AnalogInput));
        assert_eq!(object_type_from_str("ai"), Some(ObjectType::AnalogInput));
        assert_eq!(object_type_from_str("MSV"), Some(ObjectType::MultiStateValue));
        assert_eq!(object_type_from_str("multi-state-output"), Some(ObjectType::MultiStateOutput));
        assert_eq!(object_type_from_str("trend_log"), Some(ObjectType::TrendLog));
        assert_eq!(object_type_from_str("loop"), Some(ObjectType::Loop));
        assert_eq!(object_type_from_str("accumulator"), Some(ObjectType::Accumulator));
        assert_eq!(object_type_from_str("8"), Some(ObjectType::Device));
        assert_eq!(object_type_from_str("flux-capacitor"), None);
        // Every type the app names round-trips through its camelCase form.
        for ot in [ObjectType::AnalogOutput, ObjectType::BinaryValue, ObjectType::Schedule, ObjectType::Calendar] {
            assert_eq!(object_type_from_str(&object_type_to_string(&ot)), Some(ot));
        }
    }

    #[test]
    fn properties_resolve_by_name_or_number_and_never_by_guessing() {
        // ASHRAE 135 clause 21 BACnetPropertyIdentifier numbers.
        let id = |s: &str| property_id_from_str(s).map(u32::from);
        assert_eq!(id("presentValue"), Some(85));
        assert_eq!(id("present_value"), Some(85));
        assert_eq!(id("PRESENT-VALUE"), Some(85));
        assert_eq!(id("objectName"), Some(77));
        assert_eq!(id("statusFlags"), Some(111));
        assert_eq!(id("priorityArray"), Some(87));
        assert_eq!(id("relinquishDefault"), Some(104));
        assert_eq!(id("weeklySchedule"), Some(123));
        assert_eq!(id("effectivePeriod"), Some(32));
        assert_eq!(id("logBuffer"), Some(131));
        assert_eq!(id("85"), Some(85));
        // Proprietary ids are plain numbers.
        assert_eq!(id("1234"), Some(1234));
        assert_eq!(id("property-85"), Some(85));
        // A misspelt name is an error, not present-value.
        assert_eq!(id("presentvaleu"), None);
        assert_eq!(id(""), None);
    }
}
