//! A BLE client session: find the peripheral by scanning, connect, discover
//! its GATT services and make every characteristic a point. Readable ones
//! are read at once and then polled, notifying ones subscribed; values are
//! decoded by the standard SIG layout or the decoder chosen for them, and
//! the bytes behind each value go to the inspector. A lost link reconnects
//! with backoff.

use std::time::Duration;

use mt_ble::BleService;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, timeout};
use vt_protocol::{
    BleAdapter, BleCharacteristicInfo, BleClientSpec, BleDecoder, BleProperties, BleRequest,
    Direction, LogLevel, PointInfo, PointText, Quality, Reading, SessionState, Stats, TrafficFrame,
    ValueFrame,
};

use super::central::{Central, Notification};
use super::decode::{self, Value};
use super::gatt;
use super::simulated::SimCentral;
use super::system::SystemCentral;
use crate::Control;
use crate::emit::{Emitter, now_ms};

/// How often the app sees what arrived.
pub(crate) const TICK: Duration = Duration::from_millis(250);
/// How long a connect scans for the peripheral before giving up.
const FIND_TIMEOUT: Duration = Duration::from_secs(10);
const FIND_POLL: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest a read, write or service discovery may take.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
/// Report ticks between checks that the link is still up.
const LINK_CHECK_TICKS: u32 = 4;
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// Shown as the value of characteristics that can only be written.
const WRITE_ONLY: &str = "write only";
/// Shown as the value of characteristics that only push, until they do.
const AWAITING: &str = "no notification yet";

pub(crate) async fn run(
    spec: BleClientSpec,
    control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    // The platform's Bluetooth stack runs under this task: if it panics, the
    // app still hears that the session ended, and why.
    let session = out.clone();
    let task = tokio::spawn(async move { open(spec, control, &session).await });
    if let Err(error) = task.await
        && error.is_panic()
    {
        let reason = "Bluetooth stopped unexpectedly. Connect again to retry.".to_owned();
        out.log(LogLevel::Error, reason.clone());
        out.state(SessionState::Closed {
            error: Some(reason),
        });
    }
}

async fn open(spec: BleClientSpec, mut control: mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    out.state(SessionState::Connecting);
    match spec.adapter {
        BleAdapter::Simulated => {
            let (central, notifications) = SimCentral::new();
            run_with(&central, notifications, spec, control, out).await;
        }
        BleAdapter::System => {
            let opened = tokio::select! {
                opened = SystemCentral::open() => opened,
                () = until_closed(&mut control, out) => {
                    out.state(SessionState::Closed { error: None });
                    return;
                }
            };
            match opened {
                Ok((central, notifications)) => {
                    run_with(&central, notifications, spec, control, out).await;
                }
                Err(error) => {
                    out.log(LogLevel::Error, error.clone());
                    out.state(SessionState::Closed { error: Some(error) });
                }
            }
        }
    }
}

