//! The GEM host: typed helpers over an HSMS connection.
//!
//! [`GemHost`] connects to equipment (or accepts its connection), exposes one async method per
//! stream/function pair a host normally uses, and answers what equipment sends on its own:
//! event reports (S6F11 -> S6F12), alarm reports (S5F1 -> S5F2), terminal requests (S10F1 ->
//! S10F2), S1F1 and S1F13. Each received report is also published as a [`HostEvent`].
//!
//! Every helper returns the equipment's ack as an enum (`Hcack`, `Eac`, ...); a refusal is a
//! normal answer, not an error. Errors are timeouts (by T-name), S9 errors, rejects, aborts and
//! malformed replies ([`SecsError`]). [`GemHost::send_raw`] sends anything else.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::error::{Result, S9Error, S9Kind, SecsError};
use crate::gem::*;
use crate::hsms::{
    Connector, HsmsConfig, HsmsConnection, HsmsLink, HsmsListener, HsmsState, Incoming, IncomingRx,
    LogEntry,
};
use crate::item::Item;
use crate::message::SecsMessage;

/// Where and how to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GemHostConfig {
    /// `host:port` of the equipment (active mode).
    pub address: String,
    pub hsms: HsmsConfig,
    /// Item type of the ids this host sends.
    pub id_format: IdFormat,
}

impl Default for GemHostConfig {
    fn default() -> Self {
        GemHostConfig {
            address: format!("127.0.0.1:{}", crate::hsms::DEFAULT_PORT),
            hsms: HsmsConfig {
                linktest_interval_ms: Some(30_000),
                ..HsmsConfig::default()
            },
            id_format: IdFormat::U4,
        }
    }
}

impl GemHostConfig {
    pub fn new(address: impl Into<String>) -> Self {
        GemHostConfig {
            address: address.into(),
            ..Default::default()
        }
    }
}

/// Something the equipment sent that the host answered by itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostEvent {
    /// S6F11, already acknowledged with S6F12.
    EventReport(EventReport),
    /// S5F1, already acknowledged with S5F2.
    Alarm(AlarmReport),
    /// S10F1, already acknowledged with S10F2.
    Terminal(TerminalText),
    /// An S9Fx error message the equipment sent.
    S9(S9Error),
    /// A primary message with no handler; it was answered with S9F5 (or S9F3 for an unknown stream).
    Unhandled(SecsMessage),
    /// The connection closed; `reason` is empty for a clean close.
    Closed { reason: Option<String> },
}

/// A GEM host session.
pub struct GemHost {
    conn: HsmsConnection,
    events: broadcast::Sender<HostEvent>,
    fmt: IdFormat,
    data_id: AtomicU64,
}

impl GemHost {
    /// Connect to the equipment in active mode and select the session. One attempt; use
    /// [`GemHost::connect_with`] to honour T5 between attempts.
    pub async fn connect(config: GemHostConfig) -> Result<GemHost> {
        let (conn, rx) = HsmsConnection::connect(config.address.as_str(), config.hsms).await?;
        Ok(Self::from_connection(conn, rx, config.id_format))
    }

    /// Like [`GemHost::connect`], waiting out T5 since the connector's previous attempt.
    pub async fn connect_with(
        connector: &Connector,
        address: &str,
        id_format: IdFormat,
    ) -> Result<GemHost> {
        let (conn, rx) = connector.connect(address).await?;
        Ok(Self::from_connection(conn, rx, id_format))
    }

    /// Passive mode: wait for the equipment to connect and select.
    pub async fn accept(listener: &HsmsListener, id_format: IdFormat) -> Result<GemHost> {
        let (conn, rx, _) = listener
            .accept()
            .await
            .map_err(|e| SecsError::ConnectionFailed(e.to_string()))?;
        // The equipment selects the session right after connecting (T7 ends the wait).
        let mut state = conn.watch_state();
        let state = *state
            .wait_for(|s| *s != HsmsState::NotSelected)
            .await
            .map_err(|_| SecsError::Closed)?;
        if state != HsmsState::Selected {
            return Err(conn.close_reason().unwrap_or(SecsError::Closed));
        }
        Ok(Self::from_connection(conn, rx, id_format))
    }

