//! The BACnet parts of the model: the editor's client fields with the device
//! picker (Who-Is) and simulator fields, relinquishing a command, and the
//! BACnet demo plant.

use std::net::UdpSocket;

use gpui::{App, AppContext as _, Context, Entity};
use vt_app_core::forms::{BacnetClientForm, BacnetDeviceForm};
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_engine::templates;
use vt_protocol::{
    BACNET_PORT, BacnetClientSpec, BacnetDeviceInfo, BacnetDeviceSpec, BacnetDiscoverSpec,
    Command as EngineCommand, SessionId, SessionSpec,
};
use vt_ui::text_input::{InputKind, TextInput};

use super::AppModel;

impl AppModel {
    /// Give up the command the focused BACnet client wrote to its selected
    /// point (WriteProperty NULL at the session's priority).
    pub fn relinquish_selected(&mut self, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let (Some(session), Some(point)) = (live.session, live.selected) else {
            return;
        };
        live.write_result = None;
        self.engine
            .send(EngineCommand::Relinquish { session, point });
        cx.notify();
    }
}

/// How long the device picker collects I-Am answers.
const DISCOVERY_WAIT_MS: u64 = 1500;
/// Device instance of simulators made by the demo and new-simulator forms.
const DEMO_INSTANCE: u32 = 1001;
/// Name of a new client until the user names it (or picks a device).
pub const NEW_CLIENT_NAME: &str = "BACnet device";
pub const NEW_DEVICE_NAME: &str = "BACnet simulator";

/// The device picker's last Who-Is.
pub enum Discovery {
    Idle,
    Searching(SessionId),
    Found(Vec<BacnetDeviceInfo>),
    Failed(String),
}

pub struct BacnetFields {
    // Client
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub instance: Entity<TextInput>,
    pub bind: Entity<TextInput>,
    pub local_port: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub priority: Entity<TextInput>,
    pub cov: bool,
    pub discovery: Discovery,
    // Device simulator
    pub device_port: Entity<TextInput>,
    pub device_instance: Entity<TextInput>,
    pub template: String,
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

impl BacnetFields {
    /// Fields for `editing` when it is a BACnet connection, defaults otherwise.
    pub fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::BacnetClient(spec)) => spec.clone(),
            _ => BacnetClientSpec::default(),
        };
        let device = match editing {
            Some(SessionSpec::BacnetDevice(spec)) => spec.clone(),
            _ => BacnetDeviceSpec {
                device_instance: DEMO_INSTANCE,
                ..BacnetDeviceSpec::default()
            },
        };
        let client = BacnetClientForm::from_spec("", &client);
        let device = BacnetDeviceForm::from_spec("", &device);
        let unsigned = InputKind::Unsigned;
        Self {
            host: input(client.host, "192.168.1.40", InputKind::Text, cx),
            port: input(client.port, "47808", unsigned, cx),
            instance: input(client.device_instance, "Who-Is on connect", unsigned, cx),
            bind: input(client.bind, "0.0.0.0", InputKind::Text, cx),
            local_port: input(client.local_port, "0 = any free port", unsigned, cx),
            poll: input(client.poll_ms, "1000", unsigned, cx),
            priority: input(client.priority, "8", unsigned, cx),
            cov: client.cov,
            discovery: Discovery::Idle,
            device_port: input(device.port, "47808", unsigned, cx),
            device_instance: input(device.device_instance, "1001", unsigned, cx),
            template: device.template,
        }
    }

    /// Every text field, so the editor can watch them.
    pub fn inputs(&self) -> [Entity<TextInput>; 9] {
        [
            self.host.clone(),
            self.port.clone(),
            self.instance.clone(),
            self.bind.clone(),
            self.local_port.clone(),
            self.poll.clone(),
            self.priority.clone(),
            self.device_port.clone(),
            self.device_instance.clone(),
        ]
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> BacnetClientForm {
        BacnetClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            device_instance: Self::text(&self.instance, cx),
            bind: Self::text(&self.bind, cx),
            local_port: Self::text(&self.local_port, cx),
            poll_ms: Self::text(&self.poll, cx),
            cov: self.cov,
            priority: Self::text(&self.priority, cx),
        }
    }

    pub fn device_form(&self, name: String, cx: &App) -> BacnetDeviceForm {
        BacnetDeviceForm {
            name,
            port: Self::text(&self.device_port, cx),
            device_instance: Self::text(&self.device_instance, cx),
            template: self.template.clone(),
        }
    }

    /// Whether the client fields point at `device`.
    pub fn picked(&self, device: &BacnetDeviceInfo, cx: &App) -> bool {
        Self::text(&self.host, cx).trim() == device.address
            && Self::text(&self.port, cx).trim() == device.port.to_string()
            && Self::text(&self.instance, cx).trim() == device.instance.to_string()
    }
}

