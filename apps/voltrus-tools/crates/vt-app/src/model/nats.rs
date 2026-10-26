//! The NATS parts of the model: the inspector's publish and request form,
//! the JetStream browser, the creds file picker of the editor and the NATS
//! demo plant.

use std::collections::HashMap;

use gpui::{AppContext as _, Context, Entity, PathPromptOptions, Subscription, Window};
use vt_app_core::{Axis, ConnectionId, PaneView, Workspace};
use vt_engine::templates;
use vt_protocol::{
    Command as EngineCommand, NATS_PORT, NatsEvent, NatsReply, NatsRequest, NatsServerSpec,
    NatsStoredMessage, NatsStream, NatsStreamDetail, SessionId, SessionSpec, parse_headers,
    valid_subject,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};

use super::AppModel;
use super::live::Live;

/// Newest messages a stream shows when opened.
const STREAM_MESSAGES: u32 = 20;
/// Messages one fetch pulls from a consumer.
const FETCH_BATCH: u32 = 10;
const DEFAULT_TIMEOUT_MS: u64 = 2000;
/// The longest request timeout the form takes.
const MAX_TIMEOUT_MS: u64 = 60_000;

/// A request sent from the inspector, and its reply once it came.
pub enum Reply {
    Waiting {
        subject: String,
    },
    Done {
        subject: String,
        result: Result<NatsReply, String>,
    },
}

/// One connection's JetStream browser.
#[derive(Default)]
pub struct Browser {
    pub loading: bool,
    pub streams: Option<Result<Vec<NatsStream>, String>>,
    /// The stream opened below the list.
    pub open: Option<String>,
    pub detail: Option<Result<NatsStreamDetail, String>>,
    /// The consumer being fetched from, and the last fetch.
    pub fetching: Option<String>,
    pub fetched: Option<(String, Result<Vec<NatsStoredMessage>, String>)>,
}

/// The inspector's NATS state: one form, and per connection its last reply
/// and JetStream browser.
pub struct NatsPanel {
    pub subject: Entity<TextInput>,
    pub payload: Entity<TextInput>,
    pub headers: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub replies: HashMap<ConnectionId, Reply>,
    pub browsers: HashMap<ConnectionId, Browser>,
    _subscriptions: Vec<Subscription>,
}

impl NatsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let subject = cx.new(|cx| TextInput::new(cx).placeholder("plant.inverter.get").mono());
        let payload = cx.new(|cx| TextInput::new(cx).placeholder("42 or {\"kw\": 42}").mono());
        let headers = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("Nats-Msg-Id: 1; X-Unit: kW")
                .mono()
        });
        let timeout = cx.new(|cx| {
            TextInput::new(cx)
                .text(DEFAULT_TIMEOUT_MS.to_string())
                .placeholder("2000")
                .kind(InputKind::Unsigned)
                .mono()
        });
        // Enter sends a request from a client (publishes from a simulator):
        // what a NATS tool is reached for most.
        let subscriptions = [&subject, &payload, &headers, &timeout]
            .into_iter()
            .map(|field| {
                cx.subscribe_in(field, window, |model, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Submitted => {
                            if model.focused_nats_client().is_some() {
                                model.nats_request(window, cx);
                            } else {
                                model.nats_publish(window, cx);
                            }
                        }
                        InputEvent::Cancelled => window.focus(&model.focus),
                        InputEvent::Changed | InputEvent::Blurred => {}
                    }
                })
            })
            .collect();
        Self {
            subject,
            payload,
            headers,
            timeout,
            replies: HashMap::new(),
            browsers: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }
}

/// The subject a point's messages arrive on, for a point of a NATS session.
pub fn subject_of(
    workspace: &Workspace,
    connection: ConnectionId,
    live: &Live,
    point: u32,
) -> Option<String> {
    let spec = &workspace.connection(connection)?.spec;
    let (info, _) = live.point(point)?;
    match spec {
        // JSON fields are named `<subject> › <field>`.
        SessionSpec::NatsClient(_) => info.name.split(" › ").next().map(str::to_owned),
        // A simulator's values take a new value at `<subject>.set`.
        SessionSpec::NatsServer(_) => Some(info.address.clone()),
        _ => None,
    }
}

impl AppModel {
    /// The focused connection when it is a NATS client.
    pub fn focused_nats_client(&self) -> Option<ConnectionId> {
        let connection = self.focused_connection()?;
        matches!(
            self.workspace.connection(connection)?.spec,
            SessionSpec::NatsClient(_)
        )
        .then_some(connection)
    }

