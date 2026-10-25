//! What an MQTT payload carries, as point values: a bare number or boolean,
//! the numeric fields of a JSON document, or the metrics of a Sparkplug B
//! payload (decoded by `mt-mqtt`).

use mt_mqtt::sparkplug::{SparkplugAliases, SparkplugTopic, decode_payload};
use serde_json::Value;
use vt_protocol::PathStep;

/// Numeric fields taken from one JSON document at most, as in the MQTT
/// Explorer app: deeper or longer structures are summarised, not charted.
const MAX_LEAVES: usize = 64;
const MAX_DEPTH: usize = 5;
const MAX_ARRAY: usize = 32;
/// Longest payload text kept for the inspector.
const MAX_TEXT: usize = 2048;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sample {
    Number(f64),
    Bool(bool),
    Text(String),
}

/// One value found in a message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Extracted {
    /// Empty for the whole payload; otherwise a JSON path (`a.b[2]`) or a
    /// Sparkplug metric name.
    pub path: String,
    pub sample: Sample,
    /// Type label for the points table.
    pub data_type: String,
    pub unit: String,
    /// What the inspector shows for it.
    pub text: String,
}

impl Extracted {
    fn new(path: String, sample: Sample, text: String) -> Self {
        let data_type = match &sample {
            Sample::Number(_) => "Number",
            Sample::Bool(_) => "Bool",
            Sample::Text(_) => "Text",
        }
        .to_owned();
        Self {
            path,
            sample,
            data_type,
            unit: String::new(),
            text,
        }
    }
}

/// A plain (non-Sparkplug) payload. `path` picks one field of a JSON
/// payload; without it every numeric field becomes a value. Empty payloads
/// (cleared retained messages) carry nothing.
pub(crate) fn plain(payload: &[u8], path: Option<&[PathStep]>) -> Vec<Extracted> {
    let Ok(text) = std::str::from_utf8(payload) else {
        let mut hex = preview_hex(payload);
        hex.insert_str(0, "binary ");
        return vec![Extracted {
            data_type: "Binary".into(),
            ..Extracted::new(String::new(), Sample::Text(hex.clone()), hex)
        }];
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Some(sample) = scalar(trimmed) {
        return vec![Extracted::new(String::new(), sample, clip(trimmed))];
    }
    if trimmed.starts_with(['{', '['])
        && let Ok(document) = serde_json::from_str::<Value>(trimmed)
    {
        let shown = clip(&serde_json::to_string_pretty(&document).unwrap_or_default());
        return match path {
            Some(path) => match walk(&document, path).and_then(json_sample) {
                Some(sample) => vec![Extracted::new(path_label(path), sample, shown)],
                None => Vec::new(),
            },
            None => {
                let mut leaves = Vec::new();
                flatten(&document, String::new(), 0, &mut leaves);
                if leaves.is_empty() {
                    return vec![Extracted::new(
                        String::new(),
                        Sample::Text(one_line(trimmed)),
                        shown,
                    )];
                }
                leaves
                    .into_iter()
                    .map(|(path, sample)| Extracted::new(path, sample, shown.clone()))
                    .collect()
            }
        };
    }
    vec![Extracted::new(
        String::new(),
        Sample::Text(one_line(trimmed)),
        clip(trimmed),
    )]
}

/// A bare number or boolean payload.
fn scalar(text: &str) -> Option<Sample> {
    if let Ok(number) = text.parse::<f64>() {
        // `inf` and `NaN` parse, but nobody publishes them as measurements.
        return number.is_finite().then_some(Sample::Number(number));
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "on" => Some(Sample::Bool(true)),
        "false" | "off" => Some(Sample::Bool(false)),
        _ => None,
    }
}

fn json_sample(value: &Value) -> Option<Sample> {
    match value {
        Value::Number(number) => number.as_f64().map(Sample::Number),
        Value::Bool(flag) => Some(Sample::Bool(*flag)),
        Value::String(text) => {
            Some(scalar(text.trim()).unwrap_or_else(|| Sample::Text(text.clone())))
        }
        Value::Null => None,
        other => Some(Sample::Text(other.to_string())),
    }
}

fn walk<'a>(document: &'a Value, path: &[PathStep]) -> Option<&'a Value> {
    path.iter().try_fold(document, |node, step| match step {
        PathStep::Field(name) => node.get(name.as_str()),
        PathStep::Index(index) => node.get(*index),
    })
}

