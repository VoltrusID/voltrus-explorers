//! NATS protocol crate for the MacTools suite.
//!
//! Provides:
//! - A client wrapper around the official [`async_nats`] client (core NATS +
//!   JetStream), plus UI-friendly serde types.
//! - A hand-rolled embedded NATS server (core protocol only) so the
//!   NATS Explorer app can demo / develop against a real server without
//!   bundling the Go `nats-server` binary.
//!
//! The embedded server speaks the NATS text protocol
//! (INFO/CONNECT/PING/PONG/PUB/HMSG/SUB/UNSUB/MSG) with wildcard subjects
//! (`*`, `>`), optional user/pass auth, and an in-process mirror channel that
//! forwards every routed message to a caller-supplied callback (used by the
//! app to feed the message table + subject tracker). It does NOT implement
//! JetStream (`$JS.API`), clustering, or TLS — see [`server::NatsServerState`].

pub mod client;
pub mod decode;
pub mod schema;
pub mod server;
pub mod server_js;
pub mod types;

pub use client::NatsClient;
pub use decode::*;
pub use schema::SchemaPool;
pub use server::{
    start_server, start_server_on, stop_server, NatsServerState, OnMessage, TopicTracker,
};
pub use server_js::nats_subject_match;
pub use types::*;