/// The session against any central.
pub(crate) async fn run_with<C: Central>(
    central: &C,
    mut notifications: mpsc::Receiver<Notification>,
    mut spec: BleClientSpec,
    mut control: mpsc::UnboundedReceiver<Control>,
    out: &Emitter,
) {
    let label = label(&spec);
    let mut state = State::default();
    let mut attempt: u32 = 0;

    'session: loop {
        out.state(SessionState::Connecting);
        let mut found = None;
        let established = tokio::select! {
            result = establish(central, &spec, &mut state.traffic, &mut found) => result,
            () = until_closed(&mut control, out) => {
                // Closed mid-connect: dropping `establish` skips its own
                // clean-up, so stop the scan and drop a half-made link here.
                let _ = central.stop_scan().await;
                if let Some(id) = found {
                    let _ = central.disconnect(&id).await;
                }
                break 'session;
            }
        };
        let (id, services) = match established {
            Ok(established) => established,
            Err(reason) => {
                attempt += 1;
                state.stats.errors += 1;
                out.log(LogLevel::Error, reason.clone());
                state.flush(out);
                if !retry(&mut control, attempt, &reason, out).await {
                    break 'session;
                }
                continue 'session;
            }
        };
        if state.lay_out(&services, &spec) {
            out.points(state.points());
        }
        out.state(SessionState::Connected);
        out.log(
            LogLevel::Info,
            format!(
                "Connected to {label}: {} services, {} characteristics",
                services.len(),
                state.characteristics.len()
            ),
        );

        // First values, then the subscriptions wanted (all that push, or
        // the ones on before the link was lost).
        for point in 0..state.characteristics.len() {
            if state.characteristics[point].properties.read {
                state.read(central, &id, point, out).await;
            }
        }
        for point in 0..state.characteristics.len() {
            if state.characteristics[point].notifying {
                state.subscribe(central, &id, point, true, out).await;
            }
        }
        out.ble_gatt(state.info());
        state.flush(out);

        let mut report = tokio::time::interval(TICK);
        report.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let every = Duration::from_millis(spec.poll_ms.max(1));
        let mut poll = tokio::time::interval_at(Instant::now() + every, every);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut ticks: u32 = 0;
        let lost = loop {
            tokio::select! {
                Some(notification) = notifications.recv() => {
                    if notification.peripheral == id {
                        state.notified(notification, out);
                    }
                }
                _ = report.tick() => {
                    state.flush(out);
                    ticks = ticks.wrapping_add(1);
                    if ticks.is_multiple_of(LINK_CHECK_TICKS)
                        && !matches!(central.is_connected(&id).await, Ok(true))
                    {
                        break format!("Lost the connection to {label}");
                    }
                }
                _ = poll.tick(), if spec.poll_ms > 0 => {
                    state.stats.polls += 1;
                    for point in 0..state.characteristics.len() {
                        let characteristic = &state.characteristics[point];
                        if characteristic.properties.read && !characteristic.notifying {
                            state.read(central, &id, point, out).await;
                        }
                    }
                }
                command = control.recv() => match command {
                    None | Some(Control::Close) => {
                        for point in 0..state.characteristics.len() {
                            if state.characteristics[point].notifying {
                                let uuid = state.characteristics[point].uuid.clone();
                                let _ = central.unsubscribe(&id, &uuid).await;
                            }
                        }
                        let _ = central.disconnect(&id).await;
                        state.traffic.push(frame(Direction::Tx, format!("DISCONNECT {label}"), id.clone(), 0));
                        state.flush(out);
                        break 'session;
                    }
                    Some(Control::Write { point, value }) => {
                        let result = match state.characteristic(point) {
                            Some(found) => decode::encode(&found.uuid, found.decoder, value),
                            None => Err("No such characteristic".into()),
                        };
                        let result = match result {
                            Ok(bytes) => state.write(central, &id, point, bytes, out).await,
                            Err(error) => Err(error),
                        };
                        out.written(point, result);
                    }
                    Some(Control::Ble(request)) => match request {
                        BleRequest::Read { point } => {
                            if let Ok(index) = usize::try_from(point) && index < state.characteristics.len() {
                                state.read(central, &id, index, out).await;
                            }
                        }
                        BleRequest::Write { point, data } => {
                            let result = state.write(central, &id, point, data, out).await;
                            out.written(point, result);
                        }
                        BleRequest::Notify { point, enabled } => {
                            if let Ok(index) = usize::try_from(point) && index < state.characteristics.len() {
                                state.subscribe(central, &id, index, enabled, out).await;
                                out.ble_gatt(state.info());
                            }
                        }
                        BleRequest::Decode { point, decoder } => {
                            if let Some(uuid) = state.decode_with(point, decoder, out) {
                                spec.set_decoder(&uuid, decoder);
                            }
                        }
                    },
                    // Clients serve nothing, so there is nothing to hold.
                    Some(_) => {}
                },
            }
        };
        state.stats.errors += 1;
        out.log(LogLevel::Error, lost.clone());
        state.stale();
        state.flush(out);
        let _ = central.disconnect(&id).await;
        attempt = 1;
        if !retry(&mut control, attempt, &lost, out).await {
            break 'session;
        }
    }
    out.stats(state.stats);
    out.state(SessionState::Closed { error: None });
}

/// The peripheral by its name, else its identifier.
fn label(spec: &BleClientSpec) -> String {
    if spec.name.trim().is_empty() {
        spec.peripheral.clone()
    } else {
        spec.name.clone()
    }
}

