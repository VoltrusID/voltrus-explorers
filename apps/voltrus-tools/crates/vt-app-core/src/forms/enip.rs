//! EtherNet/IP client and adapter simulator forms, and the inspector's
//! explicit message.

use vt_protocol::{CipRequest, EnipAdapterSpec, EnipAddress, EnipClientSpec, EnipPoint};

use super::{Field, FormError, error, name, number};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnipPointField {
    Name,
    Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnipPointForm {
    pub name: String,
    pub address: String,
    /// Not edited in the form; kept so template units survive an edit.
    pub unit: String,
}

impl EnipPointForm {
    pub fn blank() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            unit: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnipClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub route_path: String,
    pub poll_ms: String,
    /// Rows left blank are dropped; no rows reads every tag.
    pub points: Vec<EnipPointForm>,
}

impl EnipClientForm {
    pub fn from_spec(name: &str, spec: &EnipClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            route_path: spec.route_path.clone(),
            poll_ms: spec.poll_ms.to_string(),
            points: spec
                .points
                .iter()
                .map(|point| EnipPointForm {
                    name: point.name.clone(),
                    address: point.address.to_string(),
                    unit: point.unit.clone(),
                })
                .collect(),
        }
    }

    pub fn build(&self) -> Result<(String, EnipClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the device's IP address or host name, or pick a discovered device",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let route_path = self.route_path.trim();
        if let Err(problem) = valid_route_path(route_path) {
            return Err(error(Field::RoutePath, problem));
        }
        let poll_ms = number(&self.poll_ms, Field::PollMs, "Poll interval", 50, 3_600_000)?;
        let mut points = Vec::with_capacity(self.points.len());
        for (index, row) in self.points.iter().enumerate() {
            if row.address.trim().is_empty() && row.name.trim().is_empty() {
                continue;
            }
            let address = EnipAddress::parse(&row.address).map_err(|problem| {
                error(Field::EnipPoint(index, EnipPointField::Address), problem)
            })?;
            points.push(EnipPoint {
                name: row.name.trim().to_owned(),
                address,
                unit: row.unit.clone(),
            });
        }
        Ok((
            name,
            EnipClientSpec {
                host: host.to_owned(),
                port,
                route_path: route_path.to_owned(),
                poll_ms,
                points,
            },
        ))
    }
}

