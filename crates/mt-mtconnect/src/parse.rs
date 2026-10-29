//! Parsing of MTConnect response documents (`MTConnectDevices`, `MTConnectStreams`,
//! `MTConnectAssets`, `MTConnectError`), versions 1.x and 2.x.

use crate::error::{AgentError, AgentErrorCode, AgentErrors, MtConnectError, Result};
use crate::types::*;
use crate::xml::{self, Element};

/// Any parsed MTConnect response document.
#[derive(Debug, Clone, PartialEq)]
pub enum Document {
    Devices(Devices),
    Streams(Streams),
    Assets(AssetList),
    Error(AgentErrors),
}

impl Document {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Devices(_) => "MTConnectDevices",
            Self::Streams(_) => "MTConnectStreams",
            Self::Assets(_) => "MTConnectAssets",
            Self::Error(_) => "MTConnectError",
        }
    }

    pub fn header(&self) -> &Header {
        match self {
            Self::Devices(d) => &d.header,
            Self::Streams(s) => &s.header,
            Self::Assets(a) => &a.header,
            Self::Error(e) => &e.header,
        }
    }
}

/// Parse any MTConnect document, dispatching on the root element name.
pub fn parse_document(xml_text: &str) -> Result<Document> {
    let root = xml::parse(xml_text)?;
    match root.name.as_str() {
        "MTConnectDevices" => Ok(Document::Devices(devices_from(&root))),
        "MTConnectStreams" => Ok(Document::Streams(streams_from(&root))),
        "MTConnectAssets" => Ok(Document::Assets(assets_from(&root, xml_text))),
        "MTConnectError" => Ok(Document::Error(errors_from(&root))),
        other => Err(MtConnectError::Xml(format!("not an MTConnect document: root element <{other}>"))),
    }
}

/// Parse an `MTConnectDevices` (probe) document. An `MTConnectError` document becomes
/// [`MtConnectError::Agent`].
pub fn parse_devices(xml_text: &str) -> Result<Devices> {
    match parse_document(xml_text)? {
        Document::Devices(d) => Ok(d),
        other => Err(unexpected("MTConnectDevices", other)),
    }
}

/// Parse an `MTConnectStreams` (current/sample) document.
pub fn parse_streams(xml_text: &str) -> Result<Streams> {
    match parse_document(xml_text)? {
        Document::Streams(s) => Ok(s),
        other => Err(unexpected("MTConnectStreams", other)),
    }
}

/// Parse an `MTConnectAssets` document.
pub fn parse_assets(xml_text: &str) -> Result<AssetList> {
    match parse_document(xml_text)? {
        Document::Assets(a) => Ok(a),
        other => Err(unexpected("MTConnectAssets", other)),
    }
}

pub(crate) fn unexpected(expected: &'static str, doc: Document) -> MtConnectError {
    match doc {
        Document::Error(e) => MtConnectError::Agent(Box::new(e)),
        other => MtConnectError::UnexpectedDocument { expected, found: other.kind().to_string() },
    }
}

fn opt(el: &Element, name: &str) -> Option<String> {
    el.attr(name).map(str::to_string)
}

fn header_from(root: &Element) -> Header {
    let Some(h) = root.child("Header") else {
        return Header::default();
    };
    let num = |n: &str| h.attr(n).and_then(|v| v.trim().parse::<u64>().ok());
    Header {
        creation_time: opt(h, "creationTime"),
        sender: opt(h, "sender"),
        instance_id: num("instanceId").unwrap_or(0),
        version: opt(h, "version"),
        device_model_change_time: opt(h, "deviceModelChangeTime"),
        buffer_size: num("bufferSize"),
        first_sequence: num("firstSequence"),
        last_sequence: num("lastSequence"),
        next_sequence: num("nextSequence"),
        asset_buffer_size: num("assetBufferSize"),
        asset_count: num("assetCount"),
    }
}

// ── Devices ────────────────────────────────────────────────────────────────

fn devices_from(root: &Element) -> Devices {
    let mut devices = Vec::new();
    if let Some(list) = root.child("Devices") {
        for el in &list.children {
            match el.name.as_str() {
                "Device" => devices.push(device_from(el, false)),
                "Agent" => devices.push(device_from(el, true)),
                _ => {}
            }
        }
    }
    Devices { header: header_from(root), devices }
}

fn device_from(el: &Element, is_agent: bool) -> Device {
    let description = el.child("Description").map(|d| DeviceDescription {
        manufacturer: opt(d, "manufacturer"),
        model: opt(d, "model"),
        serial_number: opt(d, "serialNumber"),
        station: opt(d, "station"),
        text: Some(d.trimmed_text().to_string()).filter(|t| !t.is_empty()),
    });
    Device {
        is_agent,
        id: el.attr("id").unwrap_or_default().to_string(),
        name: el.attr("name").unwrap_or_default().to_string(),
        uuid: opt(el, "uuid"),
        description,
        data_items: data_items_from(el),
        components: components_from(el),
    }
}

