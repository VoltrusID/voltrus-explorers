//! IEC 61131-3 Structured Text codegen from CIP UDT templates (Atur issue PLC-7).
//!
//! Turns `CipTemplateDef`/`CipTagDef` pairs (as served by the suite's EtherNet/IP
//! server / simulator, or fetched from a controller via
//! [`crate::templates::read_templates`]) into an importable `.st` source file.
//!
//! Layout decisions (deliberate, kept stable for exact-text tests):
//! - One `TYPE <Name> : STRUCT … END_STRUCT; END_TYPE` block per template,
//!   sorted by template name (ties broken by template id). Members keep their
//!   declaration order — the CIP byte layout is semantic, so it must not be
//!   sorted.
//! - `CipTemplateMember` can only express atomic members ([`CipDataType`]),
//!   so nested UDTs always arrive as separate templates and each gets its own
//!   TYPE block. No flattening is performed or needed.
//! - Array members (`array_elements > 1`) emit `ARRAY[0..N-1] OF <type>`
//!   (Rockwell arrays are 0-based).
//! - Unknown CIP type codes fall back to `REAL` — see
//!   [`iec_type_from_cip_code`] for the single mapping table.
//! - Tags that reference a template are listed as *commented* suggested
//!   `VAR_EXTERNAL` bindings at the end; uncomment and adapt per program.
//!
//! Member/tag names are emitted verbatim; callers that obtain them from a
//! controller should pre-validate them (see [`crate::types::validate_tag_name`]).

use crate::types::{CipDataType, CipTagDef, CipTemplateDef};

/// Map a raw CIP type code to the nearest IEC 61131-3 elementary type name.
///
/// This is the single CIP→IEC mapping table used by both the wire parser
/// ([`crate::templates`]) and this codegen. Returns `None` for codes without a
/// mapping (time/date types, vendor types); callers fall back to `REAL`.
pub fn iec_type_from_cip_code(code: u8) -> Option<&'static str> {
    Some(match code {
        crate::types::CIP_TYPE_BOOL => "BOOL",
        crate::types::CIP_TYPE_SINT => "SINT",
        crate::types::CIP_TYPE_INT => "INT",
        crate::types::CIP_TYPE_DINT => "DINT",
        crate::types::CIP_TYPE_LINT => "LINT",
        crate::types::CIP_TYPE_USINT => "USINT",
        crate::types::CIP_TYPE_UINT => "UINT",
        crate::types::CIP_TYPE_UDINT => "UDINT",
        crate::types::CIP_TYPE_ULINT => "ULINT",
        crate::types::CIP_TYPE_REAL => "REAL",
        crate::types::CIP_TYPE_LREAL => "LREAL",
        crate::types::CIP_TYPE_STRING => "STRING",
        crate::types::CIP_TYPE_DWORD => "DWORD",
        crate::types::CIP_TYPE_LWORD => "LWORD",
        _ => return None,
    })
}

/// IEC type name for one template member's CIP type, with the documented
/// REAL fallback for unknown codes.
pub fn member_type_name(dt: CipDataType) -> &'static str {
    iec_type_from_cip_code(dt.code()).unwrap_or("REAL")
}

