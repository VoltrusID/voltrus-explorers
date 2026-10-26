//! Siemens S7 client and CPU simulator forms.

use vt_protocol::{
    S7_MAX_PDU, S7_MIN_PDU, S7Address, S7Area, S7ClientSpec, S7ServerSpec, S7TagSpec, S7Type,
    S7Width,
};

use super::{Field, FormError, error, name, number};

/// Most tags one connection polls.
pub const MAX_S7_TAGS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S7TagField {
    Name,
    Address,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7TagForm {
    pub name: String,
    pub address: String,
    pub data_type: S7Type,
    /// Not edited in the form; kept so template units survive an edit.
    pub unit: String,
}

impl S7TagForm {
    /// The types the typed address can be read as; every type while the
    /// address does not parse yet.
    pub fn types(&self) -> &'static [S7Type] {
        self.address
            .parse::<S7Address>()
            .map_or(S7Type::ALL, |address| address.types())
    }

    /// After the address changed: keep the type if it still fits, else take
    /// the address's usual one.
    pub fn fit_type(&mut self) {
        let types = self.types();
        if !types.contains(&self.data_type)
            && let Some(first) = types.first()
        {
            self.data_type = *first;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub rack: String,
    pub slot: String,
    pub pdu_size: String,
    pub poll_ms: String,
    pub timeout_ms: String,
    pub tags: Vec<S7TagForm>,
}

impl S7ClientForm {
    pub fn from_spec(name: &str, spec: &S7ClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            rack: spec.rack.to_string(),
            slot: spec.slot.to_string(),
            pdu_size: spec.pdu_size.to_string(),
            poll_ms: spec.poll_ms.to_string(),
            timeout_ms: spec.timeout_ms.to_string(),
            tags: spec
                .tags
                .iter()
                .map(|tag| S7TagForm {
                    name: tag.name.clone(),
                    address: tag.address.to_string(),
                    data_type: tag.data_type,
                    unit: tag.unit.clone(),
                })
                .collect(),
        }
    }

    pub fn build(&self) -> Result<(String, S7ClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the PLC's IP address or host name",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let rack = number(&self.rack, Field::S7Rack, "Rack", 0, 7)?;
        let slot = number(&self.slot, Field::S7Slot, "Slot", 0, 31)?;
        let pdu_size = number(
            &self.pdu_size,
            Field::S7Pdu,
            "PDU size",
            S7_MIN_PDU,
            S7_MAX_PDU,
        )?;
        let poll_ms = number(&self.poll_ms, Field::PollMs, "Poll interval", 20, 3_600_000)?;
        let timeout_ms = number(&self.timeout_ms, Field::TimeoutMs, "Timeout", 100, 60_000)?;
        if self.tags.is_empty() {
            return Err(error(
                Field::S7Tags,
                "Add at least one tag to read, e.g. DB1.DBD0",
            ));
        }
        if self.tags.len() > MAX_S7_TAGS {
            return Err(error(
                Field::S7Tags,
                format!("A connection reads at most {MAX_S7_TAGS} tags"),
            ));
        }
        let mut tags = Vec::with_capacity(self.tags.len());
        for (index, row) in self.tags.iter().enumerate() {
            let address: S7Address = row
                .address
                .parse()
                .map_err(|problem| error(Field::S7Tag(index, S7TagField::Address), problem))?;
            if !address.types().contains(&row.data_type) {
                return Err(error(
                    Field::S7Tag(index, S7TagField::Type),
                    format!(
                        "{address} holds {}, not {}",
                        address
                            .types()
                            .iter()
                            .map(|data_type| data_type.name())
                            .collect::<Vec<_>>()
                            .join(" or "),
                        row.data_type.name()
                    ),
                ));
            }
            let label = row.name.trim();
            tags.push(S7TagSpec {
                name: if label.is_empty() {
                    address.to_string()
                } else {
                    label.to_owned()
                },
                address,
                data_type: row.data_type,
                unit: row.unit.clone(),
            });
        }
        Ok((
            name,
            S7ClientSpec {
                host: host.to_owned(),
                port,
                rack,
                slot,
                pdu_size,
                poll_ms,
                timeout_ms,
                tags,
            },
        ))
    }
}

/// A blank row for a new tag, continuing after the last one: the next bit,
/// or the next byte, word or double word of the same area.
pub fn next_s7_tag(tags: &[S7TagForm]) -> S7TagForm {
    let Some(last) = tags.last() else {
        return S7TagForm {
            name: String::new(),
            address: "DB1.DBD0".into(),
            data_type: S7Type::Real,
            unit: String::new(),
        };
    };
    let next = last.address.parse::<S7Address>().ok().and_then(|address| {
        let mut next = address;
        if address.width == S7Width::Bit && address.bit < 7 {
            next.bit += 1;
        } else if address.width == S7Width::Bit {
            next.bit = 0;
            next.offset = address.offset.checked_add(1)?;
        } else {
            // Strings take 256 bytes (String[254]); timers and counters are numbered.
            let step = match (last.data_type, address.area) {
                (S7Type::String, _) => 256,
                (_, S7Area::Timer | S7Area::Counter) => 1,
                _ => address.width.bytes(),
            };
            next.offset = address.offset.checked_add(step)?;
        }
        next.end().map(|_| next)
    });
    S7TagForm {
        name: String::new(),
        address: next.map_or_else(|| last.address.clone(), |next| next.to_string()),
        data_type: last.data_type,
        unit: String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ServerForm {
    pub name: String,
    pub port: String,
    pub template: String,
    pub pdu_size: String,
}

impl S7ServerForm {
    pub fn from_spec(name: &str, spec: &S7ServerSpec) -> Self {
        Self {
            name: name.to_owned(),
            port: spec.port.to_string(),
            template: spec.template.clone(),
            pdu_size: spec.pdu_size.to_string(),
        }
    }

    pub fn build(&self) -> Result<(String, S7ServerSpec), FormError> {
        let name = name(&self.name)?;
        Ok((
            name,
            S7ServerSpec {
                port: number(&self.port, Field::Port, "Port", 1, u16::MAX)?,
                template: self.template.clone(),
                pdu_size: number(
                    &self.pdu_size,
                    Field::S7Pdu,
                    "PDU size",
                    S7_MIN_PDU,
                    S7_MAX_PDU,
                )?,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> S7ClientForm {
        S7ClientForm::from_spec("PLC", &S7ClientSpec::default())
    }

    fn row(address: &str, data_type: S7Type) -> S7TagForm {
        S7TagForm {
            name: String::new(),
            address: address.into(),
            data_type,
            unit: String::new(),
        }
    }

    #[test]
    fn a_default_form_round_trips_to_the_same_spec() {
        let (name, spec) = form().build().unwrap();
        assert_eq!(name, "PLC");
        assert_eq!(spec, S7ClientSpec::default());
        let server = S7ServerForm::from_spec("CPU", &S7ServerSpec::default());
        assert_eq!(server.build().unwrap().1, S7ServerSpec::default());
    }

    #[test]
    fn errors_point_at_the_offending_field() {
        let mut bad = form();
        bad.slot = "32".into();
        assert_eq!(bad.build().unwrap_err().field, Field::S7Slot);

        let mut bad = form();
        bad.rack = "8".into();
        assert_eq!(bad.build().unwrap_err().field, Field::S7Rack);

        let mut bad = form();
        bad.pdu_size = "1920".into();
        let error = bad.build().unwrap_err();
        assert_eq!(error.field, Field::S7Pdu);
        assert_eq!(error.message, "PDU size must be from 240 to 960");

        let mut bad = form();
        bad.host = "  ".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Host);

        let mut bad = form();
        bad.tags.push(row("DB1.DBX4", S7Type::Bool));
        let error = bad.build().unwrap_err();
        assert_eq!(error.field, Field::S7Tag(1, S7TagField::Address));
        assert!(error.message.contains("DB1.DBX4.0"), "{}", error.message);

        let mut bad = form();
        bad.tags.push(row("MW4", S7Type::Real));
        let error = bad.build().unwrap_err();
        assert_eq!(error.field, Field::S7Tag(1, S7TagField::Type));
        assert_eq!(error.message, "MW4 holds Int or Word, not Real");

        let mut bad = form();
        bad.tags.clear();
        assert_eq!(bad.build().unwrap_err().field, Field::S7Tags);

        let mut bad = S7ServerForm::from_spec("CPU", &S7ServerSpec::default());
        bad.port = "0".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Port);
    }

    #[test]
    fn addresses_are_saved_in_their_canonical_form_and_unnamed_tags_take_them() {
        let mut typed = form();
        typed.tags = vec![row(" %db2.dbx3.1 ", S7Type::Bool)];
        let (_, spec) = typed.build().unwrap();
        assert_eq!(spec.tags[0].address.to_string(), "DB2.DBX3.1");
        assert_eq!(spec.tags[0].name, "DB2.DBX3.1");
    }

    #[test]
    fn a_new_address_keeps_a_type_that_fits_and_else_takes_the_usual_one() {
        let mut tag = row("MW4", S7Type::Word);
        tag.address = "MW6".into();
        tag.fit_type();
        assert_eq!(tag.data_type, S7Type::Word);
        tag.address = "MD6".into();
        tag.fit_type();
        assert_eq!(tag.data_type, S7Type::Real);
        tag.address = "MD".into();
        tag.fit_type();
        assert_eq!(
            tag.data_type,
            S7Type::Real,
            "half-typed addresses change nothing"
        );
        assert_eq!(tag.types(), S7Type::ALL);
    }

    #[test]
    fn new_tags_continue_after_the_last_one() {
        assert_eq!(next_s7_tag(&[]).address, "DB1.DBD0");
        for (last, data_type, next) in [
            ("DB1.DBD0", S7Type::Real, "DB1.DBD4"),
            ("MW10", S7Type::Int, "MW12"),
            ("I0.6", S7Type::Bool, "I0.7"),
            ("Q0.7", S7Type::Bool, "Q1.0"),
            ("DB5.DBB0", S7Type::String, "DB5.DBB256"),
            ("T3", S7Type::Timer, "T4"),
            ("DB1.DBD65532", S7Type::Real, "DB1.DBD65532"),
            ("garbage", S7Type::Word, "garbage"),
        ] {
            let tag = next_s7_tag(&[row(last, data_type)]);
            assert_eq!(tag.address, next, "after {last}");
            assert_eq!(tag.data_type, data_type);
        }
    }
}
