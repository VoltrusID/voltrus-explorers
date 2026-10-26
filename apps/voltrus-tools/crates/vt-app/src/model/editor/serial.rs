//! The serial monitor and virtual serial device fields of the connection
//! editor, with the port picker's state.

use gpui::{App, Context, Entity, Task, Window};
use vt_app_core::forms::{MacroForm, ParserKind, SerialDeviceForm, SerialMonitorForm};
use vt_protocol::{
    FlowControl, LineEnding, Parity, SerialDeviceSpec, SerialFormat, SerialMonitorSpec,
    SerialPortInfo, SessionId, SessionSpec,
};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, Editor, input};

pub struct MacroRow {
    pub name: Entity<TextInput>,
    pub payload: Entity<TextInput>,
    pub hex: bool,
}

impl MacroRow {
    fn new(form: MacroForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            name: input(form.name, "Read now", InputKind::Text, false, cx),
            payload: input(
                form.payload,
                "READ or 01 03 00 00",
                InputKind::Text,
                true,
                cx,
            ),
            hex: form.hex,
        }
    }
}

pub struct SerialFields {
    // Monitor
    pub port: Entity<TextInput>,
    pub baud: Entity<TextInput>,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow: FlowControl,
    pub line_ending: LineEnding,
    pub parser: ParserKind,
    pub delimiter: Entity<TextInput>,
    pub columns: Entity<TextInput>,
    pub pattern: Entity<TextInput>,
    pub macros: Vec<MacroRow>,
    // Virtual device
    pub template: String,
    pub format: SerialFormat,
    pub interval: Entity<TextInput>,
    pub alias: Entity<TextInput>,
    // Port picker
    /// The last list of ports, kept while a refresh runs so it never
    /// flickers empty.
    pub ports: Option<Result<Vec<SerialPortInfo>, String>>,
    /// The refresh the picker waits for.
    pub request: Option<SessionId>,
    /// Lists the ports every few seconds while the editor is open, so a
    /// device plugged in shows up; dropped with the editor.
    pub _refresh: Option<Task<()>>,
}

impl SerialFields {
    /// Fields for `editing` when it is a serial connection, defaults otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let monitor = match editing {
            Some(SessionSpec::SerialMonitor(spec)) => spec.clone(),
            _ => SerialMonitorSpec::default(),
        };
        let device = match editing {
            Some(SessionSpec::SerialDevice(spec)) => spec.clone(),
            _ => SerialDeviceSpec::default(),
        };
        let monitor = SerialMonitorForm::from_spec("", &monitor);
        let device = SerialDeviceForm::from_spec("", &device);
        Self {
            port: input(
                monitor.port,
                "/dev/cu.usbserial-0001",
                InputKind::Text,
                true,
                cx,
            ),
            baud: input(monitor.baud, "115200", InputKind::Unsigned, true, cx),
            data_bits: monitor.data_bits,
            parity: monitor.parity,
            stop_bits: monitor.stop_bits,
            flow: monitor.flow,
            line_ending: monitor.line_ending,
            parser: monitor.parser,
            delimiter: input(monitor.delimiter, ",", InputKind::Text, true, cx),
            columns: input(
                monitor.columns,
                "from the header line",
                InputKind::Text,
                false,
                cx,
            ),
            pattern: input(
                monitor.pattern,
                "T=(?P<temp>-?\\d+\\.\\d+)",
                InputKind::Text,
                true,
                cx,
            ),
            macros: monitor
                .macros
                .into_iter()
                .map(|form| MacroRow::new(form, cx))
                .collect(),
            template: device.template,
            format: device.format,
            interval: input(device.interval_ms, "1000", InputKind::Unsigned, true, cx),
            alias: input(device.alias, "voltrus-serial-1", InputKind::Text, true, cx),
            ports: None,
            request: None,
            _refresh: None,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.port.clone(),
            self.baud.clone(),
            self.delimiter.clone(),
            self.columns.clone(),
            self.pattern.clone(),
            self.interval.clone(),
            self.alias.clone(),
        ];
        for row in &self.macros {
            fields.push(row.name.clone());
            fields.push(row.payload.clone());
        }
        fields
    }

    pub(crate) fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn monitor_form(&self, name: String, cx: &App) -> SerialMonitorForm {
        SerialMonitorForm {
            name,
            port: Self::text(&self.port, cx),
            baud: Self::text(&self.baud, cx),
            data_bits: self.data_bits,
            parity: self.parity,
            stop_bits: self.stop_bits,
            flow: self.flow,
            line_ending: self.line_ending,
            parser: self.parser,
            delimiter: Self::text(&self.delimiter, cx),
            columns: Self::text(&self.columns, cx),
            pattern: Self::text(&self.pattern, cx),
            macros: self
                .macros
                .iter()
                .map(|row| MacroForm {
                    name: Self::text(&row.name, cx),
                    payload: Self::text(&row.payload, cx),
                    hex: row.hex,
                })
                .collect(),
        }
    }

    pub fn device_form(&self, name: String, cx: &App) -> SerialDeviceForm {
        SerialDeviceForm {
            name,
            template: self.template.clone(),
            format: self.format,
            interval_ms: Self::text(&self.interval, cx),
            alias: Self::text(&self.alias, cx),
        }
    }
}

impl Editor {
    pub fn add_macro(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let row = MacroRow::new(MacroForm::blank(), cx);
        self.watch(&row.name, window, cx);
        self.watch(&row.payload, window, cx);
        row.name.update(cx, |name, _| name.focus(window));
        self.serial.macros.push(row);
    }
}
