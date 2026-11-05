//! The licence inside the app: the gate in front of the engine, the unlock
//! and About sheets, and the StoreKit or licence-key calls behind them.
//!
//! Source builds are [`Licence::Unrestricted`] and never ask anything. Store
//! builds start from the cached licence (so a buyer is unlocked at once, also
//! offline), then ask StoreKit and follow its updates. Licence-key builds
//! start the trial on this Mac when the user asks, are unlocked at once by a
//! saved key and validate it once per launch in the background. Once the
//! licence stops
//! allowing work (trial not started or over), [`GatedEngine`] lets only
//! commands that stop sessions through: whatever path tries to connect,
//! simulate or write is refused and the unlock sheet opens.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, KeyBinding, SharedString, Subscription, Window,
    actions,
};
use vt_app_core::{Appearance, ConnectionId, store};
use vt_engine::EngineHandle;
use vt_protocol::Command as EngineCommand;
use vt_store::license::{ActivateError, Keys, LicenseConfig};
use vt_store::{
    Cache, Licence, Outcome, Prices, Snapshot, StoreError, TRIAL_PRODUCT, UNLOCK_PRODUCT, app_store,
};
use vt_ui::text_input::{InputEvent, TextInput};

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

/// What the App Store (or the licence server) is busy with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Trial,
    Unlock,
    Restore,
    /// Licence-key builds: activating the key in the field.
    Activate,
    /// Licence-key builds: moving the key here from another Mac.
    Move,
    /// Licence-key builds: removing the key from this Mac.
    RemoveKey,
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
    /// Licence-key builds: the server, the buy page and the price.
    pub config: LicenseConfig,
    /// Licence-key builds: the key client; `None` elsewhere.
    keys: Option<Arc<Keys>>,
    /// The unlock sheet's licence key field (licence-key builds).
    pub key_input: Entity<TextInput>,
    /// The server said the key is active on another Mac: offer to move it.
    pub seat_limit: bool,
    /// A button that asks again before it acts (move, remove key).
    pub confirm: Option<Busy>,
    _key_events: Subscription,
}

