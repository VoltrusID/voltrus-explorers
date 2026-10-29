//! A GEM equipment simulator.
//!
//! [`EquipmentSimulator`] serves an [`EquipmentModel`] over HSMS: passive by default (it listens
//! on port 5000, device id 0, and the host connects and selects), or active (it connects to a
//! host and keeps reconnecting, honouring T5). The model is shared: a UI can change it with
//! [`EquipmentSimulator::with_model`] while a host is connected, and everything the model queues
//! for the host (event reports, alarm reports) is sent right after.
//!
//! After a session is selected the equipment, as GEM equipment does, sends S1F13 until the host
//! answers (`initiate_communication`); a host that sends S1F13 first is answered at once.

mod handler;
pub mod model;

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, broadcast, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

use crate::error::{S9Kind, SecsError};
use crate::gem::{CommAck, decode_s1f14, encode_identity};
use crate::hsms::{
    Connector, DEFAULT_PORT, HsmsConfig, HsmsConnection, HsmsLink, HsmsListener, Incoming,
    IncomingRx, LogEntry,
};
use crate::message::SecsMessage;

pub use handler::Response;
pub use model::{
    Alarm, Chamber, CollectionEvent, EquipmentConstant, EquipmentModel, ProcessError, ProcessState,
    TerminalMessage, alid, ceid, ecid, svid,
};

/// Passive (listen) or active (connect) HSMS mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimMode {
    /// Listen on `bind_addr:port`; 0 picks a free port (see [`EquipmentSimulator::port`]).
    Passive { bind_addr: String, port: u16 },
    /// Connect to the host at `address`, and again T5 after every lost connection.
    Active { address: String },
}

/// How to start the simulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatorConfig {
    pub mode: SimMode,
    /// Device id and timers. The linktest interval is off by default (hosts send the linktests).
    pub hsms: HsmsConfig,
    /// Send S1F13 after the session is selected until the host answers.
    pub initiate_communication: bool,
    /// How often the model advances in real time; `None` leaves time to the caller
    /// ([`EquipmentSimulator::advance`]), which is what tests want.
    pub tick_ms: Option<u64>,
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        SimulatorConfig {
            mode: SimMode::Passive {
                bind_addr: "127.0.0.1".into(),
                port: DEFAULT_PORT,
            },
            hsms: HsmsConfig::default(),
            initiate_communication: true,
            tick_ms: Some(100),
        }
    }
}

impl SimulatorConfig {
    /// Passive mode on `bind_addr:port`.
    pub fn passive(bind_addr: impl Into<String>, port: u16) -> Self {
        SimulatorConfig {
            mode: SimMode::Passive {
                bind_addr: bind_addr.into(),
                port,
            },
            ..Default::default()
        }
    }

    /// Active mode towards `address`.
    pub fn active(address: impl Into<String>) -> Self {
        SimulatorConfig {
            mode: SimMode::Active {
                address: address.into(),
            },
            ..Default::default()
        }
    }
}

struct Shared {
    model: Mutex<EquipmentModel>,
    config: SimulatorConfig,
    /// The model queued something to send.
    outbox: Notify,
    log: broadcast::Sender<LogEntry>,
    sessions: AtomicU64,
    requests: AtomicU64,
    current: Mutex<Option<HsmsLink>>,
}

