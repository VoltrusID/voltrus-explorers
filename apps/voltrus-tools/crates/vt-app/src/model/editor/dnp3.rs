//! The DNP3 master and outstation simulator fields of the connection editor.

use gpui::{App, Context, Entity, Window};
use vt_app_core::forms::{Dnp3MasterForm, Dnp3OutstationForm, Dnp3PointForm, next_dnp3_point};
use vt_protocol::{
    Dnp3ControlMode, Dnp3MasterSpec, Dnp3OutstationSpec, Dnp3PointKind, SessionSpec,
};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, Editor, input};

/// Name of a new master or simulator until the user names it.
pub const NEW_MASTER_NAME: &str = "DNP3 outstation";
pub const NEW_OUTSTATION_NAME: &str = "DNP3 simulator";

pub struct Dnp3PointRow {
    pub kind: Dnp3PointKind,
    pub index: Entity<TextInput>,
    pub name: Entity<TextInput>,
    unit: String,
}

impl Dnp3PointRow {
    fn new(form: Dnp3PointForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            kind: form.kind,
            index: input(form.index, "0", InputKind::Unsigned, true, cx),
            name: input(form.name, "Name", InputKind::Text, false, cx),
            unit: form.unit,
        }
    }
}

pub struct Dnp3Fields {
    // Master
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub master_address: Entity<TextInput>,
    pub outstation_address: Entity<TextInput>,
    pub integrity: Entity<TextInput>,
    pub events: Entity<TextInput>,
    pub timeout: Entity<TextInput>,
    pub control_mode: Dnp3ControlMode,
    pub time_sync: bool,
    pub points: Vec<Dnp3PointRow>,
    // Outstation simulator
    pub bind: Entity<TextInput>,
    pub listen_port: Entity<TextInput>,
    pub own_address: Entity<TextInput>,
    pub peer_address: Entity<TextInput>,
    pub template: String,
}

impl Dnp3Fields {
    /// Fields for `editing` when it is a DNP3 connection, defaults otherwise.
    pub(super) fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let master = match editing {
            Some(SessionSpec::Dnp3Master(spec)) => spec.clone(),
            _ => Dnp3MasterSpec::default(),
        };
        let outstation = match editing {
            Some(SessionSpec::Dnp3Outstation(spec)) => spec.clone(),
            _ => Dnp3OutstationSpec::default(),
        };
        let master = Dnp3MasterForm::from_spec("", &master);
        let outstation = Dnp3OutstationForm::from_spec("", &outstation);
        let unsigned = InputKind::Unsigned;
        Self {
            host: input(master.host, "192.168.1.20", InputKind::Text, true, cx),
            port: input(master.port, "20000", unsigned, true, cx),
            master_address: input(master.master_address, "1", unsigned, true, cx),
            outstation_address: input(master.outstation_address, "10", unsigned, true, cx),
            integrity: input(master.integrity_ms, "60000", unsigned, true, cx),
            events: input(master.event_ms, "0 = integrity only", unsigned, true, cx),
            timeout: input(master.timeout_ms, "2000", unsigned, true, cx),
            control_mode: master.control_mode,
            time_sync: master.time_sync,
            points: master
                .points
                .into_iter()
                .map(|form| Dnp3PointRow::new(form, cx))
                .collect(),
            bind: input(outstation.bind, "127.0.0.1", InputKind::Text, true, cx),
            listen_port: input(outstation.port, "20000", unsigned, true, cx),
            own_address: input(outstation.outstation_address, "10", unsigned, true, cx),
            peer_address: input(outstation.master_address, "1", unsigned, true, cx),
            template: outstation.template,
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.host.clone(),
            self.port.clone(),
            self.master_address.clone(),
            self.outstation_address.clone(),
            self.integrity.clone(),
            self.events.clone(),
            self.timeout.clone(),
            self.bind.clone(),
            self.listen_port.clone(),
            self.own_address.clone(),
            self.peer_address.clone(),
        ];
        for row in &self.points {
            fields.push(row.index.clone());
            fields.push(row.name.clone());
        }
        fields
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    fn point_forms(&self, cx: &App) -> Vec<Dnp3PointForm> {
        self.points
            .iter()
            .map(|row| Dnp3PointForm {
                kind: row.kind,
                index: Self::text(&row.index, cx),
                name: Self::text(&row.name, cx),
                unit: row.unit.clone(),
            })
            .collect()
    }

    pub fn master_form(&self, name: String, cx: &App) -> Dnp3MasterForm {
        Dnp3MasterForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            master_address: Self::text(&self.master_address, cx),
            outstation_address: Self::text(&self.outstation_address, cx),
            integrity_ms: Self::text(&self.integrity, cx),
            event_ms: Self::text(&self.events, cx),
            timeout_ms: Self::text(&self.timeout, cx),
            control_mode: self.control_mode,
            time_sync: self.time_sync,
            points: self.point_forms(cx),
        }
    }

    pub fn outstation_form(&self, name: String, cx: &App) -> Dnp3OutstationForm {
        Dnp3OutstationForm {
            name,
            bind: Self::text(&self.bind, cx),
            port: Self::text(&self.listen_port, cx),
            outstation_address: Self::text(&self.own_address, cx),
            master_address: Self::text(&self.peer_address, cx),
            template: self.template.clone(),
        }
    }
}

impl Editor {
    /// A new point-name row after the last, its index focused.
    pub fn add_dnp3_point(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let forms = self.dnp3.point_forms(cx);
        let row = Dnp3PointRow::new(next_dnp3_point(&forms), cx);
        self.watch(&row.index, window, cx);
        self.watch(&row.name, window, cx);
        row.name.update(cx, |name, _| name.focus(window));
        self.dnp3.points.push(row);
    }

    /// Step a row's point type through the types a master names.
    pub fn cycle_dnp3_point_kind(&mut self, row: usize) {
        if let Some(row) = self.dnp3.points.get_mut(row) {
            let all = Dnp3PointKind::ALL;
            let at = all.iter().position(|kind| *kind == row.kind).unwrap_or(0);
            row.kind = all[(at + 1) % all.len()];
        }
    }
}
