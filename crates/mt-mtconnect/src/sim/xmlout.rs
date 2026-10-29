//! XML rendering of MTConnect response documents for the simulator.

use std::fmt::Write;

use crate::error::AgentErrorCode;
use crate::types::{Asset, Category, Component, DataItem, Device, Header, Observation, Value};

/// Schema version advertised in the document namespaces.
pub(crate) const SCHEMA_VERSION: &str = "2.0";

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

fn attr(out: &mut String, name: &str, value: &Option<String>) {
    if let Some(v) = value {
        let _ = write!(out, " {name}=\"{}\"", esc(v));
    }
}

fn root_open(out: &mut String, root: &str) {
    let ns = format!("urn:mtconnect.org:{root}:{SCHEMA_VERSION}");
    let _ = write!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root} xmlns:m=\"{ns}\" xmlns=\"{ns}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">"
    );
}

fn header(out: &mut String, h: &Header) {
    out.push_str("<Header");
    attr(out, "creationTime", &h.creation_time);
    attr(out, "sender", &h.sender);
    let _ = write!(out, " instanceId=\"{}\"", h.instance_id);
    attr(out, "version", &h.version);
    for (name, v) in [
        ("bufferSize", h.buffer_size),
        ("assetBufferSize", h.asset_buffer_size),
        ("assetCount", h.asset_count),
        ("nextSequence", h.next_sequence),
        ("firstSequence", h.first_sequence),
        ("lastSequence", h.last_sequence),
    ] {
        if let Some(v) = v {
            let _ = write!(out, " {name}=\"{v}\"");
        }
    }
    out.push_str("/>");
}

pub(crate) fn pascal(ty: &str) -> String {
    let (prefix, body) = match ty.split_once(':') {
        Some((p, b)) => (format!("{p}:"), b),
        None => (String::new(), ty),
    };
    let mut out = prefix;
    for word in body.split('_').filter(|w| !w.is_empty()) {
        let mut cs = word.chars();
        if let Some(f) = cs.next() {
            out.extend(f.to_uppercase());
            out.push_str(&cs.as_str().to_ascii_lowercase());
        }
    }
    out
}

fn data_item(out: &mut String, d: &DataItem) {
    out.push_str("<DataItem");
    let _ = write!(out, " category=\"{}\" id=\"{}\"", d.category.as_str(), esc(&d.id));
    attr(out, "name", &d.name);
    let _ = write!(out, " type=\"{}\"", esc(&d.data_type));
    attr(out, "subType", &d.sub_type);
    attr(out, "units", &d.units);
    attr(out, "nativeUnits", &d.native_units);
    attr(out, "nativeScale", &d.native_scale);
    attr(out, "coordinateSystem", &d.coordinate_system);
    attr(out, "coordinateSystemIdRef", &d.coordinate_system_id_ref);
    attr(out, "compositionId", &d.composition_id);
    attr(out, "representation", &d.representation);
    attr(out, "statistic", &d.statistic);
    attr(out, "sampleRate", &d.sample_rate);
    if d.discrete {
        out.push_str(" discrete=\"true\"");
    }
    out.push_str("/>");
}

fn data_items(out: &mut String, items: &[DataItem]) {
    if items.is_empty() {
        return;
    }
    out.push_str("<DataItems>");
    for d in items {
        data_item(out, d);
    }
    out.push_str("</DataItems>");
}

fn component(out: &mut String, c: &Component) {
    let _ = write!(out, "<{} id=\"{}\"", c.element, esc(&c.id));
    attr(out, "name", &c.name);
    attr(out, "nativeName", &c.native_name);
    attr(out, "uuid", &c.uuid);
    out.push('>');
    data_items(out, &c.data_items);
    components(out, &c.components);
    let _ = write!(out, "</{}>", c.element);
}

fn components(out: &mut String, cs: &[Component]) {
    if cs.is_empty() {
        return;
    }
    out.push_str("<Components>");
    for c in cs {
        component(out, c);
    }
    out.push_str("</Components>");
}

