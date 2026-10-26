//! OPC UA sessions over `mt-opcua`: a client that signs in with the endpoint's security policy and
//! mode, browses the address space on request, monitors variables in one subscription, writes
//! typed values and calls methods; a plant server simulator; and the GetEndpoints behind the
//! editor's endpoint picker.

pub(crate) mod client;
pub(crate) mod plant;
pub(crate) mod server;

use std::path::PathBuf;
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
    use super::clock;

    #[test]
    fn clocks_read_utc() {
        // 2026-10-26T08:30:05.042Z
        assert_eq!(clock(1_793_003_405_042), "08:30:05.042");
        assert_eq!(clock(0), "00:00:00.000");
    }
}