    /// Wrap an existing HSMS connection and start answering what the equipment sends.
    pub fn from_connection(conn: HsmsConnection, incoming: IncomingRx, fmt: IdFormat) -> GemHost {
        let (events, _) = broadcast::channel(256);
        tokio::spawn(dispatch(conn.link(), incoming, events.clone()));
        let watcher = conn.link();
        let tx = events.clone();
        tokio::spawn(async move {
            watcher.closed().await;
            let _ = tx.send(HostEvent::Closed {
                reason: watcher.close_reason().map(|e| e.to_string()),
            });
        });
        GemHost {
            conn,
            events,
            fmt,
            data_id: AtomicU64::new(1),
        }
    }

    /// The underlying HSMS connection (state, timers, log stream, close).
    pub fn connection(&self) -> &HsmsConnection {
        &self.conn
    }

    /// Reports, alarms and the other things the equipment sends by itself.
    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.events.subscribe()
    }

    /// Every message sent and received, including HSMS control messages.
    pub fn subscribe_log(&self) -> broadcast::Receiver<LogEntry> {
        self.conn.subscribe_log()
    }

    /// End the session: Separate.req and close.
    pub fn disconnect(&self) {
        self.conn.close();
    }

    // ---- raw access ------------------------------------------------------------------------

    /// Send a primary message and wait for its reply (T3).
    pub async fn send_raw(&self, message: SecsMessage) -> Result<SecsMessage> {
        self.conn.request(message).await
    }

    /// Parse a message from SML (`S1F1 W.`), send it and return the reply.
    pub async fn send_sml(&self, sml: &str) -> Result<SecsMessage> {
        self.send_raw(SecsMessage::from_sml(sml)?).await
    }

    async fn call(
        &self,
        call: &'static str,
        stream: u8,
        function: u8,
        item: Option<Item>,
    ) -> Result<Item> {
        let reply = self
            .send_raw(SecsMessage::request(stream, function, item))
            .await?;
        if reply.stream != stream || reply.function != function + 1 {
            return Err(SecsError::UnexpectedReply {
                call,
                detail: format!(
                    "expected {}, got {}",
                    SecsMessage::notify(stream, function + 1, None).name(),
                    reply.name()
                ),
            });
        }
        reply.item.ok_or(SecsError::UnexpectedReply {
            call,
            detail: "the reply has no data".into(),
        })
    }

    fn shape<T>(call: &'static str, r: std::result::Result<T, String>) -> Result<T> {
        r.map_err(|detail| SecsError::UnexpectedReply { call, detail })
    }

    async fn call_ack(
        &self,
        call: &'static str,
        stream: u8,
        function: u8,
        item: Option<Item>,
    ) -> Result<u8> {
        let reply = self.call(call, stream, function, item).await?;
        Self::shape(call, decode_ack(&reply, "ack"))
    }

    // ---- stream 1 --------------------------------------------------------------------------

    /// S1F1/S1F2. `None` when the equipment answers with an empty list.
    pub async fn are_you_there(&self) -> Result<Option<Identity>> {
        let r = self.call("S1F1", 1, 1, None).await?;
        Self::shape("S1F2", decode_identity(&r))
    }

    /// S1F13/S1F14: establish communications. Returns the equipment's `COMMACK` and identity.
    pub async fn establish_communications(&self) -> Result<(CommAck, Option<Identity>)> {
        let r = self.call("S1F13", 1, 13, Some(Item::empty_list())).await?;
        Self::shape("S1F14", decode_s1f14(&r))
    }

    /// S1F15/S1F16: request the equipment to go to host offline.
    pub async fn request_offline(&self) -> Result<OflAck> {
        let a = self.call_ack("S1F15", 1, 15, None).await?;
        Ok(OflAck::from_code(a))
    }

    /// S1F17/S1F18: request the equipment to go online.
    pub async fn request_online(&self) -> Result<OnlAck> {
        let a = self.call_ack("S1F17", 1, 17, None).await?;
        Ok(OnlAck::from_code(a))
    }

    /// S1F3/S1F4: selected status variables. An empty `svids` asks for all of them. The values
    /// come back in the order asked; an unknown SVID gives a zero-length item.
    pub async fn status_variables(&self, svids: &[u64]) -> Result<Vec<Item>> {
        let r = self.call("S1F3", 1, 3, Some(self.fmt.list(svids))).await?;
        Self::shape("S1F4", want_list(&r, "S1F4").map(<[Item]>::to_vec))
    }

    /// S1F11/S1F12: names and units of status variables (empty = all).
    pub async fn status_variable_names(&self, svids: &[u64]) -> Result<Vec<SvInfo>> {
        let r = self
            .call("S1F11", 1, 11, Some(self.fmt.list(svids)))
            .await?;
        Self::shape("S1F12", decode_s1f12(&r))
    }

    // ---- stream 2 --------------------------------------------------------------------------

    /// S2F13/S2F14: equipment constant values (empty = all).
    pub async fn equipment_constants(&self, ecids: &[u64]) -> Result<Vec<Item>> {
        let r = self
            .call("S2F13", 2, 13, Some(self.fmt.list(ecids)))
            .await?;
        Self::shape("S2F14", want_list(&r, "S2F14").map(<[Item]>::to_vec))
    }

    /// S2F15/S2F16: set equipment constants. `Eac::Accepted` applies all of them; any other code
    /// means none was applied.
    pub async fn set_equipment_constants(&self, set: &[(u64, Item)]) -> Result<Eac> {
        let a = self
            .call_ack("S2F15", 2, 15, Some(encode_s2f15(self.fmt, set)))
            .await?;
        Ok(Eac::from_code(a))
    }

    /// S2F29/S2F30: name, range, default and units of equipment constants (empty = all).
    pub async fn equipment_constant_names(&self, ecids: &[u64]) -> Result<Vec<EcInfo>> {
        let r = self
            .call("S2F29", 2, 29, Some(self.fmt.list(ecids)))
            .await?;
        Self::shape("S2F30", decode_s2f30(&r))
    }

    fn next_data_id(&self) -> u64 {
        self.data_id.fetch_add(1, Ordering::Relaxed)
    }

    /// S2F33/S2F34: define reports. A report with no VIDs deletes it; an empty slice deletes
    /// every report and link.
    pub async fn define_reports(&self, reports: &[ReportDef]) -> Result<Drack> {
        let body = encode_s2f33(self.fmt, self.next_data_id(), reports);
        let a = self.call_ack("S2F33", 2, 33, Some(body)).await?;
        Ok(Drack::from_code(a))
    }

    /// S2F35/S2F36: link reports to collection events. A link with no reports removes it; an
    /// empty slice removes every link.
    pub async fn link_event_reports(&self, links: &[EventLink]) -> Result<Lrack> {
        let body = encode_s2f35(self.fmt, self.next_data_id(), links);
        let a = self.call_ack("S2F35", 2, 35, Some(body)).await?;
        Ok(Lrack::from_code(a))
    }

    /// S2F37/S2F38: enable or disable collection events (empty `ceids` = all).
    pub async fn enable_events(&self, enable: bool, ceids: &[u64]) -> Result<Erack> {
        let a = self
            .call_ack("S2F37", 2, 37, Some(encode_s2f37(self.fmt, enable, ceids)))
            .await?;
        Ok(Erack::from_code(a))
    }

    /// S2F41/S2F42: host command with parameters (`START`, `STOP`...).
    pub async fn remote_command(
        &self,
        rcmd: &str,
        params: &[CommandParam],
    ) -> Result<CommandReply> {
        let r = self
            .call("S2F41", 2, 41, Some(encode_s2f41(rcmd, params)))
            .await?;
        Self::shape("S2F42", decode_s2f42(&r))
    }

    // ---- stream 5 --------------------------------------------------------------------------

    /// S5F3/S5F4: enable or disable one alarm's reporting, or all of them with `None`.
    pub async fn enable_alarms(&self, enable: bool, alid: Option<u64>) -> Result<Ackc5> {
        let a = self
            .call_ack("S5F3", 5, 3, Some(encode_s5f3(self.fmt, enable, alid)))
            .await?;
        Ok(Ackc5::from_code(a))
    }

    /// S5F5/S5F6: alarm state and text (empty `alids` = all alarms).
    pub async fn list_alarms(&self, alids: &[u64]) -> Result<Vec<AlarmReport>> {
        let r = self.call("S5F5", 5, 5, Some(self.fmt.list(alids))).await?;
        Self::shape("S5F6", decode_s5f6(&r))
    }

    // ---- stream 7 and 10 -------------------------------------------------------------------

    /// S7F19/S7F20: the equipment's process program ids.
    pub async fn process_program_list(&self) -> Result<Vec<String>> {
        let r = self.call("S7F19", 7, 19, None).await?;
        Self::shape(
            "S7F20",
            want_list(&r, "S7F20").and_then(|l| l.iter().map(|i| want_text(i, "PPID")).collect()),
        )
    }

    /// S10F3/S10F4: show a line of text on a terminal of the equipment.
    pub async fn terminal_display(&self, tid: u8, text: &str) -> Result<Ackc10> {
        let body = encode_terminal(&TerminalText {
            tid,
            text: text.into(),
        });
        let a = self.call_ack("S10F3", 10, 3, Some(body)).await?;
        Ok(Ackc10::from_code(a))
    }
}

