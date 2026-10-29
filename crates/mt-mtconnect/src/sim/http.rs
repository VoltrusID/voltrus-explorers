//! Minimal HTTP/1.1 server speaking the MTConnect agent REST interface
//! (`/probe`, `/current`, `/sample`, `/assets`, `/asset/{id}`).

use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::model::{QueryError, Rendered, SimModel};
use crate::error::AgentErrorCode;
use crate::util::percent_decode;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const DEFAULT_HEARTBEAT_MS: u64 = 10_000;
const BOUNDARY: &str = "voltrus-mtconnect-sim-boundary";

pub(crate) async fn serve(listener: TcpListener, model: SimModel, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((sock, _)) = accepted else { continue };
                let model = model.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(sock, model, shutdown).await;
                });
            }
            _ = shutdown.changed() => return,
        }
    }
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    close: bool,
}

fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, query_str) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in query_str.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode(k), percent_decode(v));
    }
    let close = lines.any(|l| {
        l.split_once(':')
            .is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("connection") && v.trim().eq_ignore_ascii_case("close"))
    });
    Some(Request { method, path: percent_decode(path), query, close })
}

/// Read one request head. `Ok(None)` on clean close before any byte.
async fn read_head(sock: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<String>> {
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
            buf.drain(..pos + 4);
            return Ok(Some(head));
        }
        if buf.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::other("request too large"));
        }
        let mut tmp = [0u8; 2048];
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn status_line(code: u16) -> &'static str {
    match code {
        200 => "200 OK",
        400 => "400 Bad Request",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        _ => "500 Internal Server Error",
    }
}

async fn write_response(sock: &mut TcpStream, code: u16, body: &str, close: bool) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: {}\r\nCache-Control: no-cache\r\n\r\n",
        status_line(code),
        body.len(),
        if close { "close" } else { "keep-alive" }
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body.as_bytes()).await?;
    sock.flush().await
}

fn http_status(code: &AgentErrorCode) -> u16 {
    match code {
        AgentErrorCode::NoDevice | AgentErrorCode::AssetNotFound => 404,
        AgentErrorCode::InternalError => 500,
        _ => 400,
    }
}

async fn write_query_error(sock: &mut TcpStream, model: &SimModel, e: &QueryError, close: bool) -> std::io::Result<()> {
    let body = model.render_error(&e.code, &e.message);
    write_response(sock, http_status(&e.code), &body, close).await
}

enum Endpoint {
    Probe,
    Current,
    Sample,
    Assets,
    Asset(Vec<String>),
}

/// Split `/[device/]endpoint[/ids]`.
fn route(path: &str) -> Option<(Option<String>, Endpoint)> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let endpoint = |name: &str, rest: &[&str]| -> Option<Endpoint> {
        match (name, rest) {
            ("probe", []) => Some(Endpoint::Probe),
            ("current", []) => Some(Endpoint::Current),
            ("sample", []) => Some(Endpoint::Sample),
            ("assets", []) => Some(Endpoint::Assets),
            ("assets" | "asset", [ids]) => Some(Endpoint::Asset(ids.split(';').map(str::to_string).collect())),
            _ => None,
        }
    };
    match segs.as_slice() {
        [] => Some((None, Endpoint::Probe)),
        [name, rest @ ..] => {
            if let Some(e) = endpoint(name, rest) {
                return Some((None, e));
            }
            let (device, tail) = (segs[0], &segs[1..]);
            match tail {
                [] => Some((Some(device.to_string()), Endpoint::Probe)),
                [name, rest @ ..] => endpoint(name, rest).map(|e| (Some(device.to_string()), e)),
            }
        }
    }
}

fn parse_u64(q: &HashMap<String, String>, key: &str) -> Result<Option<u64>, QueryError> {
    match q.get(key) {
        None => Ok(None),
        Some(v) => v.trim().parse::<u64>().map(Some).map_err(|_| QueryError {
            code: AgentErrorCode::InvalidParameterValue,
            message: format!("query parameter '{key}': cannot convert string '{v}' to unsigned integer"),
        }),
    }
}

async fn handle_connection(
    mut sock: TcpStream,
    model: SimModel,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    loop {
        let head = tokio::select! {
            h = read_head(&mut sock, &mut buf) => h?,
            _ = shutdown.changed() => return Ok(()),
        };
        let Some(head) = head else { return Ok(()) };
        let Some(req) = parse_request(&head) else {
            let body = model.render_error(&AgentErrorCode::InvalidRequest, "Malformed request");
            return write_response(&mut sock, 400, &body, true).await;
        };
        if req.method != "GET" {
            let body = model.render_error(&AgentErrorCode::Unsupported, "Only GET is supported");
            return write_response(&mut sock, 405, &body, true).await;
        }
        let Some((device, endpoint)) = route(&req.path) else {
            let body = model.render_error(&AgentErrorCode::InvalidUri, &format!("Cannot find handler for: GET {}", req.path));
            write_response(&mut sock, 404, &body, req.close).await?;
            if req.close {
                return Ok(());
            }
            continue;
        };

        if let Err(e) = model.check_device(device.as_deref()) {
            write_query_error(&mut sock, &model, &e, req.close).await?;
            if req.close {
                return Ok(());
            }
            continue;
        }

        let keep_open = serve_endpoint(&mut sock, &model, &req, endpoint, &mut shutdown).await?;
        if !keep_open || req.close {
            return Ok(());
        }
    }
}

