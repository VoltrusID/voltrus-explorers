//! OPC UA sessions over `mt-opcua`: a client that signs in with the endpoint's security policy and
//! mode, browses the address space on request, monitors variables in one subscription, writes
//! typed values and calls methods; a plant server simulator; and the GetEndpoints behind the
//! editor's endpoint picker.

pub(crate) mod client;
pub(crate) mod plant;
pub(crate) mod server;

use std::path::{Path, PathBuf};
use std::time::Duration;

use mt_opcua::OpcUaProtocolClient;
use vt_protocol::{Direction, OpcuaEndpoint, OpcuaSecurityMode, OpcuaSecurityPolicy, TrafficFrame};

use crate::emit::now_ms;

/// Longest a GetEndpoints waits for the server.
const DISCOVERY_WAIT: Duration = Duration::from_secs(10);

/// Keep the OPC UA certificates (the client's and each simulator's application instance
/// certificate, trusted and rejected peers, simulator user certificates) under `dir`. Only the
/// first call has an effect; without one the engine uses `<data dir>/Voltrus Tools/opcua-pki`,
/// which the macOS sandbox places inside the app's container.
pub fn set_pki_root(dir: PathBuf) {
    mt_opcua::pki::set_root(dir);
}

/// Copy a user certificate or private key the user picked into the PKI root's `users` folder,
/// readable by this user only, and return the copy's path. The sandbox's permission to read a
/// file chosen in the open panel ends with the run; the copy, inside the container, survives it.
pub fn import_user_file(source: &Path) -> std::io::Result<PathBuf> {
    init_pki();
    import_user_file_into(&mt_opcua::pki::root(), source)
}

fn import_user_file_into(root: &Path, source: &Path) -> std::io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    use std::io::Write;

    let bytes = std::fs::read(source)?;
    let users = root.join("users");
    std::fs::create_dir_all(&users)?;
    // Same-named files from different folders keep their own copies.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("user");
    let name = match source.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!("{stem}-{:016x}.{ext}", hasher.finish()),
        None => format!("{stem}-{:016x}", hasher.finish()),
    };
    let copy = users.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let mut file = options.open(&copy)?;
        // A copy made before keeps whatever mode it had; this one is private either way.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
    }
    #[cfg(not(unix))]
    options.open(&copy)?.write_all(&bytes)?;
    Ok(copy)
}

/// Settle where certificates live before the first OPC UA session touches them.
fn init_pki() {
    if let Some(data) = dirs::data_dir() {
        mt_opcua::pki::set_root(data.join("Voltrus Tools").join("opcua-pki"));
    }
}

pub(crate) fn traffic(
    direction: Direction,
    summary: impl Into<String>,
    detail: impl Into<String>,
) -> TrafficFrame {
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary: summary.into(),
        detail: detail.into(),
        bytes: 0,
    }
}

/// `hh:mm:ss.mmm` (UTC) of a Unix-millisecond time.
pub(crate) fn clock(at_ms: u64) -> String {
    let seconds = at_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (seconds / 3600) % 24,
        (seconds / 60) % 60,
        seconds % 60,
        at_ms % 1000
    )
}

/// Answer an [`vt_protocol::OpcuaRequest::Endpoints`]: the server's endpoints, most secure first.
pub(crate) async fn endpoints(url: String) -> Result<Vec<OpcuaEndpoint>, String> {
    init_pki();
    if vt_protocol::opcua_host_port(&url).is_none() {
        return Err(format!("“{url}” is not an opc.tcp:// address"));
    }
    // The client's certificate, made before the wait starts: a new key can take longer than it.
    OpcUaProtocolClient::prepare_certificate().await?;
    let found = tokio::time::timeout(
        DISCOVERY_WAIT,
        OpcUaProtocolClient::get_endpoints(url.trim()),
    )
    .await
    .map_err(|_| format!("{url} did not answer within 10 s"))??;
    let mut endpoints: Vec<OpcuaEndpoint> = found
        .into_iter()
        .map(|endpoint| OpcuaEndpoint {
            policy: OpcuaSecurityPolicy::from_uri(&endpoint.security_policy_uri),
            mode: OpcuaSecurityMode::from_key(&endpoint.security_mode)
                .unwrap_or(OpcuaSecurityMode::None),
            url: endpoint.url,
            policy_uri: endpoint.security_policy_uri,
            security_level: endpoint.security_level,
            user_tokens: endpoint.user_token_policies,
        })
        .collect();
    endpoints.sort_by(|a, b| b.security_level.cmp(&a.security_level));
    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::{clock, import_user_file_into};

    #[test]
    fn picked_user_files_are_copied_private_into_the_pki() {
        let dir = std::env::temp_dir().join(format!(
            "vt-opcua-import-{}-{}",
            std::process::id(),
            super::now_ms()
        ));
        let picked = dir.join("picked");
        std::fs::create_dir_all(&picked).unwrap();
        let source = picked.join("operator.pem");
        std::fs::write(&source, b"-----BEGIN PRIVATE KEY-----").unwrap();
        let root = dir.join("pki");

        let copy = import_user_file_into(&root, &source).unwrap();
        assert!(copy.starts_with(root.join("users")), "{}", copy.display());
        assert_eq!(copy.extension().and_then(|e| e.to_str()), Some("pem"));
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            b"-----BEGIN PRIVATE KEY-----"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&copy).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The copy outlives the picked file.
        std::fs::remove_dir_all(&picked).unwrap();
        assert!(copy.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clocks_read_utc() {
        // 2026-10-26T08:30:05.042Z
        assert_eq!(clock(1_793_003_405_042), "08:30:05.042");
        assert_eq!(clock(0), "00:00:00.000");
    }
}
