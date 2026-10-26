//! The BLE parts of the model: the editor's adapter, scan and peripheral
//! fields; the inspector's characteristic actions (read now, notifications,
//! the decoder, writing bytes or text); the BLE demo.

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use vt_app_core::ConnectionId;
use vt_app_core::forms::BleClientForm;
use vt_engine::ble::{decode, demo_clients};
use vt_protocol::{
    BleAdapter, BleClientSpec, BleDecoder, BleDecoderRule, BleDeviceInfo, BleFormat, BleRequest,
    BleScan, BleScanSpec, Command as EngineCommand, SessionId, SessionSpec,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};

use super::AppModel;

/// Name of a new connection until the user names it (or picks a peripheral).
pub const NEW_CLIENT_NAME: &str = "BLE peripheral";
/// How long the editor's scan runs.
const SCAN_MS: u64 = 8000;

/// The editor's last scan.
#[derive(Default)]
pub struct Scan {
    /// The scan still running, by its request id.
    pub request: Option<SessionId>,
    /// Peripherals in the order they were first seen, each with its latest
    /// advertisement, so rows don't jump as signals change.
    pub devices: Vec<BleDeviceInfo>,
    pub error: Option<String>,
    /// A scan finished since the editor opened.
    pub done: bool,
}

/// The editor's BLE client fields.
pub struct BleFields {
    pub adapter: BleAdapter,
    pub peripheral: Entity<TextInput>,
    /// The peripheral picked from a scan and the name it advertised.
    picked: Option<(String, String)>,
    pub poll: Entity<TextInput>,
    pub notify: bool,
    decoders: Vec<BleDecoderRule>,
    pub scan: Scan,
}

fn input(
    text: String,
    placeholder: &'static str,
    kind: InputKind,
    cx: &mut Context<AppModel>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        TextInput::new(cx)
            .text(text)
            .placeholder(placeholder)
            .kind(kind)
            .mono()
    })
}

fn text(field: &Entity<TextInput>, cx: &App) -> String {
    field.read(cx).value().to_owned()
}

impl BleFields {
    /// Fields for `editing` when it is a BLE connection, defaults otherwise.
    pub fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let spec = match editing {
            Some(SessionSpec::BleClient(spec)) => spec.clone(),
            _ => BleClientSpec::default(),
        };
        let form = BleClientForm::from_spec("", &spec);
        Self {
            adapter: form.adapter,
            peripheral: input(
                form.peripheral,
                "Pick one above, or type its identifier",
                InputKind::Text,
                cx,
            ),
            picked: (!spec.peripheral.is_empty()).then_some((spec.peripheral, spec.name)),
            poll: input(form.poll_ms, "0 = read once", InputKind::Unsigned, cx),
            notify: form.notify,
            decoders: form.decoders,
            scan: Scan::default(),
        }
    }

    /// Every text field, so the editor can watch them.
    pub fn inputs(&self) -> [Entity<TextInput>; 2] {
        [self.peripheral.clone(), self.poll.clone()]
    }

    pub fn form(&self, name: String, cx: &App) -> BleClientForm {
        let peripheral = text(&self.peripheral, cx);
        // The advertised name belongs to the picked peripheral only.
        let advertised = self
            .picked
            .as_ref()
            .filter(|(id, _)| id == peripheral.trim())
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        BleClientForm {
            name,
            adapter: self.adapter,
            peripheral,
            advertised,
            poll_ms: text(&self.poll, cx),
            notify: self.notify,
            decoders: self.decoders.clone(),
        }
    }

    /// Whether the fields point at `device`.
    pub fn picked(&self, device: &BleDeviceInfo, cx: &App) -> bool {
        text(&self.peripheral, cx).trim() == device.id
    }
}

/// How a peripheral is called in lists: its name, else its address.
pub fn device_label(device: &BleDeviceInfo) -> &str {
    if device.name.is_empty() {
        &device.address
    } else {
        &device.name
    }
}

/// Fold a scan's latest report into the list: known peripherals keep their
/// row and take the new advertisement, new ones join at the end.
fn merge(devices: &mut Vec<BleDeviceInfo>, fresh: Vec<BleDeviceInfo>) {
    for device in fresh {
        match devices.iter_mut().find(|known| known.id == device.id) {
            Some(known) => *known = device,
            None => devices.push(device),
        }
    }
}

