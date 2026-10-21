//! The `connect_*` Tauri commands the apps register.

use tauri::{AppHandle, Runtime, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::account::{self, RemoteJoin, RemoteSite, TeamDetail};
use crate::model::Me;
use crate::profiles::{self, ProfileDraft, SaveOutcome};
use crate::sync::{self, SyncRequest, SyncResult};
use crate::{Connect, ConnectConfig, ConnectInfo};

type Cmd<T> = Result<T, String>;

fn console_url(config: &ConnectConfig) -> String {
    format!("{}/app/connect", config.api_base.trim_end_matches('/'))
}

#[tauri::command]
pub fn connect_info(connect: State<'_, Connect>) -> ConnectInfo {
    ConnectInfo {
        app_slug: connect.config.app_slug.clone(),
        store: cfg!(feature = "store"),
        console_url: (!cfg!(feature = "store")).then(|| console_url(&connect.config)),
    }
}

#[tauri::command]
pub async fn connect_signed_in(connect: State<'_, Connect>) -> Cmd<bool> {
    Ok(connect.session.is_signed_in().await?)
}

/// Opens the Voltrus sign-in page in the system browser. The result arrives as [`AUTH_EVENT`].
#[tauri::command]
pub async fn connect_sign_in<R: Runtime>(app: AppHandle<R>, connect: State<'_, Connect>) -> Cmd<()> {
    let url = connect.session.begin_sign_in()?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| format!("Could not open the browser: {e}"))
}

#[tauri::command]
pub async fn connect_sign_out(connect: State<'_, Connect>) -> Cmd<()> {
    Ok(connect.session.sign_out().await?)
}

#[tauri::command]
pub async fn connect_me(connect: State<'_, Connect>) -> Cmd<Me> {
    Ok(account::me(&connect.session).await?)
}

#[tauri::command]
pub async fn connect_sync(request: SyncRequest, connect: State<'_, Connect>) -> Cmd<SyncResult> {
    Ok(sync::run(&connect.session, &connect.config.app_slug, request).await?)
}

/// Saves one profile. Used to resolve a conflict: pass the server's `version` as `base_version` to keep
/// the local copy.
#[tauri::command]
pub async fn connect_profile_save(draft: ProfileDraft, connect: State<'_, Connect>) -> Cmd<SaveOutcome> {
    Ok(profiles::save(&connect.session, &connect.config.app_slug, &draft).await?)
}

/// Opens an https page (legal links, the console) in the system browser.
#[tauri::command]
pub fn connect_open_url<R: Runtime>(app: AppHandle<R>, url: String) -> Cmd<()> {
    let parsed = Url::parse(&url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" {
        return Err("Only https links can be opened.".into());
    }
    app.opener().open_url(parsed.as_str(), None::<&str>).map_err(|e| format!("Could not open the browser: {e}"))
}

#[tauri::command]
pub fn connect_new_id() -> Cmd<String> {
    Ok(profiles::new_uuid()?)
}

#[tauri::command]
pub async fn connect_teams(connect: State<'_, Connect>) -> Cmd<Vec<TeamDetail>> {
    Ok(account::teams(&connect.session).await?)
}

#[tauri::command]
pub async fn connect_team_create(name: String, connect: State<'_, Connect>) -> Cmd<TeamDetail> {
    Ok(account::create_team(&connect.session, name.trim()).await?)
}

#[tauri::command]
pub async fn connect_team_invite(team_id: String, email: String, connect: State<'_, Connect>) -> Cmd<()> {
    Ok(account::invite(&connect.session, &team_id, email.trim()).await?)
}

#[tauri::command]
pub async fn connect_invite_accept(token: String, connect: State<'_, Connect>) -> Cmd<String> {
    Ok(account::accept_invite(&connect.session, token.trim()).await?)
}

/// Leaves a team (or, for an owner, removes `user_id`).
#[tauri::command]
pub async fn connect_team_remove_member(team_id: String, user_id: Option<String>, connect: State<'_, Connect>) -> Cmd<()> {
    let user_id = match user_id {
        Some(u) => u,
        None => account::me(&connect.session).await?.user.id,
    };
    Ok(account::remove_member(&connect.session, &team_id, &user_id).await?)
}

#[tauri::command]
pub async fn connect_remote_sites(connect: State<'_, Connect>) -> Cmd<Vec<RemoteSite>> {
    Ok(account::remote_sites(&connect.session).await?)
}

#[tauri::command]
pub async fn connect_remote_join(site_id: String, connect: State<'_, Connect>) -> Cmd<RemoteJoin> {
    Ok(account::remote_join(&connect.session, &site_id).await?)
}
