//! The EtherNet/IP parts of the model: the editor's device picker
//! (ListIdentity), the inspector's explicit message, and the demo plant.

use std::net::{TcpListener, UdpSocket};

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use vt_app_core::forms::CipRequestForm;
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{
    Command as EngineCommand, ENIP_PORT, EnipAdapterSpec, EnipAddress, EnipDeviceInfo,
    EnipDiscoverSpec, SessionId, SessionSpec,
};
use vt_ui::text_input::{InputEvent, TextInput};

use super::AppModel;
use super::editor::{EnipDiscovery, EnipFields};

/// How long the device picker collects ListIdentity answers.
const DISCOVERY_WAIT_MS: u64 = 1500;
/// Name of a new client until the user names it (or picks a device).
pub const NEW_CLIENT_NAME: &str = "EtherNet/IP device";
pub const NEW_ADAPTER_NAME: &str = "EtherNet/IP simulator";
/// The plant of the demo and of new simulators.
const DEMO_PLANT: &str = "drive";

/// Common explicit messages, one click from the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipPreset {
    GetAttribute,
    SetAttribute,
    GetAll,
}

impl CipPreset {
    pub const ALL: [CipPreset; 3] = [Self::GetAttribute, Self::SetAttribute, Self::GetAll];

    pub fn label(self) -> &'static str {
        match self {
            Self::GetAttribute => "Get attribute",
            Self::SetAttribute => "Set attribute",
            Self::GetAll => "Get all",
        }
    }

    pub fn service(self) -> u8 {
        match self {
            Self::GetAttribute => 0x0E,
            Self::SetAttribute => 0x10,
            Self::GetAll => 0x01,
        }
    }
}

/// The inspector's explicit message form.
pub struct CipForm {
    pub service: Entity<TextInput>,
    pub class: Entity<TextInput>,
    pub instance: Entity<TextInput>,
    pub attribute: Entity<TextInput>,
    pub data: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

impl CipForm {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let field = |text: &'static str, placeholder: &'static str, cx: &mut Context<AppModel>| {
            cx.new(|cx| {
                TextInput::new(cx)
                    .text(text)
                    .placeholder(placeholder)
                    .mono()
            })
        };
        let service = field("0x0E", "0x0E", cx);
        let class = field("0x01", "0x04", cx);
        let instance = field("1", "100", cx);
        let attribute = field("7", "none", cx);
        let data = field("", "hex bytes, e.g. 01 00", cx);
        let subscriptions = [&service, &class, &instance, &attribute, &data]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |model, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Submitted => model.send_cip_request(window, cx),
                        InputEvent::Cancelled => window.focus(&model.focus),
                        InputEvent::Changed | InputEvent::Blurred => {}
                    }
                })
            })
            .collect();
        Self {
            service,
            class,
            instance,
            attribute,
            data,
            _subscriptions: subscriptions,
        }
    }

    fn form(&self, cx: &gpui::App) -> CipRequestForm {
        let text = |input: &Entity<TextInput>| input.read(cx).value().to_owned();
        CipRequestForm {
            service: text(&self.service),
            class: text(&self.class),
            instance: text(&self.instance),
            attribute: text(&self.attribute),
            data: text(&self.data),
        }
    }

    /// The preset the service field holds, if any.
    pub fn preset(&self, cx: &gpui::App) -> Option<CipPreset> {
        let service = self.form(cx).build().ok().map(|request| request.service);
        CipPreset::ALL
            .into_iter()
            .find(|preset| Some(preset.service()) == service)
    }
}

