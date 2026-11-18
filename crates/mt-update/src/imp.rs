//! The `updater` feature: Tauri's updater plugin plus the install-on-quit hook.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{PROGRESS_EVENT, UpdateInfo, When, in_place_blocker};

#[derive(Default)]
struct UpdateState {
    /// The last update `update_check` found.
    offered: Mutex<Option<Update>>,
    /// Its verified package once downloaded.
    downloaded: Mutex<Option<(Update, Vec<u8>)>>,
    /// Install `downloaded` when the app quits.
    on_quit: AtomicBool,
    /// The quit-time install ran (or is running): let the exit through.
    quitting: AtomicBool,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// Registers Tauri's updater (config from `plugins.updater`) and the hook that installs a downloaded update when the
/// app quits. Both go on the builder: registering a plugin from another plugin's setup deadlocks (the plugin store
/// is locked while plugins initialize).
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.plugin(tauri_plugin_updater::Builder::new().build()).plugin(hook())
}

fn hook<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("mt-update")
        .setup(|app, _| {
            app.manage(UpdateState::default());
            #[cfg(feature = "selftest")]
            crate::selftest::start(app.clone());
            Ok(())
        })
        .on_event(|app, event| match event {
            RunEvent::ExitRequested { api, .. } => {
                let state = app.state::<UpdateState>();
                if !state.on_quit.load(Ordering::SeqCst) || state.quitting.swap(true, Ordering::SeqCst) {
                    return;
                }
                // Install off the main thread: when the bundle's folder is not writable the updater asks for an
                // administrator password through AppleScript on the main thread, which must keep running.
                api.prevent_exit();
                let app = app.clone();
                std::thread::spawn(move || {
                    install_pending(&app);
                    app.exit(0);
                });
            }
            RunEvent::Exit => {
                // Quit paths that skip ExitRequested: install now, bounded so a stuck install never hangs the quit.
                let state = app.state::<UpdateState>();
                if state.on_quit.load(Ordering::SeqCst) && !state.quitting.swap(true, Ordering::SeqCst) {
                    let app = app.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        install_pending(&app);
                        let _ = tx.send(());
                    });
                    let _ = rx.recv_timeout(Duration::from_secs(60));
                }
            }
            _ => {}
        })
        .build()
}

/// Keeps a downloaded update to install when the app quits (`update_install` with `quit`).
#[cfg(feature = "selftest")]
pub(crate) fn arm_install_on_quit<R: Runtime>(app: &AppHandle<R>, update: Update, bytes: Vec<u8>) {
    let state = app.state::<UpdateState>();
    *state.downloaded.lock().unwrap() = Some((update, bytes));
    state.on_quit.store(true, Ordering::SeqCst);
}

fn install_pending<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<UpdateState>();
    let pending = state.downloaded.lock().unwrap().take();
    if let Some((update, bytes)) = pending {
        match update.install(&bytes) {
            Ok(()) => eprintln!("mt-update: installed {} at quit", update.version),
            Err(e) => eprintln!("mt-update: installing {} at quit failed: {e}", update.version),
        }
    }
}

/// `<worker>/latest` (the DMG), from the first updater endpoint.
fn download_url<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let endpoint = app.config().plugins.0.get("updater")?.get("endpoints")?.get(0)?.as_str()?;
    let url = tauri::Url::parse(endpoint).ok()?;
    Some(format!("{}/latest", url.origin().ascii_serialization()))
}

fn info<R: Runtime>(app: &AppHandle<R>, update: &Update, pending_on_quit: bool) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone().filter(|b| !b.trim().is_empty()),
        date: update.raw_json.get("pub_date").and_then(|d| d.as_str()).map(str::to_owned),
        mode: if in_place_blocker().is_some() { "download" } else { "install" },
        download_url: download_url(app),
        pending_on_quit,
    }
}

/// Asks the feed for a newer version. `Ok(None)`: up to date. Errors (offline, server down) are for the "Check for
/// updates" button; the launch check ignores them.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let state = app.state::<UpdateState>();
    if state.on_quit.load(Ordering::SeqCst) {
        let pending = state.downloaded.lock().unwrap();
        if let Some((update, _)) = pending.as_ref() {
            return Ok(Some(info(&app, update, true)));
        }
    }
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    let result = found.as_ref().map(|u| info(&app, u, false));
    *state.offered.lock().unwrap() = found;
    Ok(result)
}

/// Downloads the offered update and checks its signature against the public key built into the app; emits
/// [`PROGRESS_EVENT`]. The package is kept in memory until `update_install`.
#[tauri::command]
pub async fn update_download(app: AppHandle) -> Result<(), String> {
    if let Some(why) = in_place_blocker() {
        return Err(format!("This copy cannot replace itself ({why}). Download the new version instead."));
    }
    let state = app.state::<UpdateState>();
    let update = state.offered.lock().unwrap().clone().ok_or("Check for updates first.")?;
    if state.downloaded.lock().unwrap().as_ref().is_some_and(|(u, _)| u.version == update.version) {
        return Ok(());
    }
    let mut downloaded = 0u64;
    let emitter = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = emitter.emit(PROGRESS_EVENT, Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    *state.downloaded.lock().unwrap() = Some((update, bytes));
    Ok(())
}

/// Installs the downloaded update: `now` replaces the app and relaunches it; `quit` waits until the app quits (an
/// open bus connection or a recording is never cut), and the next launch is the new version.
#[tauri::command]
pub async fn update_install(app: AppHandle, when: When) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    if state.downloaded.lock().unwrap().is_none() {
        return Err("Download the update first.".into());
    }
    match when {
        When::Quit => {
            state.on_quit.store(true, Ordering::SeqCst);
            Ok(())
        }
        When::Now => {
            let (update, bytes) = state.downloaded.lock().unwrap().take().ok_or("Download the update first.")?;
            // Blocking work (file moves, maybe an administrator prompt) off the async runtime.
            let (update, bytes, installed) = tauri::async_runtime::spawn_blocking(move || {
                let installed = update.install(&bytes);
                (update, bytes, installed)
            })
            .await
            .map_err(|e| e.to_string())?;
            match installed {
                Ok(()) => app.restart(),
                Err(e) => {
                    // The updater restores the old app on failure; keep the package for another try.
                    *state.downloaded.lock().unwrap() = Some((update, bytes));
                    Err(format!("The update could not be installed: {e}"))
                }
            }
        }
    }
}

/// Opens the DMG of the newest version in the browser (`download` mode, or when an install failed).
#[tauri::command]
pub fn update_open_download(app: AppHandle) -> Result<(), String> {
    let url = download_url(&app).ok_or("No download address is configured.")?;
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}
