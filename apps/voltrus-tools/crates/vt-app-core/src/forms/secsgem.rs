//! SECS/GEM host and equipment simulator forms.

use vt_protocol::{SECSGEM_MAX_IDS, SecsGemClientSpec, SecsGemSimulatorSpec};

use super::cnc::bind_address;
use super::{Field, FormError, error, name, number};

/// The largest id the app takes: ids travel as unsigned 32-bit numbers.
const MAX_ID: u64 = u32::MAX as u64;

/// Ids from text such as `10, 11 20-23`, each once, in the order typed. The
/// field is named in the messages.
pub fn parse_ids(text: &str, field: Field, what: &str) -> Result<Vec<u64>, FormError> {
    let bad = |item: &str| {
        error(
            field,
            format!("\"{item}\" is not a {what} (a number up to {MAX_ID}) or a range like 10-13"),
        )
    };
    let mut ids: Vec<u64> = Vec::new();
    for item in text.split([',', ' ', ';']).filter(|item| !item.is_empty()) {
        let (first, last) = match item.split_once('-') {
            Some((first, last)) => (first, last),
            None => (item, item),
        };
        let (Ok(first), Ok(last)) = (first.parse::<u64>(), last.parse::<u64>()) else {
            return Err(bad(item));
        };
        if first > last || last > MAX_ID {
            return Err(bad(item));
        }
        for id in first..=last {
            if !ids.contains(&id) {
                ids.push(id);
            }
            if ids.len() > SECSGEM_MAX_IDS {
                return Err(error(
                    field,
                    format!("A connection takes at most {SECSGEM_MAX_IDS} {what}s in a list"),
                ));
            }
        }
    }
    Ok(ids)
}

fn join(ids: &[u64]) -> String {
    ids.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecsGemClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub device_id: String,
    pub poll_ms: String,
    pub timeout_ms: String,
    pub svids: String,
    pub ecids: String,
    pub events: String,
    pub alarms: bool,
}

impl SecsGemClientForm {
    pub fn from_spec(name: &str, spec: &SecsGemClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            device_id: spec.device_id.to_string(),
            poll_ms: spec.poll_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
            svids: join(&spec.svids),
            ecids: join(&spec.ecids),
            events: join(&spec.events),
            alarms: spec.alarms,
        }
    }

    pub fn build(&self) -> Result<(String, SecsGemClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the equipment's IP address or host name",
            ));
        }
        Ok((
            name,
            SecsGemClientSpec {
                host: host.to_owned(),
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                device_id: number(&self.device_id, Field::DeviceId, "Device ID", 0, u16::MAX)?,
                poll_ms: number(
                    &self.poll_ms,
                    Field::PollMs,
                    "Poll interval",
                    100,
                    3_600_000,
                )?,
                timeout_ms: number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 60_000)?,
                svids: parse_ids(&self.svids, Field::SvIds, "status variable id")?,
                ecids: parse_ids(&self.ecids, Field::EcIds, "equipment constant id")?,
                events: parse_ids(&self.events, Field::Events, "collection event id")?,
                alarms: self.alarms,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecsGemSimulatorForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub device_id: String,
    pub run: bool,
}

impl SecsGemSimulatorForm {
    pub fn from_spec(name: &str, spec: &SecsGemSimulatorSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            device_id: spec.device_id.to_string(),
            run: spec.run,
        }
    }

    pub fn build(&self) -> Result<(String, SecsGemSimulatorSpec), FormError> {
        Ok((
            name(&self.name)?,
            SecsGemSimulatorSpec {
                bind: bind_address(&self.bind)?,
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                device_id: number(&self.device_id, Field::DeviceId, "Device ID", 0, u16::MAX)?,
                run: self.run,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_specs_survive_their_forms() {
        let client = SecsGemClientSpec::default();
        let form = SecsGemClientForm::from_spec("Tool", &client);
        assert_eq!(form.build().expect("valid").1, client);
        let simulator = SecsGemSimulatorSpec::default();
        let form = SecsGemSimulatorForm::from_spec("Tool", &simulator);
        assert_eq!(form.build().expect("valid").1, simulator);
    }

    #[test]
    fn ids_take_numbers_and_ranges() {
        assert_eq!(
            parse_ids("100-102, 300 301 100", Field::Events, "event").expect("valid"),
            [100, 101, 102, 300, 301]
        );
        assert!(
            parse_ids("", Field::Events, "event")
                .expect("none is fine")
                .is_empty()
        );
        for bad in ["abc", "9-3", "1-", "4294967296", "-5"] {
            assert_eq!(
                parse_ids(bad, Field::SvIds, "status variable id")
                    .expect_err(bad)
                    .field,
                Field::SvIds,
                "{bad}"
            );
        }
        assert!(parse_ids("1-1000", Field::EcIds, "constant id").is_err());
    }

    #[test]
    fn a_client_form_names_the_field_that_is_wrong() {
        let mut form = SecsGemClientForm::from_spec("Tool", &SecsGemClientSpec::default());
        form.host = " ".into();
        assert_eq!(form.build().expect_err("host").field, Field::Host);
        form.host = "10.0.0.4".into();
        form.device_id = "70000".into();
        assert_eq!(form.build().expect_err("device").field, Field::DeviceId);
        form.device_id = "1".into();
        form.events = "x".into();
        assert_eq!(form.build().expect_err("events").field, Field::Events);
    }

    #[test]
    fn the_simulator_listens_on_an_address() {
        let mut form = SecsGemSimulatorForm::from_spec("Tool", &SecsGemSimulatorSpec::default());
        form.bind = "localhost".into();
        assert_eq!(form.build().expect_err("not an address").field, Field::Bind);
    }
}
