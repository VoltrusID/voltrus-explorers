//! MQTT: a client session on `mt-mqtt`'s client, and a broker simulator that
//! publishes a device template (plain topics, a JSON nameplate and,
//! optionally, a Sparkplug B edge node).

mod broker;
pub(crate) mod client;
pub(crate) mod simulator;
mod values;

use std::net::IpAddr;

use rumqttc::mqttbytes::QoS;
use vt_protocol::MqttQos;

pub(crate) fn wire_qos(qos: MqttQos) -> QoS {
    match qos {
        MqttQos::AtMostOnce => QoS::AtMostOnce,
        MqttQos::AtLeastOnce => QoS::AtLeastOnce,
        MqttQos::ExactlyOnce => QoS::ExactlyOnce,
    }
}

pub(crate) fn qos_name(qos: QoS) -> &'static str {
    match qos {
        QoS::AtMostOnce => "QoS 0",
        QoS::AtLeastOnce => "QoS 1",
        QoS::ExactlyOnce => "QoS 2",
    }
}

/// A value as a payload: up to three decimals, no trailing zeros.
pub(crate) fn format_number(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "" | "-" | "-0" => "0".to_owned(),
        text => text.to_owned(),
    }
}

/// What the traffic monitor shows of a payload: its text, or hex when it is
/// binary, on one line.
pub(crate) fn preview(payload: &[u8]) -> String {
    const SHOWN: usize = 160;
    match std::str::from_utf8(payload) {
        Ok(text) => {
            let flat: String = text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(SHOWN)
                .collect();
            if text.chars().count() > SHOWN {
                format!("{flat}…")
            } else {
                flat
            }
        }
        Err(_) => values::preview_hex(payload),
    }
}

/// The listen address of `bind`, which may be a bracketed IPv6 address.
pub(crate) fn listen_ip(bind: &str) -> Option<IpAddr> {
    let bind = bind.trim();
    bind.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(bind)
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_publish_without_noise() {
        assert_eq!(format_number(230.0), "230");
        assert_eq!(format_number(49.987_654), "49.988");
        assert_eq!(format_number(0.000_1), "0");
        assert_eq!(format_number(-0.000_1), "0");
        assert_eq!(format_number(-12.5), "-12.5");
        assert_eq!(format_number(1_234_567.25), "1234567.25");
    }

    #[test]
    fn previews_stay_on_one_line() {
        assert_eq!(preview(b"{\n  \"a\": 1\n}"), "{   \"a\": 1 }");
        assert_eq!(preview(&[0xC3, 0x28]), "C3 28");
        let long = "x".repeat(200);
        assert_eq!(preview(long.as_bytes()).chars().count(), 161);
    }
}
