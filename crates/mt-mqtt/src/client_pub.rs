use rumqttc::{
    AsyncClient, Event, EventLoop, Incoming, MqttOptions, Outgoing, Publish, QoS,
    SubscribeReasonCode, TlsConfiguration, Transport,
};

use crate::types::{MqttClientConfig, MqttMessage, MqttQos, MqttTlsConfig};

/// A reusable MQTT client for publishing and subscribing.
pub struct MqttPubClient {
    client: AsyncClient,
    eventloop: EventLoop,
    /// Active subscriptions, kept so we can re-subscribe automatically after
    /// rumqttc reconnects (clean sessions drop them on every reconnect).
    subscriptions: Vec<(String, MqttQos)>,
    /// Reconnect bookkeeping — persisted across `next_event` calls so that the
    /// caller can time out individual polls without losing error history.
    consecutive_errors: u32,
    first_error_at: Option<std::time::Instant>,
    /// Last measured PING round-trip (PINGREQ → PINGRESP), in milliseconds.
    last_rtt_ms: Option<u32>,
    /// When the most recent PINGREQ was handed to the network by rumqttc.
    ping_sent_at: Option<std::time::Instant>,
}

impl MqttPubClient {
    /// Create a new MQTT client and connect to the broker.
    /// Waits for ConnAck to confirm the connection is actually established.
    pub async fn connect(config: MqttClientConfig) -> Result<Self, String> {
        let client_id = if config.client_id.is_empty() {
            format!("mt-mqtt-client-{}", std::process::id())
        } else {
            config.client_id
        };

        let mut options = MqttOptions::new(client_id, &config.host, config.port);
        options.set_keep_alive(std::time::Duration::from_secs(config.keep_alive_secs));

        if let (Some(user), Some(pass)) = (&config.username, &config.password) {
            options.set_credentials(user, pass);
        }

        // Optional Last Will & Testament.
        if let Some(will) = &config.last_will {
            if !will.topic.trim().is_empty() {
                let lw = rumqttc::LastWill::new(
                    will.topic.clone(),
                    will.payload.as_bytes().to_vec(),
                    will.qos.into(),
                    will.retain,
                );
                options.set_last_will(lw);
            }
        }

        // Optional TLS (MQTTS / mTLS / private CA).
        if config.tls.enabled {
            options.set_transport(build_transport(&config.tls)?);
        }

        let (client, mut eventloop) = AsyncClient::new(options, 10);

        // Poll for ConnAck to verify the connection actually works
        let ack = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            async {
                loop {
                    match eventloop.poll().await {
                        Ok(Event::Incoming(Incoming::ConnAck(ack))) => return Ok(ack),
                        Ok(_) => continue,
                        Err(e) => return Err(format!("Connection error: {}", e)),
                    }
                }
            },
        )
        .await;