pub(crate) fn devices(h: &Header, device: &Device) -> String {
    let mut out = String::new();
    root_open(&mut out, "MTConnectDevices");
    header(&mut out, h);
    out.push_str("<Devices>");
    let _ = write!(out, "<Device id=\"{}\" name=\"{}\"", esc(&device.id), esc(&device.name));
    attr(&mut out, "uuid", &device.uuid);
    out.push('>');
    if let Some(d) = &device.description {
        out.push_str("<Description");
        attr(&mut out, "manufacturer", &d.manufacturer);
        attr(&mut out, "model", &d.model);
        attr(&mut out, "serialNumber", &d.serial_number);
        attr(&mut out, "station", &d.station);
        match &d.text {
            Some(t) => {
                let _ = write!(out, ">{}</Description>", esc(t));
            }
            None => out.push_str("/>"),
        }
    }
    data_items(&mut out, &device.data_items);
    components(&mut out, &device.components);
    out.push_str("</Device></Devices></MTConnectDevices>\n");
    out
}

/// The component an observation belongs to in a streams document.
pub(crate) struct Owner {
    pub element: String,
    pub id: String,
    pub name: Option<String>,
}

/// Observations of one component, already ordered by sequence.
pub(crate) struct OwnerObservations<'a> {
    pub owner: &'a Owner,
    pub observations: Vec<&'a Observation>,
}

fn observation(out: &mut String, o: &Observation) {
    let element = &o.element;
    let _ = write!(out, "<{element} dataItemId=\"{}\"", esc(&o.data_item_id));
    attr(out, "name", &o.name);
    let _ = write!(out, " sequence=\"{}\"", o.sequence);
    attr(out, "subType", &o.sub_type);
    let _ = write!(out, " timestamp=\"{}\"", esc(&o.timestamp));
    let text = match &o.condition {
        Some(c) => {
            attr(out, "type", &o.data_item_type);
            attr(out, "nativeCode", &c.native_code);
            attr(out, "nativeSeverity", &c.native_severity);
            attr(out, "qualifier", &c.qualifier);
            c.message.clone().unwrap_or_default()
        }
        None => match &o.value {
            Value::Number(n) => n.to_string(),
            other => other.to_text(),
        },
    };
    if text.is_empty() {
        out.push_str("/>");
    } else {
        let _ = write!(out, ">{}</{element}>", esc(&text));
    }
}

pub(crate) fn streams(h: &Header, device: &Device, groups: &[OwnerObservations<'_>]) -> String {
    let mut out = String::new();
    root_open(&mut out, "MTConnectStreams");
    header(&mut out, h);
    out.push_str("<Streams>");
    let _ = write!(out, "<DeviceStream name=\"{}\"", esc(&device.name));
    attr(&mut out, "uuid", &device.uuid);
    out.push('>');
    for g in groups.iter().filter(|g| !g.observations.is_empty()) {
        let _ = write!(out, "<ComponentStream component=\"{}\"", esc(&g.owner.element));
        attr(&mut out, "name", &g.owner.name);
        let _ = write!(out, " componentId=\"{}\">", esc(&g.owner.id));
        for (cat, section) in [
            (Category::Sample, "Samples"),
            (Category::Event, "Events"),
            (Category::Condition, "Condition"),
        ] {
            let mut it = g.observations.iter().filter(|o| o.category == cat).peekable();
            if it.peek().is_none() {
                continue;
            }
            let _ = write!(out, "<{section}>");
            for o in it {
                observation(&mut out, o);
            }
            let _ = write!(out, "</{section}>");
        }
        out.push_str("</ComponentStream>");
    }
    out.push_str("</DeviceStream></Streams></MTConnectStreams>\n");
    out
}

pub(crate) fn assets(h: &Header, assets: &[&Asset]) -> String {
    let mut out = String::new();
    root_open(&mut out, "MTConnectAssets");
    header(&mut out, h);
    out.push_str("<Assets>");
    for a in assets {
        out.push_str(&a.raw_xml);
    }
    out.push_str("</Assets></MTConnectAssets>\n");
    out
}

pub(crate) fn error(h: &Header, code: &AgentErrorCode, message: &str) -> String {
    let mut out = String::new();
    root_open(&mut out, "MTConnectError");
    header(&mut out, h);
    let _ = writeln!(
        out,
        "<Errors><Error errorCode=\"{}\">{}</Error></Errors></MTConnectError>",
        esc(code.as_code()),
        esc(message)
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_cases_types() {
        assert_eq!(pascal("PATH_FEEDRATE"), "PathFeedrate");
        assert_eq!(pascal("POSITION"), "Position");
        assert_eq!(pascal("x:CUSTOM_THING"), "x:CustomThing");
    }
}
