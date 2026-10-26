//! The connection editor sheet: text fields bound to a validated form.

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use vt_app_core::ConnectionId;
use vt_app_core::forms::{BlockForm, FormError, MasterForm, SlaveForm, Transport, next_block};
use vt_protocol::{
    Area, DataType, ModbusMasterSpec, ModbusSlaveSpec, MqttBrokerSpec, MqttClientSpec, Protocol,
    Role, SessionSpec, WordOrder,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};

use super::AppModel;
use super::bacnet::{BacnetFields, NEW_CLIENT_NAME, NEW_DEVICE_NAME};

mod dnp3;
mod mqtt;

pub use dnp3::{Dnp3Fields, NEW_MASTER_NAME, NEW_OUTSTATION_NAME};
pub use mqtt::MqttFields;
mod nats;

pub use nats::NatsFields;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKind {
    Master,
    Slave,
    MqttClient,
    MqttBroker,
    BacnetClient,
    BacnetDevice,
    Dnp3Master,
    Dnp3Outstation,
    NatsClient,
    NatsServer,
}

impl EditorKind {
    /// The editor that makes a new connection of `protocol` in `role`, if
    /// the suite has one (not every protocol has a simulator).
    pub fn for_protocol(protocol: Protocol, role: Role) -> Option<Self> {
        Some(match (protocol, role) {
            (Protocol::Modbus, Role::Client) => Self::Master,
            (Protocol::Modbus, Role::Simulator) => Self::Slave,
            (Protocol::Mqtt, Role::Client) => Self::MqttClient,
            (Protocol::Mqtt, Role::Simulator) => Self::MqttBroker,
            (Protocol::Bacnet, Role::Client) => Self::BacnetClient,
            (Protocol::Bacnet, Role::Simulator) => Self::BacnetDevice,
            (Protocol::Dnp3, Role::Client) => Self::Dnp3Master,
            (Protocol::Dnp3, Role::Simulator) => Self::Dnp3Outstation,
            (Protocol::Nats, Role::Client) => Self::NatsClient,
            (Protocol::Nats, Role::Simulator) => Self::NatsServer,
        })
    }

    /// The editor of a saved connection.
    pub fn for_spec(spec: &SessionSpec) -> Self {
        match spec {
            SessionSpec::ModbusMaster(_) => Self::Master,
            SessionSpec::ModbusSlave(_) => Self::Slave,
            SessionSpec::MqttClient(_) => Self::MqttClient,
            SessionSpec::MqttBroker(_) => Self::MqttBroker,
            SessionSpec::BacnetClient(_) => Self::BacnetClient,
            SessionSpec::BacnetDevice(_) => Self::BacnetDevice,
            SessionSpec::Dnp3Master(_) => Self::Dnp3Master,
            SessionSpec::Dnp3Outstation(_) => Self::Dnp3Outstation,
            SessionSpec::NatsClient(_) => Self::NatsClient,
            SessionSpec::NatsServer(_) => Self::NatsServer,
        }
    }
}

pub struct BlockRow {
    pub name: Entity<TextInput>,
    pub start: Entity<TextInput>,
    pub count: Entity<TextInput>,
    pub area: Area,
    pub data_type: DataType,
    unit: String,
}

pub struct Editor {
    pub kind: EditorKind,
    pub editing: Option<ConnectionId>,
    pub name: Entity<TextInput>,
    pub transport: Transport,
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub serial_port: Entity<TextInput>,
    pub baud: Entity<TextInput>,
    pub data_bits: u8,
    pub parity: vt_protocol::Parity,
    pub stop_bits: u8,
    pub unit: Entity<TextInput>,
    pub poll: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub word_order: WordOrder,
    pub blocks: Vec<BlockRow>,
    pub bind: Entity<TextInput>,
    pub registers: Entity<TextInput>,
    pub template: Option<String>,
    /// New simulators: also save a client connection that polls it.
    pub add_client: bool,
    /// The first Delete press arms it; the second deletes.
    pub confirm_delete: bool,
    pub bacnet: BacnetFields,
    pub error: Option<FormError>,
    subscriptions: Vec<Subscription>,
    pub mqtt: MqttFields,
    pub dnp3: Dnp3Fields,
    pub nats: NatsFields,
}

fn input(
    text: impl Into<gpui::SharedString>,
    placeholder: &'static str,
    kind: InputKind,
    mono: bool,
    cx: &mut Context<AppModel>,
) -> Entity<TextInput> {
    let text = text.into();
    cx.new(|cx| {
        let input = TextInput::new(cx)
            .text(text)
            .placeholder(placeholder)
            .kind(kind);
        if mono { input.mono() } else { input }
    })
}

