use std::collections::HashMap;
use std::sync::Arc;

use mt_nats::{NatsClient, NatsMessage, NatsServerState, SchemaPool};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// A live client-mode subscription managed by the frontend.
pub struct ActiveSubscription {
    pub handle: JoinHandle<()>,
}

/// Client-mode state: the connected client + active subscriptions.
#[derive(Clone, Default)]
pub struct ClientState {
    pub client: Arc<Mutex<Option<NatsClient>>>,
    pub subs: Arc<Mutex<HashMap<String, ActiveSubscription>>>,
    pub recent: Arc<Mutex<Vec<NatsMessage>>>,
}

/// Embedded-server mode state.
#[derive(Clone, Default)]
pub struct ServerState {
    pub server: Arc<Mutex<Option<NatsServerState>>>,
}

/// Top-level managed app state.
#[derive(Clone, Default)]
pub struct AppState {
    pub client: ClientState,
    pub server: ServerState,
    /// Session-lived protobuf schema registry for payload decoding.
    pub schema: Arc<Mutex<SchemaPool>>,
}
