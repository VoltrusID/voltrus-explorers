//! Sparkplug B payload protobuf types, generated at build time from the
//! vendored schema in `proto/sparkplug_b.proto` (Eclipse Tahu, EPL-2.0).
//!
//! prost names the generated module after the proto package
//! (`org.eclipse.tahu.protobuf`), so the include lands as `pb::Payload`,
//! `pb::payload::Metric`, `pb::payload::metric::Value`, `pb::DataType`, …

include!(concat!(env!("OUT_DIR"), "/org.eclipse.tahu.protobuf.rs"));