        match ack {
            Ok(Ok(ack)) => {
                if ack.code != rumqttc::ConnectReturnCode::Success {
                    return Err(format!("Connection refused: {:?}", ack.code));
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("Connection timed out after 5 seconds".to_string()),
        }

        Ok(Self {
            client,
            eventloop,
            subscriptions: Vec::new(),
            consecutive_errors: 0,
            first_error_at: None,
            last_rtt_ms: None,
            ping_sent_at: None,
        })
    }

    /// Publish a message to the given topic.
    pub async fn publish(&self, msg: &MqttMessage) -> Result<(), String> {
        let qos: QoS = msg.qos.into();
        self.client
            .publish(&msg.topic, qos, msg.retained, msg.payload.clone())
            .await
            .map_err(|e| format!("Publish failed: {}", e))
    }

    /// Subscribe to a topic filter. The subscription is remembered so it can be
    /// restored automatically after a reconnect.
    pub async fn subscribe(&mut self, topic: &str, qos: MqttQos) -> Result<(), String> {
        let qos_v: QoS = qos.into();
        self.client
            .subscribe(topic, qos_v)
            .await
            .map_err(|e| format!("Subscribe failed: {}", e))?;
        // Replace if the same filter already exists, otherwise append.
        if let Some(slot) = self.subscriptions.iter_mut().find(|(t, _)| t == topic) {
            slot.1 = qos;
        } else {
            self.subscriptions.push((topic.to_string(), qos));
        }
        Ok(())
    }

    /// Subscribe to a topic filter and **block until the broker acknowledges**
    /// it (SUBACK), returning the QoS the broker actually granted. Messages
    /// that arrive while we wait are buffered and returned so the caller can
    /// dispatch them — they would otherwise be consumed off the eventloop.
    ///
    /// rumqttc's high-level `subscribe` does not expose the packet id, so a
    /// SUBACK cannot be matched to a specific request. This is safe because the
    /// client task processes actions serially (at most one manual subscribe is
    /// outstanding); SUBACKs from automatic reconnect re-subs are skipped via a
    /// local counter.
    pub async fn subscribe_confirm(&mut self, topic: &str, qos: MqttQos) -> SubscribeOutcome {
        let mut buffered: Vec<MqttClientEvent> = Vec::new();
        let mut error: Option<String> = None;
        let mut reconnected = false;

        // Queue the SUBSCRIBE and remember it so it survives reconnects.
        if let Err(e) = self.client.subscribe(topic.to_string(), qos.into()).await {
            return SubscribeOutcome {
                granted_qos: None,
                buffered,
                error: Some(format!("Subscribe failed: {}", e)),
            };
        }
        if let Some(slot) = self.subscriptions.iter_mut().find(|(t, _)| t == topic) {
            slot.1 = qos;
        } else {
            self.subscriptions.push((topic.to_string(), qos));
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // SUBACKs owed to reconnect re-subs seen during the wait — skip them.
        let mut resub_acks_pending: u32 = 0;

        let granted = loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                error = Some("Timed out waiting for SUBACK".to_string());
                break None;
            }
            match tokio::time::timeout(deadline - now, self.eventloop.poll()).await {
                Err(_) => {
                    error = Some("Timed out waiting for SUBACK".to_string());
                    break None;
                }
                Ok(Err(e)) => {
                    error = Some(format!("Connection error during subscribe: {}", e));
                    break None;
                }
                Ok(Ok(event)) => match event {
                    Event::Incoming(Incoming::SubAck(sa)) => {
                        if resub_acks_pending > 0 {
                            resub_acks_pending -= 1;
                            continue;
                        }
                        match sa.return_codes.first() {
                            Some(SubscribeReasonCode::Success(q)) => break Some(MqttQos::from(*q)),
                            Some(SubscribeReasonCode::Failure) => {
                                error = Some(
                                    "Broker rejected the subscription (Failure)".to_string(),
                                );
                                break None;
                            }
                            None => {
                                error = Some("Broker returned an empty SUBACK".to_string());
                                break None;
                            }
                        }
                    }
                    Event::Incoming(Incoming::ConnAck(_)) => {
                        // A reconnect superseded the original request (clean
                        // session). Reset bookkeeping, re-establish subs, and
                        // report best-effort QoS — our ack is now mixed into the
                        // re-sub acks and cannot be individually matched.
                        self.consecutive_errors = 0;
                        self.first_error_at = None;
                        self.resubscribe_all().await;
                        reconnected = true;
                        break None;
                    }
                    Event::Incoming(Incoming::Publish(p)) => {
                        buffered.push(MqttClientEvent::Message(publish_to_message(p)));
                        continue;
                    }
                    Event::Outgoing(Outgoing::PingReq) => {
                        self.ping_sent_at = Some(std::time::Instant::now());
                        continue;
                    }
                    Event::Incoming(Incoming::PingResp) => {
                        if let Some(sent) = self.ping_sent_at.take() {
                            self.last_rtt_ms = Some(sent.elapsed().as_millis() as u32);
                        }
                        continue;
                    }
                    _ => continue,
                },
            }
        };

        let granted_qos = granted.or_else(|| if reconnected { Some(qos) } else { None });
        SubscribeOutcome { granted_qos, buffered, error }
    }