/// Serve one request. Returns whether the connection may be reused.
async fn serve_endpoint(
    sock: &mut TcpStream,
    model: &SimModel,
    req: &Request,
    endpoint: Endpoint,
    shutdown: &mut watch::Receiver<bool>,
) -> std::io::Result<bool> {
    let path = req.query.get("path").map(String::as_str);
    match endpoint {
        Endpoint::Probe => write_response(sock, 200, &model.render_probe(), req.close).await?,
        Endpoint::Assets => {
            let count = parse_u64(&req.query, "count").ok().flatten().map(|c| c as usize);
            let removed = req.query.get("removed").is_some_and(|v| v == "true");
            let body = model.render_assets(req.query.get("type").map(String::as_str), count, removed);
            write_response(sock, 200, &body, req.close).await?;
        }
        Endpoint::Asset(ids) => {
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            match model.render_asset(&ids) {
                Ok(body) => write_response(sock, 200, &body, req.close).await?,
                Err(e) => write_query_error(sock, model, &e, req.close).await?,
            }
        }
        Endpoint::Current | Endpoint::Sample => {
            let is_sample = matches!(endpoint, Endpoint::Sample);
            let params = (|| {
                let from = parse_u64(&req.query, "from")?;
                let interval = parse_u64(&req.query, "interval")?;
                let heartbeat = parse_u64(&req.query, "heartbeat")?;
                let count = match req.query.get("count") {
                    None => None,
                    Some(v) => Some(v.trim().parse::<i64>().map_err(|_| QueryError {
                        code: AgentErrorCode::InvalidParameterValue,
                        message: format!("query parameter 'count': cannot convert string '{v}' to integer"),
                    })?),
                };
                Ok::<_, QueryError>((from, count, interval, heartbeat))
            })();
            let (from, count, interval, heartbeat) = match params {
                Ok(p) => p,
                Err(e) => {
                    write_query_error(sock, model, &e, req.close).await?;
                    return Ok(true);
                }
            };
            let first = if is_sample { model.render_sample(path, from, count) } else { model.render_current(path) };
            match (first, interval) {
                (Err(e), _) => write_query_error(sock, model, &e, req.close).await?,
                (Ok(r), None) => write_response(sock, 200, &r.xml, req.close).await?,
                (Ok(r), Some(interval)) => {
                    let spec = StreamSpec {
                        is_sample,
                        path: path.map(str::to_string),
                        count,
                        interval: Duration::from_millis(interval),
                        heartbeat: Duration::from_millis(heartbeat.unwrap_or(DEFAULT_HEARTBEAT_MS)),
                    };
                    stream(sock, model, spec, r, shutdown).await?;
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

struct StreamSpec {
    is_sample: bool,
    path: Option<String>,
    count: Option<i64>,
    interval: Duration,
    heartbeat: Duration,
}

async fn write_part(sock: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let part = format!(
        "--{BOUNDARY}\r\nContent-type: text/xml\r\nContent-length: {}\r\n\r\n{body}\r\n",
        body.len()
    );
    let chunk = format!("{:x}\r\n{part}\r\n", part.len());
    sock.write_all(chunk.as_bytes()).await?;
    sock.flush().await
}

/// `multipart/x-mixed-replace` streaming for `?interval=`: a document right away, then one for
/// every batch of new observations (or an empty one per `heartbeat`), at most one per
/// `interval`. Ends when the agent instance changes (restart) or the client disconnects.
async fn stream(
    sock: &mut TcpStream,
    model: &SimModel,
    spec: StreamSpec,
    first: Rendered,
    shutdown: &mut watch::Receiver<bool>,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary={BOUNDARY}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nCache-Control: no-cache\r\n\r\n"
    );
    sock.write_all(head.as_bytes()).await?;

    let mut seq_rx = model.watch_sequence();
    let instance = model.instance_id();
    let mut next = first.next_sequence;
    write_part(sock, &first.xml).await?;

    loop {
        // Wait for new data, a heartbeat, or shutdown.
        let heartbeat = tokio::time::sleep(spec.heartbeat);
        tokio::pin!(heartbeat);
        loop {
            let (inst, last) = *seq_rx.borrow_and_update();
            if inst != instance {
                return Ok(()); // agent restarted: close, like a real agent
            }
            if last >= next {
                break;
            }
            tokio::select! {
                changed = seq_rx.changed() => if changed.is_err() { return Ok(()) },
                _ = &mut heartbeat => break,
                _ = shutdown.changed() => return Ok(()),
            }
        }

        let rendered = if spec.is_sample {
            model.render_sample(spec.path.as_deref(), Some(next), spec.count)
        } else {
            model.render_current(spec.path.as_deref())
        };
        match rendered {
            Ok(r) => {
                next = r.next_sequence;
                write_part(sock, &r.xml).await?;
            }
            Err(e) => {
                // e.g. OUT_OF_RANGE after the buffer wrapped past `next`: report and end.
                write_part(sock, &model.render_error(&e.code, &e.message)).await?;
                return Ok(());
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(spec.interval) => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        assert!(matches!(route("/"), Some((None, Endpoint::Probe))));
        assert!(matches!(route("/current"), Some((None, Endpoint::Current))));
        assert!(matches!(route("/VMC/sample"), Some((Some(d), Endpoint::Sample)) if d == "VMC"));
        assert!(matches!(route("/VMC"), Some((Some(_), Endpoint::Probe))));
        assert!(matches!(route("/asset/A;B"), Some((None, Endpoint::Asset(ids))) if ids == ["A", "B"]));
        assert!(matches!(route("/assets/T1"), Some((None, Endpoint::Asset(_)))));
        assert!(route("/VMC/nope/x").is_none());
    }

    #[test]
    fn request_parsing() {
        let r = parse_request("GET /sample?from=5&path=%2F%2FAxes&count=2 HTTP/1.1\r\nHost: x\r\nConnection: close").unwrap();
        assert_eq!(r.path, "/sample");
        assert_eq!(r.query["from"], "5");
        assert_eq!(r.query["path"], "//Axes");
        assert!(r.close);
    }
}