/// A route path as `mt-enip` takes it: `port,link` pairs separated by `,`
/// or `/`, a link being a slot number or an IPv4 address, e.g. `1,0` or
/// `1,2,2,10.0.0.5,1,0`. Empty routes nothing.
pub fn valid_route_path(route: &str) -> Result<(), String> {
    if route.is_empty() {
        return Ok(());
    }
    let hops: Vec<&str> = route.split([',', '/']).map(str::trim).collect();
    if hops.len() % 2 != 0 || hops.iter().any(|hop| hop.is_empty()) {
        return Err("Write the route as port,slot pairs, e.g. 1,0 (backplane, slot 0)".into());
    }
    for pair in hops.chunks(2) {
        if !pair[0]
            .parse::<u16>()
            .is_ok_and(|port| (1..=0x7FFF).contains(&port))
        {
            return Err(format!(
                "“{}” is not a port number, e.g. 1 for the backplane",
                pair[0]
            ));
        }
        let link = pair[1];
        if link.parse::<u8>().is_err() && link.parse::<std::net::Ipv4Addr>().is_err() {
            return Err(format!("“{link}” is not a slot number or an IPv4 address"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnipAdapterForm {
    pub name: String,
    pub port: String,
    pub template: String,
}

impl EnipAdapterForm {
    pub fn from_spec(name: &str, spec: &EnipAdapterSpec) -> Self {
        Self {
            name: name.to_owned(),
            port: spec.port.to_string(),
            template: spec.template.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, EnipAdapterSpec), FormError> {
        let name = name(&self.name)?;
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        Ok((
            name,
            EnipAdapterSpec {
                port,
                template: self.template.clone(),
            },
        ))
    }
}

/// The inspector's explicit message, as typed: numbers in decimal or `0x`
/// hex, the data as hex bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CipRequestForm {
    pub service: String,
    pub class: String,
    pub instance: String,
    /// Empty addresses the instance, not one attribute.
    pub attribute: String,
    pub data: String,
}

impl CipRequestForm {
    pub fn build(&self) -> Result<CipRequest, String> {
        Ok(CipRequest {
            service: parse_number(&self.service, "Service", 0x7F)?
                .try_into()
                .map_err(|_| "Service must be from 0x00 to 0x7F".to_owned())?,
            class: parse_number(&self.class, "Class", u64::from(u16::MAX))?
                .try_into()
                .map_err(|_| "Class is out of range".to_owned())?,
            instance: parse_number(&self.instance, "Instance", u64::from(u32::MAX))?
                .try_into()
                .map_err(|_| "Instance is out of range".to_owned())?,
            attribute: match self.attribute.trim() {
                "" => None,
                text => Some(
                    parse_number(text, "Attribute", u64::from(u16::MAX))?
                        .try_into()
                        .map_err(|_| "Attribute is out of range".to_owned())?,
                ),
            },
            data: parse_hex(&self.data)?,
        })
    }
}

fn parse_number(text: &str, what: &str, max: u64) -> Result<u64, String> {
    let text = text.trim();
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    match parsed {
        Ok(value) if value <= max => Ok(value),
        Ok(_) => Err(format!("{what} must be at most {max} (0x{max:X})")),
        Err(_) if text.is_empty() => Err(format!("Enter the {}", what.to_lowercase())),
        Err(_) => Err(format!("{what} “{text}” is not a number: write 14 or 0x0E")),
    }
}

/// Hex bytes, with or without spaces between them: `01 02 0A`, `01020a`.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    let digits = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
        .unwrap_or(&digits);
    if let Some(bad) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(format!("“{bad}” is not a hex digit"));
    }
    if digits.len() % 2 != 0 {
        return Err("The data needs whole bytes: two hex digits each".into());
    }
    (0..digits.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&digits[at..at + 2], 16)
                .map_err(|_| format!("“{}” is not a hex byte", &digits[at..at + 2]))
        })
        .collect()
}

/// The ways a reply's bytes may read, for the inspector: the numbers their
/// size fits, and a SHORT_STRING or STRING when they spell one.
pub fn interpret_reply(data: &[u8]) -> Vec<(&'static str, String)> {
    let mut readings = Vec::new();
    match *data {
        [a] => {
            readings.push(("USINT", a.to_string()));
            readings.push(("SINT", i8::from_le_bytes([a]).to_string()));
        }
        [a, b] => {
            readings.push(("UINT", u16::from_le_bytes([a, b]).to_string()));
            readings.push(("INT", i16::from_le_bytes([a, b]).to_string()));
        }
        [a, b, c, d] => {
            let bytes = [a, b, c, d];
            readings.push(("UDINT", u32::from_le_bytes(bytes).to_string()));
            readings.push(("DINT", i32::from_le_bytes(bytes).to_string()));
            let real = f32::from_le_bytes(bytes);
            if real.is_finite() {
                readings.push(("REAL", real.to_string()));
            }
        }
        [a, b, c, d, e, f, g, h] => {
            let bytes = [a, b, c, d, e, f, g, h];
            readings.push(("ULINT", u64::from_le_bytes(bytes).to_string()));
            let real = f64::from_le_bytes(bytes);
            if real.is_finite() {
                readings.push(("LREAL", real.to_string()));
            }
        }
        _ => {}
    }
    let printable = |text: &[u8]| !text.is_empty() && text.iter().all(|b| (0x20..0x7F).contains(b));
    if let Some((&len, text)) = data.split_first()
        && usize::from(len) == text.len()
        && printable(text)
    {
        readings.push((
            "SHORT_STRING",
            format!("“{}”", String::from_utf8_lossy(text)),
        ));
    } else if let [lo, hi, text @ ..] = data
        && usize::from(u16::from_le_bytes([*lo, *hi])) == text.len()
        && printable(text)
    {
        readings.push(("STRING", format!("“{}”", String::from_utf8_lossy(text))));
    }
    readings
}

/// Bytes as rows of hex, `per_row` to a row, each with its offset.
pub fn hex_rows(data: &[u8], per_row: usize) -> Vec<(usize, String)> {
    data.chunks(per_row.max(1))
        .enumerate()
        .map(|(row, bytes)| {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
            (row * per_row.max(1), hex.join(" "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_protocol::CipType;

    fn form() -> EnipClientForm {
        EnipClientForm::from_spec("PLC", &EnipClientSpec::default())
    }

    #[test]
    fn a_spec_round_trips_through_the_form() {
        let spec = EnipClientSpec {
            host: "10.0.0.5".into(),
            route_path: "1,0".into(),
            points: vec![
                EnipPoint {
                    name: "Speed".into(),
                    address: EnipAddress::parse("Program:Main.Motor1.Speed").unwrap(),
                    unit: "RPM".into(),
                },
                EnipPoint {
                    name: String::new(),
                    address: EnipAddress::Attribute {
                        class: 4,
                        instance: 100,
                        attribute: 3,
                        offset: 6,
                        data_type: CipType::Int,
                    },
                    unit: String::new(),
                },
            ],
            ..EnipClientSpec::default()
        };
        let (name, built) = EnipClientForm::from_spec("PLC", &spec).build().unwrap();
        assert_eq!(name, "PLC");
        assert_eq!(built, spec);
    }

    #[test]
    fn blank_rows_are_dropped_and_bad_ones_pointed_at() {
        let mut form = form();
        form.points = vec![
            EnipPointForm::blank(),
            EnipPointForm {
                address: " Counter ".into(),
                ..EnipPointForm::blank()
            },
            EnipPointForm {
                name: "Oops".into(),
                address: "@4/100".into(),
                unit: String::new(),
            },
        ];
        let error = form.build().unwrap_err();
        assert_eq!(error.field, Field::EnipPoint(2, EnipPointField::Address));
        assert!(error.message.contains("@class/instance/attribute"));
        form.points.pop();
        let (_, spec) = form.build().unwrap();
        assert_eq!(spec.points.len(), 1);
        assert_eq!(spec.points[0].address.to_string(), "Counter");
        // No points at all reads every tag.
        form.points.clear();
        assert!(form.build().unwrap().1.points.is_empty());
    }

    #[test]
    fn hosts_ports_polls_and_routes_are_checked() {
        let mut bad = form();
        bad.host = " ".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Host);
        let mut bad = form();
        bad.poll_ms = "10".into();
        assert_eq!(bad.build().unwrap_err().field, Field::PollMs);
        for route in ["1,0", "1/3", "1,2,2,10.0.0.5,1,0", " 1 , 0 "] {
            let mut good = form();
            good.route_path = route.into();
            assert!(good.build().is_ok(), "{route}");
        }
        for route in ["1", "1,", "x,0", "1,300", "0,1", "1,10.0.0"] {
            let mut bad = form();
            bad.route_path = route.into();
            assert_eq!(bad.build().unwrap_err().field, Field::RoutePath, "{route}");
        }
    }

    #[test]
    fn adapters_take_a_port() {
        let mut adapter = EnipAdapterForm::from_spec("Sim", &EnipAdapterSpec::default());
        assert_eq!(adapter.build().unwrap().1, EnipAdapterSpec::default());
        adapter.port = "0".into();
        assert_eq!(adapter.build().unwrap_err().field, Field::Port);
    }

    #[test]
    fn replies_read_as_the_types_their_size_fits() {
        assert_eq!(
            interpret_reply(&[0xFE, 0xFF]),
            [("UINT", "65534".to_owned()), ("INT", "-2".to_owned())]
        );
        let real = interpret_reply(&1.5f32.to_le_bytes());
        assert!(real.contains(&("REAL", "1.5".to_owned())), "{real:?}");
        // The Identity object's product name.
        assert_eq!(
            interpret_reply(b"\x08ENIP-Sim"),
            [("SHORT_STRING", "“ENIP-Sim”".to_owned())]
        );
        assert_eq!(
            interpret_reply(b"\x02\x00Hi"),
            [
                ("UDINT", "1766326274".to_owned()),
                ("DINT", "1766326274".to_owned()),
                ("REAL", f32::from_le_bytes(*b"\x02\x00Hi").to_string()),
                ("STRING", "“Hi”".to_owned()),
            ]
        );
        // Nothing to say about odd sizes and binary data.
        assert!(interpret_reply(&[1, 2, 3]).is_empty());
        assert!(interpret_reply(&[]).is_empty());
        assert_eq!(
            hex_rows(&[0, 1, 0xAB, 3, 4], 4),
            [(0, "00 01 AB 03".to_owned()), (4, "04".to_owned())]
        );
    }

    #[test]
    fn explicit_messages_parse_hex_and_decimal() {
        let form = CipRequestForm {
            service: "0x10".into(),
            class: "4".into(),
            instance: "0x96".into(),
            attribute: "3".into(),
            data: "01 02 0a,FF".into(),
        };
        assert_eq!(
            form.build(),
            Ok(CipRequest {
                service: 0x10,
                class: 4,
                instance: 150,
                attribute: Some(3),
                data: vec![1, 2, 0x0A, 0xFF],
            })
        );
        let all = CipRequestForm {
            service: "1".into(),
            attribute: String::new(),
            data: String::new(),
            ..form.clone()
        };
        assert_eq!(all.build().map(|request| request.attribute), Ok(None));
        for (bad, problem) in [
            (
                CipRequestForm {
                    service: "0x8E".into(),
                    ..form.clone()
                },
                "at most 127",
            ),
            (
                CipRequestForm {
                    class: "".into(),
                    ..form.clone()
                },
                "Enter the class",
            ),
            (
                CipRequestForm {
                    instance: "x".into(),
                    ..form.clone()
                },
                "not a number",
            ),
            (
                CipRequestForm {
                    data: "123".into(),
                    ..form.clone()
                },
                "whole bytes",
            ),
            (
                CipRequestForm {
                    data: "zz".into(),
                    ..form.clone()
                },
                "not a hex digit",
            ),
            (
                CipRequestForm {
                    data: "é1".into(),
                    ..form.clone()
                },
                "not a hex digit",
            ),
        ] {
            let error = bad.build().unwrap_err();
            assert!(error.contains(problem), "{error}");
        }
    }
}