impl Licensing {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let cache_path = store::default_path()
            .ok()
            .map(|path| path.with_file_name("licence.json"));
        let mut cache = cache_path.as_deref().map(Cache::load).unwrap_or_default();
        let config = LicenseConfig::bundled();
        let keys = cache_path
            .as_deref()
            .and_then(|path| path.parent())
            .and_then(|dir| Keys::open(&config, dir.to_owned()))
            .map(Arc::new);
        let mut prices = Prices::default();
        if vt_store::license_build() {
            // The key in the Keychain is the unlock, not the editable cache
            // file; the server validates it after launch.
            cache.entitlements.unlocked = keys.as_ref().is_some_and(|keys| keys.saved());
            prices.unlock = config.price.clone();
        }
        let licence = if vt_store::sells() {
            cache.licence(unix_now())
        } else {
            Licence::Unrestricted
        };
        let key_input = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("Paste your licence key")
                .mono()
        });
        let key_events = cx.subscribe_in(
            &key_input,
            window,
            |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => model.activate_key(false, cx),
                InputEvent::Cancelled => model.close_licence_sheet(window, cx),
                InputEvent::Changed => {
                    model.licensing.seat_limit = false;
                    model.licensing.confirm = None;
                    cx.notify();
                }
                InputEvent::Blurred => {}
            },
        );
        Self {
            licence,
            prices,
            sheet: None,
            busy: None,
            notice: None,
            focus: cx.focus_handle(),
            focus_sheet: false,
            focus_main: false,
            gate: Rc::new(Gate::new(licence.can_operate())),
            cache,
            cache_path,
            preview: false,
            config,
            keys,
            key_input,
            seat_limit: false,
            confirm: None,
            _key_events: key_events,
        }
    }

    /// Whether this copy is sold (or previews a store copy).
    pub fn sells(&self) -> bool {
        vt_store::sells() || self.preview
    }

    /// Whether this copy unlocks with a licence key instead of the App Store.
    pub fn uses_keys(&self) -> bool {
        vt_store::license_build() && !self.preview
    }

    /// Whether the licence is really judged and saved (not a preview).
    fn live(&self) -> bool {
        vt_store::sells() && !self.preview
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
    /// re-check the trial every minute. Licence-key builds: validate the
    /// saved key once, and re-check the trial every minute.
    pub(super) fn start_licensing(&mut self, cx: &mut Context<Self>) {
        if !self.licensing.live() {
            return;
        }
        if self.licensing.uses_keys() {
            self.every_minute(cx);
            self.validate_key(cx);
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
        self.every_minute(cx);
        self.refresh_licence(cx);
    }

    /// Re-check the trial every minute, so running sessions stop when it
    /// ends.
    fn every_minute(&mut self, cx: &mut Context<Self>) {
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
        self.licensing.confirm = None;
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
            cx.defer_in(window, |model, window, cx| {
                // Licence-key builds type the key straight away once the
                // trial has started; Enter then activates it.
                let licensing = &model.licensing;
                if licensing.uses_keys()
                    && licensing.sheet == Some(Sheet::Unlock)
                    && matches!(licensing.licence, Licence::Trial { .. } | Licence::Expired)
                {
                    licensing.key_input.read(cx).focus(window);
                } else {
                    window.focus(&licensing.focus);
                }
            });
        }
        if std::mem::take(&mut licensing.focus_main) {
            cx.defer_in(window, |model, window, _| window.focus(&model.focus));
        }
    }

    /// Enter on a sheet: its main button.
    pub fn licence_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Licence-key builds: a typed key is activated first.
        if self.licensing.uses_keys()
            && self.licensing.sheet == Some(Sheet::Unlock)
            && matches!(
                self.licensing.licence,
                Licence::Trial { .. } | Licence::Expired
            )
            && !self.licensing.key_input.read(cx).value().trim().is_empty()
        {
            self.activate_key(false, cx);
            return;
        }
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
        if self.licensing.uses_keys() {
            match item {
                Busy::Trial => self.start_trial_here(cx),
                _ => self.open_buy_page(cx),
            }
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
        if self.licensing.busy.is_some() || self.licensing.uses_keys() {
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

    // ── Licence keys ────────────────────────────────────────────────────

    /// Licence-key builds: the user started the trial. It is kept in the
    /// cache and the Keychain marks, so it neither restarts nor stretches.
    fn start_trial_here(&mut self, cx: &mut Context<Self>) {
        let started = self.licensing.cache.start_trial(unix_now());
        self.recheck_licence(cx);
        let message = match self.licensing.licence {
            Licence::Trial { days_left, .. } if started => format!(
                "Your free trial has started: {} of every tool.",
                plural_days(days_left)
            ),
            Licence::Trial { days_left, .. } => {
                format!(
                    "Your free trial is running: {} left.",
                    plural_days(days_left)
                )
            }
            Licence::Unlocked => "Voltrus Tools is unlocked.".to_owned(),
            _ => "Your free trial on this Mac has already ended.".to_owned(),
        };
        if self.licensing.licence.can_operate() {
            self.licensing.sheet = None;
            self.licensing.focus_main = true;
            self.show_toast(message, false, cx);
        } else {
            self.licensing.notice = Some((message.into(), true));
        }
        cx.notify();
    }

    /// Open the console's page for the licence in the browser.
    pub fn open_buy_page(&mut self, cx: &mut Context<Self>) {
        cx.open_url(&self.licensing.config.buy_url);
        self.licensing.notice = Some((
            "The Voltrus console opened in your browser. Paste the licence key you receive \
             here and choose Activate."
                .into(),
            false,
        ));
        cx.notify();
    }

    /// Activate the key in the field; with `relocate`, move it here from the
    /// Mac that holds it (asks once more first).
    pub fn activate_key(&mut self, relocate: bool, cx: &mut Context<Self>) {
        let licensing = &mut self.licensing;
        let Some(keys) = licensing.keys.clone() else {
            return;
        };
        if licensing.busy.is_some() || licensing.licence == Licence::Unlocked {
            return;
        }
        let busy = if relocate { Busy::Move } else { Busy::Activate };
        if relocate && licensing.confirm != Some(Busy::Move) {
            licensing.confirm = Some(Busy::Move);
            cx.notify();
            return;
        }
        let key = licensing.key_input.read(cx).value().trim().to_owned();
        licensing.busy = Some(busy);
        licensing.confirm = None;
        licensing.notice = None;
        cx.notify();
        off_thread(
            cx,
            move || keys.activate(&key, relocate),
            |model, result: Result<(), ActivateError>, cx| {
                let licensing = &mut model.licensing;
                licensing.busy = None;
                match result {
                    Ok(()) => {
                        licensing.seat_limit = false;
                        licensing.cache.entitlements.unlocked = true;
                        licensing
                            .key_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        model.recheck_licence(cx);
                        model.licensing.sheet = None;
                        model.licensing.focus_main = true;
                        model.show_toast("Voltrus Tools is unlocked. Thank you!", false, cx);
                    }
                    Err(error) => {
                        licensing.seat_limit = error.seat_limit;
                        licensing.notice = Some((error.message.into(), true));
                    }
                }
                cx.notify();
            },
        );
    }

    /// Release this Mac's seat and forget the key (asks once more first).
    pub fn remove_key(&mut self, cx: &mut Context<Self>) {
        let licensing = &mut self.licensing;
        let Some(keys) = licensing.keys.clone() else {
            return;
        };
        if licensing.busy.is_some() {
            return;
        }
        if licensing.confirm != Some(Busy::RemoveKey) {
            licensing.confirm = Some(Busy::RemoveKey);
            cx.notify();
            return;
        }
        licensing.busy = Some(Busy::RemoveKey);
        licensing.confirm = None;
        licensing.notice = None;
        cx.notify();
        off_thread(
            cx,
            move || keys.remove(),
            |model, result: Result<(), String>, cx| {
                model.licensing.busy = None;
                match result {
                    Ok(()) => {
                        model.licensing.cache.entitlements.unlocked = false;
                        model.recheck_licence(cx);
                        model.licensing.notice = Some((
                            "The licence key was removed from this Mac. You can activate it \
                             on another one."
                                .into(),
                            false,
                        ));
                    }
                    Err(message) => model.licensing.notice = Some((message.into(), true)),
                }
                cx.notify();
            },
        );
    }

    /// Validate the saved key once per launch. Only the server's explicit
    /// "not valid" removes the unlock; offline it stays.
    fn validate_key(&mut self, cx: &mut Context<Self>) {
        let Some(keys) = self.licensing.keys.clone() else {
            return;
        };
        if !self.licensing.cache.entitlements.unlocked {
            return;
        }
        off_thread(
            cx,
            move || keys.status(),
            |model, valid, cx| {
                if valid {
                    return;
                }
                model.licensing.cache.entitlements.unlocked = false;
                model.recheck_licence(cx);
                model.show_toast(
                    "The licence server no longer accepts the licence key on this Mac, so it \
                     was removed. Activate it again, or write to support@voltrus.id.",
                    true,
                    cx,
                );
            },
        );
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

    /// A session of `protocol` in `role`, or `None` where the protocol has no
    /// such role (BLE has no simulator). No wildcard: a new protocol doesn't
    /// compile here until it has its own arm, so the gate test below covers
    /// it.
    fn spec_for(protocol: Protocol, role: Role) -> Option<SessionSpec> {
        Some(match (protocol, role) {
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
            (Protocol::Opcua, Role::Client) => SessionSpec::OpcuaClient(Default::default()),
            (Protocol::Opcua, Role::Simulator) => SessionSpec::OpcuaServer(Default::default()),
            (Protocol::Nats, Role::Client) => SessionSpec::NatsClient(Default::default()),
            (Protocol::Nats, Role::Simulator) => SessionSpec::NatsServer(Default::default()),
            (Protocol::Ble, Role::Client) => SessionSpec::BleClient(Default::default()),
            // The BLE demo's peripherals are simulated behind its adapter.
            (Protocol::Ble, Role::Simulator) => return None,
            (Protocol::Mtconnect, Role::Client) => SessionSpec::MtconnectClient(Default::default()),
            (Protocol::Mtconnect, Role::Simulator) => {
                SessionSpec::MtconnectAgent(Default::default())
            }
            (Protocol::Focas, Role::Client) => SessionSpec::FocasClient(Default::default()),
            (Protocol::Focas, Role::Simulator) => SessionSpec::FocasSimulator(Default::default()),
            (Protocol::SecsGem, Role::Client) => SessionSpec::SecsGemClient(Default::default()),
            (Protocol::SecsGem, Role::Simulator) => {
                SessionSpec::SecsGemSimulator(Default::default())
            }
        })
    }

    #[test]
    fn every_protocol_and_role_is_refused_once_the_trial_ends() {
        let expired = Gate::new(Licence::Expired.can_operate());
        let unlocked = Gate::new(Licence::Unlocked.can_operate());
        for &protocol in Protocol::ALL {
            for role in [Role::Client, Role::Simulator] {
                let Some(spec) = spec_for(protocol, role) else {
                    continue;
                };
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
            EngineCommand::ScanBle {
                session,
                spec: vt_protocol::BleScanSpec {
                    adapter: vt_protocol::BleAdapter::Simulated,
                    duration_ms: 1000,
                },
            },
            EngineCommand::Ble {
                session,
                request: vt_protocol::BleRequest::Read { point: 0 },
            },
            EngineCommand::Opcua {
                session,
                request: vt_protocol::OpcuaRequest::Browse {
                    node_id: "Objects".into(),
                },
            },
            EngineCommand::Opcua {
                session,
                request: vt_protocol::OpcuaRequest::Endpoints {
                    url: "opc.tcp://localhost:4840".into(),
                },
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
