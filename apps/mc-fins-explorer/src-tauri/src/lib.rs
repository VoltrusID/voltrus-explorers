mod frames;
mod plc;
mod values;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use mt_fins::{FinsClient, FinsServer};
use mt_melsec::{McClient, McServer};

use plc::{ConnectConfig, Plc, PlcInfo};

/// Live connections by device id, and the simulator (one MC and one FINS server).
struct AppState {
    plcs: Mutex<HashMap<String, Arc<Plc>>>,
    sim: Mutex<Option<Sim>>,
}

struct Sim {
    mc: McServer,
    fins: FinsServer,
}

impl AppState {
    async fn plc(&self, device_id: &str) -> Result<Arc<Plc>, String> {
        self.plcs.lock().await.get(device_id).cloned().ok_or_else(|| format!("'{device_id}' is not connected"))
    }
}

#[derive(Debug, Clone, Serialize)]
struct FrameEvent {
    device_id: String,
    direction: &'static str,
    bytes: String,
    summary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TagSpec {
    address: String,
    data_type: String,
}

// ── Connection ──

#[tauri::command]
async fn connect(app: AppHandle, state: State<'_, AppState>, device_id: String, config: ConnectConfig) -> Result<String, String> {
    if let Some(old) = state.plcs.lock().await.remove(&device_id) {
        old.disconnect().await;
    }
    let id = device_id.clone();
    let (plc, summary) = match config {
        ConnectConfig::Mc { config, xy_octal } => {
            let app = app.clone();
            let traffic: mt_melsec::TrafficFn = Arc::new(move |dir, bytes| {
                let _ = app.emit(
                    "frame",
                    FrameEvent {
                        device_id: id.clone(),
                        direction: if dir == mt_melsec::Direction::Tx { "tx" } else { "rx" },
                        bytes: frames::render(bytes),
                        summary: frames::describe_mc(bytes),
                    },
                );
            });
            let summary = format!(
                "MC protocol {} {} over {}",
                if config.frame == mt_melsec::FrameKind::E3 { "3E" } else { "4E" },
                if config.encoding == mt_melsec::Encoding::Binary { "binary" } else { "ASCII" },
                if config.transport == mt_melsec::Transport::Tcp { "TCP" } else { "UDP" },
            );
            let client = McClient::connect(config, Some(traffic)).await.map_err(|e| e.to_string())?;
            (Plc::Mc { client, xy_octal }, summary)
        }
        ConnectConfig::Fins { config } => {
            let app = app.clone();
            let traffic: mt_fins::TrafficFn = Arc::new(move |dir, bytes| {
                let _ = app.emit(
                    "frame",
                    FrameEvent {
                        device_id: id.clone(),
                        direction: if dir == mt_fins::Direction::Tx { "tx" } else { "rx" },
                        bytes: frames::render(bytes),
                        summary: frames::describe_fins(bytes),
                    },
                );
            });
            let transport = if config.transport == mt_fins::Transport::Udp { "FINS/UDP" } else { "FINS/TCP" };
            let client = FinsClient::connect(config, Some(traffic)).await.map_err(|e| e.to_string())?;
            let (sa1, da1) = client.nodes().await.unwrap_or_default();
            (Plc::Fins(client), format!("{transport}, node {sa1} → {da1}"))
        }
    };
    state.plcs.lock().await.insert(device_id, Arc::new(plc));
    Ok(summary)
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    if let Some(p) = state.plcs.lock().await.remove(&device_id) {
        p.disconnect().await;
    }
    Ok(())
}

// ── Data ──

#[tauri::command]
async fn read_tags(state: State<'_, AppState>, device_id: String, tags: Vec<TagSpec>) -> Result<Vec<serde_json::Value>, String> {
    let plc = state.plc(&device_id).await?;
    let parsed = tags
        .iter()
        .map(|t| plc.tag(&t.address, &t.data_type).map_err(|e| format!("{}: {e}", t.address)))
        .collect::<Result<Vec<_>, _>>()?;
    plc.read(&parsed).await
}

/// Check an address against a protocol without a connection (the tag editor).
#[tauri::command]
fn check_address(protocol: String, address: String, xy_octal: Option<bool>) -> Result<String, String> {
    match protocol.as_str() {
        "mc" => {
            let a = mt_melsec::parse_address(&address, xy_octal.unwrap_or(false))?;
            Ok(if a.device.is_bit() || a.bit.is_some() { "bit" } else { "word" }.into())
        }
        "fins" => {
            let a = mt_fins::parse_address(&address)?;
            Ok(if a.bit.is_some() { "bit" } else { "word" }.into())
        }
        _ => Err(format!("unknown protocol '{protocol}'")),
    }
}

#[tauri::command]
async fn write_tag(
    state: State<'_, AppState>,
    device_id: String,
    address: String,
    data_type: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let plc = state.plc(&device_id).await?;
    plc.write(&address, &data_type, &value).await
}

#[tauri::command]
async fn read_block(state: State<'_, AppState>, device_id: String, head: String, count: u32) -> Result<Vec<u16>, String> {
    if count == 0 || count > 1024 {
        return Err("read 1 to 1024 words".into());
    }
    let plc = state.plc(&device_id).await?;
    plc.read_block(&head, count).await
}

#[tauri::command]
async fn plc_info(state: State<'_, AppState>, device_id: String) -> Result<PlcInfo, String> {
    state.plc(&device_id).await?.info().await
}

#[tauri::command]
async fn plc_run(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    state.plc(&device_id).await?.run().await
}

#[tauri::command]
async fn plc_stop(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    state.plc(&device_id).await?.stop().await
}

// ── Simulator ──

#[derive(Debug, Clone, Serialize)]
struct SimInfo {
    mc_port: u16,
    fins_port: u16,
    fins_node: u8,
}

#[tauri::command]
async fn sim_start(app: AppHandle, state: State<'_, AppState>, mc_port: u16, fins_port: u16, fins_node: u8) -> Result<SimInfo, String> {
    let mut guard = state.sim.lock().await;
    if let Some(old) = guard.take() {
        old.mc.stop();
        old.fins.stop();
    }
    let log = |tag: &'static str, app: AppHandle| {
        move |peer: std::net::SocketAddr, write: bool, text: String| {
            // Reads come every poll; only changes are worth a log line.
            if write {
                let _ = app.emit("sim-log", serde_json::json!({ "level": "info", "message": format!("[{tag} simulator] {peer} {text}") }));
            }
        }
    };
    let mc_log = log("MC", app.clone());
    let mc = McServer::start(mc_port, Some(Arc::new(move |e: mt_melsec::ServerEvent| mc_log(e.peer, e.write, e.text))))
        .await
        .map_err(|e| format!("MC simulator could not listen on port {mc_port}: {e}"))?;
    let fins_log = log("FINS", app.clone());
    let fins = FinsServer::start(fins_port, fins_node, Some(Arc::new(move |e: mt_fins::ServerEvent| fins_log(e.peer, e.write, e.text))))
        .await
        .map_err(|e| format!("FINS simulator could not listen on port {fins_port}: {e}"))?;
    let info = SimInfo { mc_port: mc.port(), fins_port: fins.port(), fins_node };
    let _ = app.emit(
        "sim-log",
        serde_json::json!({ "level": "info", "message": format!("Simulator listening: MC protocol on TCP and UDP {}, FINS on UDP and TCP {} (node {fins_node})", info.mc_port, info.fins_port) }),
    );
    *guard = Some(Sim { mc, fins });
    Ok(info)
}

