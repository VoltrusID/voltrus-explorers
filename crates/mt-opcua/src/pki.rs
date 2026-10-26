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

/// The certificate's alternate names without resolving this computer's host name: `localhost`,
/// the loopback addresses and the bare host name. The OPC UA stack's own sample certificates
/// resolve the host name, which can stall for a long time where name lookup is slow.
fn local_names(application_uri: &str) -> opcua::crypto::X509Data {
    let mut addresses = vec!["127.0.0.1".to_owned(), "::1".to_owned()];
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    if !name.is_empty() {
        addresses.push(name);
    }
    let alt_host_names =
        opcua::crypto::X509Data::alt_host_names(application_uri, Some(addresses), true, false, false);
    opcua::crypto::X509Data {
        key_size: 2048,
        common_name: String::new(),
        organization: String::new(),
        organizational_unit: String::new(),
        country: "ID".to_owned(),
        state: String::new(),
        alt_host_names,
        certificate_duration_days: 3650,
    }
}

/// Make a certificate and private key at `certificate` and `private_key` (PEM) unless both
/// exist. Blocking: call it off the async workers.
pub fn ensure_certificate(
    certificate: &Path,
    private_key: &Path,
    common_name: &str,
    application_uri: &str,
) -> Result<(), String> {
    if certificate.exists() && private_key.exists() {
        return Ok(());
    }
    for path in [certificate, private_key] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
    }
    let mut data = local_names(application_uri);
    data.common_name = common_name.to_owned();
    data.organization = common_name.to_owned();
    data.organizational_unit = common_name.to_owned();
    opcua::crypto::CertificateStore::create_certificate_and_key(&data, true, certificate, private_key)
        .map(|_| ())
        .map_err(|e| format!("Cannot create the certificate: {e}"))
}

/// Make the application instance certificate of the PKI folder `pki` (the stack's own
/// `own/cert.der` and `private/private.pem`) unless it exists, off the async workers.
pub async fn ensure_own_certificate(pki: PathBuf, name: &str, application_uri: &str) -> Result<(), String> {
    let (name, uri) = (name.to_owned(), application_uri.to_owned());
    tokio::task::spawn_blocking(move || {
        ensure_certificate(&pki.join("own").join("cert.der"), &pki.join("private").join("private.pem"), &name, &uri)
    })
    .await
    .map_err(|e| format!("Certificate creation stopped: {e}"))?
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
