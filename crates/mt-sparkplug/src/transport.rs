//! MQTT transports behind [`crate::SparkplugPublisher`].

use std::time::Duration;

use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};

use crate::SparkplugError;

/// The MQTT Will registered in the CONNECT packet.
///
/// Sparkplug edge nodes register the NDEATH topic with a payload carrying the
/// pending bdSeq (Sparkplug 3.0: QoS 1, retain=false).
#[derive(Debug, Clone)]
pub struct Will {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// Publish transport used by [`crate::SparkplugPublisher`]. Mock this in
/// tests; [`RumqttcTransport`] is the real implementation.
///
/// Native async-fn-in-trait (no `#[async_trait]`): the publisher is generic
/// over the transport, so the futures never need to be boxed behind `dyn`,
/// and `Send` bounds are unnecessary — each publisher lives on one task.
#[allow(async_fn_in_trait)]
pub trait MqttTransport {
    /// Open the MQTT session, registering `will`.
    async fn connect(&mut self, will: Will) -> Result<(), SparkplugError>;

    /// Publish at QoS 0, retain=false (all non-STATE Sparkplug messages MUST
    /// use QoS 0 and MUST NOT be retained).
    async fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), SparkplugError>;
}

/// Real transport over `rumqttc` (MQTT 3.1.1, clean session = true as the
/// Sparkplug normative statements require).
///
/// Note: we deliberately do not adapt `mt-mqtt`'s `MqttPubClient` — it is
/// built around subscription/RTT bookkeeping for the explorer apps and drags
/// the embedded `rumqttd` broker into this crate's dependency tree. A
/// publish-only edge node needs very little rumqttc directly.
pub struct RumqttcTransport {
    host: String,
    port: u16,
    client_id: String,
    keep_alive: Duration,
    client: Option<AsyncClient>,
}

impl RumqttcTransport {
    /// Configure a transport for `broker_url` (`tcp://host:port`).
    pub fn new(broker_url: &str, client_id: impl Into<String>) -> Result<Self, SparkplugError> {
        let unsupported = || {
            SparkplugError::Transport(format!(
                "unsupported broker URL {broker_url:?}: expected tcp://host:port"
            ))
        };
        let rest = broker_url.strip_prefix("tcp://").ok_or_else(unsupported)?;
        let (host, port) = rest.rsplit_once(':').ok_or_else(unsupported)?;
        let port = port.parse::<u16>().map_err(|_| unsupported())?;
        Ok(Self {
            host: host.to_string(),
            port,
            client_id: client_id.into(),
            keep_alive: Duration::from_secs(30),
            client: None,
        })
    }
}

impl MqttTransport for RumqttcTransport {
    async fn connect(&mut self, will: Will) -> Result<(), SparkplugError> {
        let mut options = MqttOptions::new(&self.client_id, &self.host, self.port);
        options.set_keep_alive(self.keep_alive);
        options.set_last_will(LastWill::new(
            will.topic,
            will.payload,
            // Sparkplug 3.0: the NDEATH Will is registered at QoS 1.
            QoS::AtLeastOnce,
            false,
        ));

        let (client, mut eventloop) = AsyncClient::new(options, 16);

        // Confirm the session is actually up before anything is birthed.
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => break,
                Ok(_) => continue,
                Err(e) => {
                    return Err(SparkplugError::Transport(format!(
                        "MQTT connect to {}:{} failed: {e}",
                        self.host, self.port
                    )));
                }
            }
        }

        // Keep draining the event loop so queued publishes reach the broker.
        // Minimal-scope caveat: on the first network error we log and stop
        // draining, so a dead session surfaces as publish failures instead of
        // silent loss. The integration runtime should then call
        // `SparkplugPublisher::connect` again, which re-births with a fresh
        // bdSeq (and a Will to match).
        let host = self.host.clone();
        let port = self.port;
        tokio::spawn(async move {
            loop {
                if let Err(e) = eventloop.poll().await {
                    tracing::warn!("MQTT session to {host}:{port} lost: {e}");
                    break;
                }
            }
        });

        self.client = Some(client);
        Ok(())
    }

    async fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), SparkplugError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| SparkplugError::Transport("not connected".to_string()))?;
        client
            .publish(topic, QoS::AtMostOnce, false, payload.to_vec())
            .await
            .map_err(|e| SparkplugError::Transport(format!("publish {topic:?}: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_url_parsing() {
        let transport = RumqttcTransport::new("tcp://broker.local:1883", "edge1").unwrap();
        assert_eq!(transport.host, "broker.local");
        assert_eq!(transport.port, 1883);

        assert!(RumqttcTransport::new("mqtt://broker.local:1883", "edge1").is_err());
        assert!(RumqttcTransport::new("tcp://broker.local", "edge1").is_err());
        assert!(RumqttcTransport::new("tcp://broker.local:notaport", "edge1").is_err());
    }
}
