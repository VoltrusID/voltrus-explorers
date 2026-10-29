//! Typed, serde-serializable view of MTConnect documents.

use serde::{Deserialize, Serialize};

/// The `Header` of any MTConnect response document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub creation_time: Option<String>,
    pub sender: Option<String>,
    /// Changes whenever the agent restarts (sequence numbers restart too).
    pub instance_id: u64,
    pub version: Option<String>,
    pub device_model_change_time: Option<String>,
    pub buffer_size: Option<u64>,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub next_sequence: Option<u64>,
    pub asset_buffer_size: Option<u64>,
    pub asset_count: Option<u64>,
}

/// `category` of a data item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Category {
    Sample,
    Event,
    Condition,
}

impl Category {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "SAMPLE" => Some(Self::Sample),
            "EVENT" => Some(Self::Event),
            "CONDITION" => Some(Self::Condition),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sample => "SAMPLE",
            Self::Event => "EVENT",
            Self::Condition => "CONDITION",
        }
    }
}

/// A `DataItem` from a probe response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataItem {
    pub id: String,
    pub name: Option<String>,
    pub category: Category,
    #[serde(rename = "type")]
    pub data_type: String,
    pub sub_type: Option<String>,
    pub units: Option<String>,
    pub native_units: Option<String>,
    pub native_scale: Option<String>,
    pub coordinate_system: Option<String>,
    pub coordinate_system_id_ref: Option<String>,
    pub composition_id: Option<String>,
    /// `DATA_SET`, `TABLE`, `TIME_SERIES`, `VALUE`, ... (2.x).
    pub representation: Option<String>,
    pub statistic: Option<String>,
    pub discrete: bool,
    pub sample_rate: Option<String>,
}

impl DataItem {
    /// A data item with only the mandatory fields set.
    pub fn new(id: &str, category: Category, data_type: &str) -> Self {
        Self {
            id: id.to_string(),
            name: None,
            category,
            data_type: data_type.to_string(),
            sub_type: None,
            units: None,
            native_units: None,
            native_scale: None,
            coordinate_system: None,
            coordinate_system_id_ref: None,
            composition_id: None,
            representation: None,
            statistic: None,
            discrete: false,
            sample_rate: None,
        }
    }
}

/// A component of a device (`Axes`, `Linear`, `Controller`, `Path`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    /// The XML element name, i.e. the component type (`Linear`, `Rotary`, `Controller`, ...).
    pub element: String,
    pub id: String,
    pub name: Option<String>,
    pub native_name: Option<String>,
    pub uuid: Option<String>,
    pub data_items: Vec<DataItem>,
    pub components: Vec<Component>,
}

impl Component {
    pub fn new(element: &str, id: &str, name: Option<&str>) -> Self {
        Self {
            element: element.to_string(),
            id: id.to_string(),
            name: name.map(str::to_string),
            native_name: None,
            uuid: None,
            data_items: Vec::new(),
            components: Vec::new(),
        }
    }
}

/// The `Description` of a device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDescription {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub station: Option<String>,
    pub text: Option<String>,
}

/// A `Device` (or, in 2.x, the `Agent` pseudo-device) of a probe response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// True for the 2.x `Agent` element.
    pub is_agent: bool,
    pub id: String,
    pub name: String,
    pub uuid: Option<String>,
    pub description: Option<DeviceDescription>,
    pub data_items: Vec<DataItem>,
    pub components: Vec<Component>,
}

impl Device {
    /// Every data item of the device, depth first, with a `Component/Component` path
    /// (empty for device-level items).
    pub fn flatten_data_items(&self) -> Vec<(String, &DataItem)> {
        fn walk<'a>(c: &'a Component, prefix: &str, out: &mut Vec<(String, &'a DataItem)>) {
            let label = c.name.as_deref().unwrap_or(&c.id);
            let path = if prefix.is_empty() { label.to_string() } else { format!("{prefix}/{label}") };
            for d in &c.data_items {
                out.push((path.clone(), d));
            }
            for child in &c.components {
                walk(child, &path, out);
            }
        }
        let mut out: Vec<(String, &DataItem)> = self.data_items.iter().map(|d| (String::new(), d)).collect();
        for c in &self.components {
            walk(c, "", &mut out);
        }
        out
    }
}

/// A parsed `MTConnectDevices` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Devices {
    pub header: Header,
    pub devices: Vec<Device>,
}

impl Devices {
    /// Find a device by name or uuid or id.
    pub fn device(&self, key: &str) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.name == key || d.uuid.as_deref() == Some(key) || d.id == key)
    }

    /// Look a data item up by id across all devices.
    pub fn data_item(&self, id: &str) -> Option<&DataItem> {
        self.devices
            .iter()
            .flat_map(|d| d.flatten_data_items())
            .map(|(_, i)| i)
            .find(|i| i.id == id)
    }
}

