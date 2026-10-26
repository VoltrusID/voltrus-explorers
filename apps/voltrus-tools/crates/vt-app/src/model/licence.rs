//! The App Store licence inside the app: the gate in front of the engine, the
//! unlock and About sheets, and the StoreKit calls behind them.
//!
//! Source builds are [`Licence::Unrestricted`] and never ask StoreKit. Store
//! builds start from the cached licence (so a buyer is unlocked at once, also
//! offline), then ask StoreKit and follow its updates. Once the licence stops
//! allowing work (trial not started or over), [`GatedEngine`] lets only
//! commands that stop sessions through: whatever path tries to connect,
//! simulate or write is refused and the unlock sheet opens.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{Context, FocusHandle, KeyBinding, SharedString, Window, actions};
use vt_app_core::{Appearance, ConnectionId, store};
use vt_engine::EngineHandle;
use vt_protocol::Command as EngineCommand;
use vt_store::{
    Cache, Licence, Outcome, Prices, Snapshot, StoreError, TRIAL_PRODUCT, UNLOCK_PRODUCT, app_store,
};

use super::AppModel;

actions!(
    licence,
    [
        /// The sheet's main button: start the trial, unlock, or close.
        LicenceConfirm,
        LicenceDismiss,
        ShowAbout,
        ShowUnlock,
    ]
);

/// Key context of the unlock and About sheets.
pub const LICENCE: &str = "Licence";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", LicenceConfirm, Some(LICENCE)),
        KeyBinding::new("escape", LicenceDismiss, Some(LICENCE)),
    ]
}

// ── The gate ─────────────────────────────────────────────────────────────

/// Whether sessions may run, shared by the model and its [`GatedEngine`].
#[derive(Debug, Default)]
pub struct Gate {
    open: Cell<bool>,
    refused: Cell<bool>,
}

impl Gate {
    pub fn new(open: bool) -> Self {
        Self {
            open: Cell::new(open),
            refused: Cell::new(false),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    fn set_open(&self, open: bool) {
        self.open.set(open);
    }

    /// Whether `command` may reach the engine. Stopping, and listing this
    /// Mac's serial ports for the editor (read-only mode still edits saved
    /// connections), are always allowed; anything else needs an open gate. A
    /// refusal is remembered until [`Gate::take_refused`], which opens the
    /// unlock sheet.
    pub fn admit(&self, command: &EngineCommand) -> bool {
        if self.open.get()
            || matches!(
                command,
                EngineCommand::Close { .. }
                    | EngineCommand::Shutdown
                    | EngineCommand::ListSerialPorts { .. }
            )
        {
            return true;
        }
        self.refused.set(true);
        false
    }

    pub fn take_refused(&self) -> bool {
        self.refused.replace(false)
    }
}

/// The engine as the model sees it: every command passes the licence gate.
pub struct GatedEngine {
    engine: EngineHandle,
    gate: Rc<Gate>,
}

impl GatedEngine {
    pub fn new(engine: EngineHandle, gate: Rc<Gate>) -> Self {
        Self { engine, gate }
    }

    /// Queue a command. Returns `false` when the licence refuses it or the
    /// engine has shut down.
    pub fn send(&self, command: EngineCommand) -> bool {
        self.gate.admit(&command) && self.engine.send(command)
    }
}

// ── State ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// Start the trial, unlock, restore.
    Unlock,
    /// Version, licence state and the software licence.
    About,
}

/// What the App Store is busy with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Trial,
    Unlock,
    Restore,
}

/// A fixed licence for screenshots and trying the sheets in a source build
/// (`--licence-preview <name>`). Store builds ignore the flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// The unlock sheet before the trial.
    NotStarted,
    /// The demo plant running with 9 trial days left in the status bar.
    Trial,
    /// The unlock sheet after the trial.
    Expired,
    /// The About sheet of an unlocked copy.
    Unlocked,
    /// The demo plant running when the trial ends: its sessions stop, and
    /// connecting again is refused with the unlock sheet.
    Gate,
}

impl Preview {
    pub const NAMES: &'static [&'static str] =
        &["not-started", "trial", "expired", "unlocked", "gate"];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "not-started" => Self::NotStarted,
            "trial" => Self::Trial,
            "expired" => Self::Expired,
            "unlocked" => Self::Unlocked,
            "gate" => Self::Gate,
            _ => return None,
        })
    }
}