/// Answers the equipment's own messages and publishes them.
async fn dispatch(link: HsmsLink, mut rx: IncomingRx, events: broadcast::Sender<HostEvent>) {
    while let Some(inc) = rx.recv().await {
        handle(&link, &inc, &events);
    }
}

/// Acknowledge a report. Equipment of some stacks (secsgem) leaves the W-bit clear on S5F1 and
/// still waits for the S5F2, so reports are acknowledged either way.
fn reply_ack(link: &HsmsLink, inc: &Incoming, ack: u8) {
    let _ = link.reply(
        inc,
        SecsMessage::notify(inc.stream(), inc.function() + 1, Some(Item::byte(ack))),
    );
}

fn handle(link: &HsmsLink, inc: &Incoming, events: &broadcast::Sender<HostEvent>) {
    let msg = match &inc.message {
        Ok(m) => m,
        Err(_) => {
            let _ = link.send_s9(S9Kind::IllegalData, &inc.header);
            return;
        }
    };
    let bad_shape = || {
        let _ = link.send_s9(S9Kind::IllegalData, &inc.header);
    };
    let body = msg.item.as_ref();
    match (msg.stream, msg.function) {
        (6, 11) => match body.map(decode_s6f11) {
            Some(Ok(report)) => {
                reply_ack(link, inc, 0);
                let _ = events.send(HostEvent::EventReport(report));
            }
            _ => bad_shape(),
        },
        (5, 1) => match body.map(decode_s5f1) {
            Some(Ok(alarm)) => {
                reply_ack(link, inc, 0);
                let _ = events.send(HostEvent::Alarm(alarm));
            }
            _ => bad_shape(),
        },
        (10, 1) => match body.map(decode_terminal) {
            Some(Ok(t)) => {
                reply_ack(link, inc, 0);
                let _ = events.send(HostEvent::Terminal(t));
            }
            _ => bad_shape(),
        },
        (1, 1) => {
            if inc.w_bit() {
                let _ = link.reply(inc, SecsMessage::notify(1, 2, Some(Item::empty_list())));
            }
        }
        (1, 13) => {
            if inc.w_bit() {
                let _ = link.reply(
                    inc,
                    SecsMessage::notify(1, 14, Some(encode_s1f14(CommAck::Accepted, None))),
                );
            }
        }
        (9, f) => {
            if let (Some(kind), Some(Item::Binary(b))) = (S9Kind::from_function(f), body)
                && let Ok(mhead) = <[u8; 10]>::try_from(b.as_slice())
            {
                let _ = events.send(HostEvent::S9(S9Error { kind, mhead }));
            }
        }
        (s, _) => {
            let kind = if is_known_stream(s) {
                S9Kind::UnrecognizedFunction
            } else {
                S9Kind::UnrecognizedStream
            };
            let _ = link.send_s9(kind, &inc.header);
            let _ = events.send(HostEvent::Unhandled(msg.clone()));
        }
    }
}

/// Streams SEMI E5 defines; anything else is an unrecognized stream (S9F3).
pub(crate) fn is_known_stream(stream: u8) -> bool {
    matches!(stream, 1..=7 | 9..=21)
}