/// Scan until the peripheral advertises: by identifier, else by name (an
/// identifier can change, e.g. after the computer forgets it).
async fn find<C: Central>(central: &C, spec: &BleClientSpec) -> Result<String, String> {
    central.start_scan().await?;
    let deadline = Instant::now() + FIND_TIMEOUT;
    let found = loop {
        let devices = central.peripherals().await;
        let by_name = || {
            devices
                .iter()
                .find(|device| !spec.name.is_empty() && device.name == spec.name)
        };
        if let Some(device) = devices
            .iter()
            .find(|device| device.id == spec.peripheral)
            .or_else(by_name)
        {
            break Ok(device.id.clone());
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "{} is not advertising: check it is on, in range and not connected to another device",
                label(spec)
            ));
        }
        tokio::time::sleep(FIND_POLL).await;
    };
    let _ = central.stop_scan().await;
    found
}

/// Find, connect and discover the services. `found` holds the peripheral
/// once the scan finds it, so a caller that drops this mid-connect can
/// disconnect it.
async fn establish<C: Central>(
    central: &C,
    spec: &BleClientSpec,
    traffic: &mut Vec<TrafficFrame>,
    found: &mut Option<String>,
) -> Result<(String, Vec<BleService>), String> {
    let id = find(central, spec).await?;
    *found = Some(id.clone());
    let label = label(spec);
    traffic.push(frame(
        Direction::Tx,
        format!("CONNECT {label}"),
        id.clone(),
        0,
    ));
    timeout(CONNECT_TIMEOUT, central.connect(&id))
        .await
        .map_err(|_| format!("{label} did not answer the connection request"))??;
    let services = match timeout(OPERATION_TIMEOUT, central.services(&id)).await {
        Ok(Ok(services)) => services,
        Ok(Err(error)) => {
            let _ = central.disconnect(&id).await;
            return Err(error);
        }
        Err(_) => {
            let _ = central.disconnect(&id).await;
            return Err(format!("{label} did not list its services in time"));
        }
    };
    let characteristics: usize = services
        .iter()
        .map(|service| service.characteristics.len())
        .sum();
    traffic.push(frame(
        Direction::Rx,
        "SERVICES".into(),
        format!(
            "{} services, {characteristics} characteristics",
            services.len()
        ),
        0,
    ));
    Ok((id, services))
}

/// One characteristic, as a point.
#[derive(Clone, Debug)]
struct Characteristic {
    service: String,
    uuid: String,
    name: String,
    properties: BleProperties,
    decoder: BleDecoder,
    /// Subscribed, or to be subscribed once connected.
    notifying: bool,
    /// The last value read or pushed.
    bytes: Option<Vec<u8>>,
    /// The last decode error, logged once.
    error: Option<String>,
}

impl Characteristic {
    fn address(&self) -> String {
        format!(
            "{} › {}",
            gatt::short(&self.service),
            gatt::short(&self.uuid)
        )
    }

    /// What a characteristic that cannot be read shows before it has a value.
    fn placeholder(&self) -> Option<&'static str> {
        match (self.properties.read, self.properties.pushes()) {
            (true, _) => None,
            (false, true) => Some(AWAITING),
            (false, false) => Some(WRITE_ONLY),
        }
    }
}

#[derive(Default)]
struct State {
    characteristics: Vec<Characteristic>,
    readings: Vec<Reading>,
    /// Payloads changed since the last report.
    payloads: Vec<PointText>,
    dirty: bool,
    stats: Stats,
    reported: Stats,
    traffic: Vec<TrafficFrame>,
}

impl State {
    /// Take the services just discovered. Returns whether the points changed
    /// (the first connection, or a peripheral whose services changed); a
    /// reconnect to the same layout keeps values, decoders and subscriptions.
    fn lay_out(&mut self, services: &[BleService], spec: &BleClientSpec) -> bool {
        let fresh: Vec<Characteristic> = services
            .iter()
            .flat_map(|service| {
                service.characteristics.iter().map(move |found| {
                    let uuid = found.uuid.to_ascii_lowercase();
                    let properties = BleProperties {
                        read: found.properties.read,
                        write: found.properties.write,
                        write_without_response: found.properties.write_without_response,
                        notify: found.properties.notify,
                        indicate: found.properties.indicate,
                    };
                    Characteristic {
                        service: service.uuid.to_ascii_lowercase(),
                        name: gatt::name(&uuid).map_or_else(
                            || format!("Characteristic {}", gatt::short(&uuid)),
                            str::to_owned,
                        ),
                        decoder: spec.decoder(&uuid).unwrap_or_default(),
                        notifying: spec.notify && properties.pushes(),
                        properties,
                        uuid,
                        bytes: None,
                        error: None,
                    }
                })
            })
            .collect();
        let same = fresh.len() == self.characteristics.len()
            && fresh
                .iter()
                .zip(&self.characteristics)
                .all(|(new, old)| new.service == old.service && new.uuid == old.uuid);
        if same {
            return false;
        }
        self.characteristics = fresh;
        self.readings = self
            .characteristics
            .iter()
            .map(|characteristic| match characteristic.placeholder() {
                Some(_) => good(f64::NAN),
                None => Reading::bad(),
            })
            .collect();
        self.payloads = self.placeholders();
        self.dirty = true;
        true
    }

