//! Raw frame capture and replay.
//!
//! A [`FocasClient`](crate::FocasClient) can record every request and response frame it
//! exchanges (see `FocasClient::connect_with_capture`). The result is a [`Capture`]: a JSON
//! document with one [`Exchange`] per round trip, requests and responses as hex bytes, each
//! tagged with the call that caused it.
//!
//! [`ReplayServer`] plays such a capture back over loopback TCP. It answers a request with the
//! response recorded for the same request bytes, so a capture taken from a real controller becomes
//! a regression test for the decoder: run the real client against the replay and compare the
//! decoded values with what was verified at the machine (`tests/replay.rs`).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::client::read_frame_into;
use crate::codec::{Frame, HEADER_LEN};

/// Value of [`Capture::format`].
pub const CAPTURE_FORMAT: &str = "focas-capture/1";

/// Bytes as lower-case hex pairs separated by spaces (`a0 a0 a0 a0 00 01 ...`), the notation of
/// the protocol notes.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parse hex written by [`to_hex`]. Whitespace, `:` and `-` between bytes are ignored, as is a
/// leading `0x`.
pub fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let text = text.strip_prefix("0x").unwrap_or(text);
    let digits: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != '-')
        .collect();
    if digits.len() % 2 != 0 {
        return Err("odd number of hex digits".into());
    }
    digits
        .chunks(2)
        .map(|pair| {
            let s: String = pair.iter().collect();
            u8::from_str_radix(&s, 16).map_err(|_| format!("'{s}' is not a hex byte"))
        })
        .collect()
}

/// One request/response round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exchange {
    /// The call that caused it (`open`, `statinfo`, `macro #500`, ...).
    pub label: String,
    /// The whole request frame, hex.
    pub request: String,
    /// The whole response frame, hex. `None` when nothing (or less than a frame header) came
    /// back: a timeout or a dropped connection.
    pub response: Option<String>,
    /// Why the exchange failed on the client side, when it did.
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// A recorded session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    /// Always [`CAPTURE_FORMAT`].
    pub format: String,
    /// Free-form facts about the recording: tool and version, host, controller identity,
    /// machine state, date.
    pub meta: BTreeMap<String, String>,
    pub exchanges: Vec<Exchange>,
}

impl Capture {
    pub fn new(meta: BTreeMap<String, String>, exchanges: Vec<Exchange>) -> Self {
        Capture {
            format: CAPTURE_FORMAT.into(),
            meta,
            exchanges,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a capture always serialises")
    }

    pub fn from_json(text: &str) -> Result<Capture, String> {
        let capture: Capture = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if capture.format != CAPTURE_FORMAT {
            return Err(format!(
                "capture format '{}' is not '{CAPTURE_FORMAT}'",
                capture.format
            ));
        }
        Ok(capture)
    }

    pub fn load(path: &Path) -> Result<Capture, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Capture::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Check that every request and response is hex of exactly one frame. Returns the problems.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, ex) in self.exchanges.iter().enumerate() {
            let at = format!("exchange {i} ({})", ex.label);
            match from_hex(&ex.request).map(|b| Frame::decode(&b)) {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => out.push(format!("{at}: request is not one frame: {e}")),
                Err(e) => out.push(format!("{at}: request: {e}")),
            }
            if let Some(resp) = &ex.response {
                match from_hex(resp).map(|b| Frame::decode(&b)) {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => out.push(format!("{at}: response is not one frame: {e}")),
                    Err(e) => out.push(format!("{at}: response: {e}")),
                }
            }
        }
        out
    }
}

/// The client side of recording: the label of the call in progress and what was seen so far.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub label: String,
    pub exchanges: Vec<Exchange>,
}

/// Responses recorded for one request.
#[derive(Debug)]
struct Script {
    responses: VecDeque<Option<Vec<u8>>>,
    total: usize,
    served: usize,
}

#[derive(Debug, Default)]
struct ReplayState {
    scripts: Mutex<HashMap<Vec<u8>, Script>>,
    misses: Mutex<Vec<String>>,
}

impl ReplayState {
    /// The response to play for `request`; `Err` for a request the capture does not contain,
    /// `Ok(None)` for one whose recorded answer was a dropped connection.
    fn answer(&self, request: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let mut scripts = self.scripts.lock().unwrap_or_else(|p| p.into_inner());
        let Some(script) = scripts.get_mut(request) else {
            return Err(());
        };
        script.served += 1;
        // Several answers to the same request play in order; the last one repeats.
        if script.responses.len() > 1 {
            Ok(script.responses.pop_front().expect("len > 1"))
        } else {
            Ok(script.responses.front().cloned().flatten())
        }
    }
}

