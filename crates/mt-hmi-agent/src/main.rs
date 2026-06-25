use axum::{
    body::Body,
    extract::Query,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const HMI_DIR: &str = "/var/lib/voltrus-hmi";
const CURRENT_FILE: &str = "current.html";
const VERSIONS_DIR: &str = "versions";
const MAX_VERSIONS: usize = 5;

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

#[derive(Deserialize)]
struct RollbackQuery {
    version: String,
}

struct AppState {
    hmi_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    let dir = std::env::var("HMI_DIR").unwrap_or_else(|_| HMI_DIR.to_string());
    let hmi_dir = PathBuf::from(&dir);

    fs::create_dir_all(hmi_dir.join(VERSIONS_DIR)).ok();

    let state = Arc::new(Mutex::new(AppState { hmi_dir }));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/deploy", post(deploy))
        .route("/api/rollback", post(rollback))
        .route("/", get(serve_hmi))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to port 8080");

    println!("mt-hmi-agent listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.expect("Server error");
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: "0.1.0".to_string(),
    })
}

async fn deploy(
    state: axum::extract::State<Arc<Mutex<AppState>>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let html = match String::from_utf8(body.to_vec()) {
        Ok(h) => h,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    let s = state.lock().await;
    let current = s.hmi_dir.join(CURRENT_FILE);

    // Archive current version
    if current.exists() {
        if let Ok(content) = fs::read_to_string(&current) {
            let hash = simple_hash(&content);
            let version_file = s.hmi_dir.join(VERSIONS_DIR).join(format!("{}.html", hash));
            fs::write(&version_file, &content).ok();
        }
        // Cleanup old versions (keep last MAX_VERSIONS)
        if let Ok(entries) = fs::read_dir(s.hmi_dir.join(VERSIONS_DIR)) {
            let mut files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "html"))
                .collect();
            files.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));
            while files.len() > MAX_VERSIONS {
                if let Some(old) = files.first() {
                    fs::remove_file(old.path()).ok();
                }
                files.remove(0);
            }
        }
    }

    match fs::write(&current, &html) {
        Ok(_) => (StatusCode::OK, "Deployed successfully".to_string()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Write failed: {}", e)),
    }
}

async fn rollback(
    state: axum::extract::State<Arc<Mutex<AppState>>>,
    Query(query): Query<RollbackQuery>,
) -> impl IntoResponse {
    let s = state.lock().await;
    let version_file = s
        .hmi_dir
        .join(VERSIONS_DIR)
        .join(format!("{}.html", query.version));

    if !version_file.exists() {
        return (
            StatusCode::NOT_FOUND,
            format!("Version {} not found", query.version),
        );
    }

    match fs::copy(&version_file, s.hmi_dir.join(CURRENT_FILE)) {
        Ok(_) => (StatusCode::OK, "Rollback successful".to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Rollback failed: {}", e),
        ),
    }
}

async fn serve_hmi(state: axum::extract::State<Arc<Mutex<AppState>>>) -> impl IntoResponse {
    let s = state.lock().await;
    let current = s.hmi_dir.join(CURRENT_FILE);

    match fs::read_to_string(&current) {
        Ok(html) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(html))
            .unwrap(),
        Err(_) => Html("<html><body><h1>No HMI deployed yet</h1></body></html>").into_response(),
    }
}

fn simple_hash(s: &str) -> String {
    let mut hash: u32 = 0;
    for b in s.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    format!("{:08x}", hash)
}