fn data_items_from(parent: &Element) -> Vec<DataItem> {
    let Some(list) = parent.child("DataItems") else {
        return Vec::new();
    };
    list.children_named("DataItem")
        .filter_map(|d| {
            let id = d.attr("id")?;
            let category = Category::parse(d.attr("category")?)?;
            Some(DataItem {
                id: id.to_string(),
                name: opt(d, "name"),
                category,
                data_type: d.attr("type").unwrap_or_default().to_string(),
                sub_type: opt(d, "subType"),
                units: opt(d, "units"),
                native_units: opt(d, "nativeUnits"),
                native_scale: opt(d, "nativeScale"),
                coordinate_system: opt(d, "coordinateSystem"),
                coordinate_system_id_ref: opt(d, "coordinateSystemIdRef"),
                composition_id: opt(d, "compositionId"),
                representation: opt(d, "representation"),
                statistic: opt(d, "statistic"),
                discrete: d.attr("discrete") == Some("true"),
                sample_rate: opt(d, "sampleRate"),
            })
        })
        .collect()
}

fn components_from(parent: &Element) -> Vec<Component> {
    let Some(list) = parent.child("Components") else {
        return Vec::new();
    };
    list.children
        .iter()
        .map(|c| Component {
            element: c.name.clone(),
            id: c.attr("id").unwrap_or_default().to_string(),
            name: opt(c, "name"),
            native_name: opt(c, "nativeName"),
            uuid: opt(c, "uuid"),
            data_items: data_items_from(c),
            components: components_from(c),
        })
        .collect()
}

// ── Streams ────────────────────────────────────────────────────────────────

fn streams_from(root: &Element) -> Streams {
    let mut devices = Vec::new();
    if let Some(streams) = root.child("Streams") {
        for ds in streams.children_named("DeviceStream") {
            let components = ds
                .children_named("ComponentStream")
                .map(|cs| ComponentStream {
                    component: cs.attr("component").unwrap_or_default().to_string(),
                    name: opt(cs, "name"),
                    component_id: opt(cs, "componentId"),
                    uuid: opt(cs, "uuid"),
                    observations: observations_from(cs),
                })
                .collect();
            devices.push(DeviceStream { name: opt(ds, "name"), uuid: opt(ds, "uuid"), components });
        }
    }
    Streams { header: header_from(root), devices }
}

fn observations_from(cs: &Element) -> Vec<Observation> {
    let mut out = Vec::new();
    for section in &cs.children {
        let category = match section.name.as_str() {
            "Samples" => Category::Sample,
            "Events" => Category::Event,
            "Condition" => Category::Condition,
            _ => continue,
        };
        out.extend(section.children.iter().filter_map(|el| observation_from(el, category)));
    }
    out
}

fn observation_from(el: &Element, category: Category) -> Option<Observation> {
    let data_item_id = el.attr("dataItemId")?.to_string();
    let sequence = el.attr("sequence").and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let text = el.trimmed_text();

    let (value, condition, element) = if category == Category::Condition {
        let level = ConditionLevel::from_element(&el.name)?;
        let message = Some(text.to_string()).filter(|t| !t.is_empty());
        let value = if level == ConditionLevel::Unavailable {
            Value::Unavailable
        } else {
            Value::Text(message.clone().unwrap_or_default())
        };
        let condition = Condition {
            level,
            native_code: opt(el, "nativeCode"),
            native_severity: opt(el, "nativeSeverity"),
            qualifier: opt(el, "qualifier"),
            message,
            condition_id: opt(el, "conditionId"),
        };
        (value, Some(condition), el.name.clone())
    } else {
        (value_from(el, category, text), None, el.name.clone())
    };

    Some(Observation {
        data_item_id,
        name: opt(el, "name"),
        element,
        category,
        data_item_type: opt(el, "type"),
        sub_type: opt(el, "subType"),
        timestamp: el.attr("timestamp").unwrap_or_default().to_string(),
        sequence,
        value,
        condition,
        composition_id: opt(el, "compositionId"),
        statistic: opt(el, "statistic"),
        duration: el.attr("duration").and_then(|s| s.trim().parse().ok()),
        reset_triggered: opt(el, "resetTriggered"),
    })
}

