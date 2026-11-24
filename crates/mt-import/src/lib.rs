//!
//! Import of third-party PLC project exports into Voltrus ST sources.
//!
//! Two formats are supported, both reduced to the flat "folder of `.st`
//! files" model the Voltrus toolchain uses. Every generated file is compiled
//! with [`mt_stc::compile`] (the same call the `voltrus-plc check` command
//! makes); only files that compile cleanly are reported as imported, and
//! every element that is deliberately *not* translated is listed under
//! `skipped` with a reason. Nothing is dropped silently.
//!
//! # Report model
//!
//! [`ImportReport`] carries three vectors, each in document order:
//!
//! * `imported` — generated `.st` sources ready to write to disk,
//! * `skipped` — recognized elements deliberately not translated,
//! * `errors` — malformed input (`context: "parse"`) and generated sources
//!   that failed to compile (`context` = file name).
//!
//! # L5X (Rockwell / Studio 5000) — supported subset
//!
//! Schema reference: the Logix5000 export schema (`RSLogix5000Content`,
//! per Rockwell publication 1756-RM014; XSDs mirrored in the public
//! `benmusson/l5x-schema` repository).
//!
//! **Parsed:**
//! * Controller-scoped `Tags/Tag` — elementary types (`BOOL`, `SINT`, `INT`,
//!   `DINT`, `LINT`, `USINT`, `UINT`, `UDINT`, `ULINT`, `REAL`, `LREAL`,
//!   `TIME`, `STRING`, `BYTE`, `WORD`, `DWORD`) become variables of a
//!   generated `<Controller>_Tags` program; `TIMER` becomes a `TON`
//!   instance, `COUNTER` a `CTUD` instance.
//! * Program-scoped `Tags/Tag` — same mapping; the declarations are placed
//!   in every ST-routine file of that program (or in a `<Program>_Tags`
//!   file when the program has no ST routine, so no tag is ever lost).
//! * `Routine Type="ST"` — `STContent/Line` text becomes the body of one
//!   `PROGRAM <Program>_<Routine>` POU per routine (one POU per file, which
//!   is what the headless `check` path compiles).
//! * Program-scoped L5X exports (`<Program>` directly under the root) are
//!   handled like programs inside a controller export.
//!
//! **Deliberately skipped (each with a reason in the report):**
//! * `Routine Type="RLL"` — ladder rungs are a *graphics sheet*: `RLLContent`
//!   serializes contacts, coils and wires per rung, not IEC statements.
//!   Transpiling rung graphics to ST is out of scope; the routine is skipped
//!   with that reason. Same for `FBD` and `SFC` routines.
//! * `Routine Type="Encrypted"/"External"` — the export carries no readable
//!   ST.
//! * `DataTypes/DataType` (user-defined types) — UDT definitions are not
//!   translated; every tag whose type is such a UDT is skipped individually
//!   naming the type, so the failure mode is visible per tag.
//! * `Tag TagType="Alias"` and array tags (`Dimensions`) — alias targets and
//!   array types are not modeled by the importer.
//! * `AddOnInstructionDefinitions` — AOIs are compound (parameters + local
//!   tags + routines); not translated.
//! * `Tasks`, `Modules`, `Trends`, `DataLogs`, `QuickWatchLists` — hardware
//!   and tooling configuration with no ST counterpart.
//! * `Description` elements — documentation text is dropped (not an import
//!   failure).
//!
//! **Normalizations (documented, not reported):** identifier sanitization
//! (invalid characters become `_`, a leading digit gets a `_` prefix; a
//! collision caused by it is reported as an error, never resolved silently);
//! `Produced`/`Consumed` tags import as plain variables of their type.
//!
//! # PLCopen TC6 XML (IEC 61131-10 / TC6 "XML formats for IEC 61131-3")
//!
//! Schema reference: `tc6_xml_v201.xsd` (PLCopen TC6, v2.01).
//!
//! **Parsed:**
//! * `types/dataTypes/dataType` with a `struct` base type — becomes a
//!   `TYPE ... : STRUCT ... END_STRUCT; END_TYPE` block. A POU file embeds
//!   every struct type transitively reachable from its variables.
//! * `types/pous/pou` with `pouType="program"` or `"functionBlock"` —
//!   becomes a `PROGRAM`/`FUNCTION_BLOCK` file; `interface` var lists map
//!   `inputVars`→`VAR_INPUT`, `outputVars`→`VAR_OUTPUT`, `inOutVars`→
//!   `VAR_IN_OUT`, `localVars`/`tempVars`→`VAR`. `initialValue` with a
//!   `simpleValue` becomes `:= <value>`.
//! * Variable types: elementary types map 1:1 (`string` element → `STRING`);
//!   `derived` names matching `TON`/`TOF`/`TP`/`CTU`/`CTD`/`CTUD` become
//!   function block instances; `derived` names matching a parsed struct
//!   become struct variables.
//! * `body/ST` — the XHTML-wrapped character data becomes the POU body;
//!   element boundaries inside the XHTML wrapper become line breaks.
//!
//! **Deliberately skipped (each with a reason):**
//! * `pouType="function"` — the `mt-stc` code generator cannot assign the
//!   function-name return value, so FUNCTION POUs would not compile.
//! * Bodies in `IL` (instruction list — text but not ST) and `LD`/`FBD`/
//!   `SFC` (graphics sheets of contacts, blocks and wires).
//! * `instances` (configuration/resource/task wiring) — the flat Voltrus
//!   project model has no task configuration.
//! * `dataTypes` with `enum`, `array`, `subrangeSigned`/`subrangeUnsigned`
//!   or alias `derived` base types; variables typed with them (or with
//!   `DATE`/`DT`/`TOD`/generic `ANY*`) are skipped individually.
//! * `initialValue` given as `structValue`/`arrayValue` literals.
//! * `externalVars` — globals have no counterpart in the flat model.
//! * `documentation` elements — dropped (not an import failure).
//!
//! # Parser choice
//!
//! `quick-xml` (event pull) over `serde-xml-rs`: both formats need mixed
//! content (L5X `Line` elements are text with attributes) and xs:any-style
//! wrappers (PLCopen ST bodies embed arbitrary XHTML), which the serde XML
//! data model cannot represent cleanly. The event API also keeps malformed
//! input detectable at the exact byte offset instead of shape-mismatched
//! struct fields.