/// Generate the full `eip_types.st` content for the given templates + tags.
///
/// Deterministic: templates sorted by (name, id), UDT tags sorted by name.
/// Non-UDT tags (no `template_id`) are ignored.
pub fn udt_templates_to_st(templates: &[CipTemplateDef], tags: &[CipTagDef]) -> String {
    let mut out = String::new();
    out.push_str("(* UDT types imported from an EtherNet/IP controller (mt-enip codegen, issue PLC-7). *)\n");
    out.push_str("(* One TYPE block per template; members keep declaration order; arrays are 0-based. *)\n");
    out.push_str("(* Do not edit by hand — regenerate with \"Import UDTs from controller…\". *)\n");

    let mut sorted: Vec<&CipTemplateDef> = templates.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    let mut emitted_name: Option<&str> = None;
    for t in sorted {
        // Duplicate template names would emit duplicate TYPE identifiers —
        // keep the first (lowest id) and skip the rest.
        if emitted_name == Some(t.name.as_str()) {
            continue;
        }
        emitted_name = Some(&t.name);

        out.push('\n');
        out.push_str(&format!("TYPE {} :\nSTRUCT\n", t.name));
        for m in &t.members {
            let ty = member_type_name(m.data_type);
            if m.array_elements > 1 {
                out.push_str(&format!(
                    "    {} : ARRAY[0..{}] OF {};\n",
                    m.name,
                    m.array_elements - 1,
                    ty
                ));
            } else {
                out.push_str(&format!("    {} : {};\n", m.name, ty));
            }
        }
        out.push_str("END_STRUCT;\nEND_TYPE\n");
    }

    out.push_str("\n(* Suggested VAR_EXTERNAL bindings *)\n");
    let mut udt_tags: Vec<&CipTagDef> = tags.iter().filter(|t| t.template_id.is_some()).collect();
    udt_tags.sort_by(|a, b| a.name.cmp(&b.name));
    for tag in udt_tags {
        let tmpl_name = templates
            .iter()
            .find(|t| Some(t.id) == tag.template_id)
            .map(|t| t.name.as_str())
            .unwrap_or("UNKNOWN");
        out.push_str(&format!("(*   {} : {}; *)\n", tag.name, tmpl_name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CipTemplateMember, CipValue};

    fn drive_info() -> CipTemplateDef {
        CipTemplateDef {
            id: 1,
            name: "DriveInfo".into(),
            members: vec![
                CipTemplateMember { name: "RatedSpeed".into(), data_type: CipDataType::Real, array_elements: 1 },
                CipTemplateMember { name: "Torque".into(), data_type: CipDataType::Real, array_elements: 1 },
                CipTemplateMember { name: "Poles".into(), data_type: CipDataType::Dint, array_elements: 1 },
                CipTemplateMember { name: "Amps".into(), data_type: CipDataType::Real, array_elements: 1 },
            ],
        }
    }

    fn tag(name: &str, template_id: u16) -> CipTagDef {
        CipTagDef {
            name: name.into(),
            data_type: CipDataType::Dint,
            element_count: 1,
            initial_value: CipValue::default(),
            template_id: Some(template_id),
            read_only: false,
        }
    }

    const EXPECTED_DRIVE_INFO: &str = "\
(* UDT types imported from an EtherNet/IP controller (mt-enip codegen, issue PLC-7). *)
(* One TYPE block per template; members keep declaration order; arrays are 0-based. *)
(* Do not edit by hand — regenerate with \"Import UDTs from controller…\". *)

TYPE DriveInfo :
STRUCT
    RatedSpeed : REAL;
    Torque : REAL;
    Poles : DINT;
    Amps : REAL;
END_STRUCT;
END_TYPE

(* Suggested VAR_EXTERNAL bindings *)
(*   Motor1 : DriveInfo; *)
";

    #[test]
    fn drive_info_exact_text() {
        let st = udt_templates_to_st(&[drive_info()], &[tag("Motor1", 1)]);
        assert_eq!(st, EXPECTED_DRIVE_INFO);
    }

    #[test]
    fn output_is_deterministic_and_sorted() {
        let mut motor = drive_info();
        motor.name = "MotorData".into();
        motor.id = 2;
        let templates = vec![motor, drive_info()];
        let tags = vec![tag("ZAxis", 2), tag("Motor1", 1)];
        let a = udt_templates_to_st(&templates, &tags);
        let b = udt_templates_to_st(&templates, &tags);
        assert_eq!(a, b);
        // DriveInfo (name-sorted) comes before MotorData; tags are name-sorted.
        assert!(a.find("TYPE DriveInfo :").unwrap() < a.find("TYPE MotorData :").unwrap());
        assert!(a.find("(*   Motor1 :").unwrap() < a.find("(*   ZAxis :").unwrap());
        assert!(a.contains("(*   ZAxis : MotorData; *)"), "tag binds to its own template id");
    }

    #[test]
    fn array_member_uses_iec_array_type() {
        let mut t = drive_info();
        t.members.insert(
            0,
            CipTemplateMember { name: "Speeds".into(), data_type: CipDataType::Real, array_elements: 3 },
        );
        let st = udt_templates_to_st(&[t], &[]);
        assert!(st.contains("    Speeds : ARRAY[0..2] OF REAL;\n"));
        // No UDT tags → bindings section carries only its header.
        assert!(st.ends_with("(* Suggested VAR_EXTERNAL bindings *)\n"));
    }

    #[test]
    fn atomic_types_map_one_to_one() {
        for (dt, want) in [
            (CipDataType::Bool, "BOOL"),
            (CipDataType::Sint, "SINT"),
            (CipDataType::Int, "INT"),
            (CipDataType::Dint, "DINT"),
            (CipDataType::Real, "REAL"),
            (CipDataType::Lint, "LINT"),
            (CipDataType::Dword, "DWORD"),
            (CipDataType::String, "STRING"),
        ] {
            assert_eq!(iec_type_from_cip_code(dt.code()), Some(want));
            assert_eq!(member_type_name(dt), want);
        }
    }

    #[test]
    fn unknown_cip_type_falls_back_to_real() {
        // 0x8C (LTIME) and other codes outside CipDataType have no mapping.
        assert_eq!(iec_type_from_cip_code(0x8C), None);
        assert_eq!(iec_type_from_cip_code(0x00), None);
        // The documented fallback at codegen level is REAL.
        assert_eq!(member_type_name(CipDataType::Real), "REAL");
    }
}