impl Shared {
    fn model(&self) -> std::sync::MutexGuard<'_, EquipmentModel> {
        self.model.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<HsmsLink>> {
        self.current.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A running simulator. Dropping it without [`EquipmentSimulator::stop`] leaves the tasks running
/// until the runtime ends.
pub struct EquipmentSimulator {
    addr: Option<SocketAddr>,
    shared: Arc<Shared>,
    shutdown: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl EquipmentSimulator {
    pub async fn start(config: SimulatorConfig) -> io::Result<Self> {
        Self::start_with_model(config, EquipmentModel::default()).await
    }

    /// Start with a prepared model.
    pub async fn start_with_model(
        config: SimulatorConfig,
        model: EquipmentModel,
    ) -> io::Result<Self> {
        let (log, _) = broadcast::channel(1024);
        let shared = Arc::new(Shared {
            model: Mutex::new(model),
            config: config.clone(),
            outbox: Notify::new(),
            log,
            sessions: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            current: Mutex::new(None),
        });
        let (shutdown, shutdown_rx) = watch::channel(false);
        let mut tasks = Vec::new();
        let mut addr = None;

        match &config.mode {
            SimMode::Passive { bind_addr, port } => {
                let listener = HsmsListener::bind((bind_addr.as_str(), *port), config.hsms.clone())
                    .await?
                    .with_log(shared.log.clone());
                addr = Some(listener.local_addr()?);
                tracing::info!("GEM simulator listening on {}", addr.expect("set above"));
                tasks.push(tokio::spawn(accept_loop(
                    listener,
                    Arc::clone(&shared),
                    shutdown_rx.clone(),
                )));
            }
            SimMode::Active { address } => {
                tasks.push(tokio::spawn(connect_loop(
                    address.clone(),
                    Arc::clone(&shared),
                    shutdown_rx.clone(),
                )));
            }
        }
        if let Some(ms) = config.tick_ms {
            tasks.push(tokio::spawn(tick_loop(
                Arc::clone(&shared),
                Duration::from_millis(ms),
                shutdown_rx,
            )));
        }
        Ok(EquipmentSimulator {
            addr,
            shared,
            shutdown,
            tasks: Mutex::new(tasks),
        })
    }

    /// Where the passive simulator listens (`None` in active mode).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// The listening port (0 in active mode).
    pub fn port(&self) -> u16 {
        self.addr.map_or(0, |a| a.port())
    }

    /// Change the simulated tool. Runs under the lock the message handlers use, so keep it short.
    /// Messages the change queued for the host (events, alarms) are sent afterwards.
    pub fn with_model<R>(&self, f: impl FnOnce(&mut EquipmentModel) -> R) -> R {
        let (result, queued) = {
            let mut model = self.shared.model();
            let r = f(&mut model);
            (r, !model.outbox.is_empty())
        };
        if queued {
            self.shared.outbox.notify_one();
        }
        result
    }

    /// A copy of the model as it is now.
    pub fn model(&self) -> EquipmentModel {
        self.shared.model().clone()
    }

    /// Advance simulated time by hand (needed when `tick_ms` is `None`).
    pub fn advance(&self, dt: Duration) {
        self.with_model(|m| m.advance(dt.as_secs_f64()));
    }

    /// Every message in both directions, HSMS control messages included, across sessions.
    pub fn subscribe_log(&self) -> broadcast::Receiver<LogEntry> {
        self.shared.log.subscribe()
    }

    /// HSMS sessions currently connected.
    pub fn active_sessions(&self) -> u64 {
        self.shared.sessions.load(Ordering::Relaxed)
    }

    /// Primary messages answered so far.
    pub fn requests_served(&self) -> u64 {
        self.shared.requests.load(Ordering::Relaxed)
    }

    /// Whether a host has selected the session.
    pub fn is_selected(&self) -> bool {
        self.shared
            .current()
            .as_ref()
            .is_some_and(HsmsLink::is_selected)
    }

    /// Stop listening and drop the connection.
    pub async fn stop(&self) {
        let _ = self.shutdown.send(true);
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|p| p.into_inner()));
        for t in tasks {
            let _ = t.await;
        }
    }
}

async fn tick_loop(shared: Arc<Shared>, tick: Duration, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = Instant::now();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Instant::now();
                // a stalled runtime must not skip a whole process
                let dt = now.duration_since(last).as_secs_f64().min(1.0);
                last = now;
                let queued = {
                    let mut m = shared.model();
                    m.advance(dt);
                    !m.outbox.is_empty()
                };
                if queued {
                    shared.outbox.notify_one();
                }
            }
            _ = shutdown.changed() => break,
        }
    }
}

async fn accept_loop(
    listener: HsmsListener,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    // Dropping the set aborts every session when the loop ends.
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((conn, rx, peer)) => {
                    // one selected session at a time
                    if shared.current().as_ref().is_some_and(HsmsLink::is_selected) {
                        tracing::debug!("GEM simulator refused {peer}: a session is already selected");
                        conn.close();
                        continue;
                    }
                    let shared = Arc::clone(&shared);
                    sessions.spawn(async move {
                        run_session(conn, rx, &shared).await;
                    });
                }
                Err(e) => tracing::warn!("GEM accept failed: {e}"),
            },
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
            _ = shutdown.changed() => break,
        }
    }
}

async fn connect_loop(address: String, shared: Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    let connector = Connector::new(shared.config.hsms.clone()).with_log(shared.log.clone());
    loop {
        let connected = tokio::select! {
            c = connector.connect(address.as_str()) => c,
            _ = shutdown.changed() => return,
        };
        match connected {
            Ok((conn, rx)) => {
                tokio::select! {
                    _ = run_session(conn, rx, &shared) => {}
                    _ = shutdown.changed() => return,
                }
            }
            Err(e) => tracing::debug!("GEM simulator could not connect to {address}: {e}"),
        }
    }
}