fn value_from(el: &Element, category: Category, text: &str) -> Value {
    let entries: Vec<DataSetEntry> = el.children_named("Entry").map(entry_from).collect();
    if !entries.is_empty() {
        return Value::DataSet(entries);
    }
    if text == "UNAVAILABLE" {
        return Value::Unavailable;
    }
    if el.attr("count").is_some() && text.is_empty() {
        return Value::DataSet(Vec::new());
    }
    if el.attr("sampleCount").is_some() {
        let parsed: Option<Vec<f64>> = text.split_whitespace().map(|t| t.parse().ok()).collect();
        if let Some(v) = parsed {
            return Value::TimeSeries(v);
        }
    }
    if category == Category::Sample
        && let Ok(n) = text.parse::<f64>()
    {
        return Value::Number(n);
    }
    Value::Text(text.to_string())
}

fn entry_from(e: &Element) -> DataSetEntry {
    DataSetEntry {
        key: e.attr("key").unwrap_or_default().to_string(),
        value: e.trimmed_text().to_string(),
        removed: e.attr("removed") == Some("true"),
        cells: e.children_named("Cell").map(entry_from).collect(),
    }
}

// ── Assets ─────────────────────────────────────────────────────────────────

fn assets_from(root: &Element, source: &str) -> AssetList {
    let mut assets = Vec::new();
    if let Some(list) = root.child("Assets") {
        for a in &list.children {
            assets.push(Asset {
                asset_type: a.name.clone(),
                asset_id: a.attr("assetId").unwrap_or_default().to_string(),
                device_uuid: opt(a, "deviceUuid"),
                timestamp: opt(a, "timestamp"),
                removed: a.attr("removed") == Some("true"),
                raw_xml: source.get(a.start..a.end).unwrap_or_default().to_string(),
            });
        }
    }
    AssetList { header: header_from(root), assets }
}

// ── Errors ─────────────────────────────────────────────────────────────────

