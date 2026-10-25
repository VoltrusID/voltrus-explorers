//! A small MQTT 3.1.1 broker for the simulator, framed with rumqttc's packet
//! codec (the one `mt-mqtt`'s client speaks).
//!
//! `mt-mqtt`'s embedded broker wraps rumqttd 0.20, whose `Broker::start`
//! never returns and has no shutdown: stopping it leaves its router and
//! server threads running and its port (always on 0.0.0.0) bound for the life
//! of the process, so a simulator could not be stopped and started again.
//! This one lives in the session's task: it binds the address the user chose,
//! and dropping it closes the listener and every connection.
//!
//! It keeps what a test broker needs: clean sessions, QoS 0 to 2 in both
//! directions, retained messages, wildcard subscriptions, last wills and
//! keep-alive. No persistence, no authentication.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::BytesMut;
use rumqttc::mqttbytes::v4::{
    self, ConnAck, ConnectReturnCode, LastWill, Packet, PingResp, PubAck, PubComp, PubRec, PubRel,
    Publish, SubAck, SubscribeReasonCode, UnsubAck,
};
use rumqttc::mqttbytes::{Error as CodecError, QoS};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// Largest packet a client may send.
const MAX_PACKET: usize = 4 << 20;
/// Time a new connection has to send its CONNECT.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// What happened on the broker, for the simulator to report.
#[derive(Debug)]
pub(crate) enum BrokerEvent {
    Connected {
        client: String,
        address: SocketAddr,
        username: Option<String>,
        keep_alive: u16,
    },
    Disconnected {
        client: String,
        reason: String,
    },
    Subscribed {
        client: String,
        filters: Vec<(String, QoS)>,
    },
    Unsubscribed {
        client: String,
        filters: Vec<String>,
    },
    /// A client published; the broker has already delivered it.
    Published {
        client: String,
        publish: Publish,
    },
}

/// Connection task → broker.
#[derive(Debug)]
enum FromClient {
    Connected {
        conn: u64,
        client: String,
        username: Option<String>,
        keep_alive: u16,
        will: Option<LastWill>,
    },
    Publish(u64, Publish),
    Subscribe(u64, v4::Subscribe),
    Unsubscribe(u64, v4::Unsubscribe),
    /// `clean`: the client said DISCONNECT, so its will is discarded.
    Closed {
        conn: u64,
        clean: bool,
        reason: String,
    },
}

/// Broker → connection task.
#[derive(Debug)]
enum ToClient {
    Publish(Publish),
    SubAck(SubAck),
    UnsubAck(UnsubAck),
    /// Another connection took over the client id.
    Close,
    /// The broker is stopping.
    Shutdown,
}

struct Client {
    /// Set once the CONNECT arrives.
    id: Option<String>,
    address: SocketAddr,
    will: Option<LastWill>,
    subscriptions: Vec<(String, QoS)>,
    tx: mpsc::UnboundedSender<ToClient>,
    task: JoinHandle<()>,
}

pub(crate) struct Broker {
    listener: TcpListener,
    clients: HashMap<u64, Client>,
    /// Retained messages in the order their topics first appeared, so a new
    /// subscriber sees a device's values in its own order.
    retained: Vec<Publish>,
    from_clients: mpsc::UnboundedReceiver<FromClient>,
    from_clients_tx: mpsc::UnboundedSender<FromClient>,
    next_conn: u64,
}