    fn points(&self) -> Vec<PointInfo> {
        self.characteristics
            .iter()
            .zip(0u32..)
            .map(|(characteristic, id)| PointInfo {
                id,
                name: characteristic.name.clone(),
                address: gatt::short(&characteristic.uuid),
                data_type: decode::label(&characteristic.uuid, characteristic.decoder),
                unit: decode::unit(&characteristic.uuid, characteristic.decoder).to_owned(),
                writable: characteristic.properties.writable(),
                boolean: false,
                states: Vec::new(),
            })
            .collect()
    }

    fn info(&self) -> Vec<BleCharacteristicInfo> {
        self.characteristics
            .iter()
            .zip(0u32..)
            .map(|(characteristic, point)| BleCharacteristicInfo {
                point,
                service: characteristic.service.clone(),
                uuid: characteristic.uuid.clone(),
                properties: characteristic.properties,
                notifying: characteristic.notifying,
                decoder: characteristic.decoder,
            })
            .collect()
    }

    fn characteristic(&self, point: u32) -> Option<&Characteristic> {
        self.characteristics.get(usize::try_from(point).ok()?)
    }

    /// Decode new bytes of `point` into its reading and payload.
    fn update(&mut self, point: usize, bytes: Vec<u8>, out: &Emitter) {
        let Some(characteristic) = self.characteristics.get_mut(point) else {
            return;
        };
        let hex = decode::hex(&bytes);
        let (reading, text) =
            match decode::decode(&characteristic.uuid, characteristic.decoder, &bytes) {
                Ok(value) => {
                    characteristic.error = None;
                    match value {
                        Value::Number(number) => (good(number), hex),
                        Value::Text(text) if text == hex || hex.is_empty() => {
                            (good(f64::NAN), text)
                        }
                        Value::Text(text) => (good(f64::NAN), format!("{text}\n{hex}")),
                    }
                }
                Err(error) => {
                    if characteristic.error.as_ref() != Some(&error) {
                        out.log(LogLevel::Warn, format!("{}: {error}", characteristic.name));
                        characteristic.error = Some(error);
                    }
                    (Reading::bad(), hex)
                }
            };
        characteristic.bytes = Some(bytes);
        if let Some(slot) = self.readings.get_mut(point) {
            *slot = reading;
        }
        if let Ok(point) = u32::try_from(point) {
            self.payloads.retain(|payload| payload.point != point);
            self.payloads.push(PointText { point, text });
        }
        self.dirty = true;
    }

    async fn read<C: Central>(&mut self, central: &C, id: &str, point: usize, out: &Emitter) {
        let Some(characteristic) = self.characteristics.get(point) else {
            return;
        };
        let (uuid, name) = (characteristic.uuid.clone(), characteristic.name.clone());
        self.traffic.push(frame(
            Direction::Tx,
            format!("READ {name}"),
            characteristic.address(),
            0,
        ));
        let started = Instant::now();
        let result = timeout(OPERATION_TIMEOUT, central.read(id, &uuid))
            .await
            .unwrap_or_else(|_| Err("Read timed out".into()));
        self.stats.last_latency_us =
            u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
        match result {
            Ok(bytes) => {
                self.stats.frames += 1;
                self.traffic.push(frame(
                    Direction::Rx,
                    format!("VALUE {name}"),
                    decode::hex(&bytes),
                    len(&bytes),
                ));
                self.update(point, bytes, out);
            }
            Err(error) => {
                self.stats.errors += 1;
                self.traffic.push(frame(
                    Direction::Rx,
                    format!("ERROR {name}"),
                    error.clone(),
                    0,
                ));
                out.log(LogLevel::Warn, format!("Reading {name}: {error}"));
                if let Some(slot) = self.readings.get_mut(point) {
                    *slot = Reading::bad();
                }
                self.dirty = true;
            }
        }
    }