pub struct Licensing {
    pub licence: Licence,
    pub prices: Prices,
    pub sheet: Option<Sheet>,
    pub busy: Option<Busy>,
    /// What just happened, under the sheet's buttons; `true` for a failure.
    pub notice: Option<(SharedString, bool)>,
    pub focus: FocusHandle,
    /// Focus moves when the window next draws (the gate trips without one).
    focus_sheet: bool,
    focus_main: bool,
    pub gate: Rc<Gate>,
    cache: Cache,
    /// `licence.json` beside the workspace; `None` if there's no data folder.
    cache_path: Option<PathBuf>,
    /// A preview licence: nothing is asked, bought or saved.
    preview: bool,
}

impl Licensing {
    pub fn new(focus: FocusHandle) -> Self {
        let cache_path = store::default_path()
            .ok()
            .map(|path| path.with_file_name("licence.json"));
        let mut cache = cache_path.as_deref().map(Cache::load).unwrap_or_default();
        let licence = if vt_store::store_build() {
            cache.licence(unix_now())
        } else {
            Licence::Unrestricted
        };
        Self {
            licence,
            prices: Prices::default(),
            sheet: None,
            busy: None,
            notice: None,
            focus,
            focus_sheet: false,
            focus_main: false,
            gate: Rc::new(Gate::new(licence.can_operate())),
            cache,
            cache_path,
            preview: false,
        }
    }

    /// Whether this copy is sold through the App Store (or previews one).
    pub fn sells(&self) -> bool {
        vt_store::store_build() || self.preview
    }

    /// Whether purchases really go to the App Store.
    fn live(&self) -> bool {
        vt_store::store_build() && !self.preview
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_secs().try_into().unwrap_or(i64::MAX)
        })
}

/// Run a blocking App Store call on its own thread and hand the answer to the
/// model.
fn off_thread<T: Send + 'static>(
    cx: &mut Context<AppModel>,
    job: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut AppModel, T, &mut Context<AppModel>) + 'static,
) {
    let (answer, answered) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = answer.send_blocking(job());
    });
    cx.spawn(async move |this, cx| {
        if let Ok(result) = answered.recv().await {
            let _ = this.update(cx, |model, cx| done(model, result, cx));
        }
    })
    .detach();
}

/// StoreKit's transaction updates, from its thread to the model.
static UPDATES: OnceLock<async_channel::Sender<()>> = OnceLock::new();

extern "C" fn store_updated() {
    if let Some(updates) = UPDATES.get() {
        let _ = updates.try_send(());
    }
}

/// "1 day", "9 days".
pub(crate) fn plural_days(days: i64) -> String {
    if days == 1 {
        "1 day".to_owned()
    } else {
        format!("{days} days")
    }
}

// ── Model ────────────────────────────────────────────────────────────────