impl Broker {
    pub(crate) async fn bind(address: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(address).await?;
        let (from_clients_tx, from_clients) = mpsc::unbounded_channel();
        Ok(Self {
            listener,
            clients: HashMap::new(),
            retained: Vec::new(),
            from_clients,
            from_clients_tx,
            next_conn: 0,
        })
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Clients that completed their CONNECT.
    pub(crate) fn client_count(&self) -> usize {
        self.clients
            .values()
            .filter(|client| client.id.is_some())
            .count()
    }

    /// Serve until something worth reporting happens. Cancel-safe.
    pub(crate) async fn next(&mut self) -> BrokerEvent {
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    if let Ok((stream, address)) = accepted {
                        self.accept(stream, address);
                    }
                }
                Some(message) = self.from_clients.recv() => {
                    if let Some(event) = self.handle(message) {
                        return event;
                    }
                }
            }
        }
    }

    fn accept(&mut self, stream: TcpStream, address: SocketAddr) {
        let _ = stream.set_nodelay(true);
        self.next_conn += 1;
        let conn = self.next_conn;
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(serve(stream, conn, self.from_clients_tx.clone(), rx));
        self.clients.insert(
            conn,
            Client {
                id: None,
                address,
                will: None,
                subscriptions: Vec::new(),
                tx,
                task,
            },
        );
    }

    fn handle(&mut self, message: FromClient) -> Option<BrokerEvent> {
        match message {
            FromClient::Connected {
                conn,
                client,
                username,
                keep_alive,
                will,
            } => {
                // A second connection with the same id takes the session over.
                for (other, existing) in &self.clients {
                    if *other != conn && existing.id.as_deref() == Some(client.as_str()) {
                        let _ = existing.tx.send(ToClient::Close);
                    }
                }
                let entry = self.clients.get_mut(&conn)?;
                entry.id = Some(client.clone());
                entry.will = will;
                Some(BrokerEvent::Connected {
                    client,
                    address: entry.address,
                    username,
                    keep_alive,
                })
            }
            FromClient::Publish(conn, publish) => {
                let client = self.clients.get(&conn)?.id.clone()?;
                self.route(&publish);
                Some(BrokerEvent::Published { client, publish })
            }
            FromClient::Subscribe(conn, subscribe) => {
                let entry = self.clients.get_mut(&conn)?;
                let mut codes = Vec::with_capacity(subscribe.filters.len());
                let mut granted = Vec::new();
                for filter in subscribe.filters {
                    if vt_protocol::valid_filter(&filter.path) {
                        codes.push(SubscribeReasonCode::Success(filter.qos));
                        match entry
                            .subscriptions
                            .iter_mut()
                            .find(|(existing, _)| *existing == filter.path)
                        {
                            Some(existing) => existing.1 = filter.qos,
                            None => entry.subscriptions.push((filter.path.clone(), filter.qos)),
                        }
                        granted.push((filter.path, filter.qos));
                    } else {
                        codes.push(SubscribeReasonCode::Failure);
                    }
                }
                let _ = entry
                    .tx
                    .send(ToClient::SubAck(SubAck::new(subscribe.pkid, codes)));
                // Retained messages follow the SUBACK, flagged as retained.
                for retained in &self.retained {
                    if let Some(qos) = granted
                        .iter()
                        .filter(|(filter, _)| mt_mqtt::topic_matches(filter, &retained.topic))
                        .map(|(_, qos)| *qos)
                        .reduce(max_qos)
                    {
                        let mut copy = retained.clone();
                        copy.qos = min_qos(copy.qos, qos);
                        copy.retain = true;
                        let _ = entry.tx.send(ToClient::Publish(copy));
                    }
                }
                Some(BrokerEvent::Subscribed {
                    client: entry.id.clone()?,
                    filters: granted,
                })
            }
            FromClient::Unsubscribe(conn, unsubscribe) => {
                let entry = self.clients.get_mut(&conn)?;
                entry
                    .subscriptions
                    .retain(|(filter, _)| !unsubscribe.topics.contains(filter));
                let _ = entry
                    .tx
                    .send(ToClient::UnsubAck(UnsubAck::new(unsubscribe.pkid)));
                Some(BrokerEvent::Unsubscribed {
                    client: entry.id.clone()?,
                    filters: unsubscribe.topics,
                })
            }
            FromClient::Closed {
                conn,
                clean,
                reason,
            } => {
                let entry = self.clients.remove(&conn)?;
                if !clean && let Some(will) = entry.will {
                    let mut publish = Publish::new(will.topic, will.qos, will.message.to_vec());
                    publish.retain = will.retain;
                    self.route(&publish);
                }
                Some(BrokerEvent::Disconnected {
                    client: entry.id?,
                    reason,
                })
            }
        }
    }

    /// Publish from the broker itself (the simulated device).
    pub(crate) fn publish(&mut self, topic: &str, payload: Vec<u8>, qos: QoS, retain: bool) {
        let mut publish = Publish::new(topic, qos, payload);
        publish.retain = retain;
        self.route(&publish);
    }

    /// Keep or clear a retained message and deliver to every subscriber once,
    /// at the highest QoS among its matching subscriptions (capped by the
    /// message's own).
    fn route(&mut self, publish: &Publish) {
        if publish.retain {
            let existing = self
                .retained
                .iter()
                .position(|retained| retained.topic == publish.topic);
            match (existing, publish.payload.is_empty()) {
                (Some(index), true) => {
                    self.retained.remove(index);
                }
                (Some(index), false) => self.retained[index] = stored(publish),
                (None, false) => self.retained.push(stored(publish)),
                (None, true) => {}
            }
        }
        for client in self.clients.values() {
            if client.id.is_none() {
                continue;
            }
            let Some(qos) = client
                .subscriptions
                .iter()
                .filter(|(filter, _)| mt_mqtt::topic_matches(filter, &publish.topic))
                .map(|(_, qos)| *qos)
                .reduce(max_qos)
            else {
                continue;
            };
            let mut copy = publish.clone();
            copy.qos = min_qos(copy.qos, qos);
            // Live deliveries are not flagged retained (MQTT 3.1.1, 3.3.1.3).
            copy.retain = false;
            copy.dup = false;
            let _ = client.tx.send(ToClient::Publish(copy));
        }
    }

    /// Stop: deliver what is queued, close every connection, free the port.
    pub(crate) async fn close(mut self) {
        let tasks: Vec<JoinHandle<()>> = self
            .clients
            .drain()
            .map(|(_, client)| {
                let _ = client.tx.send(ToClient::Shutdown);
                client.task
            })
            .collect();
        let deadline = Instant::now() + Duration::from_millis(500);
        for task in tasks {
            let abort = task.abort_handle();
            if tokio::time::timeout_at(deadline, task).await.is_err() {
                abort.abort();
            }
        }
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        for client in self.clients.values() {
            client.task.abort();
        }
    }
}