impl AppModel {
    /// The device picker: send a Who-Is from the client's local address,
    /// broadcast on the BACnet port in the Port field and directed at the
    /// Host field and at every BACnet simulator saved on this computer.
    pub fn discover_bacnet(&mut self, cx: &mut Context<Self>) {
        let simulators: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::BacnetDevice(device) => Some(device.port),
                _ => None,
            })
            .collect();
        self.next_session += 1;
        let request = SessionId(self.next_session);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let fields = &mut editor.bacnet;
        let port = BacnetFields::text(&fields.port, cx)
            .trim()
            .parse::<u16>()
            .unwrap_or(BACNET_PORT);
        let host = BacnetFields::text(&fields.host, cx).trim().to_owned();
        let mut directed: Vec<String> = Vec::new();
        if !host.is_empty() {
            directed.push(if host.contains(':') && !host.starts_with('[') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            });
        }
        for simulator in simulators {
            let local = format!("127.0.0.1:{simulator}");
            if !directed.contains(&local) {
                directed.push(local);
            }
        }
        let bind = match BacnetFields::text(&fields.bind, cx).trim() {
            "" => "0.0.0.0".to_owned(),
            bind => bind.to_owned(),
        };
        fields.discovery = Discovery::Searching(request);
        editor.error = None;
        self.engine.send(EngineCommand::DiscoverBacnet {
            session: request,
            spec: BacnetDiscoverSpec {
                bind,
                broadcast_port: Some(port),
                directed,
                wait_ms: DISCOVERY_WAIT_MS,
            },
        });
        cx.notify();
    }

    /// The answer to the picker's Who-Is, if the editor still waits for it.
    pub(super) fn bacnet_devices_found(
        &mut self,
        request: SessionId,
        found: Result<Vec<BacnetDeviceInfo>, String>,
    ) {
        if let Some(editor) = self.editor_mut()
            && matches!(editor.bacnet.discovery, Discovery::Searching(waiting) if waiting == request)
        {
            editor.bacnet.discovery = match found {
                Ok(devices) => Discovery::Found(devices),
                Err(error) => Discovery::Failed(error),
            };
        }
    }

    /// Fill the client fields with a discovered device.
    pub fn pick_bacnet_device(&mut self, device: &BacnetDeviceInfo, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.error = None;
        let fields = &editor.bacnet;
        let (host, port, instance) = (
            fields.host.clone(),
            fields.port.clone(),
            fields.instance.clone(),
        );
        host.update(cx, |input, cx| input.set_text(device.address.clone(), cx));
        port.update(cx, |input, cx| input.set_text(device.port.to_string(), cx));
        instance.update(cx, |input, cx| {
            input.set_text(device.instance.to_string(), cx)
        });
        if let Some(editor) = self.editor_mut()
            && editor.name.read(cx).value().trim() == NEW_CLIENT_NAME
        {
            let name = editor.name.clone();
            let label = format!("Device {}", device.instance);
            name.update(cx, |input, cx| input.set_text(label, cx));
        }
        cx.notify();
    }

    /// After a new BACnet simulator is saved: a client of it, side by side.
    pub(super) fn add_bacnet_client(
        &mut self,
        name: &str,
        device: &BacnetDeviceSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::BacnetClient(templates::bacnet_client_for(
                "127.0.0.1",
                device.port,
                device.device_instance,
            )),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated air handler and a client subscribed to it, side by side.
    pub fn start_bacnet_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        let Some(template) = templates::find_bacnet("ahu") else {
            self.show_toast("The demo plant template is missing", true, cx);
            return None;
        };
        let Some(port) = self.free_bacnet_port() else {
            self.show_toast(
                "No free UDP port between 47808 and 47899 for the simulator",
                true,
                cx,
            );
            return None;
        };
        let device = BacnetDeviceSpec {
            port,
            device_instance: DEMO_INSTANCE,
            template: template.id.clone(),
        };
        let simulator = self.workspace.add_connection(
            format!("{} simulator", template.name),
            SessionSpec::BacnetDevice(device),
        );
        let client = self.workspace.add_connection(
            template.name.clone(),
            SessionSpec::BacnetClient(templates::bacnet_client_for(
                "127.0.0.1",
                port,
                DEMO_INSTANCE,
            )),
        );
        self.show_side_by_side(client, simulator, cx);
        Some((client, simulator))
    }

    /// A UDP port from the BACnet port up that no saved simulator uses and
    /// nothing on this computer holds.
    fn free_bacnet_port(&self) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::BacnetDevice(device) => Some(device.port),
                _ => None,
            })
            .collect();
        (BACNET_PORT..BACNET_PORT + 92)
            .find(|port| !taken.contains(port) && UdpSocket::bind(("0.0.0.0", *port)).is_ok())
    }
}
