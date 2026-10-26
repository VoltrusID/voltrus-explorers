//! The BLE client form.

use vt_protocol::{BleAdapter, BleClientSpec, BleDecoderRule};

use super::{Field, FormError, error, name};

/// Longest poll interval: an hour.
const MAX_POLL_MS: u64 = 3_600_000;
/// Shortest poll interval: a BLE link carries little, and every read is a
/// round trip.
const MIN_POLL_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct BleClientForm {
    pub name: String,
    pub adapter: BleAdapter,
    /// The peripheral's identifier, picked from a scan or typed.
    pub peripheral: String,
    /// The name the picked peripheral advertised, to find it again by.
    pub advertised: String,
    /// `0` reads once after connecting.
    pub poll_ms: String,
    pub notify: bool,
    /// Not edited in the form; kept so chosen decoders survive an edit.
    pub decoders: Vec<BleDecoderRule>,
}

impl BleClientForm {
    pub fn from_spec(name: &str, spec: &BleClientSpec) -> Self {
        Self {
            name: name.to_owned(),
            adapter: spec.adapter,
            peripheral: spec.peripheral.clone(),
            advertised: spec.name.clone(),
            poll_ms: spec.poll_ms.to_string(),
            notify: spec.notify,
            decoders: spec.decoders.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, BleClientSpec), FormError> {
        let name = name(&self.name)?;
        let peripheral = self.peripheral.trim();
        if peripheral.is_empty() {
            return Err(error(
                Field::Peripheral,
                "Scan and pick a peripheral, or enter its identifier",
            ));
        }
        let poll_ms: u64 = self.poll_ms.trim().parse().map_err(|_| {
            error(
                Field::PollMs,
                "Poll interval must be a number of milliseconds, or 0 to read once",
            )
        })?;
        if poll_ms != 0 && !(MIN_POLL_MS..=MAX_POLL_MS).contains(&poll_ms) {
            return Err(error(
                Field::PollMs,
                format!(
                    "Poll interval must be 0 (read once) or from {MIN_POLL_MS} to {MAX_POLL_MS} ms"
                ),
            ));
        }
        Ok((
            name,
            BleClientSpec {
                adapter: self.adapter,
                peripheral: peripheral.to_owned(),
                name: self.advertised.trim().to_owned(),
                poll_ms,
                notify: self.notify,
                decoders: self.decoders.clone(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use vt_protocol::{BleDecoder, BleFormat};

    use super::*;

    fn form() -> BleClientForm {
        let mut spec = BleClientSpec {
            adapter: BleAdapter::Simulated,
            peripheral: "6F1D2C3B-5A49-4E8F-9C1B-2D3E4F5A6B01".into(),
            name: "Voltrus HRM".into(),
            ..BleClientSpec::default()
        };
        spec.set_decoder("7d2e0002", BleDecoder::of(BleFormat::F32Le));
        BleClientForm::from_spec("Heart rate", &spec)
    }

    #[test]
    fn a_form_builds_the_spec_it_came_from() {
        let (name, spec) = form().build().unwrap();
        assert_eq!(name, "Heart rate");
        assert_eq!(spec.adapter, BleAdapter::Simulated);
        assert_eq!(spec.name, "Voltrus HRM");
        assert_eq!(spec.poll_ms, BleClientSpec::default().poll_ms);
        // Decoders chosen in the inspector survive an edit of the connection.
        assert_eq!(
            spec.decoder("7d2e0002").map(|decoder| decoder.format),
            Some(BleFormat::F32Le)
        );
    }

    #[test]
    fn a_peripheral_is_required() {
        let mut form = form();
        form.peripheral = "  ".into();
        assert_eq!(form.build().unwrap_err().field, Field::Peripheral);
        form.peripheral = " AA:BB:CC:DD:EE:FF ".into();
        assert_eq!(form.build().unwrap().1.peripheral, "AA:BB:CC:DD:EE:FF");
        form.name = " ".into();
        assert_eq!(form.build().unwrap_err().field, Field::Name);
    }

    #[test]
    fn polling_is_off_or_between_100_ms_and_an_hour() {
        let mut form = form();
        for (text, ok) in [
            ("0", true),
            ("100", true),
            ("3600000", true),
            ("99", false),
            ("3600001", false),
            ("-1", false),
            ("fast", false),
            ("", false),
        ] {
            form.poll_ms = text.into();
            match form.build() {
                Ok((_, spec)) => {
                    assert!(ok, "{text} accepted");
                    assert_eq!(spec.poll_ms.to_string(), text);
                }
                Err(error) => {
                    assert!(!ok, "{text} refused");
                    assert_eq!(error.field, Field::PollMs);
                }
            }
        }
    }
}
