//! The serial parts of the model: each serial pane's send bar (history,
//! hex or text, macros), the terminal's view settings and logging, the port
//! picker shared with Modbus RTU, and the serial demo.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{
    Command as EngineCommand, LineEnding, SerialDeviceSpec, SerialFormat, SerialMacro,
    SerialPortInfo, SessionId, SessionSpec, encode_send, short_port,
};
use vt_ui::text_input::{InputEvent, TextInput};

use super::AppModel;
use super::editor::{EditorKind, SerialFields};

/// Name of a new monitor until the user names it (or picks a port).
pub const NEW_MONITOR_NAME: &str = "Serial port";
pub const NEW_SIMULATOR_NAME: &str = "Serial simulator";
/// How often an open port picker lists the ports again.
const PORT_REFRESH: Duration = Duration::from_secs(2);
/// Sends remembered per pane.
const HISTORY: usize = 100;
/// Macros reachable with ⌃1 … ⌃9.
pub const MACRO_KEYS: usize = 9;

/// A serial pane's send bar.
pub struct SerialConsole {
    pub input: Entity<TextInput>,
    /// Send what is typed as hex bytes rather than text.
    pub hex: bool,
    history: Vec<String>,
    /// Where ↑↓ are in the history; `None` while typing anew.
    cursor: Option<usize>,
    /// What was typed before ↑ started walking the history.
    draft: String,
    _subscription: Subscription,
}

impl SerialConsole {
    fn new(connection: ConnectionId, window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let input = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("Type and press Enter to send · ↑↓ history")
                .mono()
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => model.send_serial(connection, cx),
                InputEvent::Cancelled => window.focus(&model.focus),
                InputEvent::Changed => {
                    if let Some(console) = model.serial_consoles.get_mut(&connection) {
                        console.cursor = None;
                    }
                }
                InputEvent::Blurred => {}
            },
        );
        Self {
            input,
            hex: false,
            history: Vec::new(),
            cursor: None,
            draft: String::new(),
            _subscription: subscription,
        }
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }
}

/// Where ↑ (`step` -1) or ↓ (+1) lands in a history of `len` sends from
/// `cursor`; `None` is back at the draft.
pub fn history_step(cursor: Option<usize>, len: usize, step: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match (cursor, step < 0) {
        (None, true) => Some(len - 1),
        (None, false) => None,
        (Some(index), true) => Some(index.saturating_sub(1)),
        (Some(index), false) if index + 1 < len => Some(index + 1),
        (Some(_), false) => None,
    }
}

/// A connection's macros and the line ending its sends use.
fn sending(spec: &SessionSpec) -> Option<(&[SerialMacro], LineEnding)> {
    match spec {
        SessionSpec::SerialMonitor(monitor) => Some((&monitor.macros, monitor.line_ending)),
        // The virtual device answers with CR LF, as instruments do.
        SessionSpec::SerialDevice(_) => Some((&[], LineEnding::CrLf)),
        _ => None,
    }
}

/// A file name from a connection name: letters, digits, `-` and `_`.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "serial".to_owned()
    } else {
        stem.to_owned()
    }
}

