//! Frontend diagram transpilers (PLC-10): FBD / LD / SFC → Structured Text.
//!
//! These are faithful Rust ports of the voltrus-plc IDE transpilers so the
//! headless CLI can compile and test graphical programs:
//!
//! - FBD: `apps/voltrus-plc/src/components/Editor/FbdEditor/transpiler/fbd-to-st.ts`
//!   (+ `topological-sort.ts`) — Kahn topological order over the node graph;
//!   temp variables `__tN` are numbered in body-generation order; signals that
//!   feed an `OUTPUT_PIN` write the output variable directly; function blocks
//!   (TON/CTU/…) emit instance calls plus copy-outs for FB outputs wired to
//!   output pins. A cycle aborts with
//!   `Diagram contains a cycle — FBD execution order must be acyclic`.
//! - LD: `.../LdEditor/transpiler/ld-to-st.ts` — power-flow model: contacts in
//!   a path AND, paths of a rung OR; register `X` → VAR_INPUT, `Y` →
//!   VAR_OUTPUT, everything else VAR; rising/falling contacts hoist
//!   `__rtrig_N` / `__ftrig_N` instances whose `CLK` calls are `unshift`ed to
//!   the front of the body (JS `Array.unshift` in a loop, i.e. reversed).
//! - SFC: `.../SfcEditor/transpiler/sfc-to-st.ts` — one BOOL per step
//!   (initial step `:= TRUE`) plus `_prev` shadows; steps numbered `_sN` by
//!   BFS from the initial step (disconnected steps appended with a warning);
//!   OR diverge/converge flattened by recursive `resolveDownstream`; jumps
//!   become `TRUE`-condition transitions to the named step's variable;
//!   action qualifiers N/S/R/P1/P0.
//!
//! Byte parity with the TypeScript output is the goal; the golden tests
//! (`tests/golden_diagrams.rs`, fixtures under `tests/fixtures/`) are
//! generated from the real TS transpilers by
//! `apps/voltrus-plc/scripts/gen-goldens.test.ts`.

pub mod blocks;
pub mod fbd;
pub mod ld;
pub mod sfc;

use serde::Serialize;

/// One transpiler diagnostic, mirroring the TS `{ message, severity }` shape.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Diag {
    pub message: String,
    pub severity: String,
}

/// Transpiler output: generated ST source (empty when an error diagnostic was
/// emitted) plus diagnostics, exactly like the TS `TranspileResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct TranspileResult {
    pub source: String,
    pub diagnostics: Vec<Diag>,
}

impl TranspileResult {
    fn empty_program(program_name: &str) -> Self {
        Self {
            source: format!("PROGRAM {program_name}\nEND_PROGRAM"),
            diagnostics: Vec::new(),
        }
    }
}

/// Diagram file kind, detected from the file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramKind {
    Fbd,
    Ld,
    Sfc,
}

impl DiagramKind {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "fbd" => Some(Self::Fbd),
            "ld" => Some(Self::Ld),
            "sfc" => Some(Self::Sfc),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Fbd => "fbd",
            Self::Ld => "ld",
            Self::Sfc => "sfc",
        }
    }
}

/// Deserialize canonical (or any object-shaped) diagram JSON and transpile it
/// to ST. Parse failures are reported as `Err` with a human-readable message.
pub fn transpile_source(kind: DiagramKind, json: &str) -> Result<TranspileResult, String> {
    match kind {
        DiagramKind::Fbd => {
            let program: fbd::FbdProgram =
                serde_json::from_str(json).map_err(|e| format!("invalid FBD JSON: {e}"))?;
            Ok(fbd::transpile(&program))
        }
        DiagramKind::Ld => {
            let program: ld::LdProgram =
                serde_json::from_str(json).map_err(|e| format!("invalid LD JSON: {e}"))?;
            Ok(ld::transpile(&program))
        }
        DiagramKind::Sfc => {
            let program: sfc::SfcProgram =
                serde_json::from_str(json).map_err(|e| format!("invalid SFC JSON: {e}"))?;
            Ok(sfc::transpile(&program))
        }
    }
}

/// `[A-Za-z_][A-Za-z0-9_]*`, identical to the TS `IEC_IDENTIFIER_RE`.
pub(crate) fn is_iec_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `value || fallback` for optional JSON strings (JS treats `""` as falsy).
pub(crate) fn or_named<'a>(value: Option<&'a str>, fallback: String) -> String {
    match value {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iec_identifier_matches_ts_regex() {
        assert!(is_iec_identifier("X0"));
        assert!(is_iec_identifier("_tmp"));
        assert!(is_iec_identifier("Motor_Start_2"));
        assert!(!is_iec_identifier(""));
        assert!(!is_iec_identifier("2fast"));
        assert!(!is_iec_identifier("has space"));
        assert!(!is_iec_identifier("has-dash"));
    }

    #[test]
    fn detects_kind_by_extension() {
        assert_eq!(DiagramKind::from_extension("fbd"), Some(DiagramKind::Fbd));
        assert_eq!(DiagramKind::from_extension("LD"), Some(DiagramKind::Ld));
        assert_eq!(DiagramKind::from_extension("sfc"), Some(DiagramKind::Sfc));
        assert_eq!(DiagramKind::from_extension("st"), None);
    }

    #[test]
    fn parse_error_is_reported() {
        let err = transpile_source(DiagramKind::Fbd, "{ not json").unwrap_err();
        assert!(err.starts_with("invalid FBD JSON"));
    }
}
