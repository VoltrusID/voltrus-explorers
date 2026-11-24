//! Metric values this publisher can encode, and their Sparkplug B types.

use mt_plc_types::PlcValue;

use crate::pb::DataType;

/// The value types this publisher knows how to encode.
///
/// Deliberately smaller than `PlcValue`: Sparkplug B supports more, but GW-3
/// only needs scalars. `From<PlcValue>` maps the PLC runtime types onto these.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue {
    Bool(bool),
    Int(i64),
    Real(f64),
    String(String),
}

impl MetricValue {
    /// The Sparkplug B [`DataType`] this value encodes as.
    pub fn data_type(&self) -> DataType {
        match self {
            MetricValue::Bool(_) => DataType::Boolean,
            MetricValue::Int(_) => DataType::Int64,
            MetricValue::Real(_) => DataType::Double,
            MetricValue::String(_) => DataType::String,
        }
    }
}

impl From<&PlcValue> for MetricValue {
    fn from(value: &PlcValue) -> Self {
        match value {
            PlcValue::Bool(v) => MetricValue::Bool(*v),
            PlcValue::Byte(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Int(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Dint(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Time(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Word(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Dword(v) => MetricValue::Int(i64::from(*v)),
            PlcValue::Real(v) => MetricValue::Real(f64::from(*v)),
            PlcValue::String(v) => MetricValue::String(v.clone()),
            PlcValue::Enum(v) => MetricValue::String(v.clone()),
            // ARRAY/STRUCT have no scalar counterpart in our value set; fall
            // back to the display form. Sites that want structured data should
            // map fields onto individual metrics instead.
            PlcValue::Array(_) | PlcValue::Struct(_) => MetricValue::String(value.to_string()),
        }
    }
}

impl From<PlcValue> for MetricValue {
    fn from(value: PlcValue) -> Self {
        MetricValue::from(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_plc_types::PlcValue;

    #[test]
    fn data_type_mapping() {
        assert_eq!(MetricValue::Bool(true).data_type(), DataType::Boolean);
        assert_eq!(MetricValue::Int(-5).data_type(), DataType::Int64);
        assert_eq!(MetricValue::Real(1.5).data_type(), DataType::Double);
        assert_eq!(
            MetricValue::String("x".into()).data_type(),
            DataType::String
        );
    }

    #[test]
    fn plc_value_conversion() {
        assert_eq!(
            MetricValue::from(PlcValue::Bool(true)),
            MetricValue::Bool(true)
        );
        assert_eq!(MetricValue::from(PlcValue::Byte(7)), MetricValue::Int(7));
        assert_eq!(MetricValue::from(PlcValue::Int(-2)), MetricValue::Int(-2));
        assert_eq!(
            MetricValue::from(PlcValue::Dint(100_000)),
            MetricValue::Int(100_000)
        );
        assert_eq!(
            MetricValue::from(PlcValue::Time(250)),
            MetricValue::Int(250)
        );
        assert_eq!(
            MetricValue::from(PlcValue::Word(65_535)),
            MetricValue::Int(65_535)
        );
        assert_eq!(
            MetricValue::from(PlcValue::Dword(4_000_000)),
            MetricValue::Int(4_000_000)
        );
        assert_eq!(
            MetricValue::from(PlcValue::Real(2.5)),
            MetricValue::Real(2.5)
        );
        assert_eq!(
            MetricValue::from(PlcValue::String("ok".into())),
            MetricValue::String("ok".into())
        );
        assert_eq!(
            MetricValue::from(PlcValue::Enum("RUN".into())),
            MetricValue::String("RUN".into())
        );
        // Non-scalar PLC types fall back to their display form.
        assert_eq!(
            MetricValue::from(PlcValue::Array(vec![PlcValue::Int(1), PlcValue::Int(2)])),
            MetricValue::String("[1, 2]".into())
        );
        assert_eq!(
            MetricValue::from(PlcValue::Struct(vec![("a".into(), PlcValue::Bool(true))])),
            MetricValue::String("(a:=TRUE)".into())
        );
    }
}
