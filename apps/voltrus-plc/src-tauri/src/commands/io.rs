use crate::state::AppState;
use mt_plc_runtime::io_manager::{IoAddress, IoArea, IoSize, create_driver};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct IoDriverConfig {
    pub protocol: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub unit_id: u8,
    #[serde(default)]
    pub opcua_endpoint_url: String,
    #[serde(default)]
    pub s7_rack: u8,
    #[serde(default)]
    pub s7_slot: u8,
    #[serde(default)]
    pub mqtt_topic_prefix: String,
    #[serde(default)]
    pub mqtt_client_id: String,
    #[serde(default)]
    pub enip_tag_prefix: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct IoMapping {
    pub variable: String,
    pub direction: String,
    pub address: String,
    pub driver_id: String,
    pub protocol_address: String,
}

#[derive(serde::Serialize, Clone)]
pub struct IoStatus {
    pub connected: bool,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub unit_id: u8,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn configure_io_driver(
    state: tauri::State<'_, AppState>,
    config: IoDriverConfig,
) -> Result<IoStatus, String> {
    let mut io = state.io.driver.lock().await;

    // Disconnect existing driver if any
    if let Some(ref mut driver) = *io {
        driver.disconnect();
    }

    // create_driver performs blocking network I/O (it `block_on`s the
    // protocol connect) — keep it off the async worker.
    let created = tokio::task::block_in_place(|| {
        create_driver(
            &config.protocol,
            &config.host,
            config.port,
            config.unit_id,
            config.s7_rack,
            config.s7_slot,
            &config.opcua_endpoint_url,
            &config.mqtt_topic_prefix,
            &config.mqtt_client_id,
        )
    });

    match created {
        Ok(driver) => {
            let connected = driver.is_connected();
            *io = Some(driver);
            let _ = state.io.config.lock().await.insert(config.clone());
            Ok(IoStatus {
                connected,
                protocol: config.protocol,
                host: config.host,
                port: config.port,
                unit_id: config.unit_id,
                error: None,
            })
        }
        Err(e) => {
            *io = None;
            let _ = state.io.config.lock().await.take();
            Ok(IoStatus {
                connected: false,
                protocol: config.protocol,
                host: config.host,
                port: config.port,
                unit_id: config.unit_id,
                error: Some(e.to_string()),
            })
        }
    }
}

#[tauri::command]
pub async fn disconnect_io_driver(
    state: tauri::State<'_, AppState>,
) -> Result<IoStatus, String> {
    let mut io = state.io.driver.lock().await;
    let config_guard = state.io.config.lock().await;
    let protocol = config_guard.as_ref().map_or("modbus".to_string(), |c| c.protocol.clone());
    let host = config_guard.as_ref().map_or(String::new(), |c| c.host.clone());
    let port = config_guard.as_ref().map_or(0, |c| c.port);
    let unit_id = config_guard.as_ref().map_or(0, |c| c.unit_id);
    drop(config_guard);

    if let Some(ref mut driver) = *io {
        driver.disconnect();
    }
    *io = None;
    let _ = state.io.config.lock().await.take();

    Ok(IoStatus {
        connected: false,
        protocol,
        host,
        port,
        unit_id,
        error: None,
    })
}

#[tauri::command]
pub async fn get_io_status(
    state: tauri::State<'_, AppState>,
) -> Result<IoStatus, String> {
    let io = state.io.driver.lock().await;
    let config = state.io.config.lock().await;
    let connected = io.as_ref().map_or(false, |d| d.is_connected());
    let protocol = io.as_ref().map_or("modbus".to_string(), |d| d.name().to_string());
    let host = config.as_ref().map_or(String::new(), |c| c.host.clone());
    let port = config.as_ref().map_or(0, |c| c.port);
    let unit_id = config.as_ref().map_or(0, |c| c.unit_id);

    Ok(IoStatus {
        connected,
        protocol,
        host,
        port,
        unit_id,
        error: None,
    })
}

#[derive(serde::Serialize, Clone)]
pub struct IoReadResult {
    pub success: bool,
    pub address: u16,
    pub value: String,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn io_test_read(
    state: tauri::State<'_, AppState>,
    address: u16,
) -> Result<IoReadResult, String> {
    let mut io = state.io.driver.lock().await;
    match *io {
        Some(ref mut driver) => {
            let addr = IoAddress {
                area: IoArea::Memory,
                offset: address as usize,
                bit: None,
                size: IoSize::Word,
            };
            // Sync driver I/O — keep it off the async worker
            match tokio::task::block_in_place(|| driver.read_inputs(&[addr])) {
                Ok(values) => Ok(IoReadResult {
                    success: true,
                    address,
                    value: values.first().map(|v| format!("{:?}", v)).unwrap_or_default(),
                    error: None,
                }),
                Err(e) => Ok(IoReadResult {
                    success: false, address, value: String::new(), error: Some(e.to_string()),
                }),
            }
        }
        None => Ok(IoReadResult {
            success: false, address, value: String::new(),
            error: Some("Not connected to any device".into()),
        }),
    }
}

#[tauri::command]
pub async fn io_test_write(
    state: tauri::State<'_, AppState>,
    address: u16,
    value: f64,
) -> Result<IoReadResult, String> {
    let mut io = state.io.driver.lock().await;
    match *io {
        Some(ref mut driver) => {
            let addr = IoAddress {
                area: IoArea::Memory,
                offset: address as usize,
                bit: None,
                size: IoSize::Word,
            };
            // Sync driver I/O — keep it off the async worker
            match tokio::task::block_in_place(|| {
                driver.write_outputs(&[(addr, mt_plc_runtime::PlcValue::Dint(value as i32))])
            }) {
                Ok(()) => Ok(IoReadResult {
                    success: true, address, value: format!("{}", value), error: None,
                }),
                Err(e) => Ok(IoReadResult {
                    success: false, address, value: String::new(), error: Some(e.to_string()),
                }),
            }
        }
        None => Ok(IoReadResult {
            success: false, address, value: String::new(),
            error: Some("Not connected to any device".into()),
        }),
    }
}

#[tauri::command]
pub async fn get_io_mappings(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<IoMapping>, String> {
    Ok(state.io.mappings.lock().await.clone())
}