impl Editor {
    pub fn new(
        kind: EditorKind,
        editing: Option<(ConnectionId, String, SessionSpec)>,
        window: &mut Window,
        cx: &mut Context<AppModel>,
    ) -> Self {
        let id = editing.as_ref().map(|(id, _, _)| *id);
        let bacnet = BacnetFields::new(editing.as_ref().map(|(_, _, spec)| spec), cx);
        let dnp3 = Dnp3Fields::new(editing.as_ref().map(|(_, _, spec)| spec), cx);
        let nats_client = match editing.as_ref().map(|(_, _, spec)| spec) {
            Some(SessionSpec::NatsClient(spec)) => spec.clone(),
            _ => vt_protocol::NatsClientSpec::default(),
        };
        let nats_server = match editing.as_ref().map(|(_, _, spec)| spec) {
            Some(SessionSpec::NatsServer(spec)) => spec.clone(),
            _ => vt_protocol::NatsServerSpec::default(),
        };
        let mut mqtt_client = MqttClientSpec::default();
        let mut mqtt_broker = MqttBrokerSpec::default();
        let (name, master, slave) = match editing {
            Some((_, name, SessionSpec::ModbusMaster(spec))) => {
                (name, spec, ModbusSlaveSpec::default())
            }
            Some((_, name, SessionSpec::ModbusSlave(spec))) => {
                (name, ModbusMasterSpec::default(), spec)
            }
            Some((_, name, SessionSpec::MqttClient(spec))) => {
                mqtt_client = spec;
                (
                    name,
                    ModbusMasterSpec::default(),
                    ModbusSlaveSpec::default(),
                )
            }
            Some((_, name, SessionSpec::MqttBroker(spec))) => {
                mqtt_broker = spec;
                (
                    name,
                    ModbusMasterSpec::default(),
                    ModbusSlaveSpec::default(),
                )
            }
            Some((_, name, SessionSpec::BacnetClient(_) | SessionSpec::BacnetDevice(_))) => (
                name,
                ModbusMasterSpec::default(),
                ModbusSlaveSpec::default(),
            ),
            Some((_, name, SessionSpec::Dnp3Master(_) | SessionSpec::Dnp3Outstation(_))) => (
                name,
                ModbusMasterSpec::default(),
                ModbusSlaveSpec::default(),
            ),
            Some((_, name, SessionSpec::NatsClient(_) | SessionSpec::NatsServer(_))) => (
                name,
                ModbusMasterSpec::default(),
                ModbusSlaveSpec::default(),
            ),
            None => (
                match kind {
                    EditorKind::Master => "Modbus device".to_owned(),
                    EditorKind::Slave => "Modbus simulator".to_owned(),
                    EditorKind::MqttClient => "MQTT broker".to_owned(),
                    EditorKind::MqttBroker => "MQTT simulator".to_owned(),
                    EditorKind::BacnetClient => NEW_CLIENT_NAME.to_owned(),
                    EditorKind::BacnetDevice => NEW_DEVICE_NAME.to_owned(),
                    EditorKind::Dnp3Master => NEW_MASTER_NAME.to_owned(),
                    EditorKind::Dnp3Outstation => NEW_OUTSTATION_NAME.to_owned(),
                    EditorKind::NatsClient => "NATS server".to_owned(),
                    EditorKind::NatsServer => "NATS simulator".to_owned(),
                },
                ModbusMasterSpec::default(),
                ModbusSlaveSpec {
                    template: Some("sma-inverter".into()),
                    ..ModbusSlaveSpec::default()
                },
            ),
        };
        let master = MasterForm::from_spec(&name, &master);
        let slave = SlaveForm::from_spec(&name, &slave);
        let unsigned = InputKind::Unsigned;
        let mut editor = Self {
            kind,
            editing: id,
            name: input(name, "Name", InputKind::Text, false, cx),
            transport: master.transport,
            host: input(master.host, "192.168.1.10", InputKind::Text, true, cx),
            port: input(
                if kind == EditorKind::Slave {
                    slave.port.clone()
                } else {
                    master.port
                },
                "502",
                unsigned,
                true,
                cx,
            ),
            serial_port: input(
                master.serial.port.clone(),
                "/dev/tty.usbserial-0001",
                InputKind::Text,
                true,
                cx,
            ),
            baud: input(master.baud, "9600", unsigned, true, cx),
            data_bits: master.serial.data_bits,
            parity: master.serial.parity,
            stop_bits: master.serial.stop_bits,
            unit: input(
                if kind == EditorKind::Slave {
                    slave.unit_id.clone()
                } else {
                    master.unit_id
                },
                "1",
                unsigned,
                true,
                cx,
            ),
            poll: input(master.poll_ms, "1000", unsigned, true, cx),
            timeout: input(master.timeout_ms, "2000", unsigned, true, cx),
            word_order: master.word_order,
            blocks: Vec::new(),
            bind: input(slave.bind, "127.0.0.1", InputKind::Text, true, cx),
            registers: input(slave.registers, "200", unsigned, true, cx),
            template: slave.template,
            add_client: id.is_none(),
            confirm_delete: false,
            bacnet,
            error: None,
            subscriptions: Vec::new(),
            mqtt: MqttFields::new(&mqtt_client, &mqtt_broker, cx),
            dnp3,
            nats: NatsFields::new(&nats_client, &nats_server, cx),
        };
        for block in master.blocks {
            editor.push_block(block, window, cx);
        }
        for field in [
            editor.name.clone(),
            editor.host.clone(),
            editor.port.clone(),
            editor.serial_port.clone(),
            editor.baud.clone(),
            editor.unit.clone(),
            editor.poll.clone(),
            editor.timeout.clone(),
            editor.bind.clone(),
            editor.registers.clone(),
        ]
        .into_iter()
        .chain(editor.mqtt.inputs())
        {
            editor.watch(&field, window, cx);
        }
        for field in editor.bacnet.inputs() {
            editor.watch(&field, window, cx);
        }
        for field in editor.dnp3.inputs() {
            editor.watch(&field, window, cx);
        }
        for field in editor.nats.inputs() {
            editor.watch(&field, window, cx);
        }
        editor.name.update(cx, |name, _| name.focus(window));
        editor
    }

