//! Air-gapped IDE host (feature `ide`): static SPA serving plus a local REST
//! shim that mirrors the cloud worker's project-file contract against disk.
//!
//! The browser IDE (apps/voltrus-plc in cloud mode) reaches a Voltrus runtime
//! on the LAN through `vplc_base`; with this module the runtime itself serves
//! the SPA under `/ide/*` and answers the same `/api/*` shapes as
//! apps/voltrus-plc-cloud (projects.ts / files.ts / auth.ts), so the bridge
//! needs no code changes. Auth is stubbed — an offline runtime accepts any
//! credentials and any bearer token — and project ids are the directory names
//! under the projects dir.
//!
//! The HTTP stack is axum (hyper + tower), the same underlying stack tonic
//! rides for gRPC; the server runs on its own port alongside the gRPC service.

use std::fs;
use std::path::{Component, Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context;
use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Directories the IDE host reads from and serves.
#[derive(Debug, Clone)]
pub struct IdeConfig {
    /// Static SPA build served under `/ide/` (build with `--base=/ide/`).
    pub ide_dir: PathBuf,
    /// Project roots: each qualifying subdirectory is one project.
    pub projects_dir: PathBuf,
}

/// Where projects live when neither the flag nor `$HOME` gives a better answer.
fn fallback_projects_dir() -> PathBuf {
    PathBuf::from("./projects")
}

impl Default for IdeConfig {
    fn default() -> Self {
        let projects_dir = match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join("voltrus").join("projects"),
            None => fallback_projects_dir(),
        };
        Self {
            ide_dir: PathBuf::from("./ide"),
            projects_dir,
        }
    }
}

/// Extensions that mark a directory as holding a PLC project.
const PLC_EXTS: [&str; 5] = ["fbd", "ld", "sfc", "st", "stc"];
/// Written into projects created through the API so an empty project still
/// lists before its first source file is saved.
const PROJECT_MARKER: &str = ".voltrus-project";
/// Same cap the cloud worker applies to project-relative file paths.
const MAX_FILE_PATH_LEN: usize = 512;
const MAX_PROJECT_ID_LEN: usize = 128;

