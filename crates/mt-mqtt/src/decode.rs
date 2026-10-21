//! Payload decoding for received MQTT messages.
//!
//! [`PayloadDecoder`] looks at the topic of each message and decides how to
//! decode it:
//!
//! - topics in the `spBv1.0/` namespace are decoded as Sparkplug B,
//! - topics that match a user rule are decoded with the user's `.proto`
//!   schema,
//! - everything else is left alone (`None`).
//!
//! One decoder is kept per connection because Sparkplug aliases are learned
//! per session. The protobuf schema and rules are shared by all of them.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::protodec::ProtoSchema;
use crate::sparkplug::{SparkplugAliases, SparkplugTopic, decode_payload};

/// Which decoder produced a [`Decoded`] result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeKind {
    Sparkplug,
    Protobuf,
}

impl DecodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DecodeKind::Sparkplug => "sparkplug",
            DecodeKind::Protobuf => "protobuf",
        }
    }
}

/// Outcome of decoding one payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub kind: DecodeKind,
    pub result: Result<Value, String>,
}

impl Decoded {
    /// `{ "kind": "sparkplug", "value": ... }` or `{ "kind": ..., "error": "..." }`.
    pub fn to_json(&self) -> Value {
        match &self.result {
            Ok(v) => json!({ "kind": self.kind.as_str(), "value": v }),
            Err(e) => json!({ "kind": self.kind.as_str(), "error": e }),
        }
    }
}

/// Maps a topic filter to a message type in the user's schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoRule {
    pub filter: String,
    /// Fully qualified message name, e.g. `acme.telemetry.Reading`.
    pub message: String,
}

/// The user's schema and topic rules.
#[derive(Debug, Default, Clone)]
pub struct ProtoConfig {
    schema: Option<ProtoSchema>,
    rules: Vec<ProtoRule>,
}

/// Shared between every decoder.
pub type SharedProtoConfig = Arc<RwLock<ProtoConfig>>;

impl ProtoConfig {
    /// Replace the schema and rules. Blank `source` clears both. Returns the
    /// message names defined in the schema. On error the old config is kept.
    pub fn set(&mut self, source: &str, rules: Vec<ProtoRule>) -> Result<Vec<String>, String> {
        if source.trim().is_empty() {
            *self = Self::default();
            return Ok(Vec::new());
        }
        let schema = ProtoSchema::compile(source)?;
        for r in &rules {
            validate_filter(&r.filter)?;
            if !schema.messages().iter().any(|m| m == &r.message) {
                return Err(format!("rule for {}: message {} is not in the schema", r.filter, r.message));
            }
        }
        let names = schema.messages().to_vec();
        self.schema = Some(schema);
        self.rules = rules;
        Ok(names)
    }

    fn message_for(&self, topic: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| topic_matches(&r.filter, topic))
            .map(|r| r.message.as_str())
    }
}