/// The inspector's BLE state: how to write bytes, the decoder fields, and
/// the adapter new connections start on.
pub struct BleState {
    /// The adapter the last BLE editor used; a new connection starts there.
    pub adapter: BleAdapter,
    /// Bytes (hex) or text to write to the selected characteristic.
    pub data: Entity<TextInput>,
    /// Write the field as UTF-8 text instead of hex bytes.
    pub text: bool,
    pub offset: Entity<TextInput>,
    pub scale: Entity<TextInput>,
    /// Why the decoder fields were refused.
    pub error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl BleState {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let data = cx.new(|cx| TextInput::new(cx).placeholder("01 FF 3A, or text").mono());
        let offset = input("0".into(), "0", InputKind::Unsigned, cx);
        let scale = input("1".into(), "1", InputKind::Number, cx);
        let on_data =
            cx.subscribe_in(
                &data,
                window,
                |model, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => model.write_ble_bytes(window, cx),
                    InputEvent::Cancelled => window.focus(&model.focus),
                    InputEvent::Changed | InputEvent::Blurred => {}
                },
            );
        let mut subscriptions = vec![on_data];
        for field in [&offset, &scale] {
            subscriptions.push(cx.subscribe_in(
                field,
                window,
                |model, _, event: &InputEvent, window, cx| match event {
                    // Enter applies the offset and scale to the current format.
                    InputEvent::Submitted => {
                        let format = model.selected_ble_decoder().map(|decoder| decoder.format);
                        if let Some(format) = format {
                            model.decode_selected_ble(format, cx);
                        }
                    }
                    InputEvent::Cancelled => window.focus(&model.focus),
                    InputEvent::Changed => {
                        model.ble.error = None;
                    }
                    InputEvent::Blurred => {}
                },
            ));
        }
        Self {
            adapter: BleAdapter::System,
            data,
            text: false,
            offset,
            scale,
            error: None,
            _subscriptions: subscriptions,
        }
    }
}

impl AppModel {
    // ── Editor ────────────────────────────────────────────────────────────

    /// Scan for peripherals on the editor's adapter.
    pub fn scan_ble(&mut self, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        self.next_session += 1;
        let request = SessionId(self.next_session);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.error = None;
        editor.ble.scan = Scan {
            request: Some(request),
            ..Scan::default()
        };
        let adapter = editor.ble.adapter;
        self.engine.send(EngineCommand::ScanBle {
            session: request,
            spec: BleScanSpec {
                adapter,
                duration_ms: SCAN_MS,
            },
        });
        cx.notify();
    }

    /// Use `adapter` in the editor (and for the next new connection), and
    /// scan it: what was picked on the other one doesn't exist here.
    pub fn set_ble_adapter(&mut self, adapter: BleAdapter, cx: &mut Context<Self>) {
        self.ble.adapter = adapter;
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if editor.ble.adapter == adapter {
            return;
        }
        editor.ble.adapter = adapter;
        editor.ble.picked = None;
        let peripheral = editor.ble.peripheral.clone();
        peripheral.update(cx, |input, cx| input.set_text("", cx));
        self.scan_ble(cx);
    }

    /// A scan's report, if the editor still waits for that scan.
    pub(super) fn ble_devices_found(&mut self, request: SessionId, found: Result<BleScan, String>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let scan = &mut editor.ble.scan;
        if scan.request != Some(request) {
            return;
        }
        match found {
            Ok(report) => {
                merge(&mut scan.devices, report.devices);
                if report.done {
                    scan.request = None;
                    scan.done = true;
                }
            }
            Err(error) => {
                scan.request = None;
                scan.done = true;
                scan.error = Some(error);
            }
        }
    }

    /// Fill the editor with a scanned peripheral.
    pub fn pick_ble_device(&mut self, device: &BleDeviceInfo, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.error = None;
        editor.ble.picked = Some((device.id.clone(), device.name.clone()));
        let peripheral = editor.ble.peripheral.clone();
        let name = editor.name.clone();
        peripheral.update(cx, |input, cx| input.set_text(device.id.clone(), cx));
        if name.read(cx).value().trim() == NEW_CLIENT_NAME {
            let label = device_label(device).to_owned();
            name.update(cx, |input, cx| input.set_text(label, cx));
        }
        cx.notify();
    }

    // ── Inspector ─────────────────────────────────────────────────────────

    /// The focused connection's session and selected point, when it is a
    /// running BLE client.
    fn selected_ble(&self) -> Option<(ConnectionId, SessionId, u32)> {
        let connection = self.focused_connection()?;
        if !matches!(
            self.workspace.connection(connection)?.spec,
            SessionSpec::BleClient(_)
        ) {
            return None;
        }
        let live = self.live.get(&connection)?;
        Some((connection, live.session?, live.selected?))
    }

    /// The decoder of the selected characteristic.
    fn selected_ble_decoder(&self) -> Option<BleDecoder> {
        let connection = self.focused_connection()?;
        let live = self.live.get(&connection)?;
        live.ble_characteristic(live.selected?)
            .map(|characteristic| characteristic.decoder)
    }

    /// Show the decoder of `point` in the inspector's fields.
    pub(super) fn sync_ble_inspector(
        &mut self,
        connection: ConnectionId,
        point: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(decoder) = self
            .live
            .get(&connection)
            .and_then(|live| live.ble_characteristic(point))
            .map(|characteristic| characteristic.decoder)
        else {
            return;
        };
        self.ble.error = None;
        self.ble.offset.update(cx, |input, cx| {
            input.set_text(decoder.offset.to_string(), cx)
        });
        self.ble.scale.update(cx, |input, cx| {
            input.set_text(decoder.scale.to_string(), cx)
        });
    }

    pub fn read_selected_ble(&mut self, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        if let Some((_, session, point)) = self.selected_ble() {
            self.engine.send(EngineCommand::Ble {
                session,
                request: BleRequest::Read { point },
            });
            cx.notify();
        }
    }