struct ProjectSummary {
    id: String,
    created_at: String,
    updated_at: String,
    file_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectSummaryBody {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
    file_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMetaBody {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
    files: Vec<FileRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileRow {
    path: String,
    size: u64,
    updated_at: String,
}

#[derive(Serialize)]
struct HealthBody {
    ok: bool,
}

#[derive(Serialize)]
struct UserBody {
    id: String,
    email: String,
}

#[derive(Serialize)]
struct AuthBody {
    token: String,
    user: UserBody,
}

/// Offline stub token — the bridge stores it and echoes it back; the shim
/// never validates it.
const STUB_TOKEN: &str = "offline";
const STUB_USER_ID: &str = "local";
const STUB_EMAIL: &str = "local@voltrus.local";

/// Build the IDE host router: REST shim + SPA statics, CORS-reflecting.
pub fn router(config: Arc<IdeConfig>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/me", get(me))
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{id}", get(get_project).delete(delete_project))
        .route("/api/projects/{id}/files", get(list_files))
        .route(
            "/api/projects/{id}/files/{*path}",
            get(get_file).put(put_file).delete(delete_file),
        )
        .route("/ide", get(ide_index))
        .route("/ide/", get(ide_index))
        .route("/ide/{*path}", get(ide_asset))
        .fallback(fallback_404)
        .layer(middleware::from_fn(cors))
        .with_state(config)
}

/// Serve the IDE host on `listener` until `shutdown` fires.
///
/// Creates both directories up front so a first boot against empty paths
/// serves an (empty) project list instead of erroring.
pub async fn serve(
    config: IdeConfig,
    listener: TcpListener,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    fs::create_dir_all(&config.projects_dir)
        .with_context(|| format!("creating projects dir {}", config.projects_dir.display()))?;
    fs::create_dir_all(&config.ide_dir)
        .with_context(|| format!("creating IDE dir {}", config.ide_dir.display()))?;
    let app = router(Arc::new(config));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await
        .context("IDE server")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<HealthBody> {
    Json(HealthBody { ok: true })
}

async fn register(Json(body): Json<serde_json::Value>) -> Response {
    auth_response(body, StatusCode::CREATED)
}

async fn login(Json(body): Json<serde_json::Value>) -> Response {
    auth_response(body, StatusCode::OK)
}

/// Cloud parity: credentials must at least be strings; beyond that an offline
/// runtime accepts anything and hands back a stub identity.
fn auth_response(body: serde_json::Value, status: StatusCode) -> Response {
    let Some(obj) = body.as_object() else {
        return err_json(StatusCode::BAD_REQUEST, "email and password are required");
    };
    let email = obj.get("email").and_then(|v| v.as_str());
    let password = obj.get("password").and_then(|v| v.as_str());
    let (Some(email), Some(_)) = (email, password) else {
        return err_json(StatusCode::BAD_REQUEST, "email and password are required");
    };
    let email = email.trim().to_lowercase();
    (
        status,
        Json(AuthBody {
            token: STUB_TOKEN.to_string(),
            user: UserBody {
                id: local_user_id(&email),
                email,
            },
        }),
    )
        .into_response()
}

async fn me() -> Json<AuthBody> {
    Json(AuthBody {
        token: STUB_TOKEN.to_string(),
        user: UserBody {
            id: STUB_USER_ID.to_string(),
            email: STUB_EMAIL.to_string(),
        },
    })
}

async fn list_projects(State(cfg): State<Arc<IdeConfig>>) -> Response {
    let mut projects: Vec<ProjectSummary> = Vec::new();
    let Ok(entries) = fs::read_dir(&cfg.projects_dir) else {
        return Json(Vec::<ProjectSummaryBody>::new()).into_response();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with('.') || !is_project_dir(&path) {
            continue;
        }
        let files = project_files(&path);
        let (created_at, dir_modified) = dir_timestamps(&path);
        let updated_at = latest_mtime(&files).map_or(dir_modified, |latest| latest.max(dir_modified));
        projects.push(ProjectSummary {
            id: name.to_string(),
            created_at,
            updated_at: iso(updated_at),
            file_count: files.len(),
        });
    }
    projects.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    Json(
        projects
            .into_iter()
            .map(|p| ProjectSummaryBody {
                id: p.id.clone(),
                name: p.id,
                created_at: p.created_at,
                updated_at: p.updated_at,
                file_count: p.file_count,
            })
            .collect::<Vec<_>>(),
    )
    .into_response()
}

async fn create_project(
    State(cfg): State<Arc<IdeConfig>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body
        .as_object()
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str());
    let Some(name) = name else {
        return err_json(StatusCode::BAD_REQUEST, "name is required");
    };
    let Some(dir_name) = sanitize_project_name(name) else {
        return err_json(StatusCode::BAD_REQUEST, "invalid project name");
    };
    let dir = cfg.projects_dir.join(&dir_name);
    if dir.exists() {
        return err_json(StatusCode::CONFLICT, "name already exists");
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("creating project dir {}: {e}", dir.display());
        return err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to create project");
    }
    // Marker keeps a brand-new (file-less) project visible in listings.
    let _ = fs::write(dir.join(PROJECT_MARKER), b"");

    let (created_at, updated_at) = dir_timestamps(&dir);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": dir_name,
            "name": name.trim(),
            "createdAt": created_at,
            "updatedAt": iso(updated_at),
        })),
    )
        .into_response()
}

async fn get_project(State(cfg): State<Arc<IdeConfig>>, Path(id): Path<String>) -> Response {
    let Some(dir) = project_dir(&cfg, &id) else {
        return err_json(StatusCode::BAD_REQUEST, "invalid project id");
    };
    if !dir.is_dir() || !is_project_dir(&dir) {
        return err_json(StatusCode::NOT_FOUND, "project not found");
    }
    let files = project_files(&dir);
    let (created_at, dir_modified) = dir_timestamps(&dir);
    let updated_at = latest_mtime(&files).map_or(dir_modified, |latest| latest.max(dir_modified));
    Json(ProjectMetaBody {
        id: id.clone(),
        name: id,
        created_at,
        updated_at: iso(updated_at),
        files: files.into_iter().map(|f| f.body).collect(),
    })
    .into_response()
}

