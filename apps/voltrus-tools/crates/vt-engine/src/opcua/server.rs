//! Plays an OPC UA server: `mt-opcua`'s plant server holding the plant's address space, driven by
//! the simulation every tick. Any client may browse it, subscribe, write and call its methods.
//!
//! A value set from the app, or written by a client, holds a measured variable at that value until
//! released; settings (setpoints, commands) take effect instead. The app sees every held point as
//! `Locked`, and every client write and method call in the traffic view.

use std::time::Duration;

use mt_opcua::{PlantServer, PlantServerConfig};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use vt_protocol::{
    Direction, LogLevel, OpcuaRequest, OpcuaServerSpec, PointInfo, PointText, Quality, Reading,
    SessionState, Stats, TrafficFrame, ValueFrame, parse_opcua_value,
};

use super::plant::{self, Effect, Kind, Plant, VARIABLES};
use super::traffic;
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often the simulation moves and the app sees its values.
pub(crate) const TICK: Duration = Duration::from_millis(500);
/// How long a port still held by a closing session is retried.
const BIND_RETRY: Duration = Duration::from_secs(2);
const BIND_RETRY_STEP: Duration = Duration::from_millis(100);
/// Common name of the user certificate clients may sign in with.
const USER_CERTIFICATE_NAME: &str = "Voltrus operator";

pub(crate) async fn run(
    spec: OpcuaServerSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    super::init_pki();
    let shared = plant::Shared::new();
    let (effect_tx, mut effects) = mpsc::unbounded_channel::<Effect>();

    // A certificate clients can sign in with, next to the server's own.
    let users = mt_opcua::pki::server_dir(spec.port).join("users");
    let certificate = users.join("operator.der");
    let private_key = users.join("operator.pem");
    let user_certificate = match mt_opcua::create_user_certificate(
        &certificate,
        &private_key,
        USER_CERTIFICATE_NAME,
    ) {
        Ok(()) => Some(certificate.clone()),
        Err(error) => {
            out.log(
                LogLevel::Warn,
                format!("Certificate sign-in is off: {error}"),
            );
            None
        }
    };
    let config = PlantServerConfig {
        port: spec.port,
        application_name: "Voltrus plant".into(),
        anonymous: spec.anonymous,
        user: (!spec.username.is_empty()).then(|| (spec.username.clone(), spec.password.clone())),
        user_certificate: user_certificate.clone(),
    };
    let server = match start(
        &config,
        plant::handler(shared.clone(), effect_tx),
        &mut control,
        out,
    )
    .await
    {
        Ok(Some(server)) => server,
        Ok(None) => {
            out.state(SessionState::Closed { error: None });
            return;
        }
        Err(error) => {
            out.log(LogLevel::Error, error.clone());
            out.state(SessionState::Closed { error: Some(error) });
            return;
        }
    };
    out.state(SessionState::Listening {
        address: server.url().to_owned(),
    });
    let mut sign_in = Vec::new();
    if spec.anonymous {
        sign_in.push("anonymous".to_owned());
    }
    if !spec.username.is_empty() {
        sign_in.push(format!("user {}", spec.username));
    }
    if user_certificate.is_some() {
        sign_in.push("the user certificate".to_owned());
    }
    out.log(
        LogLevel::Info,
        format!(
            "Serving the plant at {} (namespace {}), {} variables and 2 methods; sign in as {}",
            server.url(),
            server.namespace_index(),
            VARIABLES.len(),
            sign_in.join(", ")
        ),
    );
    if user_certificate.is_some() {
        out.log(
            LogLevel::Info,
            format!(
                "User certificate {} with private key {}",
                certificate.display(),
                private_key.display()
            ),
        );
    }
    out.points(points(&server));
    out.payloads(vec![recipe_text(&Plant::new())]);

    let mut sim = Plant::new();
    let mut held = [false; VARIABLES.len()];
    let mut last = sim.json_values();
    let mut stats = Stats::default();
    let mut frames: Vec<TrafficFrame> = Vec::new();
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut previous = Instant::now();

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Writes from clients since the last tick: hold measured values, take settings.
                for (index, variable) in VARIABLES.iter().enumerate() {
                    let Some((number, text)) = server.value(variable.key) else { continue };
                    let now = current(variable.data_type, number, text);
                    if now == last[index] {
                        continue;
                    }
                    stats.frames += 1;
                    frames.push(traffic(
                        Direction::Rx,
                        format!("Write {}", variable.name),
                        format!("{} = {}", server.node_id(variable.key), shown(&now)),
                    ));
                    take(&mut sim, index, &now);
                    if variable.kind == Kind::Measured && !held[index] {
                        held[index] = true;
                        out.locked(point_id(index), true);
                    }
                    out.log(LogLevel::Info, format!("A client wrote {} = {}", variable.name, shown(&now)));
                }
                let elapsed = previous.elapsed().as_secs_f64();
                previous = Instant::now();
                sim.step(elapsed, &held, &shared);
                publish(&server, &sim, &mut last, &mut stats, out);
                out.traffic(std::mem::take(&mut frames));
                out.stats(stats);
            }
            effect = effects.recv() => {
                let Some(effect) = effect else { continue };
                stats.frames += 1;
                let (summary, detail) = match &effect {
                    Effect::StartBatch { recipe, litres, batch } => (
                        "Call StartBatch".to_owned(),
                        format!("Recipe “{recipe}”, {litres} L → BatchId {batch}"),
                    ),
                    Effect::ResetAlarms => ("Call ResetAlarms".to_owned(), String::new()),
                };
                out.log(LogLevel::Info, format!("{summary} {detail}"));
                frames.push(traffic(Direction::Rx, summary, detail));
                sim.apply(&effect);
                publish(&server, &sim, &mut last, &mut stats, out);
            }
            command = control.recv() => match command {
                None | Some(Control::Close) => break,
                Some(Control::Write { point, value }) => {
                    let result = set(&mut sim, &mut held, point, None, Some(value), out);
                    if result.is_ok() {
                        publish(&server, &sim, &mut last, &mut stats, out);
                    }
                    out.written(point, result);
                }
                Some(Control::Opcua(OpcuaRequest::Write { point, text })) => {
                    let result = set(&mut sim, &mut held, point, Some(&text), None, out);
                    if result.is_ok() {
                        publish(&server, &sim, &mut last, &mut stats, out);
                    }
                    out.written(point, result);
                }
                Some(Control::Lock { point, locked }) => {
                    let index = usize::try_from(point).unwrap_or(usize::MAX);
                    if VARIABLES.get(index).is_some_and(|variable| variable.kind == Kind::Measured) {
                        held[index] = locked;
                        out.locked(point, locked);
                    }
                }
                // Browsing and calls are for clients; publishing for message protocols.
                Some(_) => {}
            },
        }
    }
    server.stop().await;
    out.traffic(frames);
    out.stats(stats);
    out.state(SessionState::Closed { error: None });
}