impl AppModel {
    /// Give every open serial pane its send bar; forget closed connections'.
    pub fn sync_serial_consoles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted: Vec<ConnectionId> = self
            .workspace
            .panes
            .iter()
            .map(|pane| pane.connection)
            .filter(|connection| {
                self.workspace
                    .connection(*connection)
                    .is_some_and(|saved| sending(&saved.spec).is_some())
            })
            .collect();
        self.serial_consoles
            .retain(|connection, _| wanted.contains(connection));
        for connection in wanted {
            if !self.serial_consoles.contains_key(&connection) {
                let console = SerialConsole::new(connection, window, cx);
                self.serial_consoles.insert(connection, console);
            }
        }
    }

    /// Send what is typed in a serial pane's send bar.
    pub fn send_serial(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(console) = self.serial_consoles.get(&connection) else {
            return;
        };
        let text = console.input.read(cx).value().to_owned();
        let hex = console.hex;
        let Some(ending) = self
            .workspace
            .connection(connection)
            .and_then(|saved| sending(&saved.spec))
            .map(|(_, ending)| ending)
        else {
            return;
        };
        if self.send_bytes(connection, encode_send(&text, hex, ending), cx)
            && let Some(console) = self.serial_consoles.get_mut(&connection)
        {
            if !text.is_empty() && console.history.last() != Some(&text) {
                console.history.push(text);
                if console.history.len() > HISTORY {
                    console.history.remove(0);
                }
            }
            console.cursor = None;
            console.draft.clear();
            console.input.update(cx, |input, cx| input.set_text("", cx));
        }
    }

    /// Send a connection's `index`-th macro.
    pub fn run_serial_macro(
        &mut self,
        connection: ConnectionId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(bytes) = self
            .workspace
            .connection(connection)
            .and_then(|saved| sending(&saved.spec))
            .and_then(|(macros, ending)| macros.get(index).map(|item| item.bytes(ending)))
        else {
            return;
        };
        self.send_bytes(connection, bytes, cx);
    }

    /// Hand encoded bytes to the session; `false` (with the reason on the
    /// send bar) when they could not go.
    fn send_bytes(
        &mut self,
        connection: ConnectionId,
        bytes: Result<Vec<u8>, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(live) = self.live.get_mut(&connection) else {
            return false;
        };
        let sent = match (live.session, bytes) {
            (_, Err(error)) => Err(error),
            (None, Ok(_)) => Err("Connect to send".to_owned()),
            (Some(session), Ok(bytes)) => {
                self.engine
                    .send(EngineCommand::SerialSend { session, bytes });
                Ok(())
            }
        };
        let ok = sent.is_ok();
        live.terminal.sent = sent.err().map(Err);
        cx.notify();
        ok
    }

    /// ↑ (`step` -1) and ↓ (+1) through a send bar's history.
    pub fn serial_history(
        &mut self,
        connection: ConnectionId,
        step: isize,
        cx: &mut Context<Self>,
    ) {
        let Some(console) = self.serial_consoles.get_mut(&connection) else {
            return;
        };
        if console.cursor.is_none() && step < 0 {
            console.draft = console.input.read(cx).value().to_owned();
        }
        console.cursor = history_step(console.cursor, console.history.len(), step);
        let text = match console.cursor {
            Some(index) => console.history[index].clone(),
            None => console.draft.clone(),
        };
        console
            .input
            .update(cx, |input, cx| input.set_text(text, cx));
        cx.notify();
    }

    pub fn set_serial_send_hex(
        &mut self,
        connection: ConnectionId,
        hex: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(console) = self.serial_consoles.get_mut(&connection) {
            console.hex = hex;
            cx.notify();
        }
    }

    /// The line ending of a monitor's sends, saved with the connection.
    pub fn set_serial_line_ending(
        &mut self,
        connection: ConnectionId,
        ending: LineEnding,
        cx: &mut Context<Self>,
    ) {
        if let Some(saved) = self
            .workspace
            .connections
            .iter_mut()
            .find(|saved| saved.id == connection)
            && let SessionSpec::SerialMonitor(monitor) = &mut saved.spec
        {
            monitor.line_ending = ending;
            self.save(cx);
            cx.notify();
        }
    }

    pub fn focus_serial_send(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(console) = self.serial_consoles.get(&connection) {
            console.input.read(cx).focus(window);
        }
    }

    pub fn set_terminal_hex(
        &mut self,
        connection: ConnectionId,
        hex: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.terminal.hex = hex;
            cx.notify();
        }
    }

    pub fn toggle_terminal_timestamps(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.terminal.timestamps = !live.terminal.timestamps;
            cx.notify();
        }
    }

    pub fn toggle_terminal_pause(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.terminal.paused = !live.terminal.paused;
            cx.notify();
        }
    }

    pub fn clear_terminal(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get_mut(&connection) {
            live.terminal.clear();
            cx.notify();
        }
    }

    /// Start logging to a file the user picks in the save panel (the only
    /// way a sandboxed app may write there), or stop logging.
    pub fn toggle_serial_log(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(live) = self.live.get(&connection) else {
            return;
        };
        let Some(session) = live.session else {
            self.show_toast("Connect to log what crosses the port", true, cx);
            return;
        };
        if live.terminal.logging.is_some() {
            self.engine.send(EngineCommand::SerialLog {
                session,
                path: None,
            });
            return;
        }
        let name = self
            .workspace
            .connection(connection)
            .map_or_else(|| "serial".to_owned(), |saved| file_stem(&saved.name));
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let directory = home
            .as_ref()
            .map(|home| home.join("Documents"))
            .filter(|documents| documents.is_dir())
            .or(home)
            .unwrap_or_else(std::env::temp_dir);
        let answer = cx.prompt_for_new_path(&directory, Some(&format!("{name}.csv")));
        cx.spawn(async move |this, cx| {
            let picked = answer.await;
            let _ = this.update(cx, |model, cx| match picked {
                Ok(Ok(Some(path))) => {
                    // Only if the same run is still going.
                    if model.live.get(&connection).and_then(|live| live.session) == Some(session) {
                        model.engine.send(EngineCommand::SerialLog {
                            session,
                            path: Some(path.to_string_lossy().into_owned()),
                        });
                    }
                }
                Ok(Err(error)) => {
                    model.show_toast(format!("The save panel failed: {error}"), true, cx);
                }
                Ok(Ok(None)) | Err(_) => {}
            });
        })
        .detach();
    }

    // ── Port picker ───────────────────────────────────────────────────────

    /// Called as an editor opens: a port picker lists the ports now and
    /// every few seconds while it stays open; a new virtual device gets a
    /// link name no saved device uses.
    pub(super) fn prepare_serial_editor(&mut self, cx: &mut Context<Self>) {
        let alias = self.free_serial_alias();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        match editor.kind {
            EditorKind::SerialMonitor | EditorKind::Master => {
                let task = cx.spawn(async move |this, cx| {
                    loop {
                        if this
                            .update(cx, |model, cx| model.list_serial_ports(cx))
                            .is_err()
                        {
                            break;
                        }
                        cx.background_executor().timer(PORT_REFRESH).await;
                    }
                });
                editor.serial.refresh = Some(task);
                self.list_serial_ports(cx);
            }
            EditorKind::SerialDevice if editor.editing.is_none() => {
                let field = editor.serial.alias.clone();
                field.update(cx, |input, cx| input.set_text(alias, cx));
            }
            _ => {}
        }
    }

    /// Ask the engine for the ports, if an editor with a picker is open.
    pub fn list_serial_ports(&mut self, cx: &mut Context<Self>) {
        self.next_session += 1;
        let request = SessionId(self.next_session);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let wants = match editor.kind {
            EditorKind::SerialMonitor => true,
            EditorKind::Master => editor.transport == vt_app_core::forms::Transport::Rtu,
            _ => false,
        };
        if !wants || editor.serial.request.is_some() {
            return;
        }
        editor.serial.request = Some(request);
        self.engine
            .send(EngineCommand::ListSerialPorts { session: request });
        cx.notify();
    }

    /// The engine's port list, with this app's running virtual devices.
    pub(super) fn serial_ports_found(
        &mut self,
        request: SessionId,
        found: Result<Vec<SerialPortInfo>, String>,
    ) {
        let simulators: Vec<SerialPortInfo> = self
            .workspace
            .connections
            .iter()
            .filter_map(|saved| match &saved.spec {
                SessionSpec::SerialDevice(device)
                    if self
                        .live
                        .get(&saved.id)
                        .is_some_and(|live| live.state.is_live()) =>
                {
                    Some(SerialPortInfo {
                        path: device.path(),
                        kind: "Simulator".into(),
                        description: saved.name.clone(),
                    })
                }
                _ => None,
            })
            .collect();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if editor.serial.request != Some(request) {
            return;
        }
        editor.serial.request = None;
        editor.serial.ports = Some(found.map(|mut ports| {
            ports.extend(simulators);
            ports
        }));
    }

    /// Fill the open editor's port field (a serial monitor's, or a Modbus
    /// RTU master's) with a port from the picker.
    pub fn pick_serial_port(&mut self, port: &SerialPortInfo, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.error = None;
        let field = match editor.kind {
            EditorKind::Master => editor.serial_port.clone(),
            _ => editor.serial.port.clone(),
        };
        let path = port.path.clone();
        field.update(cx, |input, cx| input.set_text(path, cx));
        if editor.kind == EditorKind::SerialMonitor
            && editor.name.read(cx).value().trim() == NEW_MONITOR_NAME
        {
            let label = if port.description.is_empty() {
                short_port(&port.path)
                    .trim_start_matches("/dev/")
                    .to_owned()
            } else {
                port.description.clone()
            };
            let name = editor.name.clone();
            name.update(cx, |input, cx| input.set_text(label, cx));
        }
        cx.notify();
    }

    /// The serial editor's port settings, opened as a Modbus RTU master:
    /// the suite polls Modbus over serial there, not in the monitor.
    pub fn serial_to_modbus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let fields: &SerialFields = &editor.serial;
        let port = SerialFields::text(&fields.port, cx);
        let baud = SerialFields::text(&fields.baud, cx);
        let (data_bits, parity, stop_bits) = (fields.data_bits, fields.parity, fields.stop_bits);
        self.open_editor(EditorKind::Master, None, window, cx);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.transport = vt_app_core::forms::Transport::Rtu;
        // Modbus RTU frames are 7 or 8 data bits.
        editor.data_bits = if data_bits == 7 { 7 } else { 8 };
        editor.parity = parity;
        editor.stop_bits = stop_bits;
        let (serial_port, baud_field) = (editor.serial_port.clone(), editor.baud.clone());
        serial_port.update(cx, |input, cx| input.set_text(port, cx));
        baud_field.update(cx, |input, cx| input.set_text(baud, cx));
        self.list_serial_ports(cx);
        cx.notify();
    }

    /// A link name for a new virtual device that no saved device uses.
    fn free_serial_alias(&self) -> String {
        let taken: Vec<&str> = self
            .workspace
            .connections
            .iter()
            .filter_map(|saved| match &saved.spec {
                SessionSpec::SerialDevice(device) => Some(device.alias.as_str()),
                _ => None,
            })
            .collect();
        (1..)
            .map(|index| format!("voltrus-serial-{index}"))
            .find(|alias| !taken.contains(&alias.as_str()))
            .unwrap_or_else(|| "voltrus-serial".to_owned())
    }

    /// After a new virtual device is saved: a monitor of it, side by side.
    pub(super) fn add_serial_monitor(
        &mut self,
        name: &str,
        device: &SerialDeviceSpec,
        cx: &mut Context<Self>,
    ) {
        let monitor = self.workspace.add_connection(
            format!("{name} monitor"),
            SessionSpec::SerialMonitor(device.monitor()),
        );
        self.workspace
            .split(monitor, Axis::Horizontal, PaneView::Traffic);
        self.connect(monitor, cx);
    }

    /// An ABB water analyser printing on a virtual port, and a monitor
    /// reading it: its terminal beside the device's points.
    pub fn start_serial_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        let Some(profile) = templates::find_modbus("abb-analyzer") else {
            self.show_toast("The demo device template is missing", true, cx);
            return None;
        };
        let device = SerialDeviceSpec {
            template: profile.id.clone(),
            format: SerialFormat::KeyValue,
            interval_ms: 500,
            alias: self.free_serial_alias(),
        };
        let mut monitor = device.monitor();
        monitor.macros.extend([
            SerialMacro {
                name: "Hold 18.5 °C".into(),
                payload: "SET Temperature 18.5".into(),
                hex: false,
            },
            SerialMacro {
                name: "Release all".into(),
                payload: "RELEASE ALL".into(),
                hex: false,
            },
        ]);
        let simulator = self.workspace.add_connection(
            format!("{} {} simulator", profile.manufacturer, profile.model),
            SessionSpec::SerialDevice(device),
        );
        let client = self.workspace.add_connection(
            format!("{} {} on serial", profile.manufacturer, profile.model),
            SessionSpec::SerialMonitor(monitor),
        );
        self.show_side_by_side(client, simulator, cx);
        if let Some(pane) = self
            .workspace
            .panes
            .iter()
            .find(|pane| pane.connection == client)
            .map(|pane| pane.id)
        {
            self.workspace.set_view(pane, PaneView::Traffic);
        }
        Some((client, simulator))
    }
}

#[cfg(test)]
mod tests {
    use super::{file_stem, history_step};

    #[test]
    fn history_walks_back_and_returns_to_the_draft() {
        assert_eq!(history_step(None, 0, -1), None, "nothing to walk");
        assert_eq!(history_step(None, 3, -1), Some(2), "↑ starts at the newest");
        assert_eq!(history_step(Some(2), 3, -1), Some(1));
        assert_eq!(history_step(Some(0), 3, -1), Some(0), "stops at the oldest");
        assert_eq!(history_step(Some(1), 3, 1), Some(2));
        assert_eq!(
            history_step(Some(2), 3, 1),
            None,
            "past the newest: the draft"
        );
        assert_eq!(history_step(None, 3, 1), None);
    }

    #[test]
    fn log_files_are_named_after_the_connection() {
        assert_eq!(file_stem("ABB AZL402 on serial"), "ABB-AZL402-on-serial");
        assert_eq!(file_stem("/dev/cu.usb"), "dev-cu-usb");
        assert_eq!(file_stem("…"), "serial");
    }
}
