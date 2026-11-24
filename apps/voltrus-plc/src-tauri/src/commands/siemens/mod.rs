//! Siemens S7 online functions over classic S7comm (ISO-on-TCP, port 102): find CPUs on the
//! network, go online, read the CPU identity, operating mode, diagnostic buffer and block list,
//! watch and modify values, and switch the CPU between RUN and STOP.
//!
//! S7-1200/1500 answer these only with PUT/GET communication allowed and an access level that
//! permits it. Program download to those CPUs is not offered: it needs S7comm-plus.

pub mod scan;
pub mod values;

use std::sync::Arc;
use std::time::Duration;

use mt_protocol::ProtocolClient;
use mt_s7::{BlockListEntry, CpuInfo, DiagBuffer, S7Client, S7Config, S7ConnectOptions, S7Value, StartMode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::state::AppState;
use values::WatchItem;

/// The open S7 connection, if any.
#[derive(Default)]
pub struct SiemensState {
    pub conn: Arc<Mutex<Option<Online>>>,
}

pub struct Online {
    client: S7Client,
    pub target: OnlineTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineTarget {
    pub host: String,
    pub port: u16,
    pub rack: u8,
    pub slot: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnlineInfo {
    pub target: OnlineTarget,
    pub pdu_size: u16,
    /// `None` when the CPU does not give its identity (some S7-1200 firmware).
    pub cpu: Option<CpuInfo>,
    pub cpu_error: Option<String>,
    pub mode: Option<String>,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

async fn with_client<T>(
    state: &tauri::State<'_, AppState>,
    f: impl AsyncFnOnce(&S7Client) -> Result<T, mt_protocol::ProtocolError>,
) -> Result<T, String> {
    let mut guard = state.siemens.conn.lock().await;
    let online = guard.as_ref().ok_or("Not online. Go online with a CPU first.")?;
    let result = f(&online.client).await;
    // A lost connection cannot be used again; drop it so the UI shows offline.
    if !online.client.is_connected().await {
        *guard = None;
        return Err(match result {
            Err(e) => format!("Connection lost: {e}"),
            Ok(_) => "Connection lost".into(),
        });
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn s7_go_online(
    state: tauri::State<'_, AppState>,
    host: String,
    port: Option<u16>,
    rack: u8,
    slot: u8,
) -> Result<OnlineInfo, String> {
    let target = OnlineTarget { host: host.trim().to_string(), port: port.unwrap_or(102), rack, slot };
    if target.host.is_empty() {
        return Err("Enter the CPU's IP address".into());
    }
    let config = S7Config {
        id: "voltrus-plc".into(),
        host: target.host.clone(),
        port: target.port,
        rack,
        slot,
        poll_interval_ms: 1000,
    };
    let options = S7ConnectOptions { timeout: CONNECT_TIMEOUT, ..S7ConnectOptions::default() };
    let client = S7Client::connect_with_options(config, options).await.map_err(|e| {
        format!("{e}. Check the IP address, rack and slot (S7-300: 0/2, S7-1200/1500: 0/1 or 0/0).")
    })?;

    let pdu_size = client.pdu_size().await.unwrap_or(0);
    let (cpu, cpu_error) = match client.read_cpu_info().await {
        Ok(info) => (Some(info), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let mode = client.read_plc_mode().await.ok().map(|m| m.mode);

    let mut guard = state.siemens.conn.lock().await;
    if let Some(old) = guard.take() {
        let _ = old.client.disconnect().await;
    }
    let info = OnlineInfo { target: target.clone(), pdu_size, cpu, cpu_error, mode };
    *guard = Some(Online { client, target });
    Ok(info)
}

#[tauri::command]
pub async fn s7_go_offline(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(online) = state.siemens.conn.lock().await.take() {
        let _ = online.client.disconnect().await;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct OnlineStatus {
    pub online: bool,
    pub target: Option<OnlineTarget>,
    pub mode: Option<String>,
    pub error: Option<String>,
}

/// The operating mode, polled by the UI. Never fails: a lost connection reads as offline.
#[tauri::command]
pub async fn s7_status(state: tauri::State<'_, AppState>) -> Result<OnlineStatus, String> {
    let target = match state.siemens.conn.lock().await.as_ref() {
        Some(online) => online.target.clone(),
        None => return Ok(OnlineStatus { online: false, target: None, mode: None, error: None }),
    };
    match with_client(&state, async |c| c.read_plc_mode().await).await {
        Ok(mode) => Ok(OnlineStatus { online: true, target: Some(target), mode: Some(mode.mode), error: None }),
        Err(e) => {
            let online = state.siemens.conn.lock().await.is_some();
            Ok(OnlineStatus { online, target: online.then_some(target), mode: None, error: Some(e) })
        }
    }
}

#[tauri::command]
pub async fn s7_diag_buffer(state: tauri::State<'_, AppState>) -> Result<DiagBuffer, String> {
    with_client(&state, async |c| c.read_diag_buffer().await).await
}

#[tauri::command]
pub async fn s7_list_blocks(state: tauri::State<'_, AppState>) -> Result<Vec<BlockListEntry>, String> {
    with_client(&state, async |c| c.list_blocks().await).await
}

/// Switch the CPU to RUN (warm or cold restart) or STOP. The UI asks for confirmation first.
#[tauri::command]
pub async fn s7_set_mode(state: tauri::State<'_, AppState>, run: bool, cold: Option<bool>) -> Result<String, String> {
    let start = if cold.unwrap_or(false) { StartMode::Cold } else { StartMode::Warm };
    with_client(&state, async |c| {
        if run { c.plc_start(start).await? } else { c.plc_stop().await? }
        // The CPU takes a moment to change mode; report what it says now.
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(c.read_plc_mode().await.map(|m| m.mode).unwrap_or_else(|_| "UNKNOWN".into()))
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchRequest {
    pub address: String,
    #[serde(default)]
    pub data_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WatchResult {
    pub value: Option<String>,
    pub error: Option<String>,
}

impl WatchResult {
    fn err(e: impl Into<String>) -> Self {
        Self { value: None, error: Some(e.into()) }
    }
}

/// Read every watch row. Rows are read in one batch; if the CPU refuses the batch (one address
/// that does not exist fails all of them), each row is read alone so only the bad ones show an
/// error.
#[tauri::command]
pub async fn s7_read_watch(
    state: tauri::State<'_, AppState>,
    items: Vec<WatchRequest>,
) -> Result<Vec<WatchResult>, String> {
    with_client(&state, async |c| Ok(read_watch(c, &items).await)).await
}

pub async fn read_watch(client: &S7Client, items: &[WatchRequest]) -> Vec<WatchResult> {
    let parsed: Vec<Result<WatchItem, String>> =
        items.iter().map(|r| WatchItem::parse(&r.address, r.data_type.as_deref())).collect();
    let tags: Vec<_> = parsed.iter().flatten().flat_map(|it| it.tags.clone()).collect();
    if tags.is_empty() {
        return parsed.into_iter().map(|p| WatchResult::err(p.err().unwrap_or_default())).collect();
    }

    let batch = client.read(&tags).await;
    let mut values = batch.as_ref().ok().map(|v| v.iter());
    let mut out = Vec::with_capacity(parsed.len());
    for item in parsed {
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                out.push(WatchResult::err(e));
                continue;
            }
        };
        let read: Result<Vec<S7Value>, String> = match values.as_mut() {
            Some(iter) => Ok(iter.by_ref().take(item.tags.len()).cloned().collect()),
            None => client.read(&item.tags).await.map_err(|e| e.to_string()),
        };
        out.push(match read.and_then(|v| item.format(&v)) {
            Ok(value) => WatchResult { value: Some(value), error: None },
            Err(e) => WatchResult::err(e),
        });
    }
    out
}

/// Write one value ("modify now"). 64-bit types are written as two DWords, high word first.
#[tauri::command]
pub async fn s7_write_watch(
    state: tauri::State<'_, AppState>,
    address: String,
    data_type: Option<String>,
    value: String,
) -> Result<(), String> {
    let item = WatchItem::parse(&address, data_type.as_deref())?;
    let values = item.parse_input(&value)?;
    with_client(&state, async |c| {
        for (tag, v) in item.tags.iter().zip(values) {
            c.write(tag, v).await?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_s7::{S7Server, S7Tag};

    async fn sim() -> (S7Server, S7Client) {
        let seed = vec![
            (S7Tag::parse_address("DB1.DBW0").unwrap(), S7Value::Word(0xFFFE)),
            (S7Tag::parse_address("DB1.DBD2").unwrap(), S7Value::DWord(1.5f32.to_bits())),
        ];
        let server = S7Server::start(0, seed).await.unwrap();
        let config = S7Config {
            id: "t".into(),
            host: "127.0.0.1".into(),
            port: server.port(),
            rack: 0,
            slot: 1,
            poll_interval_ms: 1000,
        };
        (server, S7Client::connect(config).await.unwrap())
    }

    fn req(address: &str, t: &str) -> WatchRequest {
        WatchRequest { address: address.into(), data_type: Some(t.into()) }
    }

    #[tokio::test]
    async fn watch_reads_typed_values_and_isolates_bad_rows() {
        let (server, client) = sim().await;
        let rows = vec![req("DB1.DBW0", "INT"), req("DB1.DBD2", "REAL"), req("DB99.DBW0", "INT"), req("MW0", "BOOL")];
        let out = read_watch(&client, &rows).await;
        assert_eq!(out[0].value.as_deref(), Some("-2"));
        assert_eq!(out[1].value.as_deref(), Some("1.5"));
        assert!(out[2].error.is_some(), "DB99 does not exist: {:?}", out[2]);
        assert!(out[3].error.as_deref().unwrap().contains("bit address"));
        server.stop().await;
    }

    #[tokio::test]
    async fn sixty_four_bit_values_round_trip_through_the_simulator() {
        let (server, client) = sim().await;
        let item = WatchItem::parse("MD100", Some("LREAL")).unwrap();
        for (tag, v) in item.tags.iter().zip(item.parse_input("-1234.5").unwrap()) {
            client.write(tag, v).await.unwrap();
        }
        let out = read_watch(&client, &[req("MD100", "LREAL")]).await;
        assert_eq!(out[0].value.as_deref(), Some("-1234.5"));
        server.stop().await;
    }
}
