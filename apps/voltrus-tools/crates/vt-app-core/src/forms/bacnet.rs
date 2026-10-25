//! BACnet/IP client and device simulator forms.

use vt_protocol::{BacnetClientSpec, BacnetDeviceSpec};

use super::{Field, FormError, error, name, number};

/// Highest device instance a device may have: 4194303 is the wildcard that
/// addresses whichever device receives the request (ASHRAE 135 clause 12.11.1).
const MAX_INSTANCE: u32 = 4_194_302;

/// A local address as typed, bare or in brackets (`[::]`), kept bare.
fn bind_address(text: &str) -> Result<String, FormError> {
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
                "Bind to an IP address: 0.0.0.0 for every interface, or one interface's address",
            )
        })
}

fn instance(text: &str) -> Result<u32, FormError> {
    number(
        text,
        Field::DeviceInstance,
        "Device instance",
        0,
        MAX_INSTANCE,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacnetClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    /// Empty: find the instance with a Who-Is when connecting.
    pub device_instance: String,
    pub bind: String,
    pub local_port: String,
    pub poll_ms: String,
    pub cov: bool,
    pub priority: String,
}

impl BacnetClientForm {
    pub fn from_spec(name: &str, spec: &BacnetClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            device_instance: spec
                .device_instance
                .map(|instance| instance.to_string())
                .unwrap_or_default(),
            bind: spec.bind.clone(),
            local_port: spec.local_port.to_string(),
            poll_ms: spec.poll_ms.to_string(),
            cov: spec.cov,
            priority: spec.priority.to_string(),
        }
    }

    pub fn build(&self) -> Result<(String, BacnetClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the device's IP address or host name, or pick a discovered device",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let device_instance = match self.device_instance.trim() {
            "" => None,
            text => Some(instance(text)?),
        };
        let bind = bind_address(&self.bind)?;
        let local_port = number(
            &self.local_port,
            Field::LocalPort,
            "Local port",
            0,
            u16::MAX,
        )?;
        let poll_ms = number(
            &self.poll_ms,
            Field::PollMs,
            "Poll interval",
            100,
            3_600_000,
        )?;
        let priority = number(&self.priority, Field::Priority, "Write priority", 1, 16)?;
        if !BacnetClientSpec::valid_priority(priority) {
            return Err(error(
                Field::Priority,
                "Priority 6 is reserved for minimum on/off time",
            ));
        }
        Ok((
            name,
            BacnetClientSpec {
                host: host.to_owned(),
                port,
                device_instance,
                bind,
                local_port,
                poll_ms,
                cov: self.cov,
                priority,
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacnetDeviceForm {
    pub name: String,
    pub port: String,
    pub device_instance: String,
    pub template: String,
}

impl BacnetDeviceForm {
    pub fn from_spec(name: &str, spec: &BacnetDeviceSpec) -> Self {
        Self {
            name: name.to_owned(),
            port: spec.port.to_string(),
            device_instance: spec.device_instance.to_string(),
            template: spec.template.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, BacnetDeviceSpec), FormError> {
        let name = name(&self.name)?;
        Ok((
            name,
            BacnetDeviceSpec {
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                device_instance: instance(&self.device_instance)?,
                template: self.template.clone(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> BacnetClientForm {
        BacnetClientForm::from_spec("AHU", &BacnetClientSpec::default())
    }

    #[test]
    fn default_forms_round_trip_to_the_same_spec() {
        let (name, spec) = client().build().unwrap();
        assert_eq!(name, "AHU");
        assert_eq!(spec, BacnetClientSpec::default());
        let device = BacnetDeviceForm::from_spec("Sim", &BacnetDeviceSpec::default());
        assert_eq!(device.build().unwrap().1, BacnetDeviceSpec::default());
    }

    #[test]
    fn a_blank_instance_means_discover_it() {
        let mut form = client();
        form.device_instance = "  ".into();
        assert_eq!(form.build().unwrap().1.device_instance, None);
        form.device_instance = "0".into();
        assert_eq!(form.build().unwrap().1.device_instance, Some(0));
        form.device_instance = "4194302".into();
        assert_eq!(form.build().unwrap().1.device_instance, Some(4_194_302));
        // The wildcard is not a device's own instance.
        form.device_instance = "4194303".into();
        assert_eq!(form.build().unwrap_err().field, Field::DeviceInstance);
        form.device_instance = "AHU".into();
        assert_eq!(form.build().unwrap_err().field, Field::DeviceInstance);
    }

    #[test]
    fn writes_use_a_priority_from_1_to_16_but_not_6() {
        let mut form = client();
        for (priority, ok) in [
            ("1", true),
            ("16", true),
            ("6", false),
            ("0", false),
            ("17", false),
        ] {
            form.priority = priority.into();
            match form.build() {
                Ok((_, spec)) => {
                    assert!(ok, "{priority} accepted");
                    assert_eq!(spec.priority.to_string(), priority);
                }
                Err(error) => {
                    assert!(!ok, "{priority} refused");
                    assert_eq!(error.field, Field::Priority);
                }
            }
        }
    }

    #[test]
    fn the_client_binds_an_ip_and_any_local_port() {
        let mut form = client();
        form.bind = " [::] ".into();
        form.local_port = "47808".into();
        let (_, spec) = form.build().expect("IPv6 any, the BACnet port");
        assert_eq!((spec.bind.as_str(), spec.local_port), ("::", 47808));
        form.local_port = "0".into();
        assert_eq!(form.build().unwrap().1.local_port, 0, "0 takes a free port");
        form.bind = "en0".into();
        assert_eq!(form.build().unwrap_err().field, Field::Bind);
        form.bind = "0.0.0.0".into();
        form.host = " ".into();
        assert_eq!(form.build().unwrap_err().field, Field::Host);
    }

    #[test]
    fn simulators_need_a_port_and_an_instance() {
        let mut device = BacnetDeviceForm::from_spec("Sim", &BacnetDeviceSpec::default());
        device.port = "0".into();
        assert_eq!(device.build().unwrap_err().field, Field::Port);
        device.port = "47809".into();
        device.device_instance = "5000000".into();
        assert_eq!(device.build().unwrap_err().field, Field::DeviceInstance);
    }
}
