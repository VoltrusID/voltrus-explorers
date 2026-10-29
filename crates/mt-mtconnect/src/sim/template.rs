//! Device templates for the agent simulator.

use serde::{Deserialize, Serialize};

use crate::types::{Category, Component, DataItem, Device, DeviceDescription};

/// Built-in simulated machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceTemplate {
    /// 3-axis vertical machining centre: X/Y/Z, spindle, path feedrate, controller.
    Vmc3Axis,
    /// 2-axis lathe: X/Z, spindle, path feedrate, controller.
    Lathe2Axis,
}

pub(crate) fn pos(id: &str, name: &str) -> DataItem {
    let mut d = DataItem::new(id, Category::Sample, "POSITION");
    d.name = Some(name.into());
    d.sub_type = Some("ACTUAL".into());
    d.units = Some("MILLIMETER".into());
    d.native_units = Some("MILLIMETER".into());
    d
}

fn event(id: &str, name: &str, ty: &str) -> DataItem {
    let mut d = DataItem::new(id, Category::Event, ty);
    d.name = Some(name.into());
    d
}

fn condition(id: &str, name: &str, ty: &str) -> DataItem {
    let mut d = DataItem::new(id, Category::Condition, ty);
    d.name = Some(name.into());
    d
}

impl DeviceTemplate {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Vmc3Axis => "3-axis VMC",
            Self::Lathe2Axis => "2-axis lathe",
        }
    }

    /// Axis data item ids in X, Y, Z order that the template simulates.
    pub(crate) fn axis_ids(&self) -> &'static [&'static str] {
        match self {
            Self::Vmc3Axis => &["Xpos", "Ypos", "Zpos"],
            Self::Lathe2Axis => &["Xpos", "Zpos"],
        }
    }

    /// The device description served by `/probe`.
    pub fn build(&self) -> Device {
        let (id, name, uuid, model) = match self {
            Self::Vmc3Axis => ("vmc", "VMC", "voltrus-sim-vmc-3axis", "VMC-3AX"),
            Self::Lathe2Axis => ("lathe", "Lathe", "voltrus-sim-lathe-2axis", "TURN-2AX"),
        };

        let mut axes = Component::new("Axes", "axes", Some("Axes"));
        let linear = |id: &str, name: &str, item: &str, item_name: &str| {
            let mut c = Component::new("Linear", id, Some(name));
            c.data_items.push(pos(item, item_name));
            c
        };
        axes.components.push(linear("x", "X", "Xpos", "Xact"));
        if matches!(self, Self::Vmc3Axis) {
            axes.components.push(linear("y", "Y", "Ypos", "Yact"));
        }
        axes.components.push(linear("z", "Z", "Zpos", "Zact"));

        let mut spindle = Component::new("Rotary", "c", Some("C"));
        let mut speed = DataItem::new("Sspeed", Category::Sample, "ROTARY_VELOCITY");
        speed.name = Some("Sspeed".into());
        speed.sub_type = Some("ACTUAL".into());
        speed.units = Some("REVOLUTION/MINUTE".into());
        speed.native_units = Some("REVOLUTION/MINUTE".into());
        let mut load = DataItem::new("Sload", Category::Sample, "LOAD");
        load.name = Some("Sload".into());
        load.units = Some("PERCENT".into());
        spindle.data_items.extend([speed, load]);
        axes.components.push(spindle);

        let mut controller = Component::new("Controller", "cn", Some("Controller"));
        let mut mode = event("mode", "mode", "CONTROLLER_MODE");
        mode.name = Some("mode".into());
        controller.data_items.extend([
            mode,
            event("estop", "estop", "EMERGENCY_STOP"),
            condition("sys_cond", "system", "SYSTEM"),
        ]);

        let mut path = Component::new("Path", "path1", Some("path"));
        let mut line = event("line", "line", "LINE_NUMBER");
        line.sub_type = Some("ABSOLUTE".into());
        let mut part_count = event("pcount", "part_count", "PART_COUNT");
        part_count.sub_type = Some("COMPLETE".into());
        let mut feed = DataItem::new("pfeed", Category::Sample, "PATH_FEEDRATE");
        feed.name = Some("Fpath".into());
        feed.units = Some("MILLIMETER/SECOND".into());
        feed.native_units = Some("MILLIMETER/SECOND".into());
        path.data_items.extend([
            event("exec", "execution", "EXECUTION"),
            event("program", "program", "PROGRAM"),
            line,
            part_count,
            feed,
            condition("motion_cond", "motion", "MOTION_PROGRAM"),
        ]);
        controller.components.push(path);

        Device {
            is_agent: false,
            id: id.into(),
            name: name.into(),
            uuid: Some(uuid.into()),
            description: Some(DeviceDescription {
                manufacturer: Some("Voltrus Simulated".into()),
                model: Some(model.into()),
                serial_number: Some("SIM-0001".into()),
                station: None,
                text: Some(format!("Simulated {}", self.label())),
            }),
            data_items: vec![event("avail", "avail", "AVAILABILITY")],
            components: vec![axes, controller],
        }
    }
}