fn stored(publish: &Publish) -> Publish {
    let mut copy = publish.clone();
    copy.pkid = 0;
    copy.dup = false;
    copy
}

fn max_qos(a: QoS, b: QoS) -> QoS {
    if (a as u8) >= (b as u8) { a } else { b }
}

fn min_qos(a: QoS, b: QoS) -> QoS {
    if (a as u8) <= (b as u8) { a } else { b }
}

/// One client connection: CONNECT, then packets both ways until either side
/// closes or the client misses its keep-alive.
async fn serve(
    stream: TcpStream,
    conn: u64,
    broker: mpsc::UnboundedSender<FromClient>,
    mut from_broker: mpsc::UnboundedReceiver<ToClient>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = BytesMut::with_capacity(4096);
    let (clean, reason) = match session(
        conn,
        &mut reader,
        &mut writer,
        &mut buffer,
        &broker,
        &mut from_broker,
    )
    .await
    {
        Ok(clean) => (clean, "disconnected".to_owned()),
        Err(reason) => (false, reason),
    };
    let _ = writer.shutdown().await;
    let _ = broker.send(FromClient::Closed {
        conn,
        clean,
        reason,
    });
}

/// Returns whether the client said DISCONNECT, or why the connection ended.
async fn session(
    conn: u64,
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    buffer: &mut BytesMut,
    broker: &mpsc::UnboundedSender<FromClient>,
    from_broker: &mut mpsc::UnboundedReceiver<ToClient>,
) -> Result<bool, String> {
    let connect = match tokio::time::timeout(CONNECT_TIMEOUT, read_packet(reader, buffer)).await {
        Err(_) => return Err("sent no CONNECT within 10 s".into()),
        Ok(Err(ReadError::Codec(CodecError::InvalidProtocolLevel(level)))) => {
            send(writer, |out| {
                ConnAck::new(ConnectReturnCode::RefusedProtocolVersion, false).write(out)
            })
            .await?;
            return Err(format!(
                "asked for MQTT protocol level {level}, only 4 (3.1.1) is served"
            ));
        }
        Ok(Err(error)) => return Err(error.to_string()),
        Ok(Ok(Packet::Connect(connect))) => connect,
        Ok(Ok(_)) => return Err("sent another packet before CONNECT".into()),
    };
    send(writer, |out| {
        ConnAck::new(ConnectReturnCode::Success, false).write(out)
    })
    .await?;
    let client = if connect.client_id.is_empty() {
        format!("auto-{conn}")
    } else {
        connect.client_id
    };
    let keep_alive = connect.keep_alive;
    let _ = broker.send(FromClient::Connected {
        conn,
        client,
        username: connect.login.map(|login| login.username),
        keep_alive,
        will: connect.last_will,
    });

    // A client that sends nothing for one and a half keep-alives is gone.
    let grace = (keep_alive > 0).then(|| Duration::from_millis(u64::from(keep_alive) * 1500));
    let mut deadline = grace.map(|grace| Instant::now() + grace);
    let mut next_pkid: u16 = 0;
    loop {
        let (reading, unread) = (&mut *reader, &mut *buffer);
        let until = deadline;
        let read = async move {
            match until {
                Some(until) => tokio::time::timeout_at(until, read_packet(reading, unread))
                    .await
                    .unwrap_or(Err(ReadError::TimedOut)),
                None => read_packet(reading, unread).await,
            }
        };
        tokio::select! {
            packet = read => {
                let packet = packet.map_err(|error| error.to_string())?;
                if let (Some(grace), Some(deadline)) = (grace, deadline.as_mut()) {
                    *deadline = Instant::now() + grace;
                }
                match packet {
                    Packet::Publish(publish) => {
                        match publish.qos {
                            QoS::AtMostOnce => {}
                            QoS::AtLeastOnce => send(writer, |out| PubAck::new(publish.pkid).write(out)).await?,
                            QoS::ExactlyOnce => send(writer, |out| PubRec::new(publish.pkid).write(out)).await?,
                        }
                        let _ = broker.send(FromClient::Publish(conn, publish));
                    }
                    Packet::PubRel(release) => {
                        send(writer, |out| PubComp::new(release.pkid).write(out)).await?;
                    }
                    // Our QoS 2 deliveries: release them.
                    Packet::PubRec(received) => {
                        send(writer, |out| PubRel::new(received.pkid).write(out)).await?;
                    }
                    Packet::Subscribe(subscribe) => {
                        let _ = broker.send(FromClient::Subscribe(conn, subscribe));
                    }
                    Packet::Unsubscribe(unsubscribe) => {
                        let _ = broker.send(FromClient::Unsubscribe(conn, unsubscribe));
                    }
                    Packet::PingReq => send(writer, |out| PingResp.write(out)).await?,
                    Packet::Disconnect => return Ok(true),
                    Packet::Connect(_) => return Err("a second CONNECT".into()),
                    Packet::PubAck(_) | Packet::PubComp(_) | Packet::PingResp
                    | Packet::ConnAck(_) | Packet::SubAck(_) | Packet::UnsubAck(_) => {}
                }
            }
            outgoing = from_broker.recv() => match outgoing {
                Some(ToClient::Publish(mut publish)) => {
                    if publish.qos != QoS::AtMostOnce {
                        next_pkid = next_pkid.checked_add(1).unwrap_or(1);
                        publish.pkid = next_pkid;
                    }
                    send(writer, |out| publish.write(out)).await?;
                }
                Some(ToClient::SubAck(ack)) => send(writer, |out| ack.write(out)).await?,
                Some(ToClient::UnsubAck(ack)) => send(writer, |out| ack.write(out)).await?,
                Some(ToClient::Close) => return Err("another connection took over the client id".into()),
                Some(ToClient::Shutdown) | None => return Err("the broker stopped".into()),
            },
        }
    }
}