    fn nats_session(&self, connection: ConnectionId) -> Option<SessionId> {
        self.live.get(&connection).and_then(|live| live.session)
    }

    /// The form's subject and payload, and its headers parsed. A bad subject
    /// or header line is reported on the connection's publish result.
    fn nats_message(
        &mut self,
        connection: ConnectionId,
        cx: &mut Context<Self>,
    ) -> Option<(String, String, Vec<vt_protocol::NatsHeader>)> {
        let subject = self.nats.subject.read(cx).value().trim().to_owned();
        let payload = self.nats.payload.read(cx).value().to_owned();
        let headers = parse_headers(self.nats.headers.read(cx).value());
        let problem = if !valid_subject(&subject) {
            Some("Enter a subject to send to, without * or >".to_owned())
        } else {
            headers
                .as_ref()
                .err()
                .map(|error| format!("Headers: {error}"))
        };
        if let Some(problem) = problem {
            if let Some(live) = self.live.get_mut(&connection) {
                live.publish_result = Some((subject, Err(problem)));
            }
            cx.notify();
            return None;
        }
        Some((subject, payload, headers.unwrap_or_default()))
    }

    /// Publish the form's message from the focused NATS session.
    pub fn nats_publish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(session) = self.nats_session(connection) else {
            return;
        };
        let Some((subject, payload, headers)) = self.nats_message(connection, cx) else {
            return;
        };
        if let Some(live) = self.live.get_mut(&connection) {
            live.publish_result = None;
        }
        self.engine.send(EngineCommand::Nats {
            session,
            request: NatsRequest::Publish {
                subject,
                payload,
                headers,
            },
        });
        window.focus(&self.focus);
        cx.notify();
    }

    /// Send the form's message as a request from the focused NATS client
    /// and wait for one reply.
    pub fn nats_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.send_nats_request(cx) {
            window.focus(&self.focus);
        }
    }

    /// [`Self::nats_request`] without moving the focus; whether it was sent.
    pub fn send_nats_request(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(connection) = self.focused_nats_client() else {
            self.show_toast(
                "Requests go from a NATS connection: focus one first",
                true,
                cx,
            );
            return false;
        };
        let Some(session) = self.nats_session(connection) else {
            self.show_toast("Connect first to send a request", true, cx);
            return false;
        };
        let Some((subject, payload, headers)) = self.nats_message(connection, cx) else {
            return false;
        };
        let timeout_ms = self
            .nats
            .timeout
            .read(cx)
            .value()
            .trim()
            .parse::<u64>()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(10, MAX_TIMEOUT_MS);
        if let Some(live) = self.live.get_mut(&connection) {
            live.publish_result = None;
        }
        self.nats.replies.insert(
            connection,
            Reply::Waiting {
                subject: subject.clone(),
            },
        );
        self.engine.send(EngineCommand::Nats {
            session,
            request: NatsRequest::Request {
                subject,
                payload,
                headers,
                timeout_ms,
            },
        });
        cx.notify();
        true
    }

    /// List the focused (or given) client's JetStream streams again.
    pub fn nats_refresh_streams(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.request_streams(connection);
        cx.notify();
    }

    fn request_streams(&mut self, connection: ConnectionId) {
        let Some(session) = self.nats_session(connection) else {
            return;
        };
        let browser = self.nats.browsers.entry(connection).or_default();
        browser.loading = true;
        let open = browser.open.clone();
        self.engine.send(EngineCommand::Nats {
            session,
            request: NatsRequest::Streams,
        });
        if let Some(name) = open {
            self.engine.send(EngineCommand::Nats {
                session,
                request: NatsRequest::Stream {
                    name,
                    messages: STREAM_MESSAGES,
                },
            });
        }
    }

    /// Open a stream below the list (or close it when it is open).
    pub fn nats_toggle_stream(
        &mut self,
        connection: ConnectionId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.nats_session(connection) else {
            return;
        };
        let browser = self.nats.browsers.entry(connection).or_default();
        browser.fetched = None;
        browser.fetching = None;
        if browser.open.as_deref() == Some(name.as_str()) {
            browser.open = None;
            browser.detail = None;
        } else {
            browser.open = Some(name.clone());
            browser.detail = None;
            self.engine.send(EngineCommand::Nats {
                session,
                request: NatsRequest::Stream {
                    name,
                    messages: STREAM_MESSAGES,
                },
            });
        }
        cx.notify();
    }

    /// Pull the next messages of a stream's durable consumer.
    pub fn nats_fetch(
        &mut self,
        connection: ConnectionId,
        stream: String,
        consumer: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.nats_session(connection) else {
            return;
        };
        let browser = self.nats.browsers.entry(connection).or_default();
        browser.fetching = Some(consumer.clone());
        self.engine.send(EngineCommand::Nats {
            session,
            request: NatsRequest::Fetch {
                stream,
                consumer,
                batch: FETCH_BATCH,
            },
        });
        cx.notify();
    }

    /// A NATS client's answer, for the connection that runs `session`.
    pub(super) fn nats_event(&mut self, session: SessionId, event: NatsEvent) {
        let Some(connection) = self.sessions.get(&session).copied() else {
            return;
        };
        match event {
            NatsEvent::Reply { subject, result } => {
                self.nats
                    .replies
                    .insert(connection, Reply::Done { subject, result });
            }
            NatsEvent::Streams(result) => {
                let browser = self.nats.browsers.entry(connection).or_default();
                browser.loading = false;
                browser.streams = Some(result);
            }
            NatsEvent::Stream { name, result } => {
                let browser = self.nats.browsers.entry(connection).or_default();
                if browser.open.as_deref() == Some(name.as_str()) {
                    browser.detail = Some(result);
                }
            }
            NatsEvent::Fetched {
                stream,
                consumer,
                result,
            } => {
                let browser = self.nats.browsers.entry(connection).or_default();
                browser.fetching = None;
                let refresh = result.is_ok() && browser.open.as_deref() == Some(stream.as_str());
                browser.fetched = Some((consumer, result));
                // The consumer's pending count moved on.
                if refresh {
                    self.engine.send(EngineCommand::Nats {
                        session,
                        request: NatsRequest::Stream {
                            name: stream,
                            messages: STREAM_MESSAGES,
                        },
                    });
                }
            }
        }
    }

    /// A NATS client connected: show its JetStream streams without asking.
    pub(super) fn nats_connected(&mut self, connection: ConnectionId) {
        let client = self
            .workspace
            .connection(connection)
            .is_some_and(|saved| matches!(saved.spec, SessionSpec::NatsClient(_)));
        if client {
            self.request_streams(connection);
        }
    }

    /// The editor's creds file picker: read the chosen `.creds` file into
    /// the form (the sandbox grants the app the file the user picked).
    pub fn choose_nats_creds(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use credentials".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let read = std::fs::read_to_string(&path);
            let _ = this.update(cx, |model, cx| {
                let Some(editor) = model.editor_mut() else {
                    return;
                };
                match read {
                    Ok(creds) => {
                        editor.nats.creds_file = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        editor.nats.creds = creds;
                        editor.error = None;
                    }
                    Err(error) => {
                        editor.error = Some(vt_app_core::forms::FormError {
                            field: vt_app_core::forms::Field::Credential,
                            message: format!("Cannot read {}: {error}", path.display()),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// After a new NATS simulator is saved: a client of it, side by side.
    pub(super) fn add_nats_client(
        &mut self,
        name: &str,
        server: &NatsServerSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::NatsClient(server.client()),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// The SMA inverter template published by a NATS server simulator with
    /// JetStream, and a client subscribed to it, side by side.
    pub fn start_nats_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        let Some(profile) = templates::find_modbus("sma-inverter") else {
            self.show_toast("The demo device template is missing", true, cx);
            return None;
        };
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::NatsServer(server) => Some(server.port),
                _ => None,
            })
            .collect();
        let Some(port) = (NATS_PORT..NATS_PORT + 100).find(|port| {
            !taken.contains(port) && std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok()
        }) else {
            self.show_toast(
                "No free port between 4222 and 4321 for the NATS server",
                true,
                cx,
            );
            return None;
        };
        let server = NatsServerSpec {
            port,
            template: Some(profile.id.clone()),
            ..NatsServerSpec::default()
        };
        let client = server.client();
        let simulator = self
            .workspace
            .add_connection("Inverter NATS server", SessionSpec::NatsServer(server));
        let client = self.workspace.add_connection(
            format!("{} {} over NATS", profile.manufacturer, profile.model),
            SessionSpec::NatsClient(client),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }
}
