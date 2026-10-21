//! Payload format detection and decoding.
//!
//! Pure functions (no [`crate::NatsClient`] needed) the app uses to render
//! message payloads as pretty JSON, decoded MessagePack, decoded protobuf
//! (against a [`crate::schema::SchemaPool`]), or a hex/ASCII dump.

use serde::{Deserialize, Serialize};

use crate::schema::SchemaPool;

/// Detected payload format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PayloadFormat {
    /// Printable ASCII/UTF-8 text that is not JSON.
    Utf8,
    /// Valid JSON.
    Json,
    /// MessagePack.
    MsgPack,
    /// Protobuf (requires a bound schema to decode).
    Protobuf,
    /// Hex/ASCII dump view.
    Hex,
    /// Arbitrary binary.
    Binary,
}

/// Heuristically detect a payload's format.
///
/// Order: empty → binary; `{`/`[` and parses as JSON → JSON; MessagePack
/// fixmap/fixarray/array16-map16 lead byte and parses → MsgPack; all-printable
/// → UTF-8; otherwise Binary. (Protobuf is never auto-detected — there is no
/// reliable wire-level marker; it is only attempted when a subject is bound.)
pub fn detect(bytes: &[u8]) -> PayloadFormat {
    if bytes.is_empty() {
        return PayloadFormat::Binary;
    }
    let lead = bytes
        .iter()
        // skip ascii whitespace
        .find(|&&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .copied()
        .unwrap_or(bytes[0]);

    if matches!(lead, b'{' | b'[') && serde_json::from_slice::<serde_json::Value>(bytes).is_ok() {
        return PayloadFormat::Json;
    }
    if matches!(lead, 0x80..=0x8f | 0x90..=0x9f | 0xdc..=0xdf)
        && rmp_serde::from_slice::<serde_json::Value>(bytes).is_ok()
    {
        return PayloadFormat::MsgPack;
    }
    if bytes
        .iter()
        .all(|&b| matches!(b, b'\n' | b'\r' | b'\t') || (0x20..=0x7e).contains(&b))
    {
        return PayloadFormat::Utf8;
    }
    PayloadFormat::Binary
}

/// Pretty-print valid JSON bytes. Returns the original error on parse failure.
pub fn pretty_json(bytes: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// Decode MessagePack bytes to pretty-printed JSON.
pub fn msgpack_to_json(bytes: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = rmp_serde::from_slice(bytes).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// Classic offset | hex | ascii dump (16 bytes/row).
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let off = i * 16;
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| if (0x20..=0x7e).contains(&b) { b as char } else { '.' })
            .collect();
        out.push_str(&format!("{:08x}  {:<48}  {}\n", off, hex.join(" "), ascii));
    }
    out
}

/// Decode a protobuf payload against a named message in the schema pool and
/// return pretty-printed proto-JSON.
pub fn proto_decode(bytes: &[u8], pool: &SchemaPool, message_full_name: &str) -> Result<String, String> {
    let msg = pool.decode_message(message_full_name, bytes)?;
    serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())
}