/// Why no packet came.
#[derive(Debug)]
enum ReadError {
    Closed,
    TimedOut,
    Codec(CodecError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("closed the connection"),
            Self::TimedOut => f.write_str("missed its keep-alive"),
            Self::Codec(error) => write!(f, "sent a bad packet: {error}"),
        }
    }
}

/// The next whole packet, reading more as needed. Cancel-safe: bytes read so
/// far stay in `buffer`.
async fn read_packet(
    reader: &mut OwnedReadHalf,
    buffer: &mut BytesMut,
) -> Result<Packet, ReadError> {
    loop {
        match v4::read(buffer, MAX_PACKET) {
            Err(CodecError::InsufficientBytes(_)) => {}
            result => return result.map_err(ReadError::Codec),
        }
        match reader.read_buf(buffer).await {
            Ok(0) | Err(_) => return Err(ReadError::Closed),
            Ok(_) => {}
        }
    }
}

async fn send(
    writer: &mut OwnedWriteHalf,
    encode: impl FnOnce(&mut BytesMut) -> Result<usize, CodecError>,
) -> Result<(), String> {
    let mut out = BytesMut::new();
    encode(&mut out).map_err(|error| error.to_string())?;
    writer
        .write_all(&out)
        .await
        .map_err(|error| error.to_string())
}
