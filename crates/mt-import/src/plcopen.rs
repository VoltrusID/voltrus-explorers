//! PLCopen TC6 XML (`project`, IEC 61131-10) reader.
//!
//! See the crate-level docs for the supported subset. This module walks the
//! XML once, collects struct datatypes and ST-bodied POUs, and hands
//! candidate files to the crate-level assembly + verification pipeline.

use crate::emit::{self, Decl, PouFile, StructType, VarKind};
use crate::mapping::{SUPPORTED_FBS, is_iecst_lexer_safe, map_plcopen_elementary, sanitize_ident};
use crate::xml::{Ev, PullParser, XmlError, trim_xml_ws};
use crate::{Builder, ImportError, Skipped};

/// Import one PLCopen TC6 document. Malformed XML produces a `parse` error
/// entry and an otherwise empty report; every other anomaly is enumerated in
/// `skipped` or `errors`.
pub fn import(xml: &str) -> ImportReport {
    match walk(xml) {
        Ok(builder) => builder.finish(),
        Err(err) => Builder::parse_error(err),
    }
}

use crate::ImportReport;

/// PLCopen elementary type elements the runtime rejects outright.
const UNSUPPORTED_ELEMENTARY: [&str; 4] = ["DATE", "DT", "TOD", "WSTRING"];

fn is_elementary_type_name(name: &str) -> bool {
    map_plcopen_elementary(name).is_some()
        || UNSUPPORTED_ELEMENTARY
            .iter()
            .any(|t| t.eq_ignore_ascii_case(name))
        || name.to_ascii_uppercase().starts_with("ANY")
}

/// A variable type as read from the XML, before emission is decided.
enum VarType {
    /// IEC elementary type name.
    Elementary(String),
    /// `derived name="..."` — a function block type, struct type or unknown.
    Derived(String),
    /// Recognized but unmappable; carries the display name or kind.
    Unsupported(String),
}

/// A variable being read (interface variable or struct field).
struct VarAcc {
    name: String,
    type_spec: Option<VarType>,
    init: Option<String>,
    literal_init: bool,
}

struct PouAcc {
    name: String,
    pou_type: String,
    groups: Vec<(VarKind, Vec<Decl>)>,
    /// Struct type names referenced by emitted declarations.
    type_roots: Vec<String>,
    /// The first ST body encountered.
    body: Option<String>,
    /// Text buffer while inside `body/ST`.
    st_buffer: Option<String>,
    /// 1 = the ST element itself; deeper Starts insert line breaks.
    st_depth: usize,
    var_acc: Option<VarAcc>,
    var_kind: Option<VarKind>,
    external: bool,
}

struct DataTypeAcc {
    name: String,
    kind: Option<String>,
    fields: Vec<(String, String)>,
    /// Set when the datatype is unmappable; carries the reason fragment.
    unsupported: Option<String>,
    var_acc: Option<(String, VarType)>,
}