    /// Write `data` to `point`, then read it back where the characteristic
    /// allows, so the table shows what the peripheral holds.
    async fn write<C: Central>(
        &mut self,
        central: &C,
        id: &str,
        point: u32,
        data: Vec<u8>,
        out: &Emitter,
    ) -> Result<(), String> {
        let index = usize::try_from(point).map_err(|error| error.to_string())?;
        let Some(characteristic) = self.characteristics.get(index) else {
            return Err("No such characteristic".into());
        };
        if !characteristic.properties.writable() {
            return Err(format!("{} takes no writes", characteristic.name));
        }
        let (uuid, name, readable) = (
            characteristic.uuid.clone(),
            characteristic.name.clone(),
            characteristic.properties.read,
        );
        self.traffic.push(frame(
            Direction::Tx,
            format!("WRITE {name}"),
            decode::hex(&data),
            len(&data),
        ));
        let result = timeout(OPERATION_TIMEOUT, central.write(id, &uuid, &data))
            .await
            .unwrap_or_else(|_| Err("Write timed out".into()));
        match &result {
            Ok(()) => {
                self.stats.frames += 1;
                out.log(
                    LogLevel::Info,
                    format!("Wrote {} to {name}", decode::hex(&data)),
                );
                if readable {
                    self.read(central, id, index, out).await;
                }
            }
            Err(error) => {
                self.stats.errors += 1;
                self.traffic.push(frame(
                    Direction::Rx,
                    format!("ERROR {name}"),
                    error.clone(),
                    0,
                ));
            }
        }
        result
    }

    /// Turn notifications of `point` on or off.
    async fn subscribe<C: Central>(
        &mut self,
        central: &C,
        id: &str,
        point: usize,
        enabled: bool,
        out: &Emitter,
    ) {
        let Some(characteristic) = self.characteristics.get(point) else {
            return;
        };
        let (uuid, name) = (characteristic.uuid.clone(), characteristic.name.clone());
        if enabled && !characteristic.properties.pushes() {
            out.log(LogLevel::Warn, format!("{name} sends no notifications"));
            return;
        }
        let verb = if enabled { "SUBSCRIBE" } else { "UNSUBSCRIBE" };
        self.traffic.push(frame(
            Direction::Tx,
            format!("{verb} {name}"),
            characteristic.address(),
            0,
        ));
        let result = if enabled {
            timeout(OPERATION_TIMEOUT, central.subscribe(id, &uuid)).await
        } else {
            timeout(OPERATION_TIMEOUT, central.unsubscribe(id, &uuid)).await
        }
        .unwrap_or_else(|_| Err("No answer in time".into()));
        match result {
            Ok(()) => {
                if let Some(characteristic) = self.characteristics.get_mut(point) {
                    characteristic.notifying = enabled;
                }
                out.log(
                    LogLevel::Info,
                    if enabled {
                        format!("Subscribed to {name}")
                    } else {
                        format!("Unsubscribed from {name}")
                    },
                );
            }
            Err(error) => {
                self.stats.errors += 1;
                if enabled && let Some(characteristic) = self.characteristics.get_mut(point) {
                    characteristic.notifying = false;
                }
                out.log(LogLevel::Warn, format!("{name}: {error}"));
            }
        }
    }

    fn notified(&mut self, notification: Notification, out: &Emitter) {
        let Some(point) = self.characteristics.iter().position(|characteristic| {
            characteristic
                .uuid
                .eq_ignore_ascii_case(&notification.characteristic)
        }) else {
            return;
        };
        self.stats.frames += 1;
        let name = self.characteristics[point].name.clone();
        self.traffic.push(frame(
            Direction::Rx,
            format!("NOTIFY {name}"),
            decode::hex(&notification.data),
            len(&notification.data),
        ));
        self.update(point, notification.data, out);
    }