/// A topic filter is valid when `#` is only the last level and `+`/`#` take a
/// whole level.
pub fn validate_filter(filter: &str) -> Result<(), String> {
    if filter.is_empty() {
        return Err("topic filter is empty".into());
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (i, l) in levels.iter().enumerate() {
        if (l.contains('#') && (*l != "#" || i != levels.len() - 1)) || (l.contains('+') && *l != "+") {
            return Err(format!("invalid topic filter: {filter}"));
        }
    }
    Ok(())
}

/// MQTT topic filter matching (`+` one level, `#` the rest). Like a broker,
/// wildcards in the first level do not match `$`-prefixed topics.
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && (filter.starts_with('#') || filter.starts_with('+')) {
        return false;
    }
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Per-connection payload decoder.
pub struct PayloadDecoder {
    proto: SharedProtoConfig,
    aliases: SparkplugAliases,
}

impl PayloadDecoder {
    pub fn new(proto: SharedProtoConfig) -> Self {
        Self { proto, aliases: SparkplugAliases::new() }
    }

    /// Decode `payload` if the topic is Sparkplug B or matches a rule.
    /// Empty payloads (retained-message clears) are never decoded.
    pub fn decode(&mut self, topic: &str, payload: &[u8]) -> Option<Decoded> {
        if payload.is_empty() {
            return None;
        }
        if let Some(sp) = SparkplugTopic::parse(topic) {
            // STATE messages are JSON; the JSON view handles them.
            if sp.message_type == "STATE" {
                return None;
            }
            return Some(Decoded {
                kind: DecodeKind::Sparkplug,
                result: decode_payload(&sp, payload, &mut self.aliases),
            });
        }
        let cfg = self.proto.read();
        let message = cfg.message_for(topic)?;
        let schema = cfg.schema.as_ref()?;
        Some(Decoded {
            kind: DecodeKind::Protobuf,
            result: schema.decode(message, payload),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparkplug::pb;
    use prost::Message;

    fn shared(source: &str, rules: &[(&str, &str)]) -> SharedProtoConfig {
        let cfg = Arc::new(RwLock::new(ProtoConfig::default()));
        cfg.write()
            .set(
                source,
                rules
                    .iter()
                    .map(|(f, m)| ProtoRule { filter: f.to_string(), message: m.to_string() })
                    .collect(),
            )
            .unwrap();
        cfg
    }

    const SCHEMA: &str = r#"syntax = "proto3"; package p; message Reading { string sensor = 1; }"#;

    #[test]
    fn filter_matching() {
        assert!(topic_matches("a/b", "a/b"));
        assert!(!topic_matches("a/b", "a/b/c"));
        assert!(topic_matches("a/+/c", "a/x/c"));
        assert!(!topic_matches("a/+/c", "a/x/y"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(topic_matches("a/#", "a"));
        assert!(topic_matches("#", "x/y"));
        assert!(!topic_matches("#", "$SYS/uptime"));
        assert!(!topic_matches("+/x", "$SYS/x"));
        assert!(topic_matches("$SYS/#", "$SYS/uptime"));
    }

    #[test]
    fn filter_validation() {
        assert!(validate_filter("a/+/b/#").is_ok());
        assert!(validate_filter("").is_err());
        assert!(validate_filter("a/#/b").is_err());
        assert!(validate_filter("a/b+").is_err());
    }

    #[test]
    fn sparkplug_topics_decode_without_any_config() {
        let mut d = PayloadDecoder::new(Arc::default());
        let payload = pb::Payload { seq: Some(9), ..Default::default() }.encode_to_vec();
        let out = d.decode("spBv1.0/g/NDATA/e", &payload).expect("decoded");
        assert_eq!(out.kind, DecodeKind::Sparkplug);
        assert_eq!(out.result.unwrap()["seq"], 9);
    }

    #[test]
    fn sparkplug_state_and_empty_payloads_are_skipped() {
        let mut d = PayloadDecoder::new(Arc::default());
        assert!(d.decode("spBv1.0/STATE/host", br#"{"online":true}"#).is_none());
        assert!(d.decode("spBv1.0/g/NDATA/e", b"").is_none());
    }

    #[test]
    fn bad_sparkplug_payload_reports_an_error_result() {
        let mut d = PayloadDecoder::new(Arc::default());
        let out = d.decode("spBv1.0/g/NDATA/e", b"plain text").expect("attempted");
        assert_eq!(out.kind, DecodeKind::Sparkplug);
        let json = out.to_json();
        assert_eq!(json["kind"], "sparkplug");
        assert!(json["error"].as_str().unwrap().contains("not a Sparkplug B payload"));
    }

    #[test]
    fn unmatched_topics_are_left_alone() {
        let mut d = PayloadDecoder::new(shared(SCHEMA, &[("sensors/#", "p.Reading")]));
        assert!(d.decode("other/topic", b"\x0a\x01x").is_none());
        let out = d.decode("sensors/a", b"\x0a\x02t1").expect("decoded");
        assert_eq!(out.kind, DecodeKind::Protobuf);
        assert_eq!(out.result.unwrap()["sensor"], "t1");
    }

    #[test]
    fn rules_apply_in_order_and_schema_swaps_take_effect() {
        let cfg = shared(
            r#"syntax = "proto3"; package p; message A { string a = 1; } message B { string b = 1; }"#,
            &[("t/special", "p.B"), ("t/#", "p.A")],
        );
        let mut d = PayloadDecoder::new(cfg.clone());
        assert_eq!(d.decode("t/special", b"\x0a\x01x").unwrap().result.unwrap()["b"], "x");
        assert_eq!(d.decode("t/other", b"\x0a\x01x").unwrap().result.unwrap()["a"], "x");

        cfg.write().set("", vec![]).unwrap();
        assert!(d.decode("t/other", b"\x0a\x01x").is_none());
    }

    #[test]
    fn invalid_config_is_rejected_and_old_one_kept() {
        let cfg = shared(SCHEMA, &[("a/#", "p.Reading")]);
        let mut w = cfg.write();
        assert!(w.set(SCHEMA, vec![ProtoRule { filter: "a/#".into(), message: "p.Missing".into() }]).is_err());
        assert!(w.set(SCHEMA, vec![ProtoRule { filter: "a/#/b".into(), message: "p.Reading".into() }]).is_err());
        assert!(w.set("not a proto file", vec![]).is_err());
        assert_eq!(w.message_for("a/x"), Some("p.Reading"));
    }
}