    /// Unsubscribe from a topic filter and forget it.
    pub async fn unsubscribe(&mut self, topic: &str) -> Result<(), String> {
        self.client
            .unsubscribe(topic)
            .await
            .map_err(|e| format!("Unsubscribe failed: {}", e))?;
        self.subscriptions.retain(|(t, _)| t != topic);
        Ok(())
    }

    /// Poll for the next event from the broker.
    ///
    /// Returns `None` when the connection has been permanently lost (N
    /// consecutive errors) or cleanly closed. Error counters live on `self` so
    /// the caller may time out an individual poll without losing reconnect
    /// history. On every (re)connection the recorded subscriptions are restored.
    pub async fn next_event(&mut self) -> Option<MqttClientEvent> {
        const MAX_CONSECUTIVE_ERRORS: u32 = 10;
        const MAX_DISCONNECT_DURATION: std::time::Duration = std::time::Duration::from_secs(30);

        loop {
            match self.eventloop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    return Some(MqttClientEvent::Message(publish_to_message(publish)));
                }
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    // Successful (re)connection — reset the error counter…
                    if self.consecutive_errors > 0 {
                        tracing::info!(
                            "MQTT connection restored after {} consecutive errors",
                            self.consecutive_errors
                        );
                    }
                    self.consecutive_errors = 0;
                    self.first_error_at = None;
                    // …and re-establish subscriptions (clean session drops them).
                    self.resubscribe_all().await;
                    continue;
                }
                Ok(Event::Incoming(Incoming::PingResp)) => {
                    // Complete a PING round-trip measurement.
                    if let Some(sent) = self.ping_sent_at.take() {
                        let ms = sent.elapsed().as_millis() as u32;
                        self.last_rtt_ms = Some(ms);
                        return Some(MqttClientEvent::Rtt(ms));
                    }
                    continue;
                }
                Ok(Event::Incoming(Incoming::SubAck(_))) => continue,
                Ok(Event::Incoming(_)) => continue,
                Ok(Event::Outgoing(Outgoing::PingReq)) => {
                    // rumqttc sent a keep-alive ping — stamp the RTT stopwatch.
                    self.ping_sent_at = Some(std::time::Instant::now());
                    continue;
                }
                Ok(Event::Outgoing(_)) => continue,
                Err(e) => {
                    self.consecutive_errors += 1;
                    if self.first_error_at.is_none() {
                        self.first_error_at = Some(std::time::Instant::now());
                    }

                    tracing::warn!(
                        "MQTT client event error (attempt {}/{}): {}",
                        self.consecutive_errors,
                        MAX_CONSECUTIVE_ERRORS,
                        e
                    );

                    if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        tracing::error!(
                            "MQTT connection lost after {} consecutive errors",
                            self.consecutive_errors
                        );
                        return None;
                    }

                    if let Some(start) = self.first_error_at {
                        if start.elapsed() >= MAX_DISCONNECT_DURATION {
                            tracing::error!(
                                "MQTT connection lost after {:?} of errors",
                                MAX_DISCONNECT_DURATION
                            );
                            return None;
                        }
                    }