    /// Decode `point` with `decoder` from now on: the points change type and
    /// unit, and every value is shown again. Returns the characteristic.
    fn decode_with(&mut self, point: u32, decoder: BleDecoder, out: &Emitter) -> Option<String> {
        let index = usize::try_from(point).ok()?;
        let characteristic = self.characteristics.get_mut(index)?;
        characteristic.decoder = decoder;
        characteristic.error = None;
        let uuid = characteristic.uuid.clone();
        out.log(
            LogLevel::Info,
            format!(
                "Decoding {} as {}",
                characteristic.name,
                decode::label(&uuid, decoder)
            ),
        );
        // New points reset the app's values: send every one again.
        out.points(self.points());
        out.ble_gatt(self.info());
        self.payloads = self.placeholders();
        for index in 0..self.characteristics.len() {
            if let Some(bytes) = self.characteristics[index].bytes.clone() {
                self.update(index, bytes, out);
            }
        }
        self.dirty = true;
        self.flush(out);
        Some(uuid)
    }

    /// The placeholder text of every characteristic that has no value yet.
    fn placeholders(&self) -> Vec<PointText> {
        self.characteristics
            .iter()
            .zip(0u32..)
            .filter(|(characteristic, _)| characteristic.bytes.is_none())
            .filter_map(|(characteristic, point)| {
                characteristic.placeholder().map(|text| PointText {
                    point,
                    text: text.into(),
                })
            })
            .collect()
    }

    /// The link is gone: the values are no longer current.
    fn stale(&mut self) {
        for (reading, characteristic) in self.readings.iter_mut().zip(&self.characteristics) {
            if characteristic.properties.read || characteristic.properties.pushes() {
                reading.quality = Quality::Bad;
            }
        }
        self.dirty = true;
    }

    /// Report what changed since the last report.
    fn flush(&mut self, out: &Emitter) {
        if self.dirty {
            self.dirty = false;
            if !self.payloads.is_empty() {
                out.payloads(std::mem::take(&mut self.payloads));
            }
            out.values(ValueFrame {
                at_ms: now_ms(),
                latency_us: self.stats.last_latency_us,
                readings: self.readings.clone(),
            });
        }
        out.traffic(std::mem::take(&mut self.traffic));
        if self.stats != self.reported {
            out.stats(self.stats);
            self.reported = self.stats;
        }
    }
}

fn good(value: f64) -> Reading {
    Reading {
        value,
        raw: [0; 4],
        raw_len: 0,
        quality: Quality::Good,
    }
}

fn len(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len()).unwrap_or(u32::MAX)
}

fn frame(direction: Direction, summary: String, detail: String, bytes: u32) -> TrafficFrame {
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary,
        detail,
        bytes,
    }
}

/// Report a failed or lost connection and wait before the next attempt.
/// Returns `false` when the app closed the session while waiting.
async fn retry(
    control: &mut mpsc::UnboundedReceiver<Control>,
    attempt: u32,
    reason: &str,
    out: &Emitter,
) -> bool {
    let exponent = attempt.saturating_sub(1).min(5);
    let wait = (Duration::from_millis(500) * 2u32.pow(exponent)).min(MAX_BACKOFF);
    out.state(SessionState::Reconnecting {
        attempt,
        retry_in_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        reason: reason.to_owned(),
    });
    tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = until_closed(control, out) => false,
    }
}

