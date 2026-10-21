//! Generic protobuf decoding against a user-supplied `.proto` schema.
//!
//! The schema text is compiled in-process with `protox` (no `protoc` needed)
//! and payloads are decoded with `prost-reflect` into JSON. Only one source
//! file is supported; `import` works for the Google well-known types
//! (`google/protobuf/timestamp.proto` and friends) but not for other files.

use prost_reflect::{DescriptorPool, DynamicMessage, SerializeOptions};
use protox::Compiler;
use protox::file::{ChainFileResolver, File, FileResolver, GoogleFileResolver};
use serde_json::Value;

const SCHEMA_FILE: &str = "schema.proto";

/// Serves the single pasted schema as `schema.proto`.
struct PastedSource(String);

impl FileResolver for PastedSource {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        if name == SCHEMA_FILE {
            File::from_source(SCHEMA_FILE, &self.0)
        } else {
            Err(protox::Error::file_not_found(name))
        }
    }
}

/// A compiled schema: the descriptor pool plus the message types the user
/// can pick from.
#[derive(Clone)]
pub struct ProtoSchema {
    pool: DescriptorPool,
    messages: Vec<String>,
}

impl std::fmt::Debug for ProtoSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtoSchema").field("messages", &self.messages).finish()
    }
}

impl ProtoSchema {
    /// Compile `.proto` source text. Errors carry the parser's line and
    /// column information.
    pub fn compile(source: &str) -> Result<Self, String> {
        let mut resolver = ChainFileResolver::new();
        resolver.add(PastedSource(source.to_string()));
        resolver.add(GoogleFileResolver::new());

        let mut compiler = Compiler::with_file_resolver(resolver);
        compiler
            .open_file(SCHEMA_FILE)
            .map_err(|e| format!("{e}"))?;
        let pool = compiler.descriptor_pool();

        let mut messages: Vec<String> = pool
            .all_messages()
            .filter(|m| m.parent_file().name() == SCHEMA_FILE && !m.is_map_entry())
            .map(|m| m.full_name().to_string())
            .collect();
        messages.sort();
        if messages.is_empty() {
            return Err("the schema defines no messages".into());
        }
        Ok(Self { pool, messages })
    }

    /// Fully qualified names of the messages defined in the schema, sorted.
    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    /// Decode `bytes` as message type `message` (fully qualified name) into
    /// JSON. Field names follow the schema; 64-bit integers stay numbers and
    /// default-valued fields are included so zeros are visible.
    pub fn decode(&self, message: &str, bytes: &[u8]) -> Result<Value, String> {
        let desc = self
            .pool
            .get_message_by_name(message)
            .ok_or_else(|| format!("message type {message} is not in the schema"))?;
        let msg = DynamicMessage::decode(desc, bytes).map_err(|e| format!("decode as {message} failed: {e}"))?;
        let options = SerializeOptions::new()
            .use_proto_field_name(true)
            .stringify_64_bit_integers(false)
            .skip_default_fields(false);
        msg.serialize_with_options(serde_json::value::Serializer, &options)
            .map_err(|e| format!("could not render {message}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"
        syntax = "proto3";
        package acme.telemetry;

        message Reading {
            string sensor = 1;
            double value = 2;
            int64 at_ms = 3;
            Unit unit = 4;
            repeated int32 samples = 5;
            map<string, string> tags = 6;
        }

        enum Unit {
            UNIT_UNSPECIFIED = 0;
            CELSIUS = 1;
        }

        message Envelope {
            Reading reading = 1;
        }
    "#;

    fn schema() -> ProtoSchema {
        ProtoSchema::compile(SCHEMA).expect("schema compiles")
    }

    /// Hand-encoded `Reading{sensor:"t1", value:21.5, at_ms:1700000000000, unit:CELSIUS, samples:[1,2]}`.
    fn reading_bytes() -> Vec<u8> {
        let mut b = vec![0x0a, 0x02, b't', b'1'];
        b.push(0x11);
        b.extend(21.5f64.to_le_bytes());
        b.push(0x18);
        // varint of 1_700_000_000_000
        let mut n = 1_700_000_000_000u64;
        while n >= 0x80 {
            b.push((n as u8 & 0x7f) | 0x80);
            n >>= 7;
        }
        b.push(n as u8);
        b.extend([0x20, 0x01]);
        b.extend([0x2a, 0x02, 0x01, 0x02]); // packed samples
        b
    }

    #[test]
    fn lists_messages_without_map_entries() {
        assert_eq!(
            schema().messages(),
            ["acme.telemetry.Envelope", "acme.telemetry.Reading"]
        );
    }

    #[test]
    fn decodes_to_json_with_schema_names() {
        let v = schema().decode("acme.telemetry.Reading", &reading_bytes()).unwrap();
        assert_eq!(v["sensor"], "t1");
        assert_eq!(v["value"], 21.5);
        assert_eq!(v["at_ms"], 1_700_000_000_000u64);
        assert_eq!(v["unit"], "CELSIUS");
        assert_eq!(v["samples"], serde_json::json!([1, 2]));
    }

    #[test]
    fn nested_messages_and_default_fields() {
        let mut env = vec![0x0a, reading_bytes().len() as u8];
        env.extend(reading_bytes());
        let v = schema().decode("acme.telemetry.Envelope", &env).unwrap();
        assert_eq!(v["reading"]["sensor"], "t1");
        // Unset fields still show with their default.
        let empty = schema().decode("acme.telemetry.Reading", &[]).unwrap();
        assert_eq!(empty["value"], 0.0);
        assert_eq!(empty["unit"], "UNIT_UNSPECIFIED");
    }

    #[test]
    fn bad_payload_and_bad_message_name_are_errors() {
        let s = schema();
        // Field 1 (string) declared 200 bytes long but only 1 follows.
        let err = s.decode("acme.telemetry.Reading", &[0x0a, 0xc8, 0x01, b'x']).unwrap_err();
        assert!(err.contains("decode as acme.telemetry.Reading failed"), "{err}");
        let err = s.decode("acme.telemetry.Nope", &[]).unwrap_err();
        assert!(err.contains("not in the schema"), "{err}");
    }

    #[test]
    fn syntax_errors_are_reported() {
        let err = ProtoSchema::compile("syntax = \"proto3\"; message A { string x = ; }").unwrap_err();
        assert!(!err.is_empty());
        let err = ProtoSchema::compile("syntax = \"proto3\";").unwrap_err();
        assert!(err.contains("no messages"), "{err}");
    }

    #[test]
    fn well_known_imports_resolve_but_other_files_do_not() {
        let ok = ProtoSchema::compile(
            r#"syntax = "proto3";
               import "google/protobuf/timestamp.proto";
               message Event { google.protobuf.Timestamp at = 1; }"#,
        )
        .unwrap();
        // Timestamp{seconds: 5}
        let v = ok.decode("Event", &[0x0a, 0x02, 0x08, 0x05]).unwrap();
        assert!(v["at"].is_string() || v["at"].is_object(), "{v}");

        let err = ProtoSchema::compile(r#"syntax = "proto3"; import "other.proto"; message A {}"#).unwrap_err();
        assert!(err.contains("other.proto"), "{err}");
    }
}