#[tauri::command]
async fn sim_stop(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(sim) = state.sim.lock().await.take() {
        sim.mc.stop();
        sim.fins.stop();
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct SimStatus {
    running: bool,
    /// The MC CPU is in RUN (a client can STOP it).
    mc_run: bool,
    /// The FINS CPU's mode: PROGRAM, MONITOR or RUN.
    fins_mode: String,
}

#[tauri::command]
async fn sim_status(state: State<'_, AppState>) -> Result<SimStatus, String> {
    let guard = state.sim.lock().await;
    Ok(match guard.as_ref() {
        None => SimStatus { running: false, mc_run: false, fins_mode: String::new() },
        Some(sim) => {
            let mc_run = sim.mc.memory().lock().map_err(|e| e.to_string())?.running;
            let mode = sim.fins.memory().lock().map_err(|e| e.to_string())?.mode;
            SimStatus {
                running: true,
                mc_run,
                fins_mode: match mode {
                    0x00 => "PROGRAM",
                    0x02 => "MONITOR",
                    _ => "RUN",
                }
                .into(),
            }
        }
    })
}

/// One simulated value change, written straight into the simulator's memory (the template animation).
#[derive(Debug, Clone, Deserialize)]
struct SimWrite {
    protocol: String,
    address: String,
    data_type: String,
    value: serde_json::Value,
}

#[tauri::command]
async fn sim_set(state: State<'_, AppState>, writes: Vec<SimWrite>) -> Result<(), String> {
    let guard = state.sim.lock().await;
    let sim = guard.as_ref().ok_or("Simulator is not running")?;
    for w in writes {
        let dt = values::DataType::from_name(&w.data_type).ok_or_else(|| format!("unknown data type '{}'", w.data_type))?;
        let words = values::encode(dt, &w.value)?;
        match w.protocol.as_str() {
            "mc" => {
                let a = mt_melsec::parse_address(&w.address, false)?;
                let mem = sim.mc.memory();
                let mut mem = mem.lock().map_err(|e| e.to_string())?;
                if dt == values::DataType::Bool {
                    if a.device.is_bit() {
                        mem.write_bits(a, &[words[0] != 0])
                    } else {
                        let at = mt_melsec::McAddress::new(a.device, a.number);
                        let cur = mem.read_words(at, 1).map_err(|c| format!("0x{c:04X}"))?[0];
                        let mask = 1 << a.bit.unwrap_or(0);
                        mem.write_words(at, &[if words[0] != 0 { cur | mask } else { cur & !mask }])
                    }
                } else if a.device.is_bit() {
                    let bits: Vec<bool> = (0..words.len() * 16).map(|i| words[i / 16] & (1 << (i % 16)) != 0).collect();
                    mem.write_bits(a, &bits)
                } else {
                    mem.write_words(a, &words)
                }
                .map_err(|c| format!("{}: end code 0x{c:04X}", w.address))?;
            }
            "fins" => {
                let a = mt_fins::parse_address(&w.address)?;
                let mem = sim.fins.memory();
                let mut mem = mem.lock().map_err(|e| e.to_string())?;
                if dt == values::DataType::Bool {
                    mem.write_bits(a, &[words[0] != 0])
                } else {
                    mem.write_words(a, &words)
                }
                .map_err(|c| format!("{}: end code 0x{c:04X}", w.address))?;
            }
            p => return Err(format!("unknown protocol '{p}'")),
        }
    }
    Ok(())
}

// ── Shell ──

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", ""]);
        c
    };
    cmd.arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState { plcs: Mutex::new(HashMap::new()), sim: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            read_tags,
            check_address,
            write_tag,
            read_block,
            plc_info,
            plc_run,
            plc_stop,
            sim_start,
            sim_stop,
            sim_status,
            sim_set,
            open_url,
            mt_store::store_status,
            mt_store::store_purchase,
            mt_store::store_restore,
            mt_store::store_activate,
            mt_store::store_deactivate
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_config_from_the_frontend() {
        let mc: ConnectConfig = serde_json::from_value(serde_json::json!({
            "protocol": "mc", "host": "192.168.3.39", "port": 5000, "transport": "udp", "frame": "4E",
            "encoding": "ascii", "route": { "network": 0, "pc": 255, "io": 1023, "station": 0 }, "xy_octal": true
        }))
        .unwrap();
        match mc {
            ConnectConfig::Mc { config, xy_octal } => {
                assert!(xy_octal);
                assert_eq!(config.frame, mt_melsec::FrameKind::E4);
                assert_eq!(config.encoding, mt_melsec::Encoding::Ascii);
                assert_eq!(config.transport, mt_melsec::Transport::Udp);
                assert_eq!(config.route.io, 0x03FF);
            }
            _ => panic!("expected MC"),
        }
        let fins: ConnectConfig = serde_json::from_value(serde_json::json!({
            "protocol": "fins", "host": "192.168.250.1", "port": 9600, "transport": "tcp", "dest_node": 1, "src_node": 0
        }))
        .unwrap();
        assert!(matches!(fins, ConnectConfig::Fins { config } if config.dest_node == 1 && config.transport == mt_fins::Transport::Tcp));
    }

    #[test]
    fn address_check_says_bit_or_word() {
        assert_eq!(check_address("mc".into(), "M0".into(), None).unwrap(), "bit");
        assert_eq!(check_address("mc".into(), "D0".into(), None).unwrap(), "word");
        assert_eq!(check_address("fins".into(), "CIO0.05".into(), None).unwrap(), "bit");
        assert!(check_address("fins".into(), "Q0".into(), None).is_err());
    }
}
