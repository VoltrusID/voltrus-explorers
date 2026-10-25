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

use tokio::sync::mpsc;
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

/// Messages from the engine loop to one session task.
#[derive(Debug)]
pub(crate) enum Control {
    Write { point: u32, value: f64 },
    Lock { point: u32, locked: bool },
    Close,
}

async fn run(commands: async_channel::Receiver<Command>, events: async_channel::Sender<Event>) {
    let mut sessions: HashMap<SessionId, mpsc::UnboundedSender<Control>> = HashMap::new();
    while let Ok(command) = commands.recv().await {
        match command {
            Command::Open { session, spec } => {
                if let Some(previous) = sessions.remove(&session) {
                    let _ = previous.send(Control::Close);
                }
                let (control_tx, control_rx) = mpsc::unbounded_channel();
                let emitter = Emitter::new(session, events.clone());
                tokio::spawn(run_session(spec, control_rx, emitter));
                sessions.insert(session, control_tx);
            }
            Command::Close { session } => match sessions.remove(&session) {
                Some(control) => {
                    let _ = control.send(Control::Close);
                }
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
    for control in sessions.into_values() {
        let _ = control.send(Control::Close);
    }
}

fn forward(
    sessions: &HashMap<SessionId, mpsc::UnboundedSender<Control>>,
    session: SessionId,
    control: Control,
) {
    if let Some(sender) = sessions.get(&session) {
        let _ = sender.send(control);
    }
}

async fn run_session(spec: SessionSpec, control: mpsc::UnboundedReceiver<Control>, out: Emitter) {
    match spec {
        SessionSpec::ModbusMaster(spec) => modbus::master::run(spec, control, &out).await,
        SessionSpec::ModbusSlave(spec) => modbus::slave::run(spec, control, &out).await,
    }
}
