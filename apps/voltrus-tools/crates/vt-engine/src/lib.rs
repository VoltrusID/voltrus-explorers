//! The session engine: runs every protocol session on its own tokio runtime,
//! driven by [`Command`]s, reporting through [`Event`]s.
//!
//! One task owns each session for its whole life; everything else talks to
//! it through its control channel, so a slow device never blocks the app or
//! another session.

mod emit;
mod modbus;
pub mod templates;

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use vt_protocol::{Command, Event, EventKind, SessionId, SessionSpec, SessionState};

use emit::Emitter;

/// Sends commands to a running engine. Cheap to clone.
#[derive(Clone, Debug)]
pub struct EngineHandle {
    commands: async_channel::Sender<Command>,
}

impl EngineHandle {
    /// Queue a command. Returns `false` once the engine has shut down.
    pub fn send(&self, command: Command) -> bool {
        self.commands.try_send(command).is_ok()
    }
}

/// Start the engine on its own thread. Events arrive on the returned receiver
/// until the engine shuts down.
pub fn start() -> std::io::Result<(EngineHandle, async_channel::Receiver<Event>)> {
    let (command_tx, command_rx) = async_channel::unbounded();
    let (event_tx, event_rx) = async_channel::unbounded();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("vt-session")
        .enable_all()
        .build()?;
    thread::Builder::new()
        .name("vt-engine".into())
        .spawn(move || runtime.block_on(run(command_rx, event_tx)))?;
    Ok((
        EngineHandle {
            commands: command_tx,
        },
        event_rx,
    ))
}

/// How often a session of this spec reports values, in milliseconds: a
/// master's poll interval, a simulator's publish tick. Sizes trend history.
pub fn sample_interval_ms(spec: &SessionSpec) -> u64 {
    match spec {
        SessionSpec::ModbusMaster(master) => master.poll_ms,
        SessionSpec::ModbusSlave(_) => {
            u64::try_from(modbus::slave::PUBLISH_TICK.as_millis()).unwrap_or(u64::MAX)
        }
    }
}

/// Messages from the engine loop to one session task.
#[derive(Debug)]
pub(crate) enum Control {
    Write { point: u32, value: f64 },
    Lock { point: u32, locked: bool },
    Close,
}

/// Longest an Open waits for closing sessions to free its port.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// The engine's handle on one running session.
struct Session {
    control: mpsc::UnboundedSender<Control>,
    task: JoinHandle<()>,
    /// The TCP port a simulator listens on.
    port: Option<u16>,
}

/// A simulator told to close whose listener may still hold its port.
struct Closing {
    port: u16,
    task: JoinHandle<()>,
}

impl Session {
    fn close(self, closing: &mut Vec<Closing>) {
        let _ = self.control.send(Control::Close);
        closing.retain(|session| !session.task.is_finished());
        if let Some(port) = self.port {
            closing.push(Closing {
                port,
                task: self.task,
            });
        }
    }
}

/// Wait (at most [`CLOSE_WAIT`]) for the closing simulators on `port` to
/// finish, so a simulator restarted on the same port can bind it.
async fn settle(closing: &mut Vec<Closing>, port: u16) {
    let (waiting, rest): (Vec<_>, Vec<_>) = std::mem::take(closing)
        .into_iter()
        .filter(|session| !session.task.is_finished())
        .partition(|session| session.port == port);
    *closing = rest;
    let deadline = Instant::now() + CLOSE_WAIT;
    for session in waiting {
        let _ = tokio::time::timeout_at(deadline, session.task).await;
    }
}

async fn run(commands: async_channel::Receiver<Command>, events: async_channel::Sender<Event>) {
    let mut sessions: HashMap<SessionId, Session> = HashMap::new();
    let mut closing: Vec<Closing> = Vec::new();
    while let Ok(command) = commands.recv().await {
        match command {
            Command::Open { session, spec } => {
                if let Some(previous) = sessions.remove(&session) {
                    previous.close(&mut closing);
                }
                let port = match &spec {
                    SessionSpec::ModbusSlave(slave) => Some(slave.port),
                    SessionSpec::ModbusMaster(_) => None,
                };
                if let Some(port) = port {
                    settle(&mut closing, port).await;
                }
                let (control, control_rx) = mpsc::unbounded_channel();
                let emitter = Emitter::new(session, events.clone());
                let task = tokio::spawn(run_session(spec, control_rx, emitter));
                sessions.insert(
                    session,
                    Session {
                        control,
                        task,
                        port,
                    },
                );
            }
            Command::Close { session } => match sessions.remove(&session) {
                Some(running) => running.close(&mut closing),
                None => {
                    let _ = events
                        .send(Event {
                            session,
                            kind: EventKind::State(SessionState::Closed { error: None }),
                        })
                        .await;
                }
            },
            Command::Write {
                session,
                point,
                value,
            } => forward(&sessions, session, Control::Write { point, value }),
            Command::Lock {
                session,
                point,
                locked,
            } => forward(&sessions, session, Control::Lock { point, locked }),
            Command::Shutdown => break,
        }
    }
    for running in sessions.into_values() {
        let _ = running.control.send(Control::Close);
    }
}

fn forward(sessions: &HashMap<SessionId, Session>, session: SessionId, control: Control) {
    if let Some(running) = sessions.get(&session) {
        let _ = running.control.send(control);
    }
}

async fn run_session(spec: SessionSpec, control: mpsc::UnboundedReceiver<Control>, out: Emitter) {
    match spec {
        SessionSpec::ModbusMaster(spec) => modbus::master::run(spec, control, &out).await,
        SessionSpec::ModbusSlave(spec) => modbus::slave::run(spec, control, &out).await,
    }
}
