//! One-line descriptions of captured EtherNet/IP frames, for packet monitors:
//! the encapsulation command, and for CIP traffic the service and what it
//! addresses (a tag, or a class/instance/attribute) or how the target answered.

use crate::cip::{
    decode_symbolic_path, encapsulation_status_text, extended_status_text, general_status_text,
    parse_logical_path, parse_unconnected_send, split_multiple_service_data, CipReply,
};
use crate::packets::{parse_cpf, parse_list_identity_response, CPF_CONNECTED_DATA, CPF_UNCONNECTED_DATA};
use crate::types::*;

/// A summary (`Read Tag Motor.Speed`, `Get_Attribute_Single reply · OK`) and
/// a detail line for one encapsulation frame as captured on the TCP session
/// (or a ListIdentity datagram).
pub fn describe_frame(frame: &[u8]) -> (String, String) {
    if frame.len() < ENIP_HEADER_SIZE {
        return ("Truncated frame".into(), format!("{} bytes", frame.len()));
    }
    let command = u16::from_le_bytes([frame[0], frame[1]]);
    let length = u16::from_le_bytes([frame[2], frame[3]]) as usize;
    let session = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
    let status = u32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]);
    let body = frame.get(ENIP_HEADER_SIZE..ENIP_HEADER_SIZE + length).unwrap_or(&frame[ENIP_HEADER_SIZE..]);
    let name = command_name(command);
    if status != 0 {
        return (
            format!("{name} failed"),
            format!("status 0x{status:04X}: {}", encapsulation_status_text(status)),
        );
    }
    match command {
        CMD_REGISTER_SESSION | CMD_UNREGISTER_SESSION if session != 0 => {
            (name.into(), format!("session 0x{session:08X}"))
        }
        CMD_LIST_IDENTITY => match parse_list_identity_response(frame) {
            Some(device) => (
                format!("{name} reply"),
                format!(
                    "{} · {} · rev {}.{} · serial 0x{:08X}",
                    device.product_name, device.vendor_name, device.revision_major, device.revision_minor, device.serial_number
                ),
            ),
            None => (name.into(), String::new()),
        },
        CMD_SEND_RR_DATA | CMD_SEND_UNIT_DATA => {
            let items = body.get(6..).and_then(|cpf| parse_cpf(cpf).ok()).unwrap_or_default();
            let message = items.iter().find_map(|item| match item.type_id {
                CPF_UNCONNECTED_DATA => Some(item.data.as_slice()),
                // Connected data starts with the connection's sequence count.
                CPF_CONNECTED_DATA => item.data.get(2..),
                _ => None,
            });
            match message {
                Some(message) if !message.is_empty() => describe_cip(message),
                _ => (name.into(), format!("{} bytes", body.len())),
            }
        }
        _ => (name.into(), if body.is_empty() { String::new() } else { format!("{} bytes", body.len()) }),
    }
}

/// Describe one message-router request or reply.
pub fn describe_cip(message: &[u8]) -> (String, String) {
    let Some(&service) = message.first() else {
        return ("Empty CIP message".into(), String::new());
    };
    if service & 0x80 != 0 {
        return describe_reply(message);
    }
    let path_words = message.get(1).copied().unwrap_or(0) as usize;
    let Some(path) = message.get(2..2 + path_words * 2) else {
        return (service_name(service, None).into(), "path truncated".into());
    };
    let data = &message[2 + path_words * 2..];
    let (class, instance, attribute) = parse_logical_path(path);
    if class == Some(CONNECTION_MANAGER_CLASS_ID) && service == CIP_UNCONNECTED_SEND {
        if let Some((embedded, route)) = parse_unconnected_send(message) {
            if !embedded.is_empty() {
                let (summary, detail) = describe_cip(embedded);
                let route: Vec<String> = route.iter().map(|b| b.to_string()).collect();
                let routed = format!("routed {}", route.join(","));
                return (summary, if detail.is_empty() { routed } else { format!("{detail} · {routed}") });
            }
        }
    }
    if service == CIP_MULTIPLE_SERVICE_PACKET {
        if let Ok(parts) = split_multiple_service_data(data) {
            let names: Vec<String> = parts.iter().filter(|p| !p.is_empty()).map(|p| describe_cip(p).0).collect();
            return (format!("Multiple_Service_Packet ×{}", parts.len()), names.join(" · "));
        }
    }
    let target = match decode_symbolic_path(path) {
        Some(tag) => tag,
        None => match (class, instance, attribute) {
            (Some(c), Some(i), Some(a)) => format!("0x{c:02X}/{i}/{a}"),
            (Some(c), Some(i), None) => format!("0x{c:02X}/{i}"),
            (Some(c), None, _) => format!("0x{c:02X}"),
            _ => String::new(),
        },
    };
    let name = service_name(service, class);
    let summary = if target.is_empty() { name.to_string() } else { format!("{name} {target}") };
    let detail = match service {
        CIP_READ_TAG | CIP_WRITE_TAG if data.len() >= 2 && class.is_none() => {
            let count = if service == CIP_READ_TAG {
                u16::from_le_bytes([data[0], data[1]])
            } else {
                data.get(2..4).map(|c| u16::from_le_bytes([c[0], c[1]])).unwrap_or(1)
            };
            if count > 1 { format!("{count} elements") } else { String::new() }
        }
        _ if data.is_empty() => String::new(),
        _ => format!("{} data bytes", data.len()),
    };
    (summary, detail)
}