                    // rumqttc automatically reconnects, keep polling (up to limits)
                    continue;
                }
            }
        }
    }

    /// Re-subscribe to all remembered filters. Used after a reconnect.
    /// `&mut self` (not `&self`): callers hold this across `.await` inside
    /// `next_event`, and a shared borrow would require `MqttPubClient: Sync`,
    /// which rumqttc's `EventLoop` (non-`Sync` `Box<dyn N>` socket) forbids.
    async fn resubscribe_all(&mut self) {
        if self.subscriptions.is_empty() {
            return;
        }
        tracing::info!(
            "Re-establishing {} subscription(s) after reconnect",
            self.subscriptions.len()
        );
        for (topic, qos) in &self.subscriptions {
            if let Err(e) = self.client.subscribe(topic.clone(), (*qos).into()).await {
                tracing::warn!("Re-subscribe to {} failed: {}", topic, e);
            }
        }
    }

    /// Get a reference to the underlying `AsyncClient` for advanced usage.
    pub fn raw_client(&self) -> &AsyncClient {
        &self.client
    }

    /// Poll the eventloop directly, returning the raw event.
    pub async fn raw_eventloop_poll(&mut self) -> Result<rumqttc::Event, rumqttc::ConnectionError> {
        self.eventloop.poll().await
    }

    /// Disconnect from the broker gracefully. Takes `&self` so it can be called
    /// from a `select!` arm alongside `next_event(&mut self)`.
    pub async fn disconnect(&self) -> Result<(), String> {
        self.client
            .disconnect()
            .await
            .map_err(|e| format!("Disconnect failed: {}", e))
    }
}

/// Events produced by the MQTT client.
#[derive(Debug, Clone)]
pub enum MqttClientEvent {
    /// A message was received on a subscribed topic.
    Message(MqttMessage),
    /// A fresh round-trip latency sample (PINGREQ → PINGRESP), in milliseconds.
    Rtt(u32),
}

/// Outcome of [`MqttPubClient::subscribe_confirm`].
#[derive(Debug)]
pub struct SubscribeOutcome {
    /// The QoS the broker granted (may be lower than requested), or `None` if
    /// the broker rejected/never acknowledged the subscription.
    pub granted_qos: Option<MqttQos>,
    /// Messages that arrived while waiting for the SUBACK.
    pub buffered: Vec<MqttClientEvent>,
    /// Set when the subscription could not be confirmed (timeout / rejected).
    pub error: Option<String>,
}

fn publish_to_message(p: Publish) -> MqttMessage {
    MqttMessage {
        topic: p.topic.clone(),
        payload: p.payload.to_vec(),
        qos: p.qos.into(),
        retained: p.retain,
    }
}

/// Build a rumqttc `Transport` from our TLS config.
///
/// - No CA + no client cert → platform trust store (public CAs, e.g. Let's
///   Encrypt, AWS IoT, HiveMQ Cloud).
/// - CA provided → pinned CA (private / self-signed brokers).
/// - Client cert + key → mutual TLS (mTLS). Requires a CA to also be set.
fn build_transport(tls: &MqttTlsConfig) -> Result<Transport, String> {
    let ca = tls.ca_pem.as_deref().unwrap_or("");
    let has_ca = !ca.trim().is_empty();

    let client_auth = match (&tls.client_cert_pem, &tls.client_key_pem) {
        (Some(cert), Some(key)) => {
            if cert.trim().is_empty() || key.trim().is_empty() {
                return Err("TLS client cert and key must both be non-empty".to_string());
            }
            Some((cert.as_bytes().to_vec(), key.as_bytes().to_vec()))
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err("mTLS requires both a client cert and a key".to_string());
        }
        (None, None) => None,
    };

    // No CA and no mTLS → trust the platform root store.
    if !has_ca && client_auth.is_none() {
        return Ok(Transport::tls_with_default_config());
    }

    // The rumqttc `Simple` variant requires a non-empty CA, so mTLS against a
    // public-CA broker (no custom CA) is not supported — ask for a CA.
    if !has_ca && client_auth.is_some() {
        return Err(
            "TLS with a client certificate (mTLS) also requires a CA certificate".to_string(),
        );
    }

    Ok(Transport::tls_with_config(TlsConfiguration::Simple {
        ca: ca.as_bytes().to_vec(),
        alpn: None,
        client_auth,
    }))
}