/// Level of a condition observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConditionLevel {
    Normal,
    Warning,
    Fault,
    Unavailable,
}

impl ConditionLevel {
    /// Match the `Condition` child element name (`Normal`, `Warning`, `Fault`, `Unavailable`).
    pub fn from_element(name: &str) -> Option<Self> {
        match name {
            "Normal" => Some(Self::Normal),
            "Warning" => Some(Self::Warning),
            "Fault" => Some(Self::Fault),
            "Unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }

    pub fn element_name(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Warning => "Warning",
            Self::Fault => "Fault",
            Self::Unavailable => "Unavailable",
        }
    }
}

/// Condition-specific attributes of an observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub level: ConditionLevel,
    /// The controller's own code (`nativeCode`).
    pub native_code: Option<String>,
    pub native_severity: Option<String>,
    pub qualifier: Option<String>,
    /// The element text.
    pub message: Option<String>,
    pub condition_id: Option<String>,
}

/// One entry of a data set / table observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSetEntry {
    pub key: String,
    pub value: String,
    pub removed: bool,
    /// Cells of a table entry.
    pub cells: Vec<DataSetEntry>,
}

/// The value of an observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Value {
    /// The agent reports `UNAVAILABLE`.
    Unavailable,
    Number(f64),
    Text(String),
    /// A `TIME_SERIES` sample (`sampleCount` numbers).
    TimeSeries(Vec<f64>),
    DataSet(Vec<DataSetEntry>),
}

impl Value {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable)
    }

    /// Numeric value, parsing text if it holds a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Text(t) => t.trim().parse().ok(),
            _ => None,
        }
    }

    /// Display text (`UNAVAILABLE` for unavailable values).
    pub fn to_text(&self) -> String {
        match self {
            Self::Unavailable => "UNAVAILABLE".to_string(),
            Self::Number(n) => n.to_string(),
            Self::Text(t) => t.clone(),
            Self::TimeSeries(v) => v.iter().map(f64::to_string).collect::<Vec<_>>().join(" "),
            Self::DataSet(entries) => entries
                .iter()
                .map(|e| format!("{}={}", e.key, e.value))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// One observation (sample, event or condition) from a streams document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub data_item_id: String,
    pub name: Option<String>,
    /// XML element name, e.g. `Position`, `Execution`, `Fault`.
    pub element: String,
    pub category: Category,
    /// `type` attribute (only present on conditions in the standard output).
    pub data_item_type: Option<String>,
    pub sub_type: Option<String>,
    pub timestamp: String,
    pub sequence: u64,
    pub value: Value,
    /// Present exactly when `category` is `Condition`.
    pub condition: Option<Condition>,
    pub composition_id: Option<String>,
    pub statistic: Option<String>,
    pub duration: Option<f64>,
    pub reset_triggered: Option<String>,
}

impl Observation {
    pub fn is_unavailable(&self) -> bool {
        match &self.condition {
            Some(c) => c.level == ConditionLevel::Unavailable,
            None => self.value.is_unavailable(),
        }
    }
}

/// A `ComponentStream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStream {
    /// Component type (`Linear`, `Controller`, `Device`, ...).
    pub component: String,
    pub name: Option<String>,
    pub component_id: Option<String>,
    pub uuid: Option<String>,
    pub observations: Vec<Observation>,
}

/// A `DeviceStream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStream {
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub components: Vec<ComponentStream>,
}

/// A parsed `MTConnectStreams` document (`current` or `sample` response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Streams {
    pub header: Header,
    pub devices: Vec<DeviceStream>,
}

impl Streams {
    /// All observations in document order (grouped by component, then samples, events and
    /// conditions). Use [`observations_by_sequence`](Self::observations_by_sequence) for time order.
    pub fn observations(&self) -> impl Iterator<Item = &Observation> {
        self.devices
            .iter()
            .flat_map(|d| d.components.iter())
            .flat_map(|c| c.observations.iter())
    }

    /// All observations ordered by sequence number.
    pub fn observations_by_sequence(&self) -> Vec<&Observation> {
        let mut v: Vec<&Observation> = self.observations().collect();
        v.sort_by_key(|o| o.sequence);
        v
    }

    /// Number of observations.
    pub fn len(&self) -> usize {
        self.observations().count()
    }

    pub fn is_empty(&self) -> bool {
        self.observations().next().is_none()
    }
}

/// An entry of an `MTConnectAssets` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// Asset type = XML element name (`CuttingTool`, `File`, ...).
    pub asset_type: String,
    pub asset_id: String,
    pub device_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub removed: bool,
    /// The asset element exactly as received.
    pub raw_xml: String,
}

/// A parsed `MTConnectAssets` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetList {
    pub header: Header,
    pub assets: Vec<Asset>,
}