/// Answer commands while not connected, until the app closes the session.
async fn until_closed(control: &mut mpsc::UnboundedReceiver<Control>, out: &Emitter) {
    loop {
        match control.recv().await {
            None | Some(Control::Close) => return,
            Some(Control::Write { point, .. } | Control::Ble(BleRequest::Write { point, .. })) => {
                out.written(point, Err("Not connected".into()));
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;
    use vt_protocol::{BleAdapter, BleClientSpec, Event, EventKind, SessionId, SessionState};

    use super::run_with;
    use crate::Control;
    use crate::ble::simulated::{ENV_ID, ENV_NAME, SimCentral};
    use crate::emit::Emitter;

    /// Wait for the first event `accept` takes, failing after 10 s.
    async fn wait<T>(
        events: &async_channel::Receiver<Event>,
        mut accept: impl FnMut(&EventKind) -> Option<T>,
    ) -> T {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.expect("the session reports");
                if let Some(found) = accept(&event.kind) {
                    return found;
                }
            }
        })
        .await
        .expect("the event arrived in time")
    }

    fn state(kind: &EventKind) -> Option<SessionState> {
        match kind {
            EventKind::State(state) => Some(state.clone()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_lost_link_reconnects_and_keeps_the_points() {
        let (central, notifications) = SimCentral::new();
        let (control, control_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = async_channel::unbounded();
        let out = Emitter::new(SessionId(1), events_tx);
        let spec = BleClientSpec {
            adapter: BleAdapter::Simulated,
            peripheral: ENV_ID.into(),
            name: ENV_NAME.into(),
            poll_ms: 200,
            ..BleClientSpec::default()
        };
        let session = run_with(&central, notifications, spec, control_rx, &out);
        let driver = async {
            let points = wait(&events, |kind| match kind {
                EventKind::Points(points) => Some(points.len()),
                _ => None,
            })
            .await;
            assert!(points > 5, "{points} points");
            wait(&events, |kind| {
                matches!(state(kind), Some(SessionState::Connected)).then_some(())
            })
            .await;

            // The sensor walks out of range.
            central.drop_link(ENV_ID);
            let reason = wait(&events, |kind| match state(kind) {
                Some(SessionState::Reconnecting { reason, .. }) => Some(reason),
                _ => None,
            })
            .await;
            assert!(reason.contains("Lost the connection"), "{reason}");
            // Back in range: connected again, and the same points need no
            // new table.
            let mut repeated = false;
            wait(&events, |kind| match kind {
                EventKind::Points(_) => {
                    repeated = true;
                    None
                }
                EventKind::State(SessionState::Connected) => Some(()),
                _ => None,
            })
            .await;
            assert!(!repeated, "the layout did not change");

            control.send(Control::Close).expect("the session listens");
            wait(&events, |kind| match state(kind) {
                Some(SessionState::Closed { error }) => Some(error),
                _ => None,
            })
            .await
        };
        let ((), closed) = tokio::join!(session, driver);
        assert_eq!(closed, None);
    }

    #[tokio::test]
    async fn a_peripheral_that_refuses_connections_is_retried_until_closed() {
        let (central, notifications) = SimCentral::new();
        let (control, control_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = async_channel::unbounded();
        let out = Emitter::new(SessionId(2), events_tx);
        // The simulated beacon advertises with no name and takes no connections.
        let spec = BleClientSpec {
            adapter: BleAdapter::Simulated,
            peripheral: "6F1D2C3B-5A49-4E8F-9C1B-2D3E4F5A6B03".into(),
            ..BleClientSpec::default()
        };
        let session = run_with(&central, notifications, spec, control_rx, &out);
        let driver = async {
            let reason = wait(&events, |kind| match state(kind) {
                Some(SessionState::Reconnecting {
                    reason, attempt, ..
                }) => {
                    assert_eq!(attempt, 1);
                    Some(reason)
                }
                _ => None,
            })
            .await;
            assert!(reason.contains("accepts no connections"), "{reason}");
            // A write while not connected is refused, not lost.
            control
                .send(Control::Write {
                    point: 0,
                    value: 1.0,
                })
                .expect("the session listens");
            let refused = wait(&events, |kind| match kind {
                EventKind::Written { result, .. } => Some(result.clone()),
                _ => None,
            })
            .await;
            assert_eq!(refused, Err("Not connected".into()));
            control.send(Control::Close).expect("the session listens");
            wait(&events, |kind| match state(kind) {
                Some(SessionState::Closed { error }) => Some(error),
                _ => None,
            })
            .await
        };
        let ((), closed) = tokio::join!(session, driver);
        assert_eq!(closed, None);
    }

    #[tokio::test]
    async fn closing_while_connecting_stops_the_scan() {
        let (central, notifications) = SimCentral::new();
        let (control, control_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = async_channel::unbounded();
        let out = Emitter::new(SessionId(3), events_tx);
        // Nothing advertises this: the session scans for it.
        let spec = BleClientSpec {
            adapter: BleAdapter::Simulated,
            peripheral: "00000000-0000-4000-8000-000000000000".into(),
            ..BleClientSpec::default()
        };
        let session = run_with(&central, notifications, spec, control_rx, &out);
        let driver = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !central.scanning() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the session scans");
            control.send(Control::Close).expect("the session listens");
            wait(&events, |kind| match state(kind) {
                Some(SessionState::Closed { error }) => Some(error),
                _ => None,
            })
            .await
        };
        let ((), closed) = tokio::join!(session, driver);
        assert_eq!(closed, None);
        assert!(!central.scanning(), "Close stopped the scan");
    }
}
