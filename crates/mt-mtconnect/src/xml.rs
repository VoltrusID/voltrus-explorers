//! Minimal namespace-agnostic XML tree on top of quick-xml.
//!
//! Element and attribute names are reduced to their local names, so documents parse the same
//! whether the namespace is `urn:mtconnect.org:MTConnectStreams:1.3`, `...:2.8`, or bound to a
//! prefix (`<m:MTConnectStreams>`).

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::error::MtConnectError;

#[derive(Debug, Clone, Default)]
pub(crate) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// Concatenated character data of this element (not of its descendants).
    pub text: String,
    /// Byte range of the element in the source document.
    pub start: usize,
    pub end: usize,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn trimmed_text(&self) -> &str {
        self.text.trim()
    }
}

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.into_owned(),
    }
}

fn start_element(e: &BytesStart<'_>, start: usize) -> Result<Element, MtConnectError> {
    let mut el = Element { name: local(e.name().as_ref()), start, ..Default::default() };
    for attr in e.attributes() {
        let attr = attr.map_err(|err| MtConnectError::Xml(err.to_string()))?;
        let value = attr
            .unescape_value()
            .map_err(|err| MtConnectError::Xml(err.to_string()))?
            .into_owned();
        el.attrs.push((local(attr.key.as_ref()), value));
    }
    Ok(el)
}

fn resolve_ref(name: &str) -> Option<String> {
    match name {
        "amp" => Some("&".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        "quot" => Some("\"".into()),
        "apos" => Some("'".into()),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code).map(String::from)
        }
    }
}

/// Parse `xml` into an element tree rooted at the document element.
pub(crate) fn parse(xml: &str) -> Result<Element, MtConnectError> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;

    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| MtConnectError::Xml(e.to_string()))?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(e) => stack.push(start_element(&e, before)?),
            Event::Empty(e) => {
                let mut el = start_element(&e, before)?;
                el.end = after;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let mut el = stack
                    .pop()
                    .ok_or_else(|| MtConnectError::Xml("unbalanced end tag".into()))?;
                el.end = after;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    let text = t.decode().map_err(|e| MtConnectError::Xml(e.to_string()))?;
                    top.text.push_str(&text);
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    let text = t.decode().map_err(|e| MtConnectError::Xml(e.to_string()))?;
                    top.text.push_str(&text);
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    let name = r.decode().map_err(|e| MtConnectError::Xml(e.to_string()))?;
                    match resolve_ref(&name) {
                        Some(s) => top.text.push_str(&s),
                        None => {
                            return Err(MtConnectError::Xml(format!("unknown entity &{name};")));
                        }
                    }
                }
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
        }
    }

    if !stack.is_empty() {
        return Err(MtConnectError::Xml("unexpected end of document".into()));
    }
    root.ok_or_else(|| MtConnectError::Xml("empty document".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_namespaces_and_entities() {
        let xml = r#"<?xml version="1.0"?><m:A xmlns:m="urn:x" m:k="a&amp;b"><m:B>1 &lt; 2 &#65;</m:B><C/></m:A>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "A");
        assert_eq!(root.attr("k"), Some("a&b"));
        assert_eq!(root.child("B").unwrap().text, "1 < 2 A");
        assert_eq!(&xml[root.child("B").unwrap().start..root.child("B").unwrap().end], "<m:B>1 &lt; 2 &#65;</m:B>");
        assert!(root.child("C").is_some());
    }

    #[test]
    fn malformed_is_error() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("").is_err());
    }
}