/// One HSMS session: wait for Select, then answer messages until the connection ends.
async fn run_session(conn: HsmsConnection, mut rx: IncomingRx, shared: &Arc<Shared>) {
    shared.sessions.fetch_add(1, Ordering::Relaxed);
    let link = conn.link();
    *shared.current() = Some(link.clone());

    // Wait for the session to be selected (T7 closes it if the host never does).
    let mut state = link.watch_state();
    let selected = state
        .wait_for(|s| *s != crate::hsms::HsmsState::NotSelected)
        .await
        .map(|s| *s == crate::hsms::HsmsState::Selected)
        .unwrap_or(false);

    if selected {
        shared.model().session_started();
        let mut helpers = JoinSet::new();
        helpers.spawn(pump(link.clone(), Arc::clone(shared)));
        if shared.config.initiate_communication {
            helpers.spawn(establish(link.clone(), Arc::clone(shared)));
        }
        while let Some(inc) = rx.recv().await {
            shared.requests.fetch_add(1, Ordering::Relaxed);
            serve(&link, shared, inc);
        }
        helpers.abort_all();
        shared.model().session_ended();
    }
    conn.close();
    {
        let mut current = shared.current();
        if current.as_ref().is_some_and(|c| c.same_connection(&link)) {
            *current = None;
        }
    }
    shared.sessions.fetch_sub(1, Ordering::Relaxed);
}

/// Answer one primary message.
fn serve(link: &HsmsLink, shared: &Shared, inc: Incoming) {
    let reply_s9 = |kind: S9Kind| {
        let _ = link.send_s9(kind, &inc.header);
    };
    if inc.session_id() != link.config().device_id && inc.session_id() != crate::hsms::NO_DEVICE {
        return reply_s9(S9Kind::UnrecognizedDeviceId);
    }
    let msg = match &inc.message {
        Ok(m) => m,
        Err(_) => return reply_s9(S9Kind::IllegalData),
    };
    if msg.stream == 9 {
        return; // errors the host reports about us: the log shows them
    }
    let (response, queued) = {
        let mut model = shared.model();
        let r = model.handle(msg);
        (r, !model.outbox.is_empty())
    };
    if queued {
        shared.outbox.notify_one();
    }
    match response {
        Response::Reply(item) => {
            // Answered without the W-bit too: the secsgem host sends S5F3, S2F41 and others with
            // the bit clear and waits for the reply anyway, and a reply nobody wants is harmless.
            let _ = link.reply(&inc, msg.reply(item));
        }
        Response::Abort => {
            if inc.w_bit() {
                let _ = link.reply(&inc, msg.abort());
            }
        }
        Response::S9(kind) => reply_s9(kind),
    }
}

/// Sends whatever the model queues (events, alarms) and waits for the host's acknowledgement.
async fn pump(link: HsmsLink, shared: Arc<Shared>) {
    loop {
        shared.outbox.notified().await;
        let messages = shared.model().take_outbox();
        for m in messages {
            let name = m.name();
            match link.request(m).await {
                Ok(_) => {}
                Err(SecsError::Closed | SecsError::NotConnected) => return,
                Err(e) => tracing::debug!("GEM simulator: {name} was not acknowledged: {e}"),
            }
        }
    }
}

/// Communication state model: send S1F13 until the host answers with COMMACK 0, waiting the
/// establish-communications timeout between attempts.
async fn establish(link: HsmsLink, shared: Arc<Shared>) {
    loop {
        let (enabled, identity, delay) = {
            let m = shared.model();
            let secs = m
                .constant(ecid::ESTABLISH_COMM_TIMEOUT)
                .map_or(10.0, |c| c.value_f64());
            (
                m.comm_enabled && m.comm_state != crate::gem::CommunicationState::Communicating,
                crate::gem::Identity {
                    mdln: m.mdln.clone(),
                    softrev: m.softrev.clone(),
                },
                Duration::from_secs_f64(secs),
            )
        };
        if !enabled {
            // communicating already (the host asked first) or disabled: nothing to do
            if shared.model().comm_state == crate::gem::CommunicationState::Communicating {
                return;
            }
            tokio::time::sleep(delay).await;
            continue;
        }
        let msg = SecsMessage::request(1, 13, Some(encode_identity(&identity)));
        let ok = match link.request(msg).await {
            Ok(r) => r
                .item
                .as_ref()
                .and_then(|i| decode_s1f14(i).ok())
                .is_some_and(|(ack, _)| ack == CommAck::Accepted),
            Err(SecsError::Closed | SecsError::NotConnected) => return,
            Err(_) => false,
        };
        let done = {
            let mut m = shared.model();
            if ok {
                m.communication_established();
                true
            } else if m.comm_state == crate::gem::CommunicationState::Communicating {
                true
            } else {
                m.communication_failed();
                false
            }
        };
        if done {
            return;
        }
        tokio::time::sleep(delay).await;
    }
}