/// A TCP server on loopback that answers requests from a [`Capture`].
///
/// A request the capture does not hold (the client now asks for something it did not when the
/// capture was taken) closes the connection and is listed by [`ReplayServer::misses`].
pub struct ReplayServer {
    addr: SocketAddr,
    state: Arc<ReplayState>,
    shutdown: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ReplayServer {
    /// Listen on a free loopback port and serve `capture`.
    pub async fn start(capture: &Capture) -> Result<ReplayServer, String> {
        let state = Arc::new(ReplayState::default());
        {
            let mut scripts = state.scripts.lock().unwrap_or_else(|p| p.into_inner());
            for (i, ex) in capture.exchanges.iter().enumerate() {
                let request = from_hex(&ex.request)
                    .map_err(|e| format!("exchange {i} ({}): request: {e}", ex.label))?;
                let response = match &ex.response {
                    Some(r) => Some(
                        from_hex(r)
                            .map_err(|e| format!("exchange {i} ({}): response: {e}", ex.label))?,
                    ),
                    None => None,
                };
                let script = scripts.entry(request).or_insert_with(|| Script {
                    responses: VecDeque::new(),
                    total: 0,
                    served: 0,
                });
                script.responses.push_back(response);
                script.total += 1;
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let (shutdown, rx) = watch::channel(false);
        let task = tokio::spawn(accept_loop(listener, Arc::clone(&state), rx));
        Ok(ReplayServer {
            addr,
            state,
            shutdown,
            task: Mutex::new(Some(task)),
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Requests (hex) that the capture did not contain.
    pub fn misses(&self) -> Vec<String> {
        self.state
            .misses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Recorded exchanges the client never asked for.
    pub fn unplayed(&self) -> usize {
        self.state
            .scripts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|s| s.total.saturating_sub(s.served))
            .sum()
    }

    pub async fn stop(&self) {
        let _ = self.shutdown.send(true);
        let task = self.task.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(t) = task {
            let _ = t.await;
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    state: Arc<ReplayState>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    stream.set_nodelay(true).ok();
                    let state = Arc::clone(&state);
                    sessions.spawn(async move { serve(stream, &state).await });
                }
                Err(e) => tracing::warn!("replay accept failed: {e}"),
            },
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
            _ = shutdown.changed() => break,
        }
    }
}

async fn serve(mut stream: TcpStream, state: &ReplayState) {
    let mut raw = Vec::with_capacity(HEADER_LEN);
    // The client's close ends its own session; any read error ends ours.
    while read_frame_into(&mut stream, &mut raw).await.is_ok() {
        match state.answer(&raw) {
            Ok(Some(response)) => {
                if stream.write_all(&response).await.is_err() {
                    return;
                }
            }
            // The recorded answer was silence: drop the connection like the controller did.
            Ok(None) => return,
            Err(()) => {
                state
                    .misses
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(to_hex(&raw));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_tolerates_separators() {
        let bytes = [0xa0, 0xa0, 0x00, 0x01, 0xff];
        assert_eq!(to_hex(&bytes), "a0 a0 00 01 ff");
        assert_eq!(from_hex("a0 a0 00 01 ff").unwrap(), bytes);
        assert_eq!(from_hex("A0:A0-00 01FF").unwrap(), bytes);
        assert_eq!(from_hex("0xa0a00001ff").unwrap(), bytes);
        assert!(from_hex("a0 a").is_err());
        assert!(from_hex("zz").is_err());
        assert_eq!(from_hex("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn capture_json_checks_the_format_tag() {
        let capture = Capture::new(BTreeMap::new(), Vec::new());
        assert_eq!(Capture::from_json(&capture.to_json()).unwrap(), capture);
        let wrong = capture.to_json().replace(CAPTURE_FORMAT, "other/9");
        assert!(Capture::from_json(&wrong).unwrap_err().contains("other/9"));
    }

    #[test]
    fn problems_flag_bad_frames() {
        let open = Frame::open_request(2).encode().unwrap();
        let mut capture = Capture::new(
            BTreeMap::new(),
            vec![Exchange {
                label: "open".into(),
                request: to_hex(&open),
                response: Some(to_hex(&Frame::open_response().encode().unwrap())),
                error: None,
                elapsed_ms: 1,
            }],
        );
        assert!(capture.problems().is_empty());
        capture.exchanges[0].response = Some("a0 a0".into());
        assert_eq!(capture.problems().len(), 1);
    }
}