    pub fn toggle_selected_ble_notify(&mut self, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let Some((connection, session, point)) = self.selected_ble() else {
            return;
        };
        let Some(notifying) = self
            .live
            .get(&connection)
            .and_then(|live| live.ble_characteristic(point))
            .map(|characteristic| characteristic.notifying)
        else {
            return;
        };
        self.engine.send(EngineCommand::Ble {
            session,
            request: BleRequest::Notify {
                point,
                enabled: !notifying,
            },
        });
        cx.notify();
    }

    /// Decode the selected characteristic as `format` (with the inspector's
    /// offset and scale for numbers), and remember it in the connection.
    pub fn decode_selected_ble(&mut self, format: BleFormat, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let Some((connection, session, point)) = self.selected_ble() else {
            return;
        };
        let Some(uuid) = self
            .live
            .get(&connection)
            .and_then(|live| live.ble_characteristic(point))
            .map(|characteristic| characteristic.uuid.clone())
        else {
            return;
        };
        let decoder = if format.size().is_some() {
            let offset = self.ble.offset.read(cx).value().trim().to_owned();
            let scale = self.ble.scale.read(cx).value().trim().to_owned();
            let Ok(offset) = offset.parse::<u16>() else {
                self.ble.error = Some("Offset is a byte position from 0".into());
                cx.notify();
                return;
            };
            let scale = match scale.parse::<f64>() {
                Ok(scale) if scale.is_finite() && scale != 0.0 => scale,
                _ => {
                    self.ble.error = Some("Scale is a number other than 0, e.g. 0.01".into());
                    cx.notify();
                    return;
                }
            };
            BleDecoder {
                format,
                offset,
                scale,
            }
        } else {
            BleDecoder::of(format)
        };
        self.ble.error = None;
        self.engine.send(EngineCommand::Ble {
            session,
            request: BleRequest::Decode { point, decoder },
        });
        if let Some(saved) = self.workspace.connection(connection)
            && let SessionSpec::BleClient(spec) = &saved.spec
        {
            let mut spec = spec.clone();
            spec.set_decoder(&uuid, decoder);
            let name = saved.name.clone();
            self.workspace
                .update_connection(connection, name, SessionSpec::BleClient(spec));
            self.save(cx);
        }
        cx.notify();
    }

    pub fn set_ble_text_mode(&mut self, text: bool, cx: &mut Context<Self>) {
        self.ble.text = text;
        cx.notify();
    }

    /// Write the inspector's bytes (or text) to the selected characteristic.
    pub fn write_ble_bytes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let Some((connection, session, point)) = self.selected_ble() else {
            return;
        };
        let typed = self.ble.data.read(cx).value().to_owned();
        let data = if self.ble.text {
            Ok(typed.into_bytes())
        } else {
            decode::parse_hex(&typed)
        };
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        match data {
            Ok(data) if data.is_empty() => {
                live.write_result = Some((point, Err("Type the bytes to write first".into())));
            }
            Ok(data) => {
                live.write_result = None;
                self.engine.send(EngineCommand::Ble {
                    session,
                    request: BleRequest::Write { point, data },
                });
                window.focus(&self.focus);
            }
            Err(error) => live.write_result = Some((point, Err(error))),
        }
        cx.notify();
    }

    // ── Demo ──────────────────────────────────────────────────────────────

    /// A simulated heart rate monitor and environment sensor, side by side.
    pub fn start_ble_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let mut clients = demo_clients().into_iter();
        let (Some((monitor_name, monitor)), Some((sensor_name, sensor))) =
            (clients.next(), clients.next())
        else {
            self.show_toast("The simulated peripherals are missing", true, cx);
            return None;
        };
        let monitor = self
            .workspace
            .add_connection(monitor_name, SessionSpec::BleClient(monitor));
        let sensor = self
            .workspace
            .add_connection(sensor_name, SessionSpec::BleClient(sensor));
        self.show_side_by_side(monitor, sensor, cx);
        Some((monitor, sensor))
    }
}

#[cfg(test)]
mod tests {
    use vt_protocol::BleDeviceInfo;

    use super::merge;

    fn device(id: &str, rssi: i16) -> BleDeviceInfo {
        BleDeviceInfo {
            id: id.into(),
            name: String::new(),
            address: id.into(),
            rssi: Some(rssi),
            tx_power: None,
            services: Vec::new(),
            manufacturer: Vec::new(),
            service_data: Vec::new(),
            connected: false,
        }
    }

    #[test]
    fn scan_rows_keep_their_place_as_signals_change() {
        let mut list = Vec::new();
        merge(&mut list, vec![device("a", -50), device("b", -70)]);
        // b is now the stronger one and c appears: rows stay put, c joins last.
        merge(
            &mut list,
            vec![device("b", -40), device("c", -45), device("a", -80)],
        );
        let order: Vec<(&str, Option<i16>)> = list
            .iter()
            .map(|device| (device.id.as_str(), device.rssi))
            .collect();
        assert_eq!(
            order,
            [("a", Some(-80)), ("b", Some(-40)), ("c", Some(-45))]
        );
    }
}