fn errors_from(root: &Element) -> AgentErrors {
    let mut errors = Vec::new();
    if let Some(list) = root.child("Errors") {
        for e in &list.children {
            // 1.x/2.0: <Error errorCode="X">message</Error>
            // 2.x:     <OutOfRange errorCode="OUT_OF_RANGE"><ErrorMessage>message</ErrorMessage>...
            let Some(code) = e.attr("errorCode") else { continue };
            let message = match e.child("ErrorMessage") {
                Some(m) => m.trimmed_text().to_string(),
                None => e.trimmed_text().to_string(),
            };
            errors.push(AgentError { code: AgentErrorCode::from_code(code), message });
        }
    }
    AgentErrors { header: header_from(root), errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtconnect_1_3_streams_with_conditions() {
        // Hand-written in the MTConnect 1.3 document shape (not captured from an agent).
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<MTConnectStreams xmlns="urn:mtconnect.org:MTConnectStreams:1.3">
 <Header creationTime="2014-01-01T00:00:00Z" sender="agent" instanceId="7" bufferSize="100" version="1.3.0.19" nextSequence="12" firstSequence="1" lastSequence="11"/>
 <Streams><DeviceStream name="VMC" uuid="u1">
  <ComponentStream component="Linear" name="X" componentId="x">
   <Samples><Position dataItemId="xp" name="Xact" sequence="5" subType="ACTUAL" timestamp="2014-01-01T00:00:01Z">12.5</Position>
            <Position dataItemId="xp" sequence="6" timestamp="2014-01-01T00:00:02Z">UNAVAILABLE</Position></Samples>
   <Events><Execution dataItemId="ex" sequence="7" timestamp="2014-01-01T00:00:03Z">ACTIVE</Execution></Events>
   <Condition><Fault dataItemId="mc" type="MOTION_PROGRAM" nativeCode="E42" nativeSeverity="3" qualifier="HIGH" sequence="8" timestamp="2014-01-01T00:00:04Z">Axis &amp; servo fault</Fault>
              <Normal dataItemId="sc" type="SYSTEM" sequence="9" timestamp="2014-01-01T00:00:05Z"/>
              <Unavailable dataItemId="tc" type="TEMPERATURE" sequence="10" timestamp="2014-01-01T00:00:06Z"/></Condition>
  </ComponentStream></DeviceStream></Streams></MTConnectStreams>"#;
        let s = parse_streams(xml).unwrap();
        assert_eq!(s.header.instance_id, 7);
        assert_eq!(s.header.next_sequence, Some(12));
        let obs: Vec<_> = s.observations().collect();
        assert_eq!(obs.len(), 6);
        assert_eq!(obs[0].value, Value::Number(12.5));
        assert_eq!(obs[0].sub_type.as_deref(), Some("ACTUAL"));
        assert_eq!(obs[1].value, Value::Unavailable);
        assert_eq!(obs[2].value, Value::Text("ACTIVE".into()));
        let fault = obs[3].condition.as_ref().unwrap();
        assert_eq!(fault.level, ConditionLevel::Fault);
        assert_eq!(fault.native_code.as_deref(), Some("E42"));
        assert_eq!(fault.message.as_deref(), Some("Axis & servo fault"));
        assert_eq!(obs[3].data_item_type.as_deref(), Some("MOTION_PROGRAM"));
        assert_eq!(obs[4].condition.as_ref().unwrap().level, ConditionLevel::Normal);
        assert!(obs[5].is_unavailable());
    }

    #[test]
    fn prefixed_namespace_devices_and_old_error_shape() {
        let xml = r#"<m:MTConnectDevices xmlns:m="urn:mtconnect.org:MTConnectDevices:1.3"><m:Header instanceId="3" bufferSize="10"/>
 <m:Devices><m:Device id="d" name="Mill" uuid="U"><m:Description manufacturer="ACME" model="M1" serialNumber="9">desc</m:Description>
 <m:DataItems><m:DataItem category="EVENT" id="a" type="AVAILABILITY"/></m:DataItems>
 <m:Components><m:Axes id="ax" name="Axes"><m:Components><m:Linear id="x" name="X"><m:DataItems>
   <m:DataItem category="SAMPLE" id="xp" name="Xpos" type="POSITION" subType="ACTUAL" units="MILLIMETER" nativeUnits="MILLIMETER"/>
 </m:DataItems></m:Linear></m:Components></m:Axes></m:Components></m:Device></m:Devices></m:MTConnectDevices>"#;
        let d = parse_devices(xml).unwrap();
        let dev = &d.devices[0];
        assert_eq!(dev.description.as_ref().unwrap().manufacturer.as_deref(), Some("ACME"));
        let flat = dev.flatten_data_items();
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[1].0, "Axes/X");
        assert_eq!(flat[1].1.units.as_deref(), Some("MILLIMETER"));
        assert_eq!(d.data_item("xp").unwrap().sub_type.as_deref(), Some("ACTUAL"));

        let err = r#"<MTConnectError xmlns="urn:mtconnect.org:MTConnectError:1.3"><Header instanceId="3" bufferSize="10"/><Errors><Error errorCode="OUT_OF_RANGE">'from' out of range</Error></Errors></MTConnectError>"#;
        match parse_streams(err) {
            Err(e) => {
                assert!(e.is_out_of_range());
                assert!(e.to_string().contains("'from' out of range"));
            }
            Ok(_) => panic!("expected agent error"),
        }
    }

    #[test]
    fn data_set_and_time_series() {
        let xml = r#"<MTConnectStreams><Header instanceId="1"/><Streams><DeviceStream name="d"><ComponentStream component="Device">
<Events><Variable dataItemId="v" sequence="1" timestamp="t" count="2"><Entry key="a">1</Entry><Entry key="b" removed="true"/></Variable>
<Variable dataItemId="v2" sequence="2" timestamp="t" count="0">UNAVAILABLE</Variable></Events>
<Samples><Vibration dataItemId="ts" sequence="3" timestamp="t" sampleCount="3" sampleRate="100">1.5 2 3</Vibration></Samples>
</ComponentStream></DeviceStream></Streams></MTConnectStreams>"#;
        let s = parse_streams(xml).unwrap();
        let o: Vec<_> = s.observations().collect();
        match &o[0].value {
            Value::DataSet(e) => {
                assert_eq!(e.len(), 2);
                assert!(e[1].removed);
            }
            v => panic!("{v:?}"),
        }
        assert_eq!(o[1].value, Value::Unavailable);
        assert_eq!(o[2].value, Value::TimeSeries(vec![1.5, 2.0, 3.0]));
    }

    #[test]
    fn asset_raw_xml_is_exact_slice() {
        let xml = r#"<MTConnectAssets><Header instanceId="1" assetBufferSize="4" assetCount="1"/><Assets><CuttingTool assetId="T1" deviceUuid="u" timestamp="t"><Status>NEW</Status></CuttingTool><Asset assetId="F1" removed="true"/></Assets></MTConnectAssets>"#;
        let a = parse_assets(xml).unwrap();
        assert_eq!(a.assets.len(), 2);
        assert_eq!(a.assets[0].raw_xml, r#"<CuttingTool assetId="T1" deviceUuid="u" timestamp="t"><Status>NEW</Status></CuttingTool>"#);
        assert!(a.assets[1].removed);
        assert_eq!(a.header.asset_count, Some(1));
    }

    #[test]
    fn wrong_document_kind() {
        let xml = r#"<MTConnectAssets><Header instanceId="1"/><Assets/></MTConnectAssets>"#;
        assert!(matches!(parse_streams(xml), Err(MtConnectError::UnexpectedDocument { .. })));
        assert!(parse_document("<html/>").is_err());
    }
}