fn describe_reply(message: &[u8]) -> (String, String) {
    let request = message[0] & 0x7F;
    let name = service_name(request, None);
    let Ok(reply) = CipReply::parse(message) else {
        return (format!("{name} reply"), "truncated".into());
    };
    if request == CIP_MULTIPLE_SERVICE_PACKET && (reply.status == 0 || reply.status == 0x1E) {
        if let Ok(parts) = split_multiple_service_data(&reply.data) {
            let failed = parts.iter().filter(|p| p.get(2).is_some_and(|s| *s != 0 && *s != 0x06)).count();
            let detail = if failed == 0 { "every service OK".to_string() } else { format!("{failed} failed") };
            return (format!("Multiple_Service_Packet reply ×{}", parts.len()), detail);
        }
    }
    let status = match reply.status {
        0 => "OK".to_string(),
        0x06 => "partial transfer".to_string(),
        code => {
            let mut text = format!("0x{code:02X} {}", general_status_text(code));
            if let Some(ext) = reply.ext.first() {
                text.push_str(&format!(" (0x{ext:04X}"));
                if let Some(meaning) = extended_status_text(code, *ext) {
                    text.push_str(&format!(" {meaning}"));
                }
                text.push(')');
            }
            text
        }
    };
    let detail = if reply.data.is_empty() { String::new() } else { format!("{} bytes", reply.data.len()) };
    (format!("{name} reply · {status}"), detail)
}

fn command_name(command: u16) -> &'static str {
    match command {
        CMD_LIST_INTERFACES => "ListInterfaces",
        CMD_LIST_IDENTITY => "ListIdentity",
        CMD_LIST_SERVICES => "ListServices",
        CMD_REGISTER_SESSION => "RegisterSession",
        CMD_UNREGISTER_SESSION => "UnregisterSession",
        CMD_SEND_RR_DATA => "SendRRData",
        CMD_SEND_UNIT_DATA => "SendUnitData",
        0x0000 => "NOP",
        _ => "Unknown command",
    }
}

