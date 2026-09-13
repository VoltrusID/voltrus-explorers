//! Small quick-xml wrapper shared by both importers.

use quick_xml::Reader;
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};

/// Fatal XML problem; surfaces in the report as `context: "parse"`.
#[derive(Debug, thiserror::Error)]
pub enum XmlError {
    #[error("{0}")]
    Xml(#[from] quick_xml::Error),
    #[error("{0}")]
    Text(String),
}

/// A pulled event with namespace-insensitive, fully decoded access. L5X has
/// no namespaces; PLCopen files use a default namespace, and matching on
/// local names keeps the two parsers uniform.
#[derive(Debug)]
pub(crate) enum Ev {
    /// `<tag ...>` or `<tag ... />` (the latter is followed by `Ev::End`).
    Start(Start),
    End(String),
    /// Character data, with entity references already resolved and CDATA
    /// inlined.
    Text(String),
    Eof,
}

/// Element name plus attributes, all decoded and owned.
#[derive(Debug)]
pub(crate) struct Start {
    pub local: String,
    attrs: Vec<(String, String)>,
}

impl Start {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Stateful pull parser over one XML document.
pub(crate) struct PullParser<'a> {
    reader: Reader<&'a [u8]>,
    buf: Vec<u8>,
    /// Self-closing tags synthesize their `Ev::End` on the following pull.
    pending_end: Option<String>,
}

impl<'a> PullParser<'a> {
    pub fn new(source: &'a str) -> PullParser<'a> {
        let mut reader = Reader::from_str(source);
        reader.config_mut().trim_text(false);
        PullParser {
            reader,
            buf: Vec::new(),
            pending_end: None,
        }
    }

    pub fn next(&mut self) -> Result<Ev, XmlError> {
        if let Some(name) = self.pending_end.take() {
            return Ok(Ev::End(name));
        }
        match self.reader.read_event_into(&mut self.buf)? {
            Event::Start(e) => Ok(Ev::Start(owned_start(&e)?)),
            Event::Empty(e) => {
                let start = owned_start(&e)?;
                self.pending_end = Some(start.local.clone());
                Ok(Ev::Start(start))
            }
            Event::End(e) => Ok(Ev::End(local_name(e.name().as_ref()))),
            Event::Text(t) => Ok(Ev::Text(decode_text(&t)?)),
            Event::CData(t) => {
                let bytes: Cow<'_, [u8]> = t.into_inner();
                Ok(Ev::Text(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Event::GeneralRef(r) => Ok(Ev::Text(resolve_ref(&r)?)),
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => self.next(),
            Event::Eof => Ok(Ev::Eof),
        }
    }
}

use std::borrow::Cow;

fn owned_start(e: &BytesStart<'_>) -> Result<Start, XmlError> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| XmlError::Text(e.to_string()))?;
        attrs.push((
            local_name(a.key.as_ref()),
            a.unescape_value()
                .map_err(|e| XmlError::Text(e.to_string()))?
                .into_owned(),
        ));
    }
    Ok(Start {
        local: local_name(e.name().as_ref()),
        attrs,
    })
}

fn local_name(raw: &[u8]) -> String {
    let raw = std::str::from_utf8(raw).unwrap_or("");
    match raw.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => raw.to_string(),
    }
}

fn decode_text(t: &BytesText<'_>) -> Result<String, XmlError> {
    t.xml_content()
        .map(|c| c.into_owned())
        .map_err(|e| XmlError::Text(e.to_string()))
}

fn resolve_ref(r: &BytesRef<'_>) -> Result<String, XmlError> {
    let name = r
        .decode()
        .map_err(|e| XmlError::Text(e.to_string()))?
        .into_owned();
    Ok(match name.as_str() {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "apos" => "'".into(),
        "quot" => "\"".into(),
        other => match other.strip_prefix('#').and_then(|n| {
            let n = n.strip_prefix('x').or_else(|| n.strip_prefix('X'));
            match n {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => other[1..].parse::<u32>().ok(),
            }
        }) {
            Some(cp) => char::from_u32(cp).map(String::from).unwrap_or_default(),
            None => String::new(),
        },
    })
}
/// Trim leading/trailing XML whitespace (space, tab, CR, LF — not Unicode
/// spaces) while keeping interior newlines intact.
pub(crate) fn trim_xml_ws(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\r' | '\n'))
}

