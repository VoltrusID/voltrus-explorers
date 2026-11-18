//! In-app updates for the direct-download (Developer ID DMG) builds.
//!
//! With the `updater` feature the app passes its builder through [`register`] and registers the `update_*` commands.
//! Updates come from the Tauri updater feed of the app's download worker (`plugins.updater` in
//! `src-tauri/tauri.updater.conf.json`, merged into the config by `scripts/build-macos.sh` for DMG builds only): a
//! minisign-signed `.app.tar.gz` of the notarized universal app. Two modes, chosen at run time:
//!
//! - `install`: download, verify the signature, swap the bundle in place, relaunch (now, or when the app quits so an
//!   open bus connection or recording is never cut).
//! - `download`: the app cannot replace itself (it runs in the App Sandbox, from the DMG or from a translocated
//!   copy): the update is announced and "Download" opens the DMG at `<worker>/latest` in the browser.
//!
//! Without the feature (App Store builds, `--features store`; source builds) nothing is compiled in: [`register`] adds
//! nothing and `update_check` always answers "no update". The App Store forbids apps that update themselves.

use serde::Serialize;

pub mod migrate;

/// Event carrying download progress (`{ downloaded, total }`, bytes) while an update downloads.
pub const PROGRESS_EVENT: &str = "mt-update://progress";

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// The version on offer, e.g. `1.2.0`.
    pub version: String,
    /// The running version.
    pub current_version: String,
    /// Release notes from the feed (plain text or Markdown-ish lines).
    pub notes: Option<String>,
    /// Release date (RFC 3339) when the feed has one.
    pub date: Option<String>,
    /// `install` (in place) or `download` (open the DMG in the browser).
    pub mode: &'static str,
    /// The DMG for `download` mode (and as a manual fallback): `<worker>/latest`.
    pub download_url: Option<String>,
    /// The update is downloaded and will be installed when the app quits.
    pub pending_on_quit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    /// Install now and relaunch.
    Now,
    /// Install when the app quits; the next launch is the new version.
    Quit,
}

/// Why the running copy cannot replace itself, if it cannot. `None`: it can.
pub fn in_place_blocker() -> Option<&'static str> {
    if std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some() {
        return Some("sandbox");
    }
    let exe = std::env::current_exe().ok()?;
    let path = exe.to_string_lossy();
    // Gatekeeper's App Translocation runs a quarantined app from a random read-only mount; a copy started from the
    // DMG lives under /Volumes. Neither can be replaced; the user should drag the app to Applications first.
    if path.contains("/AppTranslocation/") || path.starts_with("/Volumes/") {
        return Some("read-only location");
    }
    None
}

// A `#[tauri::command]` cannot be `pub` at the crate root (its helper macros clash there), hence the module.
mod supported {
    /// Whether this build updates itself; the frontend hides every update control when it does not.
    #[tauri::command]
    pub fn update_supported() -> bool {
        cfg!(feature = "updater")
    }
}
pub use supported::update_supported;

#[cfg(feature = "updater")]
mod imp;
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "updater")]
pub use imp::{register, update_check, update_download, update_install, update_open_download};

#[cfg(not(feature = "updater"))]
mod off {
    use super::{UpdateInfo, When};
    use tauri::Runtime;

    /// Builds without the `updater` feature: nothing to register.
    pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
        builder
    }

    #[tauri::command]
    pub async fn update_check() -> Result<Option<UpdateInfo>, String> {
        Ok(None)
    }

    #[tauri::command]
    pub async fn update_download() -> Result<(), String> {
        Err("This build does not update itself.".into())
    }

    #[tauri::command]
    pub async fn update_install(when: When) -> Result<(), String> {
        let _ = when;
        Err("This build does not update itself.".into())
    }

    #[tauri::command]
    pub fn update_open_download() -> Result<(), String> {
        Err("This build does not update itself.".into())
    }
}
#[cfg(not(feature = "updater"))]
pub use off::{register, update_check, update_download, update_install, update_open_download};
