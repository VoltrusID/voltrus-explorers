//! One relay session: connect, authenticate via the pairing token, serve
//! requests until the socket drops. Reconnection lives in [`run_with_reconnect`].

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::connect_async;

use crate::backoff::Backoff;
use crate::driver::DeviceDriver;
use crate::protocol::{DeviceFrame, EventFrame, ResponseFrame};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Relay base URL, e.g. `wss://ide.voltrus.id/api/relay`.
    pub server: String,
    pub device_id: String,
    pub pairing_token: String,
}

impl SessionConfig {
    /// Full upgrade URL: `<server>/<deviceId>?role=device&token=<pairing token>`.
    ///
    /// No trailing slash: Hono's default strict routing 404s `/relay/:id/`.
    pub fn ws_url(&self) -> String {
        format!(
            "{}/{}?role=device&token={}",
            self.server.trim_end_matches('/'),
            self.device_id,
            self.pairing_token
        )
    }
}

/// Serve one ide request against the driver; errors become `ok:false` replies.
async fn dispatch(
    frame: DeviceFrame,
    driver: &mut impl DeviceDriver,
    events: &UnboundedSender<EventFrame>,
) -> ResponseFrame {
    let payload = frame.payload.clone().unwrap_or(Value::Null);
    let result: Result<Value> = match frame.op.as_str() {
        "browse" => driver.browse().await.map(|vars| json!({ "variables": vars })),
        "subscribe" => {
            let names = payload
                .get("names")
                .and_then(|v| v.as_array())
                .map(|names| names.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            driver.subscribe(names, events.clone()).await
        }
        "write" => match (payload.get("name").and_then(|v| v.as_str()), payload.get("value")) {
            (Some(name), Some(value)) => driver.write(name, value.clone()).await,
            _ => Err(anyhow::anyhow!("write needs {{name, value}}")),
        },
        "force" => match (payload.get("name").and_then(|v| v.as_str()), payload.get("value")) {
            (Some(name), Some(value)) => {
                driver.force(name, value.clone()).await.map(|_| json!({ "name": name, "forced": true }))
            }
            _ => Err(anyhow::anyhow!("force needs {{name, value}}")),
        },
        "unforce" => match payload.get("name").and_then(|v| v.as_str()) {
            Some(name) => driver.unforce(name).await.map(|_| json!({ "name": name, "forced": false })),
            None => Err(anyhow::anyhow!("unforce needs {{name}}")),
        },
        "status" => driver.status().await,
        "deploy" => match payload.get("module_json").and_then(|v| v.as_str()) {
            Some(module_json) => {
                driver
                    .deploy(
                        module_json,
                        payload.get("config_json").and_then(|v| v.as_str()).map(String::from),
                    )
                    .await
            }
            None => Err(anyhow::anyhow!("deploy needs {{module_json}}")),
        }
        other => Err(anyhow::anyhow!("unsupported op: {other}")),
    };
    match result {
        Ok(value) => ResponseFrame::ok(&frame.id, value),
        Err(e) => ResponseFrame::err(&frame.id, format!("{e:#}")),
    }
}

/// Connect, greet, and serve until the relay closes the socket. Returns when
/// the connection ends (clean close or error); the caller decides on retries.
pub async fn run_session(config: &SessionConfig, driver: &mut impl DeviceDriver) -> Result<()> {
    if !config.server.starts_with("ws://") && !config.server.starts_with("wss://") {
        anyhow::bail!("invalid relay server URL (expected ws:// or wss://…), got {}", config.server);
    }
    let request = config
        .ws_url()
        .into_client_request()
        .context("invalid relay server URL (expected ws:// or wss://…)")?;
    let (stream, _response) =
        connect_async(request).await.map_err(|e| anyhow::anyhow!("relay connect failed: {e}"))?;
    let (mut write, mut read) = stream.split();

    // Hello: informational only — the pairing token in the upgrade URL did the
    // auth. The DO ignores frames without `id` and `event`.
    let hello = json!({ "op": "hello", "payload": {
        "agent": "relay-agent",
        "version": env!("CARGO_PKG_VERSION"),
    } });
    write
        .send(Message::Text(hello.to_string().into()))
        .await
        .context("failed to send hello")?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    // Held so `event_rx.recv()` pends (never ends) while the session lives.
    let keepalive = event_tx.clone();

    let result = loop {
        tokio::select! {
            incoming = read.next() => {
                match incoming {
                    None => break Ok(()), // Relay closed the stream cleanly.
                    Some(Err(e)) => break Err(anyhow::anyhow!("relay socket error: {e}")),
                    Some(Ok(Message::Text(text))) => {
                        let frame: DeviceFrame = match serde_json::from_str(&text) {
                            Ok(frame) => frame,
                            Err(_) => continue, // Heartbeats and malformed junk: ignored.
                        };
                        let response = dispatch(frame, driver, &event_tx).await;
                        let text = serde_json::to_string(&response).expect("response frames serialize");
                        if let Err(e) = write.send(Message::Text(text.into())).await {
                            break Err(anyhow::anyhow!("failed to send response: {e}"));
                        }
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {} // tungstenite auto-replies.
                    Some(Ok(_)) => {} // Binary frames and close handshake: nothing to do.
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(event) => {
                        let text = serde_json::to_string(&event).expect("event frames serialize");
                        if let Err(e) = write.send(Message::Text(text.into())).await {
                            break Err(anyhow::anyhow!("failed to send event: {e}"));
                        }
                    }
                    None => unreachable!("keepalive sender outlives the loop"),
                }
            }
        }
    };

    drop(keepalive);
    result
}

/// Reconnect loop: exponential backoff with jitter, fresh driver per attempt.
/// Only a clean relay-side close is terminal; everything else retries.
pub async fn run_with_reconnect<D: DeviceDriver + Clone, F: Fn() -> D>(
    config: &SessionConfig,
    make_driver: F,
    base: Duration,
    max: Duration,
) -> ! {
    let mut backoff = Backoff::new(base, max);
    loop {
        let started = std::time::Instant::now();
        match run_session(config, &mut make_driver()).await {
            Ok(()) => std::process::exit(0),
            Err(e) => eprintln!("relay session ended: {e:#}"),
        }
        // A connection that lived a while resets the backoff sequence.
        if started.elapsed() > Duration::from_secs(30) {
            backoff.reset();
        }
        let delay = backoff.next_delay();
        eprintln!("retrying in {delay:.1?}");
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::SessionConfig;

    #[test]
    fn ws_url_has_no_trailing_slash_before_query() {
        // Hono's default strict routing 404s `/relay/:id/` — a trailing slash
        // would make every device dial fail forever.
        let config = SessionConfig {
            server: "wss://ide.voltrus.id/api/relay/".to_string(),
            device_id: "dev-1".to_string(),
            pairing_token: "tok".to_string(),
        };
        assert_eq!(
            config.ws_url(),
            "wss://ide.voltrus.id/api/relay/dev-1?role=device&token=tok"
        );
    }
}