use serde::Serialize;

pub mod emit;
pub mod l5x;
pub mod mapping;
pub mod plcopen;
mod xml;

use emit::PouFile;

/// Result of one import run. All vectors are in document order, so output is
/// deterministic for a given input.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ImportReport {
    /// Generated `.st` files that compiled cleanly.
    pub imported: Vec<ImportedFile>,
    /// Recognized elements deliberately not translated, with reasons.
    pub skipped: Vec<Skipped>,
    /// Malformed input (`context: "parse"`) and generated sources that
    /// failed to compile (`context` = file name).
    pub errors: Vec<ImportError>,
}

impl ImportReport {
    /// True when at least one file was produced and nothing went wrong:
    /// no parse errors and no generated file that failed to compile.
    pub fn is_success(&self) -> bool {
        !self.imported.is_empty() && self.errors.is_empty()
    }
}

/// One generated ST source, ready to be written as `<name>` in a project
/// folder.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImportedFile {
    pub name: String,
    pub st_source: String,
}

/// An element recognized but deliberately not translated.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

/// A fatal problem: malformed input or a generated file that did not compile.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImportError {
    pub context: String,
    pub message: String,
}

/// Import a Rockwell Studio 5000 L5X export.
pub fn import_l5x(xml: &str) -> ImportReport {
    l5x::import(xml)
}

/// Import a PLCopen TC6 XML project.
pub fn import_plcopen(xml: &str) -> ImportReport {
    plcopen::import(xml)
}

/// Internal accumulator: candidates plus report entries, finalized by
/// compiling every candidate through `mt_stc` (the same call the headless
/// `check` path makes). Compiling candidates is what turns them into
/// `imported` files; failures become `errors` and are never written out.
pub(crate) struct Builder {
    report: ImportReport,
    candidates: Vec<PouFile>,
    used_names: Vec<String>,
}

impl Builder {
    pub(crate) fn new() -> Builder {
        Builder {
            report: ImportReport::default(),
            candidates: Vec::new(),
            used_names: Vec::new(),
        }
    }

    pub(crate) fn parse_error(err: xml::XmlError) -> ImportReport {
        ImportReport {
            imported: Vec::new(),
            skipped: Vec::new(),
            errors: vec![ImportError {
                context: "parse".into(),
                message: err.to_string(),
            }],
        }
    }

    pub(crate) fn skipped(&mut self, entry: Skipped) {
        self.report.skipped.push(entry);
    }

    pub(crate) fn errors(&mut self, entry: ImportError) {
        self.report.errors.push(entry);
    }

    pub(crate) fn candidate(&mut self, file: PouFile) {
        self.candidates.push(file);
    }

    pub(crate) fn extend_candidates(&mut self, files: Vec<PouFile>) {
        self.candidates.extend(files);
    }

    pub(crate) fn finish(mut self) -> ImportReport {
        for file in self.candidates {
            if self.used_names.iter().any(|n| n == &file.name) {
                self.report.errors.push(ImportError {
                    context: file.name.clone(),
                    message: "duplicate generated file name; the file is not emitted".into(),
                });
                continue;
            }
            let source = file.render();
            let output = mt_stc::compile(&source);
            if output.success {
                self.used_names.push(file.name.clone());
                self.report.imported.push(ImportedFile {
                    name: file.name,
                    st_source: source,
                });
            } else {
                let first = output
                    .diagnostics
                    .iter()
                    .find(|d| d.severity == "error")
                    .map(|d| format!("{}:{}: {}", d.line, d.col, d.message))
                    .unwrap_or_else(|| "unknown compile error".into());
                let total = output
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == "error")
                    .count();
                let mut message = format!("generated source failed to compile: {first}");
                if total > 1 {
                    message.push_str(&format!(" (and {} more)", total - 1));
                }
                self.report.errors.push(ImportError {
                    context: file.name,
                    message,
                });
            }
        }
        self.report
    }
}
