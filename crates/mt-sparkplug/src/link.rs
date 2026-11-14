//! MQTT connection for an [`crate::EdgeNode`]: connects with the NDEATH
//! Will (QoS 1, retain off), subscribes to NCMD/DCMD and optionally the
//! primary host STATE topic, publishes at QoS 0, and reports what it
//! receives as [`LinkEvent`]s.

use std::time::Duration;

use rumqttc::{
    AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS, TlsConfiguration, Transport,
};
use tokio::sync::mpsc;

use crate::SparkplugError;
use crate::edge::Outgoing;

/// Broker connection settings.
#[derive(Debug, Clone, Default)]
pub struct BrokerConfig {
    pub host: String,
    /// 0 = 1883, or 8883 with TLS.
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
    /// PEM CA for private brokers; empty = system roots.
    pub ca_pem: Option<String>,
    /// 0 = 30 s.
    pub keep_alive_secs: u16,
}

/// What the broker connection delivered.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// NCMD or DCMD for this edge node.
    Command { topic: String, payload: Vec<u8> },
    /// Primary host STATE changed (Sparkplug 3.0 JSON or 2.2 text).
    HostState { online: bool },
    /// The connection ended; reconnect with a new session.
    Lost(String),
}

/// `STATE` payload: `{"online":true,...}` (3.0) or `ONLINE`/`OFFLINE` (2.2).
pub fn parse_state(payload: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(payload).ok()?.trim();
    match text {
        "ONLINE" => return Some(true),
        "OFFLINE" => return Some(false),
        _ => {}
    }
    // Minimal JSON read: avoid a serde dependency for one boolean.
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("\"online\":true") {
        Some(true)
    } else if compact.contains("\"online\":false") {
        Some(false)
    } else {
        None
    }
}

pub struct EdgeLink {
    client: AsyncClient,
    pub events: mpsc::Receiver<LinkEvent>,
}

impl EdgeLink {
    /// Connect with `will` as the MQTT Will and subscribe to `filters` plus
    /// `spBv1.0/STATE/<primary_host>` when given. Returns once the broker
    /// accepted the connection.
    pub async fn connect(
        cfg: &BrokerConfig,
        will: Outgoing,
        filters: &[String],
        primary_host: Option<&str>,
    ) -> Result<Self, SparkplugError> {
        let port = match cfg.port {
            0 if cfg.tls => 8883,
            0 => 1883,
            p => p,
        };
        let mut opts = MqttOptions::new(&cfg.client_id, &cfg.host, port);
        opts.set_keep_alive(Duration::from_secs(if cfg.keep_alive_secs == 0 {
            30
        } else {
            cfg.keep_alive_secs.into()
        }));
        // Sparkplug requires a clean session.
        opts.set_clean_session(true);
        opts.set_max_packet_size(1 << 20, 1 << 20);
        opts.set_last_will(LastWill::new(
            will.topic,
            will.payload,
            QoS::AtLeastOnce,
            false,
        ));
        if let Some(user) = &cfg.username {
            opts.set_credentials(user, cfg.password.clone().unwrap_or_default());
        }
        if cfg.tls {
            let transport = match cfg.ca_pem.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(ca) => Transport::tls_with_config(TlsConfiguration::Simple {
                    ca: ca.as_bytes().to_vec(),
                    alpn: None,
                    client_auth: None,
                }),
                None => Transport::tls_with_default_config(),
            };
            opts.set_transport(transport);
        }

        let (client, mut el) = AsyncClient::new(opts, 64);
        loop {
            match el.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => break,
                Ok(_) => {}
                Err(e) => {
                    return Err(SparkplugError::Transport(format!(
                        "connect to {}:{port}: {e}",
                        cfg.host
                    )));
                }
            }
        }
        let state_topic = primary_host.map(|h| format!("spBv1.0/STATE/{h}"));
        for f in filters {
            client
                .subscribe(f, QoS::AtLeastOnce)
                .await
                .map_err(|e| SparkplugError::Transport(format!("subscribe {f}: {e}")))?;
        }
        if let Some(t) = &state_topic {
            client
                .subscribe(t, QoS::AtLeastOnce)
                .await
                .map_err(|e| SparkplugError::Transport(format!("subscribe {t}: {e}")))?;
        }

        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            loop {
                match el.poll().await {
                    Ok(Event::Incoming(Incoming::Publish(p))) => {
                        let ev = if Some(&p.topic) == state_topic.as_ref() {
                            match parse_state(&p.payload) {
                                Some(online) => LinkEvent::HostState { online },
                                None => continue,
                            }
                        } else {
                            LinkEvent::Command {
                                topic: p.topic,
                                payload: p.payload.to_vec(),
                            }
                        };
                        if tx.send(ev).await.is_err() {
                            return;
                        }
                    }
                    Ok(Event::Incoming(Incoming::Disconnect)) => {
                        let _ = tx
                            .send(LinkEvent::Lost("broker closed the connection".into()))
                            .await;
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = tx.send(LinkEvent::Lost(e.to_string())).await;
                        return;
                    }
                }
            }
        });
        Ok(Self { client, events: rx })
    }

    /// Publish at QoS 0, retain off.
    pub async fn publish(&self, msg: &Outgoing) -> Result<(), SparkplugError> {
        self.client
            .publish(&msg.topic, QoS::AtMostOnce, false, msg.payload.clone())
            .await
            .map_err(|e| SparkplugError::Transport(format!("publish {}: {e}", msg.topic)))
    }

    /// Send DISCONNECT (the broker then drops the Will).
    pub async fn disconnect(&self) {
        let _ = self.client.disconnect().await;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_state;

    #[test]
    fn state_payloads() {
        assert_eq!(
            parse_state(br#"{"online": true, "timestamp": 1}"#),
            Some(true)
        );
        assert_eq!(
            parse_state(br#"{"online":false,"timestamp":1}"#),
            Some(false)
        );
        assert_eq!(parse_state(b"ONLINE"), Some(true));
        assert_eq!(parse_state(b"OFFLINE"), Some(false));
        assert_eq!(parse_state(b"garbage"), None);
    }
}
