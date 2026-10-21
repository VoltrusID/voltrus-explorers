//! Client-side CIP Template object (class 0x6C) fetch — UDT member layouts.
//!
//! Counterpart to the server side in [`crate::server`], which serves Template
//! instances as:
//!
//! - GetAttributeList (0x03) reply payload (after the CIP reply header):
//!   `[count:u16]` then per attribute `[id:u16][status:u16][value]` with
//!   attr 4 = ObjDefinitionSize (u32, 32-bit words), 5 = StructureSize (u32),
//!   2 = MemberCount (u16), 1 = StructureHandle (u16).
//! - Read Tag (0x4C) reply payload: the definition blob — per member
//!   `[info:u16][type:u16][offset:u32]` (8 bytes each), then a NUL name table
//!   `{TemplateName};\0{member0}\0{member1}\0…`, zero-padded to 4 bytes.
//!
//! Documented PLC-7 decision: member type codes unknown to [`CipDataType`]
//! fall back to `Real` at parse time (the nearest safe numeric container);
//! `CipTemplateMember` cannot carry an "unknown" marker.

use mt_protocol::{ProtocolClient, ProtocolError};

use crate::client::EnipClient;
use crate::types::{CipDataType, CipTemplateDef, CipTemplateMember};

/// CIP Template object class id.
const TEMPLATE_CLASS_ID: u16 = 0x6C;
/// CIP GetAttributeList service code (reply 0x83).
const CIP_GET_ATTRIBUTE_LIST: u8 = 0x03;

/// Attributes of one Template instance, as served by GetAttributeList.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TemplateAttrs {
    /// ObjDefinitionSize in 32-bit words (definition bytes ≈ words*4 - 23).
    pub obj_definition_words: u32,
    /// Packed byte length of one struct instance.
    pub structure_size: u16,
    /// Number of members in the definition blob.
    pub member_count: u16,
    /// Structure handle (echoes the template id).
    pub handle: u16,
}

/// Parse a GetAttributeList reply payload for a Template instance.
/// Returns `None` on truncated or status-flagged data.
pub fn parse_template_attrs(data: &[u8]) -> Option<TemplateAttrs> {
    if data.len() < 2 {
        return None;
    }
    let count = u16::from_le_bytes(data[0..2].try_into().ok()?) as usize;
    let mut attrs = TemplateAttrs::default();
    let mut p = 2;
    for _ in 0..count {
        if p + 4 > data.len() {
            return None;
        }
        let id = u16::from_le_bytes(data[p..p + 2].try_into().ok()?);
        let status = u16::from_le_bytes(data[p + 2..p + 4].try_into().ok()?);
        p += 4;
        let value_len = match id {
            4 | 5 => 4,
            2 | 1 => 2,
            _ => return None,
        };
        if status != 0 || p + value_len > data.len() {
            return None;
        }
        let value = &data[p..p + value_len];
        match id {
            4 => attrs.obj_definition_words = u32::from_le_bytes(value.try_into().ok()?),
            5 => attrs.structure_size = u32::from_le_bytes(value.try_into().ok()?) as u16,
            2 => attrs.member_count = u16::from_le_bytes(value.try_into().ok()?),
            1 => attrs.handle = u16::from_le_bytes(value.try_into().ok()?),
            _ => unreachable!("value_len match bounds id"),
        }
        p += value_len;
    }
    Some(attrs)
}

/// Parse a Template definition blob into (template name, members).
/// `member_count` comes from [`parse_template_attrs`] — the blob carries no
/// member count of its own.
pub fn parse_template_blob(blob: &[u8], member_count: usize) -> Option<(String, Vec<CipTemplateMember>)> {
    let def_bytes = member_count.checked_mul(8)?;
    if blob.len() < def_bytes {
        return None;
    }
    let mut members = Vec::with_capacity(member_count);
    for i in 0..member_count {
        let off = i * 8;
        // [info:u16][type:u16][offset:u32]: info is the array length for array
        // members (the BOOL bit position for BOOL members), 0 for scalars.
        let info = u16::from_le_bytes(blob[off..off + 2].try_into().ok()?);
        let type_word = u16::from_le_bytes(blob[off + 2..off + 4].try_into().ok()?);
        let data_type = CipDataType::from_code(type_word as u8).unwrap_or(CipDataType::Real);
        members.push(CipTemplateMember {
            name: String::new(), // filled from the name table below
            data_type,
            array_elements: if data_type == CipDataType::Bool || info == 0 { 1 } else { info },
        });
    }

    // Name table: "{Template};\0{member0}\0{member1}\0…" (+ zero padding).
    let names: Vec<String> = blob[def_bytes..]
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .collect();
    let mut it = names.into_iter();
    let template_name = it.next()?.trim_end_matches(';').to_string();
    for m in members.iter_mut() {
        m.name = it.next()?;
    }
    Some((template_name, members))
}

/// Fetch the given Template instances (ids come from browsed UDT tags'
/// `template_id`) from a controller. One-shot: connects, reads, drops.
pub async fn read_templates(
    config: &crate::types::EnipConfig,
    template_ids: &[u16],
) -> Result<Vec<CipTemplateDef>, ProtocolError> {
    let client = EnipClient::connect(config.clone()).await?;
    let mut out = Vec::with_capacity(template_ids.len());
    for id in template_ids.iter().copied() {
        out.push(read_template(&client, id).await?);
    }
    Ok(out)
}

