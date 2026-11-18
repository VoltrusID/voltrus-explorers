//! Unattended checks for test builds (`selftest` feature, never in a release): run the app with
//! `MT_UPDATE_SELFTEST=report` (or `install`) and read stderr (`open --stderr <file> --env ...`).
//!
//! - where it runs and whether it is sandboxed, and whether this copy could replace itself;
//! - a Keychain item of the app (written on first run, read back later: shows that a newer build, sandboxed or not,
//!   still reads what an older one stored, as the licence key in `mt-store` must be);
//! - a local-storage value of the web view (same, for the saved profiles and settings);
//! - with `install`: check the feed, download, verify, install and relaunch, logging each step;
//! - with `quit`: the same up to the download, then arm the install for when the app quits.

use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

const ITEM: &str = "mt-update-selftest";

pub fn start<R: Runtime>(app: AppHandle<R>) {
    let Ok(mode) = std::env::var("MT_UPDATE_SELFTEST") else { return };
    std::thread::spawn(move || run(app, &mode));
}

fn run<R: Runtime>(app: AppHandle<R>, mode: &str) {
    let version = app.package_info().version.to_string();
    let log = |s: String| eprintln!("[selftest {version}] {s}");
    let id = app.config().identifier.clone();
    log(format!(
        "exe={:?} sandbox={:?} in_place_blocker={:?}",
        std::env::current_exe().ok(),
        std::env::var("APP_SANDBOX_CONTAINER_ID").ok(),
        crate::in_place_blocker()
    ));
    if let Some(home) = std::env::var_os("HOME") {
        let web = std::path::Path::new(&home).join("Library/WebKit").join(&id).join("WebsiteData");
        log(format!("home={home:?} unsandboxed_webkit_data_exists={}", web.exists()));
        let old = std::path::Path::new(&home).join("Library/Containers").join(&id).join("Data/Library/WebKit/WebsiteData");
        log(format!("container_webkit_readable={:?}", std::fs::read_dir(&old).map(|d| d.count())));
    }
    keychain(&id, &version, &log);

    std::thread::sleep(Duration::from_secs(3));
    if let Some(w) = app.webview_windows().into_values().next() {
        let js = format!(
            "(function(){{var k='{ITEM}';var v=localStorage.getItem(k);if(!v)localStorage.setItem(k,'written by {version} sandbox={}');location.hash='ls='+encodeURIComponent(v||'none');}})()",
            std::env::var("APP_SANDBOX_CONTAINER_ID").is_ok()
        );
        let _ = w.eval(&js);
        std::thread::sleep(Duration::from_secs(1));
        let hash = w.url().ok().and_then(|u| u.fragment().map(str::to_owned)).unwrap_or_default();
        log(format!("local_storage_before={hash}"));
    }

    if mode != "install" && mode != "quit" {
        return;
    }
    // Let WebKit write local storage to disk before the app is replaced.
    std::thread::sleep(Duration::from_secs(4));
    let result = tauri::async_runtime::block_on(async {
        let updater = app.updater().map_err(|e| format!("updater: {e}"))?;
        let update = updater.check().await.map_err(|e| format!("check: {e}"))?;
        let Some(update) = update else { return Ok::<_, String>(false) };
        log(format!("update offered: {} -> {} url={}", update.current_version, update.version, update.download_url));
        let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| format!("download: {e}"))?;
        log(format!("downloaded and verified {} bytes", bytes.len()));
        if mode == "quit" {
            // What "Install when I quit" does: the next quit (Cmd+Q, Dock, last window) installs it.
            crate::imp::arm_install_on_quit(&app, update, bytes);
            log("armed: installs when the app quits".into());
            return Ok(false);
        }
        update.install(&bytes).map_err(|e| format!("install: {e}"))?;
        log("installed".into());
        Ok(true)
    });
    match result {
        Ok(true) => {
            log("restarting".into());
            app.restart();
        }
        Ok(false) => log("no restart".into()),
        Err(e) => log(format!("FAILED {e}")),
    }
}

#[cfg(target_os = "macos")]
fn keychain(service: &str, version: &str, log: &impl Fn(String)) {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    match get_generic_password(service, ITEM) {
        Ok(v) => log(format!("keychain_read={:?}", String::from_utf8_lossy(&v))),
        Err(e) => {
            log(format!("keychain_read_error={e} (code {})", e.code()));
            let text = format!("written by {version} sandbox={}", std::env::var("APP_SANDBOX_CONTAINER_ID").is_ok());
            match set_generic_password(service, ITEM, text.as_bytes()) {
                Ok(()) => log(format!("keychain_written={text:?}")),
                Err(e) => log(format!("keychain_write_error={e}")),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn keychain(_: &str, _: &str, _: &impl Fn(String)) {}