/// A path as the user writes it: `a.b[2]`.
pub(crate) fn path_label(path: &[PathStep]) -> String {
    let mut label = String::new();
    for step in path {
        match step {
            PathStep::Field(name) => {
                if !label.is_empty() {
                    label.push('.');
                }
                label.push_str(name);
            }
            PathStep::Index(index) => {
                label.push('[');
                label.push_str(&index.to_string());
                label.push(']');
            }
        }
    }
    label
}

/// Numeric and boolean leaves of a JSON document, with their paths.
fn flatten(value: &Value, path: String, depth: usize, out: &mut Vec<(String, Sample)>) {
    if out.len() >= MAX_LEAVES {
        return;
    }
    match value {
        Value::Number(number) => {
            if let Some(number) = number.as_f64() {
                out.push((path, Sample::Number(number)));
            }
        }
        Value::Bool(flag) => out.push((path, Sample::Bool(*flag))),
        Value::Array(items) if depth < MAX_DEPTH => {
            for (index, item) in items.iter().take(MAX_ARRAY).enumerate() {
                flatten(item, format!("{path}[{index}]"), depth + 1, out);
            }
        }
        Value::Object(fields) if depth < MAX_DEPTH => {
            for (name, item) in fields {
                let child = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                flatten(item, child, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// What a Sparkplug B message means for the points of its node or device.
#[derive(Debug, PartialEq)]
pub(crate) enum Sparkplug {
    /// Metrics of `source` (`group/node` or `group/node/device`).
    Metrics {
        source: String,
        metrics: Vec<Extracted>,
    },
    /// The node or device went offline: its values are no longer current.
    Death { source: String },
    /// Primary host state and commands to edge nodes carry no values.
    Ignored,
}

/// Decode a message on a Sparkplug B topic. Births teach `aliases` the
/// metric names that later data messages refer to by alias only.
pub(crate) fn sparkplug(
    topic: &SparkplugTopic,
    payload: &[u8],
    aliases: &mut SparkplugAliases,
) -> Result<Sparkplug, String> {
    let Some(group) = &topic.group_id else {
        return Ok(Sparkplug::Ignored);
    };
    let mut source = format!("{group}/{}", topic.edge_node_id);
    if let Some(device) = &topic.device_id {
        source.push('/');
        source.push_str(device);
    }
    match topic.message_type.as_str() {
        "NDEATH" | "DDEATH" => return Ok(Sparkplug::Death { source }),
        "NBIRTH" | "DBIRTH" | "NDATA" | "DDATA" => {}
        // NCMD and DCMD go to the edge node; a monitor only sees them pass.
        _ => return Ok(Sparkplug::Ignored),
    }
    let decoded = decode_payload(topic, payload, aliases)?;
    let metrics = decoded
        .get("metrics")
        .and_then(Value::as_array)
        .map(|metrics| metrics.iter().filter_map(metric).collect())
        .unwrap_or_default();
    Ok(Sparkplug::Metrics { source, metrics })
}

/// One decoded metric (`mt_mqtt::sparkplug` JSON) as a value. A metric sent
/// by alias whose birth this connection missed has no name yet: it is
/// skipped until the node births again, rather than kept as a nameless point.
fn metric(metric: &Value) -> Option<Extracted> {
    let name = metric.get("name").and_then(Value::as_str)?.to_owned();
    let sample = json_sample(metric.get("value")?)?;
    let mut extracted = Extracted::new(
        name,
        sample,
        clip(&serde_json::to_string_pretty(metric).unwrap_or_default()),
    );
    if let Some(data_type) = metric.get("datatype").and_then(Value::as_str) {
        data_type.clone_into(&mut extracted.data_type);
    }
    // Ignition's convention for engineering units in metric properties.
    if let Some(unit) = metric
        .pointer("/properties/engUnit/value")
        .and_then(Value::as_str)
    {
        unit.clone_into(&mut extracted.unit);
    }
    Some(extracted)
}

/// The first line of `text`, short enough for one table cell.
fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut shown: String = line.chars().take(120).collect();
    if shown.len() < text.len() {
        shown.push('…');
    }
    shown
}

/// `text` cut to what the inspector shows.
fn clip(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_owned();
    }
    let mut end = MAX_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The first bytes of a binary payload in hex.
pub(crate) fn preview_hex(payload: &[u8]) -> String {
    const SHOWN: usize = 16;
    let mut text = payload
        .iter()
        .take(SHOWN)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if payload.len() > SHOWN {
        text.push_str(" …");
    }
    text
}

#[cfg(test)]
mod tests {
    use mt_mqtt::sparkplug::pb;
    use prost::Message as _;
    use vt_protocol::parse_json_path;

    use super::*;

    fn samples(payload: &str, path: &str) -> Vec<(String, Sample)> {
        let path = parse_json_path(path).expect("a valid path");
        let path = (!path.is_empty()).then_some(path.as_slice());
        plain(payload.as_bytes(), path)
            .into_iter()
            .map(|extracted| (extracted.path, extracted.sample))
            .collect()
    }

    #[test]
    fn bare_numbers_and_booleans_are_values() {
        assert_eq!(
            samples(" 230.5\n", ""),
            [(String::new(), Sample::Number(230.5))]
        );
        assert_eq!(
            samples("-1e3", ""),
            [(String::new(), Sample::Number(-1000.0))]
        );
        assert_eq!(samples("ON", ""), [(String::new(), Sample::Bool(true))]);
        assert_eq!(samples("false", ""), [(String::new(), Sample::Bool(false))]);
        // Non-finite numbers and words stay text.
        assert_eq!(
            samples("NaN", ""),
            [(String::new(), Sample::Text("NaN".into()))]
        );
        assert_eq!(
            samples("online", ""),
            [(String::new(), Sample::Text("online".into()))]
        );
        // A cleared retained message carries nothing.
        assert!(samples("  ", "").is_empty());
    }

    #[test]
    fn json_documents_give_every_numeric_field() {
        let mut found = samples(
            r#"{"ac": {"power": 1200.5, "on": true}, "phases": [230.1, 229.8], "id": "inv-1", "n": null}"#,
            "",
        );
        // Field order follows serde_json's map, which a feature may change.
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found,
            [
                ("ac.on".to_owned(), Sample::Bool(true)),
                ("ac.power".to_owned(), Sample::Number(1200.5)),
                ("phases[0]".to_owned(), Sample::Number(230.1)),
                ("phases[1]".to_owned(), Sample::Number(229.8)),
            ]
        );
        // Without numbers the document is shown as text.
        assert_eq!(
            samples(r#"{"state": "idle"}"#, ""),
            [(String::new(), Sample::Text(r#"{"state": "idle"}"#.into()))]
        );
        // Broken JSON is text too.
        assert_eq!(
            samples("{\"a\": 1", ""),
            [(String::new(), Sample::Text("{\"a\": 1".into()))]
        );
    }

    #[test]
    fn a_json_path_picks_one_field() {
        let payload = r#"{"phases": [{"v": 230.1}, {"v": 229.8}], "mode": "3"}"#;
        assert_eq!(
            samples(payload, "phases[1].v"),
            [("phases[1].v".to_owned(), Sample::Number(229.8))]
        );
        // Numbers sent as strings still chart.
        assert_eq!(
            samples(payload, "mode"),
            [("mode".to_owned(), Sample::Number(3.0))]
        );
        assert!(samples(payload, "phases[5].v").is_empty());
        assert!(samples(payload, "missing").is_empty());
    }

    #[test]
    fn json_stays_within_the_leaf_and_depth_limits() {
        let wide: Vec<String> = (0..100)
            .map(|index| format!("\"k{index:03}\": {index}"))
            .collect();
        let wide = format!("{{{}}}", wide.join(","));
        assert_eq!(samples(&wide, "").len(), MAX_LEAVES);
        let deep = r#"{"a":{"b":{"c":{"d":{"e":{"f":1}}}}}, "top": 2}"#;
        assert_eq!(samples(deep, ""), [("top".to_owned(), Sample::Number(2.0))]);
        let long: Vec<String> = (0..50).map(|index| index.to_string()).collect();
        assert_eq!(
            samples(&format!("[{}]", long.join(",")), "").len(),
            MAX_ARRAY
        );
    }

    #[test]
    fn binary_payloads_show_as_hex() {
        let found = plain(&[0xFF, 0x00, 0x10], None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data_type, "Binary");
        assert_eq!(found[0].sample, Sample::Text("binary FF 00 10".into()));
    }

    fn sparkplug_metric(name: Option<&str>, alias: u64, value: f64) -> pb::Metric {
        pb::Metric {
            name: name.map(str::to_owned),
            alias: Some(alias),
            datatype: Some(10),
            value: Some(pb::metric::Value::DoubleValue(value)),
            properties: name.map(|_| pb::PropertySet {
                keys: vec!["engUnit".into()],
                values: vec![pb::PropertyValue {
                    r#type: Some(12),
                    is_null: None,
                    value: Some(pb::property_value::Value::StringValue("kW".into())),
                }],
            }),
            ..pb::Metric::default()
        }
    }

    #[test]
    fn sparkplug_data_resolves_aliases_learned_at_birth() {
        let mut aliases = SparkplugAliases::new();
        // Data before any birth names nothing: no nameless values.
        let early = SparkplugTopic::parse("spBv1.0/Plant/DDATA/edge/inv").expect("a topic");
        let payload = pb::Payload {
            metrics: vec![sparkplug_metric(None, 7, 11.0)],
            seq: Some(3),
            ..pb::Payload::default()
        };
        assert_eq!(
            sparkplug(&early, &payload.encode_to_vec(), &mut aliases),
            Ok(Sparkplug::Metrics {
                source: "Plant/edge/inv".into(),
                metrics: Vec::new()
            })
        );
        let birth = SparkplugTopic::parse("spBv1.0/Plant/DBIRTH/edge/inv").expect("a topic");
        let payload = pb::Payload {
            metrics: vec![sparkplug_metric(Some("AC Power"), 7, 12.5)],
            seq: Some(0),
            ..pb::Payload::default()
        };
        let Ok(Sparkplug::Metrics { source, metrics }) =
            sparkplug(&birth, &payload.encode_to_vec(), &mut aliases)
        else {
            panic!("birth metrics");
        };
        assert_eq!(source, "Plant/edge/inv");
        assert_eq!(metrics[0].path, "AC Power");
        assert_eq!(metrics[0].unit, "kW");
        assert_eq!(metrics[0].data_type, "Double");

        let data = SparkplugTopic::parse("spBv1.0/Plant/DDATA/edge/inv").expect("a topic");
        let payload = pb::Payload {
            metrics: vec![sparkplug_metric(None, 7, 13.0)],
            seq: Some(1),
            ..pb::Payload::default()
        };
        let Ok(Sparkplug::Metrics { metrics, .. }) =
            sparkplug(&data, &payload.encode_to_vec(), &mut aliases)
        else {
            panic!("data metrics");
        };
        assert_eq!(metrics[0].path, "AC Power", "named from the birth");
        assert_eq!(metrics[0].sample, Sample::Number(13.0));

        let death = SparkplugTopic::parse("spBv1.0/Plant/NDEATH/edge").expect("a topic");
        assert_eq!(
            sparkplug(&death, &[], &mut aliases),
            Ok(Sparkplug::Death {
                source: "Plant/edge".into()
            })
        );
        assert!(sparkplug(&data, b"not protobuf \xFF", &mut aliases).is_err());
    }
}
