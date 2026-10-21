//! Voltrus Connect client for the explorer apps.
//!
//! Sign in with a Voltrus account (OAuth 2.0 code + PKCE in the system browser, redirected back through
//! the `voltrus-<slug>://connect` deep link), keep tokens in the Keychain, sync saved profiles, manage
//! teams, list Voltrus Remote sites, and (App Store builds) link StoreKit subscriptions to the account.
//!
//! Apps call [`setup`] with their `connect.json` and register the `connect_*` commands. The StoreKit
//! plugin comes from `mt_store::setup`, which the app must call first.

mod account;
mod api;
mod commands;
mod error;
mod model;
mod pkce;
mod profiles;
mod session;
mod store;
mod sync;
mod vault;

#[cfg(test)]
mod testutil;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

pub use account::{RemoteJoin, RemoteSite, TeamDetail};
pub use error::ConnectError;
pub use model::Me;
pub use profiles::{Profile, ProfileDraft, SaveOutcome};
pub use session::Session;
pub use sync::{SyncRequest, SyncResult};

const DEFAULT_API_BASE: &str = "https://console.voltrus.id";
/// Event the frontend listens to for the end of a sign-in started by [`connect_sign_in`].
pub const AUTH_EVENT: &str = "connect://auth";

/// `src-tauri/connect.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectConfig {
    /// Registered client id, also the profile `app` value and the URL scheme suffix: `voltrus-<slug>`.
    pub app_slug: String,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    /// Auto-renewable Connect subscription product ids (App Store builds).
    #[serde(default)]
    pub subscription_products: Vec<String>,
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.into()
}

impl ConnectConfig {
    pub fn redirect_uri(&self) -> String {
        format!("voltrus-{}://connect", self.app_slug)
    }
}

/// Managed state: the configuration and the signed-in session.
pub struct Connect {
    pub config: ConnectConfig,
    pub session: Arc<Session>,
}

#[derive(Debug, Serialize)]
pub struct ConnectInfo {
    pub app_slug: String,
    /// App Store build: subscriptions are bought through StoreKit.
    pub store: bool,
    /// Account page for non-store builds; `None` in store builds, which never link out to it.
    pub console_url: Option<String>,
    /// Console settings page that deletes the account (App Store guideline 5.1.1(v)).
    pub delete_account_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct AuthEvent {
    ok: bool,
    error: Option<String>,
}

fn build_session(config: &ConnectConfig) -> Result<Session, ConnectError> {
    let api = api::Api::new(&config.api_base)?;
    let vault = vault::platform_vault(&format!("id.voltrus.connect.{}", config.app_slug));
    let device = format!("{} on {}", config.app_slug, std::env::consts::OS);
    Ok(Session::new(api, vault, &config.app_slug, &config.redirect_uri(), &device))
}

/// Registers deep links, the opener, the Connect state and its commands' dependencies.
pub fn setup<R: Runtime>(builder: tauri::Builder<R>, config_json: &str) -> tauri::Builder<R> {
    let config: ConnectConfig = serde_json::from_str(config_json).expect("invalid connect.json");
    let session = Arc::new(build_session(&config).expect("Connect HTTP client"));
    let state = Connect { config, session };
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri::plugin::Builder::<R>::new("mt-connect")
                .setup(move |app, _| {
                    app.manage(state);
                    #[cfg(any(windows, target_os = "linux"))]
                    let _ = app.deep_link().register_all();
                    let handle = app.clone();
                    app.deep_link().on_open_url(move |event| handle_urls(&handle, event.urls()));
                    // Cold start through the deep link.
                    if let Ok(Some(urls)) = app.deep_link().get_current() {
                        handle_urls(app, urls);
                    }
                    Ok(())
                })
                .build(),
        )
}

fn handle_urls<R: Runtime>(app: &AppHandle<R>, urls: Vec<Url>) {
    let Some(connect) = app.try_state::<Connect>() else { return };
    for url in urls {
        if !connect.session.owns_callback(&url) {
            continue;
        }
        let session = connect.session.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let payload = match session.complete_sign_in(&url).await {
                Ok(()) => AuthEvent { ok: true, error: None },
                // Someone else's redirect: not ours to report.
                Err(ConnectError::ForeignCallback) => return,
                Err(e) => AuthEvent { ok: false, error: Some(e.to_string()) },
            };
            let _ = app.emit(AUTH_EVENT, payload);
        });
    }
}

pub use commands::{
    connect_info, connect_invite_accept, connect_me, connect_new_id, connect_open_url, connect_profile_save, connect_remote_join,
    connect_remote_sites, connect_sign_in, connect_sign_out, connect_signed_in, connect_sync, connect_team_create,
    connect_team_invite, connect_team_remove_member, connect_teams,
};
pub use store::{connect_link_purchases, connect_store_products, connect_subscribe};
