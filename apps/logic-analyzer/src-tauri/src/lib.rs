pub mod core;

use std::path::PathBuf;
use std::sync::Arc;

use mt_logic::capture::Measurement;
use mt_logic::{Annotation, ChannelInfo, DecoderConfig};
use tauri::{Emitter, Manager};

use crate::core::{CaptureInfo, CaptureRequest, Core, DecoderView, Devices, FirmwareStatus, Opened};

type State<'a> = tauri::State<'a, Arc<Core>>;

/// Runs CPU-heavy work (decoding, summaries of large captures) off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn la_devices(core: State<'_>) -> Result<Devices, String> {
    let c = core.inner().clone();
    blocking(move || Ok(c.devices())).await
}

#[tauri::command]
fn la_firmware_status(core: State<'_>) -> FirmwareStatus {
    core.firmware_status()
}

#[tauri::command]
fn la_firmware_choose(core: State<'_>, path: Option<String>) -> Result<FirmwareStatus, String> {
    core.firmware_choose(path)
}

/// Fetches fx2lafw from sigrok.org into the app's firmware folder. Only on the user's click.
#[tauri::command]
async fn la_firmware_download(core: State<'_>) -> Result<Vec<String>, String> {
    let dir = core.firmware_dir().ok_or("the app has no data folder")?;
    mt_logic::fx2::firmware::download(&dir).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn la_capture_start(core: State<'_>, request: CaptureRequest) -> Result<(), String> {
    core.inner().start(request)
}

#[tauri::command]
fn la_capture_stop(core: State<'_>) {
    core.stop();
}

#[tauri::command]
fn la_capture_info(core: State<'_>) -> Option<CaptureInfo> {
    core.info()
}

#[tauri::command]
fn la_set_channels(core: State<'_>, channels: Vec<ChannelInfo>) -> Result<(), String> {
    core.set_channels(channels)
}

#[tauri::command]
async fn la_waveform(core: State<'_>, start: u64, end: u64, buckets: usize) -> Result<Vec<u8>, String> {
    let c = core.inner().clone();
    blocking(move || c.waveform(start, end, buckets)).await
}

#[tauri::command]
fn la_edge(core: State<'_>, channel: u8, from: u64, forward: bool) -> Result<Option<u64>, String> {
    core.edge(channel, from, forward)
}

#[tauri::command]
async fn la_measure(core: State<'_>, channel: u8, start: u64, end: u64) -> Result<Option<Measurement>, String> {
    let c = core.inner().clone();
    blocking(move || c.measure(channel, start, end)).await
}

#[tauri::command]
async fn la_estimate_baud(core: State<'_>, channel: u8) -> Result<Option<f64>, String> {
    let c = core.inner().clone();
    blocking(move || c.estimate_baud(channel)).await
}

#[tauri::command]
async fn la_decode(core: State<'_>, id: u32, config: DecoderConfig) -> Result<DecoderView, String> {
    let c = core.inner().clone();
    blocking(move || c.decode(id, config)).await
}

#[tauri::command]
fn la_decoder_remove(core: State<'_>, id: u32) {
    core.remove_decoder(id);
}

#[tauri::command]
async fn la_annotations(core: State<'_>, id: u32, start: u64, end: u64, min_width: f64) -> Result<Vec<Annotation>, String> {
    let c = core.inner().clone();
    blocking(move || c.annotations(id, start, end, min_width)).await
}

#[tauri::command]
async fn la_open(core: State<'_>, path: String) -> Result<Opened, String> {
    let c = core.inner().clone();
    blocking(move || c.open_file(&PathBuf::from(path))).await
}

#[tauri::command]
async fn la_save(core: State<'_>, path: String, session: serde_json::Value) -> Result<(), String> {
    let c = core.inner().clone();
    blocking(move || c.save_file(&PathBuf::from(path), &session)).await
}

/// Opens an https page (privacy policy, sigrok.org) in the system browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Only https links can be opened.".into());
    }
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn run() {
    mt_store::setup(tauri::Builder::default(), include_str!("../store.json"))
        // Native open and save panels plus reads and writes of the paths the user picked (the only file
        // access the sandboxed build has): captures, exports, a firmware file.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let fw_dir = app.path().app_data_dir().ok().map(|d| d.join("firmware").join(format!("fx2lafw-{}", mt_logic::fx2::firmware::RELEASE)));
            let usb: Option<Arc<dyn mt_logic::fx2::UsbBackend>> = Some(Arc::new(mt_logic::fx2::nusb_backend::NusbBackend));
            app.manage(Arc::new(Core::new(
                Arc::new(move |event, payload| {
                    let _ = handle.emit(event, payload);
                }),
                usb,
                fw_dir,
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            la_devices, la_firmware_status, la_firmware_choose, la_firmware_download,
            la_capture_start, la_capture_stop, la_capture_info, la_set_channels,
            la_waveform, la_edge, la_measure, la_estimate_baud,
            la_decode, la_decoder_remove, la_annotations,
            la_open, la_save,
            open_url,
            mt_store::request_review, mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_store::store_activate, mt_store::store_deactivate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