/// Start the server on the spec's port, retrying for [`BIND_RETRY`] while a closing session may
/// still hold it. `Ok(None)` when the app closed the session meanwhile.
async fn start(
    config: &PlantServerConfig,
    handler: mt_opcua::MethodHandler,
    control: &mut mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) -> Result<Option<PlantServer>, String> {
    let deadline = Instant::now() + BIND_RETRY;
    let nodes = plant::nodes();
    loop {
        match PlantServer::start(config.clone(), &nodes, handler.clone()).await {
            Ok(server) => return Ok(Some(server)),
            Err(error) => {
                let error = error.to_string();
                if !error.contains("in use") || Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY_STEP) => {}
            () = until_closed(control, out) => return Ok(None),
        }
    }
}

/// Wait for the app to close the session before the server runs, refusing writes.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(
                Control::Write { point, .. } | Control::Opcua(OpcuaRequest::Write { point, .. }),
            ) => {
                out.written(point, Err("The server is not running yet".into()));
            }
            Some(_) => {}
        }
    }
}

fn point_id(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// The plant's variables as points. The app may set any of them: forcing a measurement is what a
/// simulator is for.
fn points(server: &PlantServer) -> Vec<PointInfo> {
    VARIABLES
        .iter()
        .enumerate()
        .map(|(index, variable)| PointInfo {
            id: point_id(index),
            name: variable
                .key
                .trim_start_matches("Plant.")
                .replace('.', " › "),
            address: server.node_id(variable.key),
            data_type: variable.data_type.into(),
            unit: variable.unit.into(),
            writable: true,
            boolean: variable.data_type == "Boolean",
            states: Vec::new(),
        })
        .collect()
}

/// A variable's value read back from the address space, as the JSON the simulation writes.
fn current(data_type: &str, number: Option<f64>, text: String) -> serde_json::Value {
    match (data_type, number) {
        ("String", _) | (_, None) => serde_json::Value::String(text),
        (data_type, Some(number)) => plant::json(data_type, number),
    }
}

fn shown(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => format!("“{text}”"),
        other => other.to_string(),
    }
}