/// Name of a CIP service code. Codes from 0x4B up are object-specific: the
/// Connection Manager's differ from a Logix controller's tag services.
pub fn service_name(service: u8, class: Option<u16>) -> &'static str {
    if class == Some(CONNECTION_MANAGER_CLASS_ID) {
        match service {
            CIP_FORWARD_CLOSE => return "Forward_Close",
            CIP_UNCONNECTED_SEND => return "Unconnected_Send",
            CIP_FORWARD_OPEN => return "Forward_Open",
            CIP_LARGE_FORWARD_OPEN => return "Large_Forward_Open",
            _ => {}
        }
    }
    match service {
        0x01 => "Get_Attributes_All",
        0x02 => "Set_Attributes_All",
        0x03 => "Get_Attribute_List",
        0x04 => "Set_Attribute_List",
        0x05 => "Reset",
        0x06 => "Start",
        0x07 => "Stop",
        0x08 => "Create",
        0x09 => "Delete",
        CIP_MULTIPLE_SERVICE_PACKET => "Multiple_Service_Packet",
        0x0D => "Apply_Attributes",
        CIP_GET_ATTRIBUTE_SINGLE => "Get_Attribute_Single",
        CIP_SET_ATTRIBUTE_SINGLE => "Set_Attribute_Single",
        0x11 => "Find_Next_Object_Instance",
        0x15 => "Restore",
        0x16 => "Save",
        0x17 => "No_Operation",
        0x18 => "Get_Member",
        0x19 => "Set_Member",
        CIP_READ_TAG => "Read Tag",
        CIP_WRITE_TAG => "Write Tag",
        0x4E => "Read Modify Write Tag",
        CIP_READ_TAG_FRAGMENTED => "Read Tag Fragmented",
        CIP_WRITE_TAG_FRAGMENTED => "Write Tag Fragmented",
        CIP_FORWARD_OPEN => "Forward_Open",
        CIP_GET_INSTANCE_ATTRIBUTE_LIST => "Get_Instance_Attribute_List",
        CIP_LARGE_FORWARD_OPEN => "Large_Forward_Open",
        _ => "Service",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cip::{build_multiple_service_packet, build_read_tag_request, build_unconnected_send};
    use crate::packets::{build_generic_cip_request, build_list_identity_request, build_list_identity_response, build_send_rr_data};

    #[test]
    fn requests_name_the_service_and_its_target() {
        let read = build_send_rr_data(1, &build_read_tag_request("Program:Main.Speed", 1).unwrap(), &[]);
        assert_eq!(describe_frame(&read).0, "Read Tag Program:Main.Speed");
        let array = build_send_rr_data(1, &build_read_tag_request("Arr[3]", 4).unwrap(), &[]);
        assert_eq!(describe_frame(&array), ("Read Tag Arr[3]".into(), "4 elements".into()));
        let get = build_send_rr_data(1, &build_generic_cip_request(0x0E, 4, 100, Some(3), &[]), &[]);
        assert_eq!(describe_frame(&get).0, "Get_Attribute_Single 0x04/100/3");
        // Routed requests describe the embedded message.
        let routed = build_unconnected_send(&build_read_tag_request("Counter", 1).unwrap(), &[1, 0]);
        let (summary, detail) = describe_frame(&build_send_rr_data(1, &routed, &[]));
        assert_eq!(summary, "Read Tag Counter");
        assert_eq!(detail, "routed 1,0");
        let batch = build_multiple_service_packet(&[
            build_read_tag_request("A", 1).unwrap(),
            build_read_tag_request("B", 1).unwrap(),
        ]);
        let (summary, detail) = describe_frame(&build_send_rr_data(1, &batch, &[]));
        assert_eq!(summary, "Multiple_Service_Packet ×2");
        assert_eq!(detail, "Read Tag A · Read Tag B");
    }

    #[test]
    fn replies_carry_their_status() {
        let ok = build_send_rr_data(1, &[0xCC, 0, 0, 0, 0xC4, 0, 7, 0, 0, 0], &[]);
        assert_eq!(describe_frame(&ok), ("Read Tag reply · OK".into(), "6 bytes".into()));
        let missing = build_send_rr_data(1, &[0xCC, 0, 0x04, 1, 0, 0], &[]);
        assert!(describe_frame(&missing).0.starts_with("Read Tag reply · 0x04 "), "{:?}", describe_frame(&missing));
        let bad_attribute = build_send_rr_data(1, &[0x8E, 0, 0x14, 0], &[]);
        assert!(describe_frame(&bad_attribute).0.starts_with("Get_Attribute_Single reply · 0x14"));
    }

    #[test]
    fn discovery_and_odd_frames() {
        assert_eq!(describe_frame(&build_list_identity_request()).0, "ListIdentity");
        let (summary, detail) = describe_frame(&build_list_identity_response([127, 0, 0, 1], 44818));
        assert_eq!(summary, "ListIdentity reply");
        assert!(detail.starts_with("ENIP-Sim · Rockwell"), "{detail}");
        assert_eq!(describe_frame(&[0x6F, 0]).0, "Truncated frame");
        // A failed encapsulation command names its status.
        let mut refused = build_send_rr_data(9, &[0x4C, 0], &[]);
        refused[8] = 0x64;
        assert_eq!(describe_frame(&refused).0, "SendRRData failed");
    }
}