async fn delete_project(State(cfg): State<Arc<IdeConfig>>, Path(id): Path<String>) -> Response {
    let Some(dir) = project_dir(&cfg, &id) else {
        return err_json(StatusCode::BAD_REQUEST, "invalid project id");
    };
    if !dir.is_dir() {
        return err_json(StatusCode::NOT_FOUND, "project not found");
    }
    match fs::remove_dir_all(&dir) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("removing project dir {}: {e}", dir.display());
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to delete project")
        }
    }
}

async fn list_files(State(cfg): State<Arc<IdeConfig>>, Path(id): Path<String>) -> Response {
    let Some(dir) = project_dir(&cfg, &id) else {
        return err_json(StatusCode::BAD_REQUEST, "invalid project id");
    };
    if !dir.is_dir() {
        return err_json(StatusCode::NOT_FOUND, "project not found");
    }
    let files = project_files(&dir);
    Json(files.into_iter().map(|f| f.body).collect::<Vec<_>>()).into_response()
}

async fn get_file(
    State(cfg): State<Arc<IdeConfig>>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let Some(resolved) = resolve_file(&cfg, &id, &path) else {
        return invalid_path_response();
    };
    if !resolved.is_file() {
        return err_json(StatusCode::NOT_FOUND, "file not found");
    }
    match fs::read(&resolved) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(content) => Json(serde_json::json!({ "content": content })).into_response(),
            Err(_) => err_json(StatusCode::BAD_REQUEST, "file is not valid UTF-8"),
        },
        Err(e) => {
            tracing::warn!("reading {}: {e}", resolved.display());
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to read file")
        }
    }
}

async fn put_file(
    State(cfg): State<Arc<IdeConfig>>,
    Path((id, path)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let content = body
        .as_object()
        .and_then(|o| o.get("content"))
        .and_then(|v| v.as_str());
    let Some(content) = content else {
        return err_json(StatusCode::BAD_REQUEST, "content is required");
    };
    let Some(resolved) = resolve_file(&cfg, &id, &path) else {
        return invalid_path_response();
    };
    if let Some(parent) = resolved.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!("creating {}: {e}", parent.display());
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to write file");
        }
    }
    match fs::write(&resolved, content.as_bytes()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("writing {}: {e}", resolved.display());
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to write file")
        }
    }
}

async fn delete_file(
    State(cfg): State<Arc<IdeConfig>>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let Some(resolved) = resolve_file(&cfg, &id, &path) else {
        return invalid_path_response();
    };
    // Cloud parity: deleting an absent key still 204s.
    match fs::remove_file(&resolved) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("removing {}: {e}", resolved.display());
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "failed to delete file")
        }
    }
}

// ---------------------------------------------------------------------------
// Static SPA
// ---------------------------------------------------------------------------

async fn ide_index(State(cfg): State<Arc<IdeConfig>>) -> Response {
    serve_static(&cfg.ide_dir.join("index.html"))
}

