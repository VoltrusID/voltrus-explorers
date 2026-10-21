//! Runtime protobuf schema registry over [`prost_reflect::DescriptorPool`].
//!
//! [`SchemaPool`] compiles `.proto` source at runtime (via [`protox`]), merges
//! it into a descriptor pool, and maps NATS subject patterns to fully-qualified
//! message names so payloads can be decoded by [`crate::decode`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use prost_reflect::prost_types::FileDescriptorSet;

use crate::nats_subject_match;

/// A session-lived registry of `.proto` sources plus subject→message bindings.
#[derive(Debug, Default, Clone)]
pub struct SchemaPool {
    pool: DescriptorPool,
    /// Subject wildcard pattern → fully-qualified message name.
    bindings: HashMap<String, String>,
}

impl SchemaPool {
    pub fn new() -> Self {
        Self {
            pool: DescriptorPool::new(),
            bindings: HashMap::new(),
        }
    }

    /// Compile raw `.proto` source (written to a temp file so [`protox::compile`]
    /// can parse it) and merge into the pool.
    pub fn add_proto_source(&mut self, src: &str) -> Result<(), String> {
        let path = write_temp_proto(src)?;
        // protox requires the source file's directory to be on the include path.
        let includes: Vec<PathBuf> = path.parent().into_iter().map(|p| p.to_path_buf()).collect();
        let fds = protox::compile(std::iter::once(path), includes.into_iter())
            .map_err(|e| e.to_string())?;
        self.merge(fds)
    }

    /// Compile a `.proto` file from disk and merge into the pool. Its parent
    /// directory is added as an include path so relative imports resolve.
    pub fn add_proto_file(&mut self, path: &Path) -> Result<(), String> {
        let includes: Vec<PathBuf> = path.parent().map(|p| vec![p.to_path_buf()]).unwrap_or_default();
        let fds = protox::compile(std::iter::once(path.to_path_buf()), includes.into_iter())
            .map_err(|e| e.to_string())?;
        self.merge(fds)
    }

    fn merge(&mut self, fds: FileDescriptorSet) -> Result<(), String> {
        self.pool.add_file_descriptor_set(fds).map_err(|e| e.to_string())
    }

    /// Bind a NATS subject wildcard pattern to a fully-qualified message name.
    pub fn bind(&mut self, pattern: String, message_full_name: String) {
        self.bindings.insert(pattern, message_full_name);
    }

    /// Remove a binding. Returns `true` if one was present.
    pub fn unbind(&mut self, pattern: &str) -> bool {
        self.bindings.remove(pattern).is_some()
    }

    /// All subject→message bindings.
    pub fn bindings(&self) -> &HashMap<String, String> {
        &self.bindings
    }

    /// Fully-qualified names of every message known to the pool (for the
    /// binding dropdown).
    pub fn message_names(&self) -> Vec<String> {
        self.pool
            .all_messages()
            .map(|m| m.full_name().to_string())
            .collect()
    }

    /// Resolve the bound message descriptor for a NATS subject (wildcard match
    /// against all bindings, first match wins).
    pub fn resolve(&self, subject: &str) -> Option<MessageDescriptor> {
        for (pattern, full_name) in &self.bindings {
            if nats_subject_match(pattern, subject) {
                if let Some(desc) = self.pool.get_message_by_name(full_name) {
                    return Some(desc);
                }
            }
        }
        None
    }

    /// Decode bytes as the named message → [`DynamicMessage`] (serde-serializable
    /// to proto-JSON when the `serde` feature is on).
    pub fn decode_message(&self, full_name: &str, bytes: &[u8]) -> Result<DynamicMessage, String> {
        let desc = self
            .pool
            .get_message_by_name(full_name)
            .ok_or_else(|| format!("message '{}' not found in schema pool", full_name))?;
        DynamicMessage::decode(desc, bytes).map_err(|e| e.to_string())
    }

    /// Decode bytes using the subject's binding (wildcard match).
    pub fn decode_for_subject(&self, subject: &str, bytes: &[u8]) -> Result<DynamicMessage, String> {
        let desc = self
            .resolve(subject)
            .ok_or_else(|| format!("no schema binding matches subject '{}'", subject))?;
        DynamicMessage::decode(desc, bytes).map_err(|e| e.to_string())
    }
}

/// Write `.proto` source to a unique temp file and return its path.
fn write_temp_proto(src: &str) -> Result<PathBuf, String> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    use std::io::Write;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!("mt-nats-proto-{}-{}.proto", std::process::id(), n));
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    f.write_all(src.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}