/// Read one Template instance: GetAttributeList for the sizes, then Read Tag
/// (service 0x4C with `offset u32 | length u16`, repeated while the device
/// answers partial-transfer) for the definition blob.
async fn read_template(client: &EnipClient, id: u16) -> Result<CipTemplateDef, ProtocolError> {
    // Request payload: [count:u16][attr 4][attr 5][attr 2][attr 1].
    let mut req = 4u16.to_le_bytes().to_vec();
    for attr in [4u16, 5, 2, 1] {
        req.extend_from_slice(&attr.to_le_bytes());
    }
    let attrs_raw = client
        .generic_cip(CIP_GET_ATTRIBUTE_LIST, TEMPLATE_CLASS_ID, id as u32, None, req)
        .await?;
    let attrs = parse_template_attrs(&attrs_raw).ok_or_else(|| {
        ProtocolError::Other(format!("unparseable Template attributes for instance {id}"))
    })?;

    // Definition bytes to request: (ObjectDefinitionSize × 4) − 21, the size
    // pycomm3 requests from real controllers; the device clips at the end.
    let blob_len = (attrs.obj_definition_words as usize * 4).saturating_sub(21);
    let blob = client.read_template_blob(id as u32, blob_len).await?;
    let (name, members) = parse_template_blob(&blob, attrs.member_count as usize)
        .ok_or_else(|| ProtocolError::Other(format!("unparseable Template blob for instance {id}")))?;

    Ok(CipTemplateDef {
        id,
        name: if name.is_empty() { format!("Template{id}") } else { name },
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of server.rs `build_definition_blob` (private there): per member
    /// `[info:u16][type:u16][offset:u32]`, then `{Name};\0{m0}\0{m1}\0…`
    /// zero-padded to 4 bytes.
    fn blob_for(name: &str, members: &[(&str, CipDataType)]) -> Vec<u8> {
        let mut blob = Vec::new();
        let mut offset = 0u32;
        for (_, dt) in members {
            let size = dt.element_size().max(1) as u32;
            offset = (offset + size - 1) / size * size;
            blob.extend_from_slice(&0u16.to_le_bytes());
            blob.extend_from_slice(&(dt.code() as u16).to_le_bytes());
            blob.extend_from_slice(&offset.to_le_bytes());
            offset += size;
        }
        blob.extend_from_slice(name.as_bytes());
        blob.push(0x3B); // ';'
        blob.push(0x00);
        for (mn, _) in members {
            blob.extend_from_slice(mn.as_bytes());
            blob.push(0x00);
        }
        while blob.len() % 4 != 0 {
            blob.push(0x00);
        }
        blob
    }

    /// Mirror of server.rs `handle_template_get_attributes` payload
    /// (post-CIP-header): [count:u16] + per attr [id:u16][status:u16][value].
    fn attrs_payload(obj_def: u32, struct_size: u16, member_count: u16, handle: u16) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&4u16.to_le_bytes());
        for (id, value) in [
            (4u16, Value::W32(obj_def)),
            (5, Value::W32(struct_size as u32)),
            (2, Value::W16(member_count)),
            (1, Value::W16(handle)),
        ] {
            d.extend_from_slice(&id.to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            match value {
                Value::W32(v) => d.extend_from_slice(&v.to_le_bytes()),
                Value::W16(v) => d.extend_from_slice(&v.to_le_bytes()),
            }
        }
        d
    }

    enum Value {
        W32(u32),
        W16(u16),
    }

    #[test]
    fn attrs_parse_roundtrip() {
        let d = attrs_payload(9, 16, 4, 1);
        let a = parse_template_attrs(&d).expect("parses");
        assert_eq!(
            a,
            TemplateAttrs {
                obj_definition_words: 9,
                structure_size: 16,
                member_count: 4,
                handle: 1,
            }
        );
    }

    #[test]
    fn attrs_parse_rejects_truncated() {
        let d = attrs_payload(9, 16, 4, 1);
        assert_eq!(parse_template_attrs(&d[..d.len() - 1]), None);
        assert_eq!(parse_template_attrs(&d[..3]), None);
        assert_eq!(parse_template_attrs(&[]), None);
    }

    #[test]
    fn blob_parse_roundtrip_drive_info() {
        let members = [
            ("RatedSpeed", CipDataType::Real),
            ("RatedCurrent", CipDataType::Real),
            ("RatedPower", CipDataType::Real),
            ("Poles", CipDataType::Dint),
        ];
        let blob = blob_for("DriveInfo", &members);
        let (name, parsed) = parse_template_blob(&blob, members.len()).expect("parses");
        assert_eq!(name, "DriveInfo");
        assert_eq!(parsed.len(), 4);
        for (m, (want_name, want_dt)) in parsed.iter().zip(members.iter()) {
            assert_eq!(m.name, *want_name);
            assert_eq!(m.data_type, *want_dt);
            assert_eq!(m.array_elements, 1);
        }
    }

    #[test]
    fn unknown_type_code_falls_back_to_real() {
        // 0x8C (LTIME) is not in CipDataType — documented fallback is Real.
        let mut blob = blob_for("Weird", &[("T", CipDataType::Dint)]);
        // Overwrite member 0's type word (offset 2..4) with 0x8C.
        blob[2..4].copy_from_slice(&0x8Cu16.to_le_bytes());
        let (name, parsed) = parse_template_blob(&blob, 1).expect("parses");
        assert_eq!(name, "Weird");
        assert_eq!(parsed[0].data_type, CipDataType::Real);
    }

    #[test]
    fn blob_parse_rejects_short_blob() {
        let blob = blob_for("DriveInfo", &[("A", CipDataType::Real)]);
        assert!(parse_template_blob(&blob, 3).is_none(), "member_count beyond blob");
    }
}
