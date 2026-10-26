//! DNP3 master and outstation simulator forms.

use vt_protocol::{
    DNP3_MAX_ADDRESS, Dnp3ControlMode, Dnp3MasterSpec, Dnp3OutstationSpec, Dnp3PointKind,
    Dnp3PointName,
};

use super::{Field, FormError, error, name, number};

/// Which field of a point-name row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dnp3PointField {
    Index,
    Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dnp3PointForm {
    pub kind: Dnp3PointKind,
    pub index: String,
    pub name: String,
    /// Not edited in the form; kept so plant units survive an edit.
    pub unit: String,
}

impl Dnp3PointForm {
    fn from_name(point: &Dnp3PointName) -> Self {
        Self {
            kind: point.kind,
            index: point.index.to_string(),
            name: point.name.clone(),
            unit: point.unit.clone(),
        }
    }
}

/// A blank row for a new point name, continuing after the last row.
pub fn next_dnp3_point(rows: &[Dnp3PointForm]) -> Dnp3PointForm {
    let kind = rows
        .last()
        .map_or(Dnp3PointKind::AnalogInput, |last| last.kind);
    let index = rows
        .iter()
        .filter(|row| row.kind == kind)
        .filter_map(|row| row.index.trim().parse::<u16>().ok())
        .max()
        .and_then(|highest| highest.checked_add(1))
        .unwrap_or(0);
    Dnp3PointForm {
        kind,
        index: index.to_string(),
        name: String::new(),
        unit: String::new(),
    }
}

fn address(text: &str, field: Field, what: &str) -> Result<u16, FormError> {
    number(text, field, what, 0, DNP3_MAX_ADDRESS)
}

