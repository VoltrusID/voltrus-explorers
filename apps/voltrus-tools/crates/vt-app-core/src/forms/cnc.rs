//! MTConnect and FANUC client and simulator forms.

use vt_protocol::{
    FOCAS_MAX_MACROS, FocasClientSpec, FocasProfile, FocasSimulatorSpec, MtconnectAgentSpec,
    MtconnectClientSpec, MtconnectTemplate,
};

use super::{Field, FormError, error, name, number};

/// A local address as typed, bare or in brackets (`[::]`), kept bare.
pub(super) fn bind_address(text: &str) -> Result<String, FormError> {
    let text = text.trim();
    let bare = text
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(text);
    bare.parse::<std::net::IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| {
            error(
                Field::Bind,
                "Listen on an IP address, e.g. 127.0.0.1, 0.0.0.0 or ::",
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtconnectClientForm {
    pub name: String,
    pub url: String,
    pub device: String,
    pub interval_ms: String,
    pub timeout_ms: String,
}

impl MtconnectClientForm {
    pub fn from_spec(name: &str, spec: &MtconnectClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            url: spec.url.clone(),
            device: spec.device.clone(),
            interval_ms: spec.interval_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
        }
    }

    pub fn build(&self) -> Result<(String, MtconnectClientSpec), FormError> {
        let name = name(&self.name)?;
        let typed = self.url.trim();
        if typed.is_empty() {
            return Err(error(
                Field::Endpoint,
                "Enter the agent's address, e.g. http://localhost:5000",
            ));
        }
        // A bare host and port is an http agent.
        let url = if typed.contains("://") {
            typed.to_owned()
        } else {
            format!("http://{typed}")
        };
        if !(url.starts_with("http://") || url.starts_with("https://"))
            || vt_protocol::mtconnect_host_port(&url).is_empty()
        {
            return Err(error(
                Field::Endpoint,
                "The agent's address starts with http:// or https://",
            ));
        }
        Ok((
            name,
            MtconnectClientSpec {
                url: url.trim_end_matches('/').to_owned(),
                device: self.device.trim().to_owned(),
                interval_ms: number(
                    &self.interval_ms,
                    Field::IntervalMs,
                    "Stream interval",
                    50,
                    3_600_000,
                )?,
                timeout_ms: number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 120_000)?,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtconnectAgentForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub template: MtconnectTemplate,
    pub shdr: bool,
    pub shdr_port: String,
}

impl MtconnectAgentForm {
    pub fn from_spec(name: &str, spec: &MtconnectAgentSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            template: spec.template,
            shdr: spec.shdr,
            shdr_port: spec.shdr_port.to_string(),
        }
    }

    pub fn build(&self) -> Result<(String, MtconnectAgentSpec), FormError> {
        let name = name(&self.name)?;
        let bind = bind_address(&self.bind)?;
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let shdr_port = number(&self.shdr_port, Field::ShdrPort, "SHDR port", 1, u16::MAX)?;
        if self.shdr && shdr_port == port {
            return Err(error(
                Field::ShdrPort,
                "The SHDR adapter needs a different port than the agent",
            ));
        }
        Ok((
            name,
            MtconnectAgentSpec {
                bind,
                port,
                template: self.template,
                shdr: self.shdr,
                shdr_port,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocasClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub poll_ms: String,
    pub timeout_ms: String,
    /// Macro variable numbers, separated by commas, spaces or ranges
    /// (`500-503`).
    pub macros: String,
}

impl FocasClientForm {
    pub fn from_spec(name: &str, spec: &FocasClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            poll_ms: spec.poll_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
            macros: spec
                .macros
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    pub fn build(&self) -> Result<(String, FocasClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the CNC's IP address or host name",
            ));
        }
        Ok((
            name,
            FocasClientSpec {
                host: host.to_owned(),
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                poll_ms: number(&self.poll_ms, Field::PollMs, "Poll interval", 50, 3_600_000)?,
                timeout_ms: number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 60_000)?,
                macros: parse_macros(&self.macros)?,
            },
        ))
    }
}

/// Macro variable numbers from text such as `500, 501 510-512`, each once,
/// in the order typed.
pub fn parse_macros(text: &str) -> Result<Vec<u32>, FormError> {
    let bad = |item: &str| {
        error(
            Field::Macros,
            format!(
                "\"{item}\" is not a macro variable number (1 to 99999) or a range like 500-503"
            ),
        )
    };
    let mut numbers: Vec<u32> = Vec::new();
    for item in text.split([',', ' ', ';']).filter(|item| !item.is_empty()) {
        let item = item.trim_start_matches('#');
        let (first, last) = match item.split_once('-') {
            Some((first, last)) => (first, last),
            None => (item, item),
        };
        let (Ok(first), Ok(last)) = (first.parse::<u32>(), last.parse::<u32>()) else {
            return Err(bad(item));
        };
        if !(1..=99_999).contains(&first) || !(first..=99_999).contains(&last) {
            return Err(bad(item));
        }
        for number in first..=last {
            if !numbers.contains(&number) {
                numbers.push(number);
            }
            if numbers.len() > FOCAS_MAX_MACROS {
                return Err(error(
                    Field::Macros,
                    format!("A connection reads at most {FOCAS_MAX_MACROS} macro variables"),
                ));
            }
        }
    }
    Ok(numbers)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocasSimulatorForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub profile: FocasProfile,
    pub run: bool,
}

impl FocasSimulatorForm {
    pub fn from_spec(name: &str, spec: &FocasSimulatorSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            profile: spec.profile,
            run: spec.run,
        }
    }

    pub fn build(&self) -> Result<(String, FocasSimulatorSpec), FormError> {
        Ok((
            name(&self.name)?,
            FocasSimulatorSpec {
                bind: bind_address(&self.bind)?,
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                profile: self.profile,
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
        let client = MtconnectClientSpec::default();
        let form = MtconnectClientForm::from_spec("Agent", &client);
        assert_eq!(form.build().expect("valid").1, client);
        let agent = MtconnectAgentSpec::default();
        let form = MtconnectAgentForm::from_spec("Agent", &agent);
        assert_eq!(form.build().expect("valid").1, agent);
        let cnc = FocasClientSpec::default();
        let form = FocasClientForm::from_spec("CNC", &cnc);
        assert_eq!(form.build().expect("valid").1, cnc);
        let simulator = FocasSimulatorSpec::default();
        let form = FocasSimulatorForm::from_spec("CNC", &simulator);
        assert_eq!(form.build().expect("valid").1, simulator);
    }

    #[test]
    fn an_agent_address_may_leave_out_the_scheme_and_the_trailing_slash() {
        let mut form = MtconnectClientForm::from_spec("Agent", &MtconnectClientSpec::default());
        form.url = "demo.mtconnect.org:80/".into();
        assert_eq!(
            form.build().expect("valid").1.url,
            "http://demo.mtconnect.org:80"
        );
        form.url = "ftp://agent".into();
        assert_eq!(form.build().expect_err("not http").field, Field::Endpoint);
        form.url = " ".into();
        assert_eq!(form.build().expect_err("empty").field, Field::Endpoint);
    }

    #[test]
    fn the_shdr_adapter_needs_its_own_port() {
        let mut form = MtconnectAgentForm::from_spec("Agent", &MtconnectAgentSpec::default());
        form.shdr = true;
        form.shdr_port = form.port.clone();
        assert_eq!(form.build().expect_err("same port").field, Field::ShdrPort);
        form.shdr = false;
        assert!(form.build().is_ok(), "ignored while the adapter is off");
    }

    #[test]
    fn macro_variables_take_numbers_and_ranges() {
        assert_eq!(
            parse_macros("500, #501 510-512 500").expect("valid"),
            [500, 501, 510, 511, 512]
        );
        assert!(parse_macros("").expect("none is fine").is_empty());
        for bad in ["abc", "0", "100000", "9-3", "1-"] {
            assert_eq!(
                parse_macros(bad).expect_err(bad).field,
                Field::Macros,
                "{bad}"
            );
        }
        assert!(parse_macros("1-100").is_err(), "more than the limit");
    }

    #[test]
    fn the_simulators_listen_on_an_address() {
        let mut form = FocasSimulatorForm::from_spec("CNC", &FocasSimulatorSpec::default());
        form.bind = "[::1]".into();
        assert_eq!(form.build().expect("valid").1.bind, "::1");
        form.bind = "localhost".into();
        assert_eq!(form.build().expect_err("not an address").field, Field::Bind);
        form.bind = "0.0.0.0".into();
        form.port = "0".into();
        assert_eq!(form.build().expect_err("port").field, Field::Port);
    }
}
