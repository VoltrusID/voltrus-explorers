//! Where the OPC UA stacks keep their certificates and keys.
//!
//! The host app calls [`set_root`] once at startup with a directory it owns (for the sandboxed Mac App Store
//! build: the app's data directory, which lives inside the container and survives restarts). Without it the
//! system temp directory is used, which is only meant for tests and quick local runs.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

const CLIENT_DIR: &str = "client";
const SERVER_PREFIX: &str = "server-";

/// Set the directory that holds all PKI folders. Only the first call has an effect.
pub fn set_root(dir: impl Into<PathBuf>) {
    let _ = ROOT.set(dir.into());
}

/// The configured PKI root, or `<temp dir>/opcua-explorer-pki` when none was set.
pub fn root() -> PathBuf {
    ROOT.get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("opcua-explorer-pki"))
}

/// PKI folder of the client (also used by discovery).
pub fn client_dir() -> PathBuf {
    root().join(CLIENT_DIR)
}

/// PKI folder of the built-in server listening on `port`.
pub fn server_dir(port: u16) -> PathBuf {
    root().join(format!("{SERVER_PREFIX}{port}"))
}

/// Every PKI folder that exists under the root: the client one plus each server one.
pub fn all_dirs() -> Vec<PathBuf> {
    dirs_in(&root())
}

fn dirs_in(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join(CLIENT_DIR)];
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut servers: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(SERVER_PREFIX))
            .map(|e| e.path())
            .collect();
        servers.sort();
        dirs.extend(servers);
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_client_and_server_dirs() {
        let root = std::env::temp_dir().join(format!("mt-opcua-pki-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("server-4840")).unwrap();
        std::fs::create_dir_all(root.join("server-4841")).unwrap();
        std::fs::create_dir_all(root.join("unrelated")).unwrap();
        let dirs = dirs_in(&root);
        assert_eq!(dirs, vec![root.join("client"), root.join("server-4840"), root.join("server-4841")]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