fn distinct(master: u16, outstation: u16) -> Result<(), FormError> {
    if master == outstation {
        return Err(error(
            Field::OutstationAddress,
            "The master and the outstation need different link addresses",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dnp3MasterForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub master_address: String,
    pub outstation_address: String,
    pub integrity_ms: String,
    pub event_ms: String,
    pub timeout_ms: String,
    pub control_mode: Dnp3ControlMode,
    pub time_sync: bool,
    pub points: Vec<Dnp3PointForm>,
}

impl Dnp3MasterForm {
    pub fn from_spec(name: &str, spec: &Dnp3MasterSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            master_address: spec.master_address.to_string(),
            outstation_address: spec.outstation_address.to_string(),
            integrity_ms: spec.integrity_ms.to_string(),
            event_ms: spec.event_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
            control_mode: spec.control_mode,
            time_sync: spec.time_sync,
            points: spec.points.iter().map(Dnp3PointForm::from_name).collect(),
        }
    }

    pub fn build(&self) -> Result<(String, Dnp3MasterSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the outstation's IP address or host name",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let master_address = address(&self.master_address, Field::LinkAddress, "Master address")?;
        let outstation_address = address(
            &self.outstation_address,
            Field::OutstationAddress,
            "Outstation address",
        )?;
        distinct(master_address, outstation_address)?;
        let integrity_ms = number(
            &self.integrity_ms,
            Field::IntegrityMs,
            "Integrity poll",
            1000,
            86_400_000,
        )?;
        let event_ms: u64 = number(&self.event_ms, Field::EventMs, "Event poll", 0, 3_600_000)?;
        if (1..100).contains(&event_ms) {
            return Err(error(
                Field::EventMs,
                "Poll events every 100 ms or slower, or 0 for integrity polls only",
            ));
        }
        let timeout_ms = number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 60_000)?;
        let mut points: Vec<Dnp3PointName> = Vec::with_capacity(self.points.len());
        for (row, point) in self.points.iter().enumerate() {
            let index = number(
                &point.index,
                Field::Dnp3Point(row, Dnp3PointField::Index),
                "Index",
                0,
                u16::MAX,
            )?;
            if points
                .iter()
                .any(|named| named.kind == point.kind && named.index == index)
            {
                return Err(error(
                    Field::Dnp3Point(row, Dnp3PointField::Index),
                    format!("{} {index} is named twice", point.kind.short()),
                ));
            }
            points.push(Dnp3PointName {
                kind: point.kind,
                index,
                name: point.name.trim().to_owned(),
                unit: point.unit.clone(),
            });
        }
        Ok((
            name,
            Dnp3MasterSpec {
                host: host.to_owned(),
                port,
                master_address,
                outstation_address,
                integrity_ms,
                event_ms,
                timeout_ms,
                control_mode: self.control_mode,
                time_sync: self.time_sync,
                points,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dnp3OutstationForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub outstation_address: String,
    pub master_address: String,
    pub template: String,
}

impl Dnp3OutstationForm {
    pub fn from_spec(name: &str, spec: &Dnp3OutstationSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            outstation_address: spec.outstation_address.to_string(),
            master_address: spec.master_address.to_string(),
            template: spec.template.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, Dnp3OutstationSpec), FormError> {
        let name = name(&self.name)?;
        let bind = self.bind.trim();
        let bare = bind
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(bind);
        let Ok(ip) = bare.parse::<std::net::IpAddr>() else {
            return Err(error(
                Field::Bind,
                "Listen on an IP address, e.g. 0.0.0.0, 127.0.0.1 or ::",
            ));
        };
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let outstation_address = address(
            &self.outstation_address,
            Field::OutstationAddress,
            "Outstation address",
        )?;
        let master_address = address(&self.master_address, Field::LinkAddress, "Master address")?;
        distinct(master_address, outstation_address)?;
        Ok((
            name,
            Dnp3OutstationSpec {
                bind: ip.to_string(),
                port,
                outstation_address,
                master_address,
                template: self.template.clone(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master() -> Dnp3MasterForm {
        Dnp3MasterForm::from_spec("RTU", &Dnp3MasterSpec::default())
    }

    #[test]
    fn default_forms_round_trip_to_the_same_spec() {
        let (name, spec) = master().build().unwrap();
        assert_eq!(name, "RTU");
        assert_eq!(spec, Dnp3MasterSpec::default());
        let outstation = Dnp3OutstationForm::from_spec("Sim", &Dnp3OutstationSpec::default());
        assert_eq!(outstation.build().unwrap().1, Dnp3OutstationSpec::default());
    }

    #[test]
    fn link_addresses_are_distinct_and_not_reserved() {
        let mut form = master();
        form.outstation_address = "65519".into();
        assert_eq!(form.build().unwrap().1.outstation_address, 0xFFEF);
        // 0xFFF0 and up are broadcast or reserved.
        form.outstation_address = "65533".into();
        assert_eq!(form.build().unwrap_err().field, Field::OutstationAddress);
        form.outstation_address = "1".into();
        let clash = form.build().unwrap_err();
        assert_eq!(clash.field, Field::OutstationAddress);
        assert!(clash.message.contains("different"));
        form.outstation_address = "10".into();
        form.master_address = "-1".into();
        assert_eq!(form.build().unwrap_err().field, Field::LinkAddress);

        let mut outstation = Dnp3OutstationForm::from_spec("Sim", &Dnp3OutstationSpec::default());
        outstation.master_address = "10".into();
        assert_eq!(
            outstation.build().unwrap_err().field,
            Field::OutstationAddress
        );
        outstation.master_address = "1".into();
        outstation.bind = " [::1] ".into();
        assert_eq!(outstation.build().unwrap().1.bind, "::1");
        outstation.bind = "localhost".into();
        assert_eq!(outstation.build().unwrap_err().field, Field::Bind);
    }

    #[test]
    fn polls_have_sensible_periods() {
        let mut form = master();
        form.event_ms = "0".into();
        assert_eq!(form.build().unwrap().1.event_ms, 0, "integrity polls only");
        form.event_ms = "50".into();
        assert_eq!(form.build().unwrap_err().field, Field::EventMs);
        form.event_ms = "100".into();
        assert!(form.build().is_ok());
        form.integrity_ms = "500".into();
        assert_eq!(form.build().unwrap_err().field, Field::IntegrityMs);
        form.integrity_ms = "60000".into();
        form.timeout_ms = "50".into();
        assert_eq!(form.build().unwrap_err().field, Field::TimeoutMs);
    }

    #[test]
    fn point_names_are_checked_row_by_row() {
        let mut form = master();
        form.points = vec![
            Dnp3PointForm {
                kind: Dnp3PointKind::BinaryOutput,
                index: "0".into(),
                name: " Breaker ".into(),
                unit: String::new(),
            },
            Dnp3PointForm {
                kind: Dnp3PointKind::AnalogInput,
                index: "0".into(),
                name: "Current".into(),
                unit: "A".into(),
            },
        ];
        let (_, spec) = form.build().unwrap();
        assert_eq!(spec.points[0].name, "Breaker");
        assert_eq!(spec.points[1].unit, "A", "units survive an edit");
        // The same kind and index twice.
        form.points[1].kind = Dnp3PointKind::BinaryOutput;
        let twice = form.build().unwrap_err();
        assert_eq!(twice.field, Field::Dnp3Point(1, Dnp3PointField::Index));
        assert_eq!(twice.message, "BO 0 is named twice");
        form.points[1].index = "x".into();
        assert_eq!(
            form.build().unwrap_err().field,
            Field::Dnp3Point(1, Dnp3PointField::Index)
        );
        // A new row continues after the last one of its kind.
        form.points[1].index = "4".into();
        let next = next_dnp3_point(&form.points);
        assert_eq!(
            (next.kind, next.index.as_str()),
            (Dnp3PointKind::BinaryOutput, "5")
        );
        assert_eq!(next_dnp3_point(&[]).kind, Dnp3PointKind::AnalogInput);
    }
}
