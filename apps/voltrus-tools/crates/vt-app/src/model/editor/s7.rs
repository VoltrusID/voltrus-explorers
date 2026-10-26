//! The Siemens S7 client and CPU simulator fields of the connection editor.

use gpui::{App, Context, Entity, Window};
use vt_app_core::forms::{S7ClientForm, S7ServerForm, S7TagForm, next_s7_tag};
use vt_protocol::{S7ClientSpec, S7ServerSpec, S7Type, SessionSpec};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};

use super::{AppModel, Editor, input};

/// Name of a new client or simulator until the user names it.
pub const NEW_S7_CLIENT_NAME: &str = "S7 PLC";
pub const NEW_S7_SERVER_NAME: &str = "S7 simulator";

/// Where the CPU of each controller family sits, for ISO-on-TCP.
pub const CPU_PRESETS: [(&str, u8, u8); 3] =
    [("S7-1200 / 1500", 0, 1), ("S7-300", 0, 2), ("S7-400", 0, 3)];

pub struct S7TagRow {
    pub name: Entity<TextInput>,
    pub address: Entity<TextInput>,
    /// The type chosen; shown (and saved) only while the address can hold it.
    pub data_type: S7Type,
    unit: String,
}

impl S7TagRow {
    fn new(form: S7TagForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            name: input(form.name, "Tag name", InputKind::Text, false, cx),
            address: input(form.address, "DB1.DBD0", InputKind::Text, true, cx),
            data_type: form.data_type,
            unit: form.unit,
        }
    }

    /// The row as the form validates it, its type fitted to the address.
    pub fn form(&self, cx: &App) -> S7TagForm {
        let mut form = S7TagForm {
            name: self.name.read(cx).value().to_owned(),
            address: self.address.read(cx).value().to_owned(),
            data_type: self.data_type,
            unit: self.unit.clone(),
        };
        form.fit_type();
        form
    }
}

pub struct S7Fields {
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub rack: Entity<TextInput>,
    pub slot: Entity<TextInput>,
    pub pdu: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub tags: Vec<S7TagRow>,
    pub server_bind: Entity<TextInput>,
    pub server_port: Entity<TextInput>,
    pub server_pdu: Entity<TextInput>,
    pub template: String,
}

impl S7Fields {
    /// Fields for `editing` when it is an S7 connection, defaults otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::S7Client(spec)) => spec.clone(),
            _ => S7ClientSpec::default(),
        };
        let server = match editing {
            Some(SessionSpec::S7Server(spec)) => spec.clone(),
            _ => S7ServerSpec::default(),
        };
        let client = S7ClientForm::from_spec("", &client);
        let server = S7ServerForm::from_spec("", &server);
        let unsigned = InputKind::Unsigned;
        Self {
            host: input(client.host, "192.168.0.1", InputKind::Text, true, cx),
            port: input(client.port, "102", unsigned, true, cx),
            rack: input(client.rack, "0", unsigned, true, cx),
            slot: input(client.slot, "1", unsigned, true, cx),
            pdu: input(client.pdu_size, "480", unsigned, true, cx),
            poll: input(client.poll_ms, "1000", unsigned, true, cx),
            timeout: input(client.timeout_ms, "3000", unsigned, true, cx),
            tags: client
                .tags
                .into_iter()
                .map(|form| S7TagRow::new(form, cx))
                .collect(),
            server_bind: input(server.bind, "127.0.0.1", InputKind::Text, true, cx),
            server_port: input(server.port, "102", unsigned, true, cx),
            server_pdu: input(server.pdu_size, "480", unsigned, true, cx),
            template: server.template,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.host.clone(),
            self.port.clone(),
            self.rack.clone(),
            self.slot.clone(),
            self.pdu.clone(),
            self.poll.clone(),
            self.timeout.clone(),
            self.server_bind.clone(),
            self.server_port.clone(),
            self.server_pdu.clone(),
        ];
        for row in &self.tags {
            fields.push(row.name.clone());
            fields.push(row.address.clone());
        }
        fields
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> S7ClientForm {
        S7ClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            rack: Self::text(&self.rack, cx),
            slot: Self::text(&self.slot, cx),
            pdu_size: Self::text(&self.pdu, cx),
            poll_ms: Self::text(&self.poll, cx),
            timeout_ms: Self::text(&self.timeout, cx),
            tags: self.tags.iter().map(|row| row.form(cx)).collect(),
        }
    }

    pub fn server_form(&self, name: String, cx: &App) -> S7ServerForm {
        S7ServerForm {
            name,
            bind: Self::text(&self.server_bind, cx),
            port: Self::text(&self.server_port, cx),
            template: self.template.clone(),
            pdu_size: Self::text(&self.server_pdu, cx),
        }
    }

    /// The CPU preset the rack and slot fields match, if any.
    pub fn preset(&self, cx: &App) -> Option<usize> {
        let rack = Self::text(&self.rack, cx);
        let slot = Self::text(&self.slot, cx);
        CPU_PRESETS
            .iter()
            .position(|(_, preset_rack, preset_slot)| {
                rack.trim().parse::<u8>() == Ok(*preset_rack)
                    && slot.trim().parse::<u8>() == Ok(*preset_slot)
            })
    }
}

impl Editor {
    /// A new tag row after the last one, focused.
    pub fn add_s7_tag(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let current: Vec<S7TagForm> = self.s7.tags.iter().map(|row| row.form(cx)).collect();
        let row = S7TagRow::new(next_s7_tag(&current), cx);
        self.watch(&row.name, window, cx);
        self.watch(&row.address, window, cx);
        self.watch_s7_address(&row.address, window, cx);
        row.address.update(cx, |address, _| address.focus(window));
        self.s7.tags.push(row);
    }

    /// Redraw as tag addresses are typed: the types a row offers follow
    /// its address.
    pub(super) fn watch_s7_addresses(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let addresses: Vec<Entity<TextInput>> =
            self.s7.tags.iter().map(|row| row.address.clone()).collect();
        for address in addresses {
            self.watch_s7_address(&address, window, cx);
        }
    }

    fn watch_s7_address(
        &mut self,
        address: &Entity<TextInput>,
        window: &mut Window,
        cx: &mut Context<AppModel>,
    ) {
        self.subscriptions.push(cx.subscribe_in(
            address,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Changed) {
                    cx.notify();
                }
            },
        ));
    }

    /// Fill the rack and slot of a controller family.
    pub fn set_s7_cpu(&mut self, preset: usize, cx: &mut Context<AppModel>) {
        let Some((_, rack, slot)) = CPU_PRESETS.get(preset) else {
            return;
        };
        self.error = None;
        self.s7
            .rack
            .update(cx, |input, cx| input.set_text(rack.to_string(), cx));
        self.s7
            .slot
            .update(cx, |input, cx| input.set_text(slot.to_string(), cx));
    }
}