/// Take a value into the simulation.
fn take(sim: &mut Plant, index: usize, value: &serde_json::Value) {
    if index == plant::RECIPE {
        if let Some(text) = value.as_str() {
            text.clone_into(&mut sim.recipe);
        }
    } else if let Some(slot) = sim.values.get_mut(index) {
        *slot = match value {
            serde_json::Value::Bool(on) => f64::from(u8::from(*on)),
            other => other.as_f64().unwrap_or(*slot),
        };
    }
}

/// Set a point from the app: typed text (OPC UA writes) or a number (the generic write).
fn set(
    sim: &mut Plant,
    held: &mut [bool],
    point: u32,
    text: Option<&str>,
    number: Option<f64>,
    out: &Emitter,
) -> Result<(), String> {
    let index = usize::try_from(point).unwrap_or(usize::MAX);
    let variable = VARIABLES
        .get(index)
        .ok_or_else(|| format!("No point {point}"))?;
    let text = match (text, number) {
        (Some(text), _) => text.to_owned(),
        (None, Some(number)) if variable.data_type == "Boolean" => (number != 0.0).to_string(),
        (None, Some(number)) => number.to_string(),
        (None, None) => return Err("Nothing to set".into()),
    };
    let value = match parse_opcua_value(variable.data_type, &text)? {
        vt_protocol::OpcuaScalar::Boolean(on) => serde_json::Value::Bool(on),
        vt_protocol::OpcuaScalar::Int(n) => serde_json::json!(n),
        vt_protocol::OpcuaScalar::UInt(n) => serde_json::json!(n),
        vt_protocol::OpcuaScalar::Float(n) => serde_json::json!(n),
        vt_protocol::OpcuaScalar::Text(text) => serde_json::Value::String(text),
    };
    take(sim, index, &value);
    if variable.kind == Kind::Measured && !held[index] {
        held[index] = true;
        out.locked(point, true);
    }
    Ok(())
}

/// The recipe is text: the inspector shows it from the point's payload.
fn recipe_text(sim: &Plant) -> PointText {
    PointText {
        point: point_id(plant::RECIPE),
        text: sim.recipe.clone(),
    }
}

/// Write the simulation's values that changed into the address space and report them.
fn publish(
    server: &PlantServer,
    sim: &Plant,
    last: &mut [serde_json::Value],
    stats: &mut Stats,
    out: &Emitter,
) {
    let values = sim.json_values();
    let changed: Vec<(&str, &str, serde_json::Value)> = VARIABLES
        .iter()
        .zip(&values)
        .zip(last.iter())
        .filter(|((_, now), before)| now != before)
        .map(|((variable, now), _)| (variable.key, variable.data_type, now.clone()))
        .collect();
    if values[plant::RECIPE] != last[plant::RECIPE] {
        out.payloads(vec![recipe_text(sim)]);
    }
    if !changed.is_empty() {
        match server.set_values(&changed) {
            Ok(()) => {
                for (slot, value) in last.iter_mut().zip(&values) {
                    slot.clone_from(value);
                }
            }
            Err(error) => {
                stats.errors += 1;
                out.log(LogLevel::Error, error.to_string());
            }
        }
    }
    stats.polls += 1;
    out.values(frame(sim));
}

fn frame(sim: &Plant) -> ValueFrame {
    ValueFrame {
        at_ms: now_ms(),
        latency_us: 0,
        readings: VARIABLES
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index == plant::RECIPE {
                    // Text: the value is in the point's payload.
                    Reading {
                        value: f64::NAN,
                        raw: [0; 4],
                        raw_len: 0,
                        quality: Quality::Good,
                    }
                } else {
                    Reading {
                        value: sim.values[index],
                        raw: [0; 4],
                        raw_len: 0,
                        quality: Quality::Good,
                    }
                }
            })
            .collect(),
    }
}