impl AppModel {
    /// Send the inspector's explicit message from the focused client.
    pub fn send_cip_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dispatch_cip_request(cx) {
            window.focus(&self.focus);
        }
    }

    /// Send the explicit message; `false` when it could not go (no running
    /// client, or the form says why in its reply).
    pub fn dispatch_cip_request(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(connection) = self.focused_connection() else {
            return false;
        };
        let built = self.cip.form(cx).build();
        let Some(live) = self.live.get_mut(&connection) else {
            return false;
        };
        let Some(session) = live.session else {
            return false;
        };
        let sent = match built {
            Ok(request) => {
                live.cip_reply = None;
                live.cip_waiting = true;
                self.engine
                    .send(EngineCommand::CipRequest { session, request });
                true
            }
            Err(error) => {
                live.cip_waiting = false;
                live.cip_reply = Some(Err(error));
                false
            }
        };
        cx.notify();
        sent
    }

    pub fn set_cip_preset(&mut self, preset: CipPreset, cx: &mut Context<Self>) {
        let service = format!("0x{:02X}", preset.service());
        self.cip
            .service
            .update(cx, |input, cx| input.set_text(service, cx));
        if preset == CipPreset::GetAll {
            self.cip
                .attribute
                .update(cx, |input, cx| input.set_text("", cx));
        }
        cx.notify();
    }

    /// Selecting an attribute point aims the explicit message at it.
    pub(super) fn aim_cip_at(
        &mut self,
        connection: ConnectionId,
        point: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(SessionSpec::EnipClient(spec)) = self
            .workspace
            .connection(connection)
            .map(|connection| &connection.spec)
        else {
            return;
        };
        let Some(EnipAddress::Attribute {
            class,
            instance,
            attribute,
            ..
        }) = usize::try_from(point)
            .ok()
            .and_then(|index| spec.points.get(index))
            .map(|point| point.address.clone())
        else {
            return;
        };
        let texts = [
            (
                &self.cip.service,
                format!("0x{:02X}", CipPreset::GetAttribute.service()),
            ),
            (&self.cip.class, format!("0x{class:02X}")),
            (&self.cip.instance, instance.to_string()),
            (&self.cip.attribute, attribute.to_string()),
        ];
        for (input, text) in texts {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    /// The device picker: ListIdentity broadcast on the Port field's port,
    /// and directed at the Host field and at every EtherNet/IP simulator saved
    /// on this computer.
    pub fn discover_enip(&mut self, cx: &mut Context<Self>) {
        let simulators: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::EnipAdapter(adapter) => Some(adapter.port),
                _ => None,
            })
            .collect();
        self.next_session += 1;
        let request = SessionId(self.next_session);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let fields = &mut editor.enip;
        let port = EnipFields::text(&fields.port, cx)
            .trim()
            .parse::<u16>()
            .unwrap_or(ENIP_PORT);
        let host = EnipFields::text(&fields.host, cx).trim().to_owned();
        let mut directed: Vec<String> = Vec::new();
        if !host.is_empty() && !host.contains(':') {
            directed.push(format!("{host}:{port}"));
        }
        for simulator in simulators {
            let local = format!("127.0.0.1:{simulator}");
            if !directed.contains(&local) {
                directed.push(local);
            }
        }
        fields.discovery = EnipDiscovery::Searching(request);
        editor.error = None;
        self.engine.send(EngineCommand::DiscoverEnip {
            session: request,
            spec: EnipDiscoverSpec {
                broadcast_port: Some(port),
                directed,
                wait_ms: DISCOVERY_WAIT_MS,
            },
        });
        cx.notify();
    }

    /// The answer to the picker's ListIdentity, if the editor still waits for it.
    pub(super) fn enip_devices_found(
        &mut self,
        request: SessionId,
        found: Result<Vec<EnipDeviceInfo>, String>,
    ) {
        if let Some(editor) = self.editor_mut()
            && matches!(editor.enip.discovery, EnipDiscovery::Searching(waiting) if waiting == request)
        {
            editor.enip.discovery = match found {
                Ok(devices) => EnipDiscovery::Found(devices),
                Err(error) => EnipDiscovery::Failed(error),
            };
        }
    }

    /// Fill the client fields with a discovered device.
    pub fn pick_enip_device(&mut self, device: &EnipDeviceInfo, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.error = None;
        let (host, port, name) = (
            editor.enip.host.clone(),
            editor.enip.port.clone(),
            editor.name.clone(),
        );
        host.update(cx, |input, cx| input.set_text(device.address.clone(), cx));
        port.update(cx, |input, cx| input.set_text(device.port.to_string(), cx));
        if name.read(cx).value().trim() == NEW_CLIENT_NAME && !device.product.is_empty() {
            let label = device.product.clone();
            name.update(cx, |input, cx| input.set_text(label, cx));
        }
        cx.notify();
    }

    /// After a new EtherNet/IP simulator is saved: a client of it, side by side.
    pub(super) fn add_enip_client(
        &mut self,
        name: &str,
        adapter: &EnipAdapterSpec,
        cx: &mut Context<Self>,
    ) {
        let Some(template) = templates::find_enip(&adapter.template) else {
            return;
        };
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::EnipClient(templates::enip_client_for(
                template,
                "127.0.0.1",
                adapter.port,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated motor drive and a client reading its tags, side by side.
    pub fn start_enip_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        let Some(template) = templates::find_enip(DEMO_PLANT) else {
            self.show_toast("The demo plant template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_enip_port() else {
            self.show_toast(
                "No free port between 44818 and 44917 for the adapter",
                true,
                cx,
            );
            return None;
        };
        let simulator = self.workspace.add_connection(
            format!("{} simulator", template.name),
            SessionSpec::EnipAdapter(EnipAdapterSpec {
                port,
                template: template.id.clone(),
            }),
        );
        let client = self.workspace.add_connection(
            template.name.clone(),
            SessionSpec::EnipClient(templates::enip_client_for(template, "127.0.0.1", port)),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A port from the EtherNet/IP port up that no saved simulator uses and
    /// nothing on this computer holds, for TCP and UDP.
    fn free_enip_port(&self) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::EnipAdapter(adapter) => Some(adapter.port),
                _ => None,
            })
            .collect();
        (ENIP_PORT..ENIP_PORT + 100).find(|port| {
            !taken.contains(port)
                && TcpListener::bind(("0.0.0.0", *port)).is_ok()
                && UdpSocket::bind(("0.0.0.0", *port)).is_ok()
        })
    }
}