fn walk(xml: &str) -> Result<Builder, XmlError> {
    let mut parser = PullParser::new(xml);
    let mut builder = Builder::new();
    let mut path: Vec<String> = Vec::new();

    let mut root_seen = false;
    let mut structs: Vec<StructType> = Vec::new();
    let mut pou: Option<PouAcc> = None;
    let mut datatype: Option<DataTypeAcc> = None;
    let mut in_data_types = false;

    // Classify an element that appears where a variable type is expected.
    fn set_type_from_element(name: &str, parent: &str) -> Option<VarType> {
        let target = parent == "type" || parent == "baseType";
        if !target || !is_elementary_type_name(name) {
            return None;
        }
        if let Some(mapped) = map_plcopen_elementary(name) {
            Some(VarType::Elementary(mapped))
        } else {
            Some(VarType::Unsupported(name.to_ascii_uppercase()))
        }
    }

    loop {
        let ev = match parser.next()? {
            Ev::Start(s) => {
                let name = s.local.clone();
                let parent = path.last().cloned().unwrap_or_default();
                match name.as_str() {
                    "project" if path.is_empty() => root_seen = true,
                    "instances" if parent == "project" => {
                        builder.skipped(Skipped {
                            name: "instances".into(),
                            reason: "configuration/resource/task wiring has no counterpart \
                                     in the flat Voltrus project model"
                                .into(),
                        });
                    }
                    "dataTypes" if parent == "types" => in_data_types = true,
                    "dataType" if in_data_types => {
                        datatype = Some(DataTypeAcc {
                            name: s.attr("name").unwrap_or("?").to_string(),
                            kind: None,
                            fields: Vec::new(),
                            unsupported: None,
                            var_acc: None,
                        });
                    }
                    "pou" if parent == "pous" => {
                        pou = Some(PouAcc {
                            name: s.attr("name").unwrap_or("?").to_string(),
                            pou_type: s.attr("pouType").unwrap_or("?").to_string(),
                            groups: vec![
                                (VarKind::VarInput, Vec::new()),
                                (VarKind::VarOutput, Vec::new()),
                                (VarKind::VarInOut, Vec::new()),
                                (VarKind::Var, Vec::new()),
                            ],
                            type_roots: Vec::new(),
                            body: None,
                            st_buffer: None,
                            st_depth: 0,
                            var_acc: None,
                            var_kind: None,
                            external: false,
                        });
                    }
                    "inputVars" | "outputVars" | "inOutVars" | "localVars" | "tempVars"
                    | "externalVars"
                        if parent == "interface" =>
                    {
                        if let Some(p) = &mut pou {
                            p.var_kind = Some(match name.as_str() {
                                "inputVars" => VarKind::VarInput,
                                "outputVars" => VarKind::VarOutput,
                                "inOutVars" => VarKind::VarInOut,
                                _ => VarKind::Var,
                            });
                            p.external = name == "externalVars";
                        }
                    }
                    // Struct field inside a dataType's <struct>.
                    "variable"
                        if datatype
                            .as_ref()
                            .is_some_and(|d| d.kind.as_deref() == Some("struct"))
                            && parent == "struct" =>
                    {
                        if let Some(d) = &mut datatype {
                            d.var_acc = Some((
                                s.attr("name").unwrap_or("?").to_string(),
                                VarType::Unsupported(String::new()),
                            ));
                        }
                    }
                    // Interface variable of an open POU.
                    "variable" if pou.as_ref().is_some_and(|p| p.var_kind.is_some()) => {
                        if let Some(p) = &mut pou {
                            p.var_acc = Some(VarAcc {
                                name: s.attr("name").unwrap_or("?").to_string(),
                                type_spec: None,
                                init: None,
                                literal_init: false,
                            });
                        }
                    }
                    "struct" if parent == "baseType" => {
                        if let Some(d) = &mut datatype {
                            d.kind = Some("struct".into());
                        }
                    }
                    "enum" | "array" | "subrangeSigned" | "subrangeUnsigned"
                        if parent == "baseType" =>
                    {
                        if let Some(d) = &mut datatype {
                            d.kind = Some(name.clone());
                            d.unsupported = Some(format!("{name} datatype"));
                        }
                    }
                    "derived" if parent == "baseType" || parent == "type" => {
                        let derived_name = s.attr("name").unwrap_or("?").to_string();
                        if parent == "baseType" {
                            if let Some(d) = &mut datatype {
                                d.kind = Some("derived".into());
                                d.unsupported = Some("derived (alias) type".into());
                            }
                        } else if let Some(d) = &mut datatype {
                            if let Some((_, spec)) = &mut d.var_acc {
                                *spec = VarType::Derived(derived_name);
                            }
                        } else if let Some(p) = &mut pou {
                            if let Some(v) = &mut p.var_acc {
                                v.type_spec = Some(VarType::Derived(derived_name));
                            }
                        }
                    }
                    "initialValue" if pou.as_ref().is_some_and(|p| p.var_acc.is_some()) => {}
                    "simpleValue" if parent == "initialValue" => {
                        if let Some(p) = &mut pou {
                            if let Some(v) = &mut p.var_acc {
                                v.init = s.attr("value").map(str::to_owned);
                            }
                        }
                    }
                    "structValue" | "arrayValue" if parent == "initialValue" => {
                        if let Some(p) = &mut pou {
                            if let Some(v) = &mut p.var_acc {
                                v.literal_init = true;
                            }
                        }
                    }
                    "ST" | "IL" | "LD" | "FBD" | "SFC" if parent == "body" => {
                        if let Some(p) = &mut pou {
                            if name == "ST" {
                                p.st_buffer = Some(String::new());
                                p.st_depth = 1;
                            } else {
                                builder.skipped(Skipped {
                                    name: format!("{}/body#{}", p.name, name),
                                    reason: body_skip_reason(&name),
                                });
                            }
                        }
                    }
                    _ => {
                        // Elementary type elements (<BOOL/>, <string/>...) in
                        // type positions.
                        if let Some(spec) = set_type_from_element(&name, &parent) {
                            if parent == "baseType" {
                                if let Some(d) = &mut datatype {
                                    if d.kind.is_none() {
                                        d.kind = Some("elementary".into());
                                        d.unsupported = match &spec {
                                            VarType::Elementary(t) => {
                                                Some(format!("alias to elementary type {t}"))
                                            }
                                            VarType::Unsupported(t) => {
                                                Some(format!("alias to type {t}"))
                                            }
                                            VarType::Derived(_) => None,
                                        };
                                    }
                                }
                            } else if let Some(d) = &mut datatype {
                                if let Some((_, field_spec)) = &mut d.var_acc {
                                    *field_spec = spec;
                                }
                            } else if let Some(p) = &mut pou {
                                if let Some(v) = &mut p.var_acc {
                                    v.type_spec = Some(spec);
                                }
                            }
                        }
                    }
                }
                path.push(name.clone());
                // Inside an ST body, an element boundary within the XHTML
                // wrapper becomes a line break (bodies usually carry one
                // text chunk or one <p> per line).
                if let Some(p) = &mut pou {
                    if p.st_buffer.is_some() {
                        p.st_depth += 1;
                        if p.st_depth > 2 {
                            let buf = p.st_buffer.as_mut().unwrap();
                            if !buf.is_empty() && !buf.ends_with('\n') {
                                buf.push('\n');
                            }
                        }
                    }
                }
                continue;
            }
            other => other,
        };

        match ev {
            Ev::Text(text) => {
                if let Some(buf) = pou.as_mut().and_then(|p| p.st_buffer.as_mut()) {
                    buf.push_str(&text);
                }
            }
            Ev::End(name) => {
                match name.as_str() {
                    "ST" | "IL" | "LD" | "FBD" | "SFC"
                        if path.len() >= 2 && path[path.len() - 2] == "body" =>
                    {
                        if let Some(p) = &mut pou {
                            if name == "ST" {
                                if let Some(buf) = p.st_buffer.take() {
                                    let trimmed = trim_xml_ws(&buf).to_string();
                                    if trimmed.is_empty() {
                                        builder.errors(ImportError {
                                            context: p.name.clone(),
                                            message: "ST body contains no text".into(),
                                        });
                                    } else if p.body.is_none() {
                                        p.body = Some(trimmed);
                                    } else {
                                        builder.skipped(Skipped {
                                            name: format!("{}/body#ST", p.name),
                                            reason: "second ST body of the POU; only the \
                                                     first is imported"
                                                .into(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    "variable" if datatype.as_ref().is_some_and(|d| d.var_acc.is_some()) => {
                        if let Some(d) = &mut datatype {
                            if let Some((fname, spec)) = d.var_acc.take() {
                                match spec {
                                    VarType::Elementary(t) => d.fields.push((fname, t)),
                                    VarType::Derived(n) => {
                                        d.fields.push((fname, sanitize_ident(&n)))
                                    }
                                    VarType::Unsupported(_) => {
                                        if d.unsupported.is_none() {
                                            d.unsupported =
                                                Some("field with unsupported type".into());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "variable" if pou.as_ref().is_some_and(|p| p.var_acc.is_some()) => {
                        let kind = pou.as_ref().and_then(|p| p.var_kind);
                        let external = pou.as_ref().is_some_and(|p| p.external);
                        if let (Some(p), Some(kind)) = (pou.as_mut(), kind) {
                            if let Some(v) = p.var_acc.take() {
                                finish_variable(v, kind, external, &structs, p, &mut builder);
                            }
                            p.external = false;
                        }
                    }
                    "inputVars" | "outputVars" | "inOutVars" | "localVars" | "tempVars"
                    | "externalVars" => {
                        if let Some(p) = &mut pou {
                            p.var_kind = None;
                        }
                    }
                    "pou" => {
                        if let Some(p) = pou.take() {
                            finish_pou(p, &structs, &mut builder);
                        }
                    }
                    "dataType" => {
                        if let Some(d) = datatype.take() {
                            finish_datatype(d, &mut structs, &mut builder);
                        }
                    }
                    "dataTypes" if in_data_types => in_data_types = false,
                    _ => {}
                }
                path.pop();
            }
            Ev::Eof => {
                if !path.is_empty() {
                    return Err(XmlError::Text(format!(
                        "unexpected end of document: {} unclosed element(s), innermost <{}>",
                        path.len(),
                        path[path.len() - 1]
                    )));
                }
                break;
            }
            Ev::Start(_) => unreachable!("start events are consumed above"),
        }
    }

    if !root_seen {
        return Err(XmlError::Text(
            "root element is not <project>: not a PLCopen TC6 document".into(),
        ));
    }

    Ok(builder)
}

fn body_skip_reason(lang: &str) -> String {
    match lang {
        "IL" => "instruction list (IL) body is text but not Structured Text".into(),
        "LD" => {
            "ladder diagram (LD) body is a graphics sheet of contacts and coils, not IEC ST".into()
        }
        "FBD" => "function block diagram (FBD) body is a graphics sheet of blocks and wires, not \
             IEC ST"
            .into(),
        "SFC" => "sequential function chart (SFC) body encodes steps and transitions graphically, \
             not IEC ST"
            .into(),
        other => format!("unsupported body language '{other}'"),
    }
}

/// Decide one interface variable and, when mappable, add its declaration to
/// the POU's var group and struct reachability roots.
fn finish_variable(
    v: VarAcc,
    kind: VarKind,
    external: bool,
    structs: &[StructType],
    pou: &mut PouAcc,
    builder: &mut Builder,
) {
    let Some(spec) = v.type_spec else {
        builder.errors(ImportError {
            context: format!("{}/{}", pou.name, v.name),
            message: "variable has no <type> element".into(),
        });
        return;
    };
    if external {
        builder.skipped(Skipped {
            name: format!("{}/{}", pou.name, v.name),
            reason: "external variables reference global scope, which has no counterpart in \
                     the flat Voltrus project model"
                .into(),
        });
        return;
    }
    if v.literal_init {
        builder.skipped(Skipped {
            name: format!("{}/{}", pou.name, v.name),
            reason: "initialValue is a struct/array literal, which is not translated".into(),
        });
        return;
    }
    let (type_text, struct_root) = match spec {
        VarType::Elementary(t) => (t, false),
        VarType::Derived(name) => {
            if SUPPORTED_FBS
                .iter()
                .any(|fb| fb.eq_ignore_ascii_case(&name))
            {
                (name.to_ascii_uppercase(), false)
            } else if structs.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
                let sanitized = sanitize_ident(&name);
                (sanitized, true)
            } else {
                builder.skipped(Skipped {
                    name: format!("{}/{}", pou.name, v.name),
                    reason: format!(
                        "unknown derived type '{name}': not an elementary type, a supported \
                         function block (TON/TOF/TP/CTU/CTD/CTUD) or a parsed struct"
                    ),
                });
                return;
            }
        }
        VarType::Unsupported(what) => {
            builder.skipped(Skipped {
                name: format!("{}/{}", pou.name, v.name),
                reason: format!("type '{what}' is not supported by the mt-stc runtime"),
            });
            return;
        }
    };
    let sanitized = sanitize_ident(&v.name);
    if let Some(skip) = lexer_skip(&sanitized, format!("{}/{}", pou.name, v.name)) {
        builder.skipped(skip);
        return;
    }
    if struct_root {
        pou.type_roots.push(type_text.clone());
    }
    let group = pou
        .groups
        .iter_mut()
        .find(|(k, _)| *k == kind)
        .map(|(_, g)| g);
    if let Some(group) = group {
        group.push(Decl::new(&sanitized, &type_text, v.init.as_deref()));
    }
}

/// Shared iecst lexer-defect gate for emitted identifiers.
fn lexer_skip(sanitized: &str, context: String) -> Option<Skipped> {
    if is_iecst_lexer_safe(sanitized) {
        return None;
    }
    Some(Skipped {
        name: context,
        reason: format!(
            "name '{sanitized}' trips an iecst 0.5 lexer defect (an embedded 'type' segment \
             is mis-tokenized); rename it in the source export"
        ),
    })
}

fn finish_datatype(d: DataTypeAcc, structs: &mut Vec<StructType>, builder: &mut Builder) {
    let display = format!("dataTypes/{}", d.name);
    match (d.kind.as_deref(), d.unsupported) {
        (Some("struct"), None) => {
            if d.fields.is_empty() {
                builder.errors(ImportError {
                    context: display,
                    message: "struct datatype has no fields".into(),
                });
                return;
            }
            let name = sanitize_ident(&d.name);
            if let Some(skip) = lexer_skip(&name, display.clone()) {
                builder.skipped(skip);
                return;
            }
            if structs.iter().any(|s| s.name == name) {
                builder.errors(ImportError {
                    context: display,
                    message: format!("duplicate datatype name after sanitization: '{name}'"),
                });
                return;
            }
            let mut fields = Vec::with_capacity(d.fields.len());
            for (f, t) in d.fields {
                let fname = sanitize_ident(&f);
                if let Some(skip) = lexer_skip(&fname, format!("{display}/{f}")) {
                    builder.skipped(skip);
                    return;
                }
                fields.push((fname, t));
            }
            structs.push(StructType { name, fields });
        }
        (Some(_), Some(what)) => {
            let reason = if what == "derived (alias) type" {
                "derived (alias) datatype is not translated".to_string()
            } else if let Some(t) = what.strip_prefix("alias to ") {
                format!("alias datatype to {t} is not translated")
            } else {
                format!("{what} is not translated; only struct datatypes are mapped")
            };
            builder.skipped(Skipped {
                name: display,
                reason,
            });
        }
        _ => {
            builder.errors(ImportError {
                context: display,
                message: "datatype has no recognizable baseType".into(),
            });
        }
    }
}

fn finish_pou(p: PouAcc, structs: &[StructType], builder: &mut Builder) {
    let pou_kind = match p.pou_type.as_str() {
        "program" => "PROGRAM",
        "functionBlock" => "FUNCTION_BLOCK",
        "function" => {
            builder.skipped(Skipped {
                name: p.name.clone(),
                reason: "FUNCTION POUs are not supported: the mt-stc code generator cannot \
                         assign the function-name return value"
                    .into(),
            });
            return;
        }
        other => {
            builder.errors(ImportError {
                context: p.name.clone(),
                message: format!("unknown pouType '{other}'"),
            });
            return;
        }
    };
    let has_vars = p.groups.iter().any(|(_, g)| !g.is_empty());
    if p.body.is_none() && !has_vars {
        builder.skipped(Skipped {
            name: p.name.clone(),
            reason: "POU has no ST body and no importable variables; there is nothing to \
                     emit"
                .into(),
        });
        return;
    }
    let name = sanitize_ident(&p.name);
    if let Some(skip) = lexer_skip(&name, p.name.clone()) {
        builder.skipped(skip);
        return;
    }
    let types = emit::reachable_structs(&p.type_roots, structs);
    builder.candidate(PouFile {
        name: format!("{name}.st"),
        pou_kind,
        pou_name: name,
        groups: p.groups,
        body: p.body.unwrap_or_default(),
        types,
    });
}
