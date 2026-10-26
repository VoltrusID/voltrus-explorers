//! The EtherNet/IP client and adapter simulator fields of the connection
//! editor, with the device picker's last ListIdentity.

use gpui::{App, Context, Entity, Window};
use vt_app_core::forms::{EnipAdapterForm, EnipClientForm, EnipPointForm};
use vt_protocol::{EnipAdapterSpec, EnipClientSpec, EnipDeviceInfo, SessionId, SessionSpec};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, Editor, input};

/// The device picker's last ListIdentity.
pub enum EnipDiscovery {
    Idle,
    Searching(SessionId),
    Found(Vec<EnipDeviceInfo>),
    Failed(String),
}

pub struct EnipPointRow {
    pub name: Entity<TextInput>,
    pub address: Entity<TextInput>,
    unit: String,
}

impl EnipPointRow {
    fn new(form: EnipPointForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            name: input(form.name, "Name", InputKind::Text, false, cx),
            address: input(
                form.address,
                "Program:Main.Speed or @0x04/100/3+4:REAL",
                InputKind::Text,
                true,
                cx,
            ),
            unit: form.unit,
        }
    }
}

pub struct EnipFields {
    // Client
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub route: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub points: Vec<EnipPointRow>,
    pub discovery: EnipDiscovery,
    // Adapter simulator
    pub adapter_port: Entity<TextInput>,
    pub template: String,
}

impl EnipFields {
    /// Fields for `editing` when it is an EtherNet/IP connection, defaults
    /// otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::EnipClient(spec)) => spec.clone(),
            _ => EnipClientSpec::default(),
        };
        let adapter = match editing {
            Some(SessionSpec::EnipAdapter(spec)) => spec.clone(),
            _ => EnipAdapterSpec::default(),
        };
        let client = EnipClientForm::from_spec("", &client);
        let adapter = EnipAdapterForm::from_spec("", &adapter);
        Self {
            host: input(client.host, "192.168.1.20", InputKind::Text, true, cx),
            port: input(client.port, "44818", InputKind::Unsigned, true, cx),
            route: input(
                client.route_path,
                "none, or 1,0 for slot 0",
                InputKind::Text,
                true,
                cx,
            ),
            poll: input(client.poll_ms, "1000", InputKind::Unsigned, true, cx),
            points: client
                .points
                .into_iter()
                .map(|form| EnipPointRow::new(form, cx))
                .collect(),
            discovery: EnipDiscovery::Idle,
            adapter_port: input(adapter.port, "44818", InputKind::Unsigned, true, cx),
            template: adapter.template,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.host.clone(),
            self.port.clone(),
            self.route.clone(),
            self.poll.clone(),
            self.adapter_port.clone(),
        ];
        for row in &self.points {
            fields.push(row.name.clone());
            fields.push(row.address.clone());
        }
        fields
    }

    pub(crate) fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> EnipClientForm {
        EnipClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            route_path: Self::text(&self.route, cx),
            poll_ms: Self::text(&self.poll, cx),
            points: self
                .points
                .iter()
                .map(|row| EnipPointForm {
                    name: Self::text(&row.name, cx),
                    address: Self::text(&row.address, cx),
                    unit: row.unit.clone(),
                })
                .collect(),
        }
    }

    pub fn adapter_form(&self, name: String, cx: &App) -> EnipAdapterForm {
        EnipAdapterForm {
            name,
            port: Self::text(&self.adapter_port, cx),
            template: self.template.clone(),
        }
    }

    /// Whether the client fields point at `device`.
    pub fn picked(&self, device: &EnipDeviceInfo, cx: &App) -> bool {
        Self::text(&self.host, cx).trim() == device.address
            && Self::text(&self.port, cx).trim() == device.port.to_string()
    }
}

impl Editor {
    /// A blank point row, focused for typing.
    pub fn add_enip_point(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let row = EnipPointRow::new(EnipPointForm::blank(), cx);
        self.watch(&row.name, window, cx);
        self.watch(&row.address, window, cx);
        row.address.update(cx, |address, _| address.focus(window));
        self.enip.points.push(row);
    }
}
