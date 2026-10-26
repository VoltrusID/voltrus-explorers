//! NATS: a client session on `mt-nats`'s client (core NATS, request/reply
//! and JetStream browsing) and a server simulator on `mt-nats`'s embedded
//! server that publishes a device template.

pub(crate) mod client;
pub(crate) mod server;

use vt_protocol::{NatsHeader, TrafficFrame};

use crate::emit::now_ms;

/// Longest payload text the inspector and the JetStream browser show.
const MAX_TEXT: usize = 2048;

pub(crate) fn frame(
    direction: vt_protocol::Direction,
    summary: String,
    detail: String,
    bytes: usize,
) -> TrafficFrame {
    TrafficFrame {
        at_ms: now_ms(),
        direction,
        summary,
        detail,
        bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
    }
}

/// `host` as a URL host: an IPv6 address goes in brackets.
pub(crate) fn url_host(host: &str) -> String {
    let host = host.trim();
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// A payload as text for display: UTF-8, or hex when binary, clipped.
pub(crate) fn payload_text(payload: &[u8]) -> String {
    match std::str::from_utf8(payload) {
        Ok(text) => clip(text),
        Err(_) => {
            let mut hex = String::with_capacity(payload.len().min(MAX_TEXT / 3) * 3 + 8);
            hex.push_str("binary ");
            for (index, byte) in payload.iter().take(MAX_TEXT / 3).enumerate() {
                if index > 0 {
                    hex.push(' ');
                }
                hex.push_str(&format!("{byte:02X}"));
            }
            if payload.len() > MAX_TEXT / 3 {
                hex.push_str(" …");
            }
            hex
        }
    }
}

/// `text` cut to what the inspector shows.
pub(crate) fn clip(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_owned();
    }
    let mut end = MAX_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Headers as `async_nats` takes them.
pub(crate) fn header_map(headers: &[NatsHeader]) -> async_nats::HeaderMap {
    let mut map = async_nats::HeaderMap::new();
    for header in headers {
        map.append(header.name.as_str(), header.value.as_str());
    }
    map
}

/// Headers of a received message, in a stable order.
pub(crate) fn headers_of(map: Option<&async_nats::HeaderMap>) -> Vec<NatsHeader> {
    let mut headers: Vec<NatsHeader> = map
        .map(|map| {
            map.iter()
                .flat_map(|(name, values)| {
                    values.iter().map(move |value| NatsHeader {
                        name: name.to_string(),
                        value: value.as_str().to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    headers.sort_by(|a, b| a.name.cmp(&b.name));
    headers
}

/// `Name: value` lines, for the inspector and traffic.
pub(crate) fn header_lines(headers: &[NatsHeader]) -> String {
    headers
        .iter()
        .map(|header| format!("{}: {}", header.name, header.value))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_payloads_show_as_hex_and_text_is_clipped() {
        assert_eq!(payload_text(b"42"), "42");
        assert_eq!(payload_text(&[0xC3, 0x28, 0x00]), "binary C3 28 00");
        let long = "é".repeat(MAX_TEXT);
        let shown = payload_text(long.as_bytes());
        assert!(shown.ends_with('…'));
        assert!(shown.len() <= MAX_TEXT + '…'.len_utf8());
        let many = vec![0xFF; MAX_TEXT];
        assert!(payload_text(&many).ends_with(" …"));
    }

    #[test]
    fn ipv6_hosts_are_bracketed_once() {
        assert_eq!(url_host("::1"), "[::1]");
        assert_eq!(url_host("[::1]"), "[::1]");
        assert_eq!(url_host(" nats.local "), "nats.local");
    }

    #[test]
    fn headers_survive_a_map_round_trip_in_name_order() {
        let headers = vec![
            NatsHeader {
                name: "X-Unit".into(),
                value: "kW".into(),
            },
            NatsHeader {
                name: "Nats-Msg-Id".into(),
                value: "42".into(),
            },
        ];
        let back = headers_of(Some(&header_map(&headers)));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "Nats-Msg-Id");
        assert_eq!(header_lines(&back), "Nats-Msg-Id: 42\nX-Unit: kW");
    }
}
