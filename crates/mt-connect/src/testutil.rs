//! A tiny in-process HTTP/1.1 server so the tests exercise the real reqwest path.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    /// Path and query.
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

impl Req {
    pub fn form(&self) -> std::collections::HashMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes()).into_owned().collect()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

pub struct Reply(pub u16, pub String);

pub fn json(status: u16, body: serde_json::Value) -> Reply {
    Reply(status, body.to_string())
}

pub struct MockServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Req>>>,
}

impl MockServer {
    pub async fn start(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head_end, content_length) = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            break (pos + 4, len);
                        }
                    };
                    while buf.len() < head_end + content_length {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or("").split_whitespace();
                    let method = first.next().unwrap_or("").to_string();
                    let path = first.next().unwrap_or("").to_string();
                    let authorization = lines.find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("authorization").then(|| v.trim().to_string())
                    });
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let req = Req { method, path, authorization, body };
                    log.lock().push(req.clone());
                    let Reply(status, body) = handler(&req);
                    let res = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(res.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { base, requests }
    }

    pub fn count(&self, method: &str, path_prefix: &str) -> usize {
        self.requests.lock().iter().filter(|r| r.method == method && r.path.starts_with(path_prefix)).count()
    }
}

/// A base URL nothing listens on. Port 1 (tcpmux) is privileged and unused, so connects are refused. A freshly
/// released ephemeral port is not safe: a MockServer in a parallel test can bind it and answer.
pub async fn dead_base() -> String {
    "http://127.0.0.1:1".to_string()
}