impl AppModel {
    /// Store builds: ask StoreKit now, whenever a transaction arrives, and
    /// re-check the trial every minute.
    pub(super) fn start_licensing(&mut self, cx: &mut Context<Self>) {
        if !self.licensing.live() {
            return;
        }
        let (updates, updated) = async_channel::bounded(1);
        if UPDATES.set(updates).is_ok() {
            app_store::listen(store_updated);
        }
        cx.spawn(async move |this, cx| {
            while updated.recv().await.is_ok() {
                if this
                    .update(cx, |model, cx| model.refresh_licence(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                if this
                    .update(cx, |model, cx| model.recheck_licence(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        self.refresh_licence(cx);
    }

    /// Whether sessions may run now. If not, the unlock sheet opens and the
    /// caller should stop.
    pub fn licence_allows(&mut self, cx: &mut Context<Self>) -> bool {
        if self.licensing.gate.is_open() {
            return true;
        }
        self.open_licence_sheet(Sheet::Unlock, cx);
        false
    }

    pub fn open_licence_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        let licensing = &mut self.licensing;
        if licensing.sheet != Some(sheet) {
            licensing.notice = None;
        }
        licensing.sheet = Some(sheet);
        licensing.focus_sheet = true;
        cx.notify();
    }

    pub fn close_licence_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.licensing.sheet = None;
        self.licensing.notice = None;
        window.focus(&self.focus);
        cx.notify();
    }

    /// From the root view, which has the window: open the sheet after a
    /// refused command, and move focus to it (or back) so Enter and Esc
    /// reach the right place.
    pub fn sync_licence_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let licensing = &mut self.licensing;
        if licensing.gate.take_refused() && licensing.sheet.is_none() {
            licensing.sheet = Some(Sheet::Unlock);
            licensing.notice = None;
            licensing.focus_sheet = true;
        }
        if std::mem::take(&mut licensing.focus_sheet) && licensing.sheet.is_some() {
            cx.defer_in(window, |model, window, _| {
                window.focus(&model.licensing.focus);
            });
        }
        if std::mem::take(&mut licensing.focus_main) {
            cx.defer_in(window, |model, window, _| window.focus(&model.focus));
        }
    }

    /// Enter on a sheet: its main button.
    pub fn licence_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.licensing.sheet, self.licensing.licence) {
            (Some(Sheet::Unlock), Licence::NotStarted) => self.buy(Busy::Trial, cx),
            (Some(Sheet::Unlock), Licence::Trial { .. } | Licence::Expired) => {
                self.buy(Busy::Unlock, cx)
            }
            _ => self.close_licence_sheet(window, cx),
        }
    }

    /// Start the trial or buy the unlock.
    pub fn buy(&mut self, item: Busy, cx: &mut Context<Self>) {
        if self.licensing.busy.is_some() {
            return;
        }
        if !self.licensing.live() {
            self.licensing.notice = Some(("A preview: nothing is bought.".into(), false));
            cx.notify();
            return;
        }
        let product = if item == Busy::Trial {
            TRIAL_PRODUCT
        } else {
            UNLOCK_PRODUCT
        };
        self.licensing.busy = Some(item);
        self.licensing.notice = None;
        cx.notify();
        off_thread(
            cx,
            move || app_store::purchase(product),
            move |model, result, cx| {
                model.licensing.busy = None;
                match result {
                    Ok(Outcome::Purchased { original }) => {
                        model.licensing.cache.purchased(product, original);
                        model.recheck_licence(cx);
                        let message = match model.licensing.licence {
                            Licence::Unlocked => "Voltrus Tools is unlocked. Thank you!".to_owned(),
                            Licence::Trial { days_left, .. } => format!(
                                "Your free trial has started: {} of every tool.",
                                plural_days(days_left)
                            ),
                            // A trial started long ago on this Mac.
                            _ => "Your free trial on this Mac has already ended.".to_owned(),
                        };
                        if model.licensing.licence.can_operate() {
                            model.licensing.sheet = None;
                            model.licensing.focus_main = true;
                            model.show_toast(message, false, cx);
                        } else {
                            model.licensing.notice = Some((message.into(), true));
                        }
                    }
                    Ok(Outcome::Pending) => {
                        model.licensing.notice = Some((
                            "Waiting for approval. Voltrus Tools updates as soon as the purchase is approved."
                                .into(),
                            false,
                        ));
                    }
                    Ok(Outcome::Cancelled) | Err(StoreError::Cancelled) => {}
                    Err(StoreError::Failed(message)) => {
                        model.licensing.notice = Some((message.into(), true));
                    }
                }
                cx.notify();
            },
        );
    }

    pub fn restore_purchases(&mut self, cx: &mut Context<Self>) {
        if self.licensing.busy.is_some() {
            return;
        }
        if !self.licensing.live() {
            self.licensing.notice = Some(("A preview: nothing is restored.".into(), false));
            cx.notify();
            return;
        }
        self.licensing.busy = Some(Busy::Restore);
        self.licensing.notice = None;
        cx.notify();
        off_thread(cx, app_store::restore, |model, result, cx| {
            model.licensing.busy = None;
            match result {
                Ok(snapshot) => {
                    model.take_snapshot(snapshot, cx);
                    let message = match model.licensing.licence {
                        Licence::Unlocked => "Restored: Voltrus Tools is unlocked.".to_owned(),
                        Licence::Trial { days_left, .. } => {
                            format!("Restored your trial: {} left.", plural_days(days_left))
                        }
                        Licence::Expired => {
                            "Restored your trial, which has ended. No unlock was found for this Apple ID."
                                .to_owned()
                        }
                        _ => "No purchases were found for this Apple ID.".to_owned(),
                    };
                    model.licensing.notice = Some((message.into(), false));
                }
                Err(StoreError::Cancelled) => {}
                Err(StoreError::Failed(message)) => {
                    model.licensing.notice = Some((message.into(), true));
                }
            }
            cx.notify();
        });
    }

    /// Ask StoreKit for the current entitlements and prices. Offline or
    /// unavailable, the cached licence stays.
    fn refresh_licence(&mut self, cx: &mut Context<Self>) {
        off_thread(cx, app_store::snapshot, |model, result, cx| {
            if let Ok(snapshot) = result {
                model.take_snapshot(snapshot, cx);
            }
        });
    }

    fn take_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.licensing.cache.update(&snapshot);
        if snapshot.prices != Prices::default() {
            self.licensing.prices = snapshot.prices;
            cx.notify();
        }
        self.recheck_licence(cx);
    }

    /// Judge the cached entitlements at the current time, and save them.
    fn recheck_licence(&mut self, cx: &mut Context<Self>) {
        if !self.licensing.live() {
            return;
        }
        let licence = self.licensing.cache.licence(unix_now());
        if let Some(path) = &self.licensing.cache_path {
            // Unsaved, the licence is still right this session; the next
            // check tries again.
            let _ = self.licensing.cache.save(path);
        }
        self.apply_licence(licence, cx);
    }

    /// Adopt `licence`. When it stops allowing work, running sessions stop
    /// and the unlock sheet opens.
    fn apply_licence(&mut self, licence: Licence, cx: &mut Context<Self>) {
        if self.licensing.licence == licence {
            return;
        }
        let could = self.licensing.licence.can_operate();
        self.licensing.licence = licence;
        self.licensing.gate.set_open(licence.can_operate());
        if could && !licence.can_operate() {
            let running: Vec<ConnectionId> = self
                .live
                .iter()
                .filter(|(_, live)| live.session.is_some())
                .map(|(connection, _)| *connection)
                .collect();
            for connection in running {
                self.stop(connection);
            }
            self.open_licence_sheet(Sheet::Unlock, cx);
        }
        cx.notify();
    }

    /// Show a fixed licence (`--licence-preview`); see [`Preview`].
    pub fn preview_licence(
        &mut self,
        preview: Preview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const DAY: i64 = 86_400;
        // Like demo runs: the dark house theme, whatever the system says, so
        // screenshots are comparable.
        self.workspace.ui.appearance = Appearance::Dark;
        self.apply_theme(window, cx);
        let licensing = &mut self.licensing;
        licensing.preview = true;
        licensing.prices = Prices {
            trial: Some("Free".to_owned()),
            unlock: Some("$49.00".to_owned()),
        };
        let trial = Licence::Trial {
            ends: unix_now() + 9 * DAY - 3600,
            days_left: 9,
        };
        match preview {
            Preview::NotStarted => {
                self.apply_licence(Licence::NotStarted, cx);
                self.open_licence_sheet(Sheet::Unlock, cx);
            }
            Preview::Trial => {
                self.apply_licence(trial, cx);
                self.start_demo_plant(cx);
            }
            Preview::Expired => {
                self.apply_licence(Licence::Expired, cx);
                self.open_licence_sheet(Sheet::Unlock, cx);
            }
            Preview::Unlocked => {
                self.apply_licence(Licence::Unlocked, cx);
                self.open_licence_sheet(Sheet::About, cx);
            }
            Preview::Gate => {
                self.apply_licence(trial, cx);
                let started = self.start_demo_plant(cx);
                self.apply_licence(Licence::Expired, cx);
                if let Some((client, _)) = started {
                    self.connect(client, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use vt_protocol::{
        Dnp3Action, EnipDiscoverSpec, ModbusMasterSpec, Protocol, Role, SessionId, SessionSpec,
    };

    use super::*;

    fn open() -> EngineCommand {
        EngineCommand::Open {
            session: SessionId(1),
            spec: SessionSpec::ModbusMaster(ModbusMasterSpec::default()),
        }
    }

    #[test]
    fn an_open_gate_admits_everything() {
        let gate = Gate::new(true);
        assert!(gate.admit(&open()));
        assert!(!gate.take_refused());
    }

    #[test]
    fn an_expired_trial_refuses_connecting_and_asks_to_unlock() {
        let gate = Gate::new(Licence::Expired.can_operate());
        assert!(!gate.admit(&open()));
        // The root view takes the refusal and opens the unlock sheet, once.
        assert!(gate.take_refused());
        assert!(!gate.take_refused());
        // Stopping is never refused, and doesn't ask to unlock.
        assert!(gate.admit(&EngineCommand::Close {
            session: SessionId(1)
        }));
        assert!(gate.admit(&EngineCommand::Shutdown));
        // Neither is listing serial ports for the editor.
        assert!(gate.admit(&EngineCommand::ListSerialPorts {
            session: SessionId(2)
        }));
        assert!(!gate.take_refused());
    }

    #[test]
    fn an_unstarted_trial_refuses_writes_and_simulators() {
        let gate = Gate::new(Licence::NotStarted.can_operate());
        assert!(!gate.admit(&EngineCommand::Write {
            session: SessionId(1),
            point: 0,
            value: 1.0,
        }));
        assert!(gate.take_refused());
    }

    #[test]
    fn the_gated_engine_drops_refused_commands() {
        let (engine, _events) = vt_engine::start().unwrap();
        let gate = Rc::new(Gate::new(false));
        let gated = GatedEngine::new(engine, gate.clone());
        assert!(!gated.send(open()));
        assert!(gate.take_refused());
        assert!(gated.send(EngineCommand::Close {
            session: SessionId(1)
        }));
        // Unlocking opens the gate for the same engine.
        gate.set_open(Licence::Unlocked.can_operate());
        assert!(gated.send(open()));
        assert!(gated.send(EngineCommand::Shutdown));
    }

    /// A session of `protocol` in `role`. No wildcard: a new protocol doesn't
    /// compile here until it has its own arm, so the gate test below covers
    /// it.
    fn spec_for(protocol: Protocol, role: Role) -> SessionSpec {
        match (protocol, role) {
            (Protocol::Modbus, Role::Client) => SessionSpec::ModbusMaster(Default::default()),
            (Protocol::Modbus, Role::Simulator) => SessionSpec::ModbusSlave(Default::default()),
            (Protocol::Mqtt, Role::Client) => SessionSpec::MqttClient(Default::default()),
            (Protocol::Mqtt, Role::Simulator) => SessionSpec::MqttBroker(Default::default()),
            (Protocol::Bacnet, Role::Client) => SessionSpec::BacnetClient(Default::default()),
            (Protocol::Bacnet, Role::Simulator) => SessionSpec::BacnetDevice(Default::default()),
            (Protocol::Dnp3, Role::Client) => SessionSpec::Dnp3Master(Default::default()),
            (Protocol::Dnp3, Role::Simulator) => SessionSpec::Dnp3Outstation(Default::default()),
            (Protocol::Enip, Role::Client) => SessionSpec::EnipClient(Default::default()),
            (Protocol::Enip, Role::Simulator) => SessionSpec::EnipAdapter(Default::default()),
            (Protocol::Serial, Role::Client) => SessionSpec::SerialMonitor(Default::default()),
            (Protocol::Serial, Role::Simulator) => SessionSpec::SerialDevice(Default::default()),
            (Protocol::S7, Role::Client) => SessionSpec::S7Client(Default::default()),
            (Protocol::S7, Role::Simulator) => SessionSpec::S7Server(Default::default()),
            (Protocol::Nats, Role::Client) => SessionSpec::NatsClient(Default::default()),
            (Protocol::Nats, Role::Simulator) => SessionSpec::NatsServer(Default::default()),
        }
    }

    #[test]
    fn every_protocol_and_role_is_refused_once_the_trial_ends() {
        let expired = Gate::new(Licence::Expired.can_operate());
        let unlocked = Gate::new(Licence::Unlocked.can_operate());
        for &protocol in Protocol::ALL {
            for role in [Role::Client, Role::Simulator] {
                let spec = spec_for(protocol, role);
                assert_eq!((spec.protocol(), spec.role()), (protocol, role));
                let open = EngineCommand::Open {
                    session: SessionId(1),
                    spec,
                };
                assert!(
                    !expired.admit(&open),
                    "{protocol:?} {role:?} ran after the trial"
                );
                assert!(
                    expired.take_refused(),
                    "{protocol:?} {role:?} didn't ask to unlock"
                );
                assert!(
                    unlocked.admit(&open),
                    "{protocol:?} {role:?} refused when unlocked"
                );
            }
        }
    }

    #[test]
    fn protocol_commands_are_refused_once_the_trial_ends() {
        let gate = Gate::new(Licence::Expired.can_operate());
        let session = SessionId(1);
        for command in [
            EngineCommand::Dnp3 {
                session,
                action: Dnp3Action::IntegrityPoll,
            },
            EngineCommand::DiscoverEnip {
                session,
                spec: EnipDiscoverSpec {
                    broadcast_port: None,
                    directed: vec!["127.0.0.1:44818".into()],
                    wait_ms: 100,
                },
            },
            EngineCommand::SerialSend {
                session,
                bytes: b"AT\r\n".to_vec(),
            },
            EngineCommand::SerialLog {
                session,
                path: Some("/tmp/log.csv".into()),
            },
            EngineCommand::ReadS7Plc { session },
            EngineCommand::Relinquish { session, point: 0 },
            EngineCommand::Nats {
                session,
                request: vt_protocol::NatsRequest::Streams,
            },
        ] {
            assert!(!gate.admit(&command), "{command:?} passed after the trial");
            assert!(gate.take_refused());
        }
    }

    #[test]
    fn previews_parse_and_store_builds_start_locked() {
        for name in Preview::NAMES {
            assert!(Preview::parse(name).is_some(), "{name}");
        }
        assert_eq!(Preview::parse("free"), None);
        assert!(!Licence::NotStarted.can_operate());
    }
}