async fn ide_asset(State(cfg): State<Arc<IdeConfig>>, Path(path): Path<String>) -> Response {
    let Some(resolved) = safe_join(&cfg.ide_dir, &path) else {
        return err_json(StatusCode::BAD_REQUEST, "invalid path");
    };
    if !resolved.is_file() {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    serve_static(&resolved)
}

fn serve_static(resolved: &StdPath) -> Response {
    match fs::read(resolved) {
        Ok(bytes) => {
            let is_html = is_html_path(resolved);
            let cache = if is_html {
                "no-store"
            } else {
                "public, max-age=300"
            };
            (
                [
                    (header::CONTENT_TYPE, content_type(resolved)),
                    (header::CACHE_CONTROL, cache),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => err_json(StatusCode::NOT_FOUND, "not found"),
    }
}

/// MIME by extension for the SPA asset types that matter; anything else is
/// served as a generic octet stream.
fn content_type(path: &StdPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn is_html_path(path: &StdPath) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

// ---------------------------------------------------------------------------
// CORS — reflect any Origin (LAN use), allow Authorization + Content-Type.
// ---------------------------------------------------------------------------

async fn cors(req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).cloned();
    if req.method() == Method::OPTIONS {
        let mut res = StatusCode::NO_CONTENT.into_response();
        apply_cors(&mut res, origin);
        return res;
    }
    let mut res = next.run(req).await;
    apply_cors(&mut res, origin);
    res
}

fn apply_cors(res: &mut Response, origin: Option<HeaderValue>) {
    let Some(origin) = origin else { return };
    let headers = res.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
}

// ---------------------------------------------------------------------------
// Disk layout helpers
// ---------------------------------------------------------------------------

/// Cloud parity for project-relative file paths: non-empty, ≤512 chars,
/// no `..`, no NUL, no leading `/`.
fn valid_file_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_FILE_PATH_LEN
        && !path.contains("..")
        && !path.contains('\0')
        && !path.starts_with('/')
}

fn valid_project_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_PROJECT_ID_LEN
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains('\0')
        && !id.starts_with('.')
        && id != "."
        && id != ".."
}

/// Join `root` with a relative path, accepting only normal components —
/// a second guard behind the string validators.
fn safe_join(root: &StdPath, rel: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for component in StdPath::new(rel).components() {
        match component {
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    Some(out)
}

/// The project directory for `id`, or None when the id fails validation.
fn project_dir(cfg: &IdeConfig, id: &str) -> Option<PathBuf> {
    if !valid_project_id(id) {
        return None;
    }
    Some(cfg.projects_dir.join(id))
}

/// Validate and resolve a project file path on disk.
fn resolve_file(cfg: &IdeConfig, id: &str, path: &str) -> Option<PathBuf> {
    if !valid_file_path(path) {
        return None;
    }
    let dir = project_dir(cfg, id)?;
    if !dir.is_dir() {
        return None;
    }
    safe_join(&dir, path)
}

fn invalid_path_response() -> Response {
    err_json(StatusCode::BAD_REQUEST, "invalid path")
}

fn err_json(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Filesystem-safe project directory name, or None when the name is unusable.
fn sanitize_project_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_PROJECT_ID_LEN
        || trimmed.starts_with('.')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.contains("..")
        || trimmed == "."
        || trimmed == ".."
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// A directory is a project when it holds at least one PLC source file
/// (recursively), was created through the API (marker file), or is empty —
/// the last two keep a freshly-created project listed before its first save.
fn is_project_dir(dir: &StdPath) -> bool {
    if dir.join(PROJECT_MARKER).is_file() {
        return true;
    }
    let files = walk_files(dir);
    if files.is_empty() {
        return true;
    }
    files.iter().any(|rel| {
        StdPath::new(rel)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| PLC_EXTS.iter().any(|p| p.eq_ignore_ascii_case(ext)))
    })
}

/// Recursively collect project-relative file paths (slash-separated),
/// skipping dotfiles and dot directories.
fn walk_files(dir: &StdPath) -> Vec<String> {
    let mut out = Vec::new();
    walk_inner(dir, "", &mut out);
    out.sort();
    out
}

fn walk_inner(dir: &StdPath, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let child_rel = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        if path.is_dir() {
            walk_inner(&path, &child_rel, out);
        } else if path.is_file() {
            out.push(child_rel);
        }
    }
}

struct FileWithMeta {
    body: FileRow,
    mtime: Option<SystemTime>,
}

fn project_files(project_dir: &StdPath) -> Vec<FileWithMeta> {
    walk_files(project_dir)
        .into_iter()
        .map(|rel| {
            let path = project_dir.join(&rel);
            let meta = fs::metadata(&path).ok();
            FileWithMeta {
                body: FileRow {
                    path: rel,
                    size: meta.as_ref().map_or(0, |m| m.len()),
                    updated_at: meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .map_or_else(iso_now, iso),
                },
                mtime: meta.and_then(|m| m.modified().ok()),
            }
        })
        .collect()
}

fn latest_mtime(files: &[FileWithMeta]) -> Option<SystemTime> {
    files.iter().filter_map(|f| f.mtime).max()
}

/// (created, modified) for a directory; birth time falls back to mtime and
/// then to now, matching how tolerant consumers treat these stubs.
fn dir_timestamps(dir: &StdPath) -> (String, SystemTime) {
    let meta = fs::metadata(dir).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .unwrap_or_else(std::time::SystemTime::now);
    let created = meta
        .as_ref()
        .and_then(|m| m.created().ok())
        .unwrap_or(modified);
    (iso(created), modified)
}

/// RFC 3339 with millisecond precision — the shape `Date.toISOString()`
/// produces, which the IDE's project tables render.
fn iso(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Deterministic pseudo-id derived from the stub login email.
fn local_user_id(email: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in email.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("local-{hash:08x}")
}

async fn fallback_404() -> Response {
    err_json(StatusCode::NOT_FOUND, "not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::Duration;

    struct TestResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl TestResponse {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }

        fn json(&self) -> Value {
            serde_json::from_slice(&self.body).expect("JSON body")
        }
    }

    /// Minimal HTTP/1.1 client: real sockets through `serve`, one request per
    /// connection (`connection: close`), response read to EOF.
    async fn request(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: Option<(&str, Vec<u8>)>,
    ) -> TestResponse {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (headers, payload) = match &body {
            Some((ct, bytes)) => (
                format!("content-type: {ct}\r\ncontent-length: {}\r\n", bytes.len()),
                bytes.clone(),
            ),
            None => (String::new(), Vec::new()),
        };
        let req = format!("{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\n{headers}\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        stream.write_all(&payload).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header/body separator");
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let mut lines = head.split("\r\n");
        let status_line = lines.next().expect("status line");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("HTTP status code");
        let headers = lines
            .map(|l| {
                let (n, v) = l.split_once(':').expect("header line");
                (n.trim().to_ascii_lowercase(), v.trim().to_string())
            })
            .collect();
        TestResponse {
            status,
            headers,
            body: raw[split + 4..].to_vec(),
        }
    }

    async fn json_request(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: Value,
    ) -> TestResponse {
        request(
            addr,
            method,
            path,
            Some(("application/json", serde_json::to_vec(&body).unwrap())),
        )
        .await
    }

    /// Boot a server over an ephemeral port against tempdirs. The caller must
    /// hold the returned sender — dropping it shuts the server down.
    async fn spawn(ide_dir: &StdPath, projects_dir: &StdPath) -> (std::net::SocketAddr, watch::Sender<bool>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = IdeConfig {
            ide_dir: ide_dir.to_path_buf(),
            projects_dir: projects_dir.to_path_buf(),
        };
        let (tx, rx) = watch::channel(false);
        tokio::spawn(serve(config, listener, rx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        (addr, tx)
    }

    fn tempdir(name: &str) -> PathBuf {
        tempfile::Builder::new()
            .prefix(name)
            .tempdir()
            .map(|d| d.keep())
            .unwrap()
    }

    async fn create_project(addr: std::net::SocketAddr, name: &str) -> TestResponse {
        json_request(addr, "POST", "/api/projects", serde_json::json!({ "name": name })).await
    }

    #[tokio::test]
    async fn project_crud_round_trip_mirrors_cloud_contract() {
        let ide = tempdir("vrt-ide-crud");
        let projects = tempdir("vrt-projects-crud");
        let (addr, _shutdown) = spawn(&ide, &projects).await;

        // Register + login accept anything and answer the cloud auth shape.
        let res = json_request(
            addr,
            "POST",
            "/api/auth/register",
            serde_json::json!({ "email": "tech@site.lan", "password": "whatever" }),
        )
        .await;
        assert_eq!(res.status, 201);
        let auth = res.json();
        assert_eq!(auth["token"], "offline");
        assert_eq!(auth["user"]["email"], "tech@site.lan");
        assert!(auth["user"]["id"].as_str().unwrap().starts_with("local-"));

        let res = json_request(
            addr,
            "POST",
            "/api/auth/login",
            serde_json::json!({ "email": "tech@site.lan", "password": "other" }),
        )
        .await;
        assert_eq!(res.status, 200, "offline mode accepts any credentials");
        assert_eq!(res.json()["token"], "offline");

        let res = request(addr, "GET", "/api/auth/me", None).await;
        assert_eq!(res.status, 200);
        assert_eq!(res.json()["user"]["id"], "local");

        let res = request(addr, "GET", "/api/health", None).await;
        assert_eq!(res.status, 200);
        assert_eq!(res.json()["ok"], true);

        // Create → mkdir on disk, cloud-shaped 201.
        let res = create_project(addr, "pump_house").await;
        assert_eq!(res.status, 201);
        let created = res.json();
        assert_eq!(created["id"], "pump_house");
        assert_eq!(created["name"], "pump_house");
        assert!(created["createdAt"].as_str().unwrap().contains('T'));
        assert!(projects.join("pump_house").is_dir());

        // Duplicate name conflicts (disk ids are the names).
        assert_eq!(create_project(addr, "pump_house").await.status, 409);
        // Missing/blank name mirrors the cloud 400.
        assert_eq!(
            json_request(addr, "POST", "/api/projects", serde_json::json!({})).await.status,
            400
        );

        // PUT a nested file exactly the way the bridge encodes it.
        let program = "FUNCTION_BLOCK main\nEND_FUNCTION_BLOCK\n";
        let res = json_request(
            addr,
            "PUT",
            "/api/projects/pump_house/files/pumps%2Fmain.st",
            serde_json::json!({ "content": program }),
        )
        .await;
        assert_eq!(res.status, 204);
        assert_eq!(
            fs::read_to_string(projects.join("pump_house/pumps/main.st")).unwrap(),
            program
        );

        // GET the file back: {content}.
        let res = request(addr, "GET", "/api/projects/pump_house/files/pumps%2Fmain.st", None).await;
        assert_eq!(res.status, 200);
        assert_eq!(res.json()["content"], program);

        // File listing rows: [{path, size, updatedAt}].
        let res = request(addr, "GET", "/api/projects/pump_house/files", None).await;
        assert_eq!(res.status, 200);
        let rows = res.json();
        assert_eq!(rows[0]["path"], "pumps/main.st");
        assert_eq!(rows[0]["size"], program.len());
        assert!(rows[0]["updatedAt"].as_str().unwrap().contains('T'));

        // Project meta: id/name/timestamps + files sorted by path.
        let second = "PROGRAM pump\nEND_PROGRAM\n";
        json_request(
            addr,
            "PUT",
            "/api/projects/pump_house/files/aardvark.st",
            serde_json::json!({ "content": second }),
        )
        .await;
        let res = request(addr, "GET", "/api/projects/pump_house", None).await;
        assert_eq!(res.status, 200);
        let meta = res.json();
        assert_eq!(meta["id"], "pump_house");
        assert_eq!(meta["files"][0]["path"], "aardvark.st");
        assert_eq!(meta["files"][1]["path"], "pumps/main.st");

        // Project list: camelCase summary with fileCount.
        let res = request(addr, "GET", "/api/projects", None).await;
        assert_eq!(res.status, 200);
        let list = res.json();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], "pump_house");
        assert_eq!(list[0]["fileCount"], 2);

        // Overwrite updates content and size.
        let res = json_request(
            addr,
            "PUT",
            "/api/projects/pump_house/files/aardvark.st",
            serde_json::json!({ "content": "x" }),
        )
        .await;
        assert_eq!(res.status, 204);
        let res = request(addr, "GET", "/api/projects/pump_house/files/aardvark.st", None).await;
        assert_eq!(res.json()["content"], "x");

        // Delete a file → 204, then 404.
        assert_eq!(
            request(addr, "DELETE", "/api/projects/pump_house/files/aardvark.st", None)
                .await
                .status,
            204
        );
        assert_eq!(
            request(addr, "GET", "/api/projects/pump_house/files/aardvark.st", None)
                .await
                .status,
            404
        );
        // Deleting an absent file still 204s (R2 parity).
        assert_eq!(
            request(addr, "DELETE", "/api/projects/pump_house/files/aardvark.st", None)
                .await
                .status,
            204
        );

        // Unknown file / unknown project: JSON 404s with the cloud's messages.
        let res = request(
            addr,
            "GET",
            "/api/projects/pump_house/files/ghost%2Fghost.st",
            None,
        )
        .await;
        assert_eq!(res.status, 404);
        assert_eq!(res.json()["error"], "file not found");
        let res = request(addr, "GET", "/api/projects/ghost_town", None).await;
        assert_eq!(res.status, 404);
        assert_eq!(res.json()["error"], "project not found");

        // Delete the project → 204, gone from disk and API.
        assert_eq!(
            request(addr, "DELETE", "/api/projects/pump_house", None).await.status,
            204
        );
        assert!(!projects.join("pump_house").exists());
        assert_eq!(request(addr, "GET", "/api/projects/pump_house", None).await.status, 404);
    }

    #[tokio::test]
    async fn path_traversal_and_invalid_paths_rejected() {
        let ide = tempdir("vrt-ide-trav");
        let projects = tempdir("vrt-projects-trav");
        fs::write(tempdir("vrt-escape-trav").join("secret.txt"), "seed").ok();
        fs::create_dir_all(projects.join("safe_proj")).unwrap();
        let (addr, _shutdown) = spawn(&ide, &projects).await;

        let secret = format!(
            "..%2F..%2F{}%2Fsecret.txt",
            tempdir("vrt-escape-trav").file_name().unwrap().to_str().unwrap()
        );
        for path in [
            "..%2F..%2Fetc%2Fpasswd".to_string(),
            secret,
            "%2Fetc%2Fpasswd".to_string(), // leading slash after decode
            "ok%2F..%2F..%2Fup.st".to_string(),
            "bad%00null.st".to_string(),
            format!("{}.st", "a".repeat(513)),
        ] {
            let put = json_request(
                addr,
                "PUT",
                &format!("/api/projects/safe_proj/files/{path}"),
                serde_json::json!({ "content": "nope" }),
            )
            .await;
            assert_eq!(put.status, 400, "PUT should reject {path}");
            assert_eq!(put.json()["error"], "invalid path");
            let get = request(
                addr,
                "GET",
                &format!("/api/projects/safe_proj/files/{path}"),
                None,
            )
            .await;
            assert_eq!(get.status, 400, "GET should reject {path}");
            let del = request(
                addr,
                "DELETE",
                &format!("/api/projects/safe_proj/files/{path}"),
                None,
            )
            .await;
            assert_eq!(del.status, 400, "DELETE should reject {path}");
        }

        // A malicious project id must not escape either.
        let res = request(addr, "GET", "/api/projects/..%2F..%2Fetc", None).await;
        assert_eq!(res.status, 400, "traversal project id rejected");

        // Nothing escaped the projects dir.
        let leaked = fs::read_dir(&projects)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains("etc"));
        assert!(!leaked);

        // Static side: encoded traversal is rejected, not served.
        let res = request(addr, "GET", "/ide/%2e%2e%2f%2e%2e%2fcargo.toml", None).await;
        assert_ne!(res.status, 200);
    }

    #[tokio::test]
    async fn serves_spa_from_ide_dir() {
        let ide = tempdir("vrt-ide-spa");
        let projects = tempdir("vrt-projects-spa");
        fs::create_dir_all(ide.join("assets")).unwrap();
        fs::write(ide.join("index.html"), "<!doctype html><title>voltrus</title>").unwrap();
        fs::write(ide.join("assets/app-9f3c12.js"), "console.log(1)").unwrap();
        fs::write(ide.join("assets/logo.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let (addr, _shutdown) = spawn(&ide, &projects).await;

        for path in ["/ide", "/ide/"] {
            let res = request(addr, "GET", path, None).await;
            assert_eq!(res.status, 200, "{path} serves index");
            assert!(res.header("content-type").unwrap().starts_with("text/html"));
            assert_eq!(res.header("cache-control"), Some("no-store"));
            assert!(res.text().contains("voltrus"));
        }

        let res = request(addr, "GET", "/ide/assets/app-9f3c12.js", None).await;
        assert_eq!(res.status, 200);
        assert!(res.header("content-type").unwrap().starts_with("text/javascript"));
        assert_eq!(res.header("cache-control"), Some("public, max-age=300"));

        let res = request(addr, "GET", "/ide/assets/logo.png", None).await;
        assert_eq!(res.status, 200);
        assert_eq!(res.header("content-type"), Some("image/png"));

        assert_eq!(request(addr, "GET", "/ide/missing.js", None).await.status, 404);
    }

    #[tokio::test]
    async fn projects_listed_from_disk_layout() {
        let ide = tempdir("vrt-ide-list");
        let projects = tempdir("vrt-projects-list");

        // Real project: PLC sources nested arbitrarily.
        fs::create_dir_all(projects.join("pump_house/pumps")).unwrap();
        fs::write(projects.join("pump_house/main.fbd"), "fbd").unwrap();
        fs::write(projects.join("pump_house/pumps/valve.ld"), "ld").unwrap();
        // Marker-created project: empty until first save, still listed.
        fs::create_dir(projects.join("fresh_proj")).unwrap();
        fs::write(projects.join("fresh_proj/.voltrus-project"), b"").unwrap();
        // PLC sources in every flavor.
        fs::create_dir(projects.join("station")).unwrap();
        fs::write(projects.join("station/seq.sfc"), "sfc").unwrap();
        fs::write(projects.join("station/calc.stc"), "stc").unwrap();
        // Not projects: no PLC source anywhere, no marker, not empty.
        fs::create_dir(projects.join("notes")).unwrap();
        fs::write(projects.join("notes/readme.txt"), "plain").unwrap();
        // Dot directories never list.
        fs::create_dir(projects.join(".hidden_proj")).unwrap();
        fs::write(projects.join(".hidden_proj/x.st"), "st").unwrap();
        // Plain files at the root are not projects.
        fs::write(projects.join("loose.st"), "loose").unwrap();

        let (addr, _shutdown) = spawn(&ide, &projects).await;
        let res = request(addr, "GET", "/api/projects", None).await;
        assert_eq!(res.status, 200);
        let list = res.json();
        let ids: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["fresh_proj", "pump_house", "station"],
            "only PLC-bearing or marker-created directories list"
        );
        let by_id: std::collections::HashMap<&str, &Value> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["id"].as_str().unwrap(), p))
            .collect();
        assert_eq!(by_id["pump_house"]["fileCount"], 2);
        assert_eq!(by_id["fresh_proj"]["fileCount"], 0);
        assert_eq!(by_id["station"]["fileCount"], 2);
        assert!(by_id["pump_house"]["createdAt"].as_str().unwrap().contains('T'));

        // Every listed project is addressable; non-projects are not.
        assert_eq!(request(addr, "GET", "/api/projects/pump_house", None).await.status, 200);
        let res = request(addr, "GET", "/api/projects/notes", None).await;
        assert_eq!(res.status, 404);
        assert_eq!(res.json()["error"], "project not found");

        // Creating through the API lists immediately (marker file).
        let res = create_project(addr, "api_proj").await;
        assert_eq!(res.status, 201);
        let res = request(addr, "GET", "/api/projects", None).await;
        let list = res.json();
        let ids: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"api_proj"), "freshly created project lists");
    }

    #[tokio::test]
    async fn cors_preflight_and_origin_reflection() {
        let ide = tempdir("vrt-ide-cors");
        let projects = tempdir("vrt-projects-cors");
        let (addr, _shutdown) = spawn(&ide, &projects).await;

        // Preflight: OPTIONS with an Origin gets it reflected plus the
        // method/header allowances the browser IDE needs.
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(
                b"OPTIONS /api/projects HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\
                  origin: http://192.168.1.50:5173\r\n\
                  access-control-request-method: PUT\r\n\
                  access-control-request-headers: authorization, content-type\r\n\r\n",
            )
            .await
            .unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let head = String::from_utf8_lossy(&raw).into_owned();
        assert!(head.starts_with("HTTP/1.1 204"), "preflight is 204: {head}");
        assert!(head.to_ascii_lowercase().contains(
            "access-control-allow-origin: http://192.168.1.50:5173"
        ));
        assert!(head
            .to_ascii_lowercase()
            .contains("access-control-allow-headers: authorization, content-type"));
        assert!(head.to_ascii_lowercase().contains("access-control-allow-methods: get, post, put, delete, options"));

        // Plain requests also echo the Origin (LAN SPA on another port).
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /api/health HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\
                  origin: http://rpi5.local:5173\r\n\r\n",
            )
            .await
            .unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let head = String::from_utf8_lossy(&raw).to_ascii_lowercase();
        assert!(head.contains("access-control-allow-origin: http://rpi5.local:5173"));

        // No Origin header → no CORS headers on the response.
        let res = request(addr, "GET", "/api/health", None).await;
        assert_eq!(res.header("access-control-allow-origin"), None);
    }
}