    /// Enter saves, Escape closes, editing clears the error.
    fn watch(
        &mut self,
        field: &Entity<TextInput>,
        window: &mut Window,
        cx: &mut Context<AppModel>,
    ) {
        self.subscriptions.push(cx.subscribe_in(
            field,
            window,
            |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => model.save_editor(window, cx),
                InputEvent::Cancelled => model.close_overlay(window, cx),
                InputEvent::Changed => {
                    if let Some(editor) = model.editor_mut() {
                        editor.error = None;
                    }
                }
                InputEvent::Blurred => {}
            },
        ));
    }

    fn push_block(&mut self, block: BlockForm, window: &mut Window, cx: &mut Context<AppModel>) {
        let row = BlockRow {
            name: input(block.name, "Block name", InputKind::Text, false, cx),
            start: input(block.start, "0", InputKind::Unsigned, true, cx),
            count: input(block.count, "10", InputKind::Unsigned, true, cx),
            area: block.area,
            data_type: block.data_type,
            unit: block.unit,
        };
        for field in [row.name.clone(), row.start.clone(), row.count.clone()] {
            self.watch(&field, window, cx);
        }
        self.blocks.push(row);
    }

    pub fn add_block(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let current = self.block_forms(cx);
        self.push_block(next_block(&current), window, cx);
    }

    fn block_forms(&self, cx: &App) -> Vec<BlockForm> {
        self.blocks
            .iter()
            .map(|row| BlockForm {
                name: row.name.read(cx).value().to_owned(),
                area: row.area,
                start: row.start.read(cx).value().to_owned(),
                count: row.count.read(cx).value().to_owned(),
                data_type: row.data_type,
                unit: row.unit.clone(),
            })
            .collect()
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn master_form(&self, cx: &App) -> MasterForm {
        MasterForm {
            name: Self::text(&self.name, cx),
            transport: self.transport,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            serial: vt_protocol::SerialLink {
                port: Self::text(&self.serial_port, cx),
                baud: 0,
                data_bits: self.data_bits,
                parity: self.parity,
                stop_bits: self.stop_bits,
            },
            baud: Self::text(&self.baud, cx),
            unit_id: Self::text(&self.unit, cx),
            poll_ms: Self::text(&self.poll, cx),
            timeout_ms: Self::text(&self.timeout, cx),
            word_order: self.word_order,
            blocks: self.block_forms(cx),
        }
    }

    pub fn slave_form(&self, cx: &App) -> SlaveForm {
        SlaveForm {
            name: Self::text(&self.name, cx),
            bind: Self::text(&self.bind, cx),
            port: Self::text(&self.port, cx),
            unit_id: Self::text(&self.unit, cx),
            template: self.template.clone(),
            registers: Self::text(&self.registers, cx),
        }
    }

    /// The validated session, or the first problem.
    pub fn build(&self, cx: &App) -> Result<(String, SessionSpec), FormError> {
        match self.kind {
            EditorKind::Master => self
                .master_form(cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::ModbusMaster(spec))),
            EditorKind::Slave => self
                .slave_form(cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::ModbusSlave(spec))),
            EditorKind::MqttClient => self
                .mqtt
                .client_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::MqttClient(spec))),
            EditorKind::MqttBroker => self
                .mqtt
                .broker_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::MqttBroker(spec))),
            EditorKind::BacnetClient => self
                .bacnet
                .client_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::BacnetClient(spec))),
            EditorKind::BacnetDevice => self
                .bacnet
                .device_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::BacnetDevice(spec))),
            EditorKind::Dnp3Master => self
                .dnp3
                .master_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::Dnp3Master(spec))),
            EditorKind::Dnp3Outstation => self
                .dnp3
                .outstation_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::Dnp3Outstation(spec))),
            EditorKind::NatsClient => self
                .nats
                .client_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::NatsClient(spec))),
            EditorKind::NatsServer => self
                .nats
                .server_form(Self::text(&self.name, cx), cx)
                .build()
                .map(|(name, spec)| (name, SessionSpec::NatsServer(spec))),
        }
    }
}
