//! Type mapping and identifier normalization shared by the importers.

/// Result of mapping one external type name to an IEC 61131-3 ST type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeDecision {
    /// Declare the variable with this IEC type (elementary or function block).
    Emit(String),
    /// Do not import the variable; the reason explains why.
    Skip(String),
}

/// Function block types the `mt-stc` code generator implements natively.
pub const SUPPORTED_FBS: [&str; 6] = ["TON", "TOF", "TP", "CTU", "CTD", "CTUD"];

/// Map a Rockwell (Logix) tag `DataType` attribute to an IEC ST type.
///
/// Elementary Rockwell types map 1:1 onto types the `mt-stc` runtime
/// understands. `TIMER` becomes a `TON` instance and `COUNTER` a `CTUD`
/// instance (both have matching codegen support). Everything else — user
/// defined types, `CONTROL` sequencer structures, hardware types — is
/// skipped with an explicit reason.
pub fn map_rockwell_type(data_type: &str) -> TypeDecision {
    match data_type.to_ascii_uppercase().as_str() {
        "BOOL" | "SINT" | "INT" | "DINT" | "LINT" | "USINT" | "UINT" | "UDINT" | "ULINT"
        | "REAL" | "LREAL" | "TIME" | "STRING" | "BYTE" | "WORD" | "DWORD" => {
            TypeDecision::Emit(data_type.to_ascii_uppercase())
        }
        "TIMER" => TypeDecision::Emit("TON".into()),
        "COUNTER" => TypeDecision::Emit("CTUD".into()),
        "CONTROL" => TypeDecision::Skip(
            "CONTROL-type tag (sequencer control structure used by SEQ instructions) has no \
             equivalent function block in the mt-stc runtime; re-implement the sequencer in ST"
                .into(),
        ),
        _ => TypeDecision::Skip(format!(
            "derived/unknown type '{data_type}': only elementary types and TIMER/COUNTER \
             function block instances are mapped; user-defined types are not translated"
        )),
    }
}

/// Detect identifiers that trip an iecst 0.5 lexer defect: a contiguous
/// `type` segment (any case) is mis-tokenized into the TYPE keyword unless
/// it is a clean suffix (`mytype` parses, `x_type` / `type_x` / `Motor_Type`
/// do not). Such names are never emitted; callers report them instead.
pub fn is_iecst_lexer_safe(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    if l == "type" {
        return false;
    }
    match l.find("type") {
        None => true,
        Some(pos) => pos + 4 == l.len() && !l[..pos].ends_with('_'),
    }
}

/// Map a PLCopen TC6 elementary type element name to an IEC ST type name.
///
/// Returns `None` for elementary types the `mt-stc` runtime does not
/// implement (`DATE`, `DT`, `TOD`, the generic `ANY*` types).
pub fn map_plcopen_elementary(name: &str) -> Option<String> {
    match name.to_ascii_uppercase().as_str() {
        "BOOL" | "BYTE" | "WORD" | "DWORD" | "LWORD" | "SINT" | "INT" | "DINT" | "LINT"
        | "USINT" | "UINT" | "UDINT" | "ULINT" | "REAL" | "LREAL" | "TIME" | "STRING" => {
            Some(name.to_ascii_uppercase())
        }
        // PLCopen spells the string element lowercase, but be liberal.
        "WSTRING" => Some("STRING".into()),
        _ => None,
    }
}

/// Sanitize an external identifier into something the IEC ST lexer accepts.
///
/// Invalid characters become `_`; a leading digit gets a `_` prefix. The
/// transformation is deterministic and documented in the crate docs;
/// collisions created by it are reported as errors, never resolved silently.
pub fn sanitize_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, ch) in name.chars().enumerate() {
        if i == 0 && ch.is_ascii_digit() {
            out.push('_');
        }
        let ok = ch.is_ascii_alphanumeric() || ch == '_';
        out.push(if ok { ch } else { '_' });
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rockwell_elementary_types_pass_through() {
        for t in ["BOOL", "INT", "DINT", "REAL", "STRING", "LINT", "TIME"] {
            assert_eq!(map_rockwell_type(t), TypeDecision::Emit(t.into()), "{t}");
        }
        // Case-insensitive.
        assert_eq!(map_rockwell_type("bool"), TypeDecision::Emit("BOOL".into()));
    }

    #[test]
    fn rockwell_fb_types_map_to_iec_blocks() {
        assert_eq!(map_rockwell_type("TIMER"), TypeDecision::Emit("TON".into()));
        assert_eq!(
            map_rockwell_type("Counter"),
            TypeDecision::Emit("CTUD".into())
        );
    }

    #[test]
    fn rockwell_control_type_is_skipped_with_reason() {
        match map_rockwell_type("CONTROL") {
            TypeDecision::Skip(reason) => {
                assert!(reason.contains("sequencer"), "{reason}");
            }
            other => panic!("expected skip, got {other:?}"),
        }
    }

    #[test]
    fn iecst_lexer_defect_detection() {
        // Clean names.
        for n in [
            "RunTimer",
            "mytype",
            "notype",
            "_ty_pe",
            "typ_e",
            "CycleTimer",
        ] {
            assert!(is_iecst_lexer_safe(n), "{n}");
        }
        // The defect: embedded non-suffix `type` segments.
        for n in [
            "no_type",
            "n_type",
            "x_type",
            "type_x",
            "Motor_Type",
            "a_type_b",
            "Type",
        ] {
            assert!(!is_iecst_lexer_safe(n), "{n}");
        }
    }

    #[test]
    fn rockwell_udt_is_skipped_with_reason_naming_the_type() {
        match map_rockwell_type("RecipeUDT") {
            TypeDecision::Skip(reason) => {
                assert!(reason.contains("RecipeUDT"), "{reason}");
            }
            other => panic!("expected skip, got {other:?}"),
        }
    }

    #[test]
    fn plcopen_elementary_mapping() {
        assert_eq!(map_plcopen_elementary("BOOL"), Some("BOOL".into()));
        // Lowercase string element per the TC6 schema.
        assert_eq!(map_plcopen_elementary("string"), Some("STRING".into()));
        assert_eq!(map_plcopen_elementary("DATE"), None);
        assert_eq!(map_plcopen_elementary("ANY_NUM"), None);
    }

    #[test]
    fn sanitize_replaces_invalid_characters() {
        assert_eq!(sanitize_ident("Motor Start"), "Motor_Start");
        assert_eq!(sanitize_ident("a-b.c"), "a_b_c");
        assert_eq!(sanitize_ident("9lives"), "_9lives");
        assert_eq!(sanitize_ident(""), "_");
        assert_eq!(sanitize_ident("clean_1"), "clean_1");
    }
}
