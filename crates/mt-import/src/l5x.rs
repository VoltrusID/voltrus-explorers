//! Rockwell / Studio 5000 L5X (`RSLogix5000Content`) reader.
//!
//! See the crate-level docs for the supported subset. This module walks the
//! XML once, collects mappable tags and ST routines, and hands candidate
//! files to the crate-level assembly + verification pipeline.
use crate::emit::{Decl, PouFile, VarKind};
use crate::mapping::{TypeDecision, is_iecst_lexer_safe, map_rockwell_type, sanitize_ident};
use crate::xml::{Ev, PullParser, XmlError, trim_xml_ws};
use crate::{Builder, ImportError, Skipped};

/// Import one L5X document. Malformed XML produces a `parse` error entry and
/// an otherwise empty report; every other anomaly is enumerated in `skipped`
/// or `errors` — nothing is dropped silently.
pub fn import(xml: &str) -> ImportReport {
    match walk(xml) {
        Ok(builder) => builder.finish(),
        Err(err) => Builder::parse_error(err),
    }
}

use crate::ImportReport;

/// Tags collected at controller or program scope.
#[derive(Default)]
struct TagSet {
    decls: Vec<Decl>,
    names: Vec<String>,
}

impl TagSet {
    fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }
}

struct ProgramAcc {
    name: String,
    /// `Use="Context"` programs are reference-only; nothing inside is imported.
    context: bool,
    tags: TagSet,
    files: Vec<PouFile>,
}

struct RoutineAcc {
    name: String,
    kind: String,
    st_lines: Vec<String>,
    in_st_content: bool,
    line: Option<String>,
}

fn walk(xml: &str) -> Result<Builder, XmlError> {
    let mut parser = PullParser::new(xml);
    let mut builder = Builder::new();
    let mut path: Vec<String> = Vec::new();

    let mut root_seen = false;
    let mut controller_name: Option<String> = None;
    let mut controller_tags = TagSet::default();
    let mut in_data_types = false;
    let mut program: Option<ProgramAcc> = None;
    let mut routine: Option<RoutineAcc> = None;
    // (name, tag_type, data_type, dimensions) of the open <Tag>.
    let mut tag_attrs: Option<(String, String, String, String)> = None;

    loop {
        let ev = match parser.next()? {
            Ev::Start(s) => {
                let name = s.local.clone();
                match name.as_str() {
                    "RSLogix5000Content" if path.is_empty() => root_seen = true,
                    "Controller" if path == ["RSLogix5000Content"] => {
                        controller_name = s.attr("Name").map(str::to_owned);
                    }
                    "DataTypes" if controller_name.is_some() && parent_is(&path, "Controller") => {
                        in_data_types = true;
                    }
                    "DataType" if in_data_types => {
                        builder.skipped(Skipped {
                            name: format!("DataTypes/{}", s.attr("Name").unwrap_or("?")),
                            reason: "user-defined type (UDT) definitions are not translated; \
                                     tags using this type are skipped individually with their \
                                     own reason"
                                .into(),
                        });
                    }
                    "AddOnInstructionDefinition" if parent_is_collection(&path) => {
                        builder.skipped(Skipped {
                            name: format!(
                                "AddOnInstructionDefinitions/{}",
                                s.attr("Name").unwrap_or("?")
                            ),
                            reason: "Add-On Instruction definitions (parameters, local tags \
                                     and nested routines) are a compound feature and are not \
                                     translated"
                                .into(),
                        });
                    }
                    "Tasks" | "Modules" | "Trends" | "DataLogs" | "QuickWatchLists"
                        if parent_is(&path, "Controller") =>
                    {
                        builder.skipped(Skipped {
                            name: name.clone(),
                            reason: format!(
                                "{name} are controller hardware/configuration data with no \
                                 counterpart in a flat ST project"
                            ),
                        });
                    }
                    "Tag" if is_tag_scope(&path) => {
                        tag_attrs = Some((
                            s.attr("Name").unwrap_or("").to_string(),
                            s.attr("TagType").unwrap_or("Base").to_string(),
                            s.attr("DataType").unwrap_or("").to_string(),
                            s.attr("Dimensions").unwrap_or("").to_string(),
                        ));
                    }
                    "Program" if parent_is(&path, "Programs") || path == ["RSLogix5000Content"] => {
                        program = Some(ProgramAcc {
                            name: sanitize_ident(s.attr("Name").unwrap_or("Program")),
                            context: s.attr("Use") == Some("Context"),
                            tags: TagSet::default(),
                            files: Vec::new(),
                        });
                        if program.as_ref().is_some_and(|p| p.context) {
                            builder.skipped(Skipped {
                                name: program.as_ref().unwrap().name.clone(),
                                reason: "context program (reference only, not part of the \
                                         export target); nothing inside is imported"
                                    .into(),
                            });
                        }
                    }
                    "Routine" if parent_is(&path, "Routines") => {
                        routine = Some(RoutineAcc {
                            name: s.attr("Name").unwrap_or("Routine").to_string(),
                            kind: s.attr("Type").unwrap_or("Typeless").to_string(),
                            st_lines: Vec::new(),
                            in_st_content: false,
                            line: None,
                        });
                    }
                    "STContent" if parent_is(&path, "Routine") => {
                        if let Some(r) = &mut routine {
                            r.in_st_content = true;
                        }
                    }
                    "Line" if routine.as_ref().is_some_and(|r| r.in_st_content) => {
                        if let Some(r) = &mut routine {
                            r.line = Some(String::new());
                        }
                    }
                    "Parameters" | "LocalTags" if parent_is(&path, "Program") => {
                        builder.skipped(Skipped {
                            name: format!(
                                "{}/{name}",
                                program.as_ref().map(|p| p.name.clone()).unwrap_or_default()
                            ),
                            reason: "program parameters/local tags are Add-On Instruction \
                                     scoping; plain programs export plain Tags"
                                .into(),
                        });
                    }
                    _ => {}
                }
                path.push(name);
                continue;
            }
            other => other,
        };

        match ev {
            Ev::Text(text) => {
                if let Some(line) = routine.as_mut().and_then(|r| r.line.as_mut()) {
                    line.push_str(&text);
                }
            }
            Ev::End(name) => {
                match name.as_str() {
                    "Line" => {
                        if let Some(r) = &mut routine {
                            if let Some(line) = r.line.take() {
                                r.st_lines.push(trim_xml_ws(&line).to_string());
                            }
                        }
                    }
                    "STContent" => {
                        if let Some(r) = &mut routine {
                            r.in_st_content = false;
                        }
                    }
                    "Tag" if tag_attrs.is_some() => {
                        if let Some((raw_name, tag_type, data_type, dimensions)) = tag_attrs.take()
                        {
                            let in_context_program = program.as_ref().is_some_and(|p| p.context);
                            let (scope, tags) = match &mut program {
                                Some(p) => (format!("{}/Tags", p.name), &mut p.tags),
                                None => ("Controller/Tags".to_string(), &mut controller_tags),
                            };
                            if !in_context_program {
                                add_tag(
                                    &raw_name,
                                    &tag_type,
                                    &data_type,
                                    &dimensions,
                                    scope,
                                    tags,
                                    &mut builder,
                                );
                            }
                        }
                    }
                    "Routine" => {
                        if let (Some(r), Some(p)) = (&mut routine, &mut program) {
                            if !p.context {
                                finish_routine(r, p, &mut builder);
                            }
                        }
                        routine = None;
                    }
                    "Program" => {
                        if let Some(p) = program.take() {
                            finish_program(p, &mut builder);
                        }
                    }
                    "DataTypes" if in_data_types => in_data_types = false,
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
            "root element is not RSLogix5000Content: not an L5X export".into(),
        ));
    }

    // Controller-scoped tags become a standalone tags-only POU.
    if !controller_tags.is_empty() {
        let name = format!(
            "{}_Tags",
            sanitize_ident(controller_name.as_deref().unwrap_or("Controller"))
        );
        builder.candidate(PouFile {
            name: format!("{name}.st"),
            pou_kind: "PROGRAM",
            pou_name: name,
            groups: vec![(VarKind::Var, controller_tags.decls)],
            body: String::new(),
            types: Vec::new(),
        });
    }

    Ok(builder)
}

fn parent_is(path: &[String], name: &str) -> bool {
    path.last().is_some_and(|p| p == name)
}

/// Collections whose children we skip as a unit.
fn parent_is_collection(path: &[String]) -> bool {
    path.last()
        .is_some_and(|p| p == "AddOnInstructionDefinitions")
}

/// `Tags` directly under `Controller` or under a `Program`.
fn is_tag_scope(path: &[String]) -> bool {
    parent_is(path, "Tags")
        && matches!(
            path.iter().rev().nth(1).map(|s| s.as_str()),
            Some("Controller" | "Program")
        )
}

fn add_tag(
    raw_name: &str,
    tag_type: &str,
    data_type: &str,
    dimensions: &str,
    scope: String,
    tags: &mut TagSet,
    builder: &mut Builder,
) {
    let display = format!("{scope}/{raw_name}");
    if tag_type.eq_ignore_ascii_case("Alias") {
        builder.skipped(Skipped {
            name: display,
            reason: "alias tags are not imported; their target may be controller-scoped or \
                     itself an alias"
                .into(),
        });
        return;
    }
    if !dimensions.is_empty() {
        builder.skipped(Skipped {
            name: display,
            reason: format!("array tags (dimensions '{dimensions}') are not supported"),
        });
        return;
    }
    match map_rockwell_type(data_type) {
        TypeDecision::Emit(iec_type) => {
            let name = sanitize_ident(raw_name);
            if !is_iecst_lexer_safe(&name) {
                builder.skipped(Skipped {
                    name: display,
                    reason: format!(
                        "sanitized name '{name}' trips an iecst 0.5 lexer defect (an \
                         embedded 'type' segment is mis-tokenized); rename the tag in the \
                         source export"
                    ),
                });
                return;
            }
            if tags.names.iter().any(|n| n == &name) {
                builder.errors(ImportError {
                    context: display,
                    message: format!(
                        "sanitized name '{name}' collides with an already imported tag; the \
                         tag is not emitted"
                    ),
                });
                return;
            }
            tags.names.push(name.clone());
            tags.decls.push(Decl::new(&name, &iec_type, None));
        }
        TypeDecision::Skip(reason) => {
            builder.skipped(Skipped {
                name: display,
                reason,
            });
        }
    }
}

fn finish_routine(routine: &mut RoutineAcc, program: &mut ProgramAcc, builder: &mut Builder) {
    let display = format!("{}/Routines/{}", program.name, routine.name);
    match routine.kind.as_str() {
        "ST" => {
            if routine.st_lines.iter().all(|l| l.is_empty()) {
                builder.errors(ImportError {
                    context: display,
                    message: "ST routine has no STContent lines".into(),
                });
                return;
            }
            let pou_name = sanitize_ident(&format!("{}_{}", program.name, routine.name));
            program.files.push(PouFile {
                name: format!("{pou_name}.st"),
                pou_kind: "PROGRAM",
                pou_name,
                groups: vec![(VarKind::Var, program.tags.decls.clone())],
                body: routine.st_lines.join("\n"),
                types: Vec::new(),
            });
        }
        "RLL" => builder.skipped(Skipped {
            name: display,
            reason: "ladder logic (LD): RLLContent rungs are a graphics sheet (contacts, \
                     coils and wires serialized per rung), not IEC ST; ladder import is not \
                     supported"
                .into(),
        }),
        "FBD" => builder.skipped(Skipped {
            name: display,
            reason: "function block diagram (FBD): FBDContent encodes blocks and wires \
                     graphically, not IEC ST"
                .into(),
        }),
        "SFC" => builder.skipped(Skipped {
            name: display,
            reason: "sequential function chart (SFC): SFCContent encodes steps, transitions \
                     and actions graphically, not IEC ST"
                .into(),
        }),
        "Encrypted" => builder.skipped(Skipped {
            name: display,
            reason: "routine source is protected (encrypted); the export contains no \
                     readable ST"
                .into(),
        }),
        "External" => builder.skipped(Skipped {
            name: display,
            reason: "external routine (ExternalContent) carries no ST source".into(),
        }),
        other => builder.skipped(Skipped {
            name: display,
            reason: format!("unknown routine type '{other}'"),
        }),
    }
}

fn finish_program(program: ProgramAcc, builder: &mut Builder) {
    if program.context {
        return;
    }
    if !program.files.is_empty() {
        builder.extend_candidates(program.files);
    } else if !program.tags.is_empty() {
        // Program tags must never vanish just because the program had no ST
        // routine; emit a tags-only POU like the controller scope does.
        builder.candidate(PouFile {
            name: format!("{}_Tags.st", program.name),
            pou_kind: "PROGRAM",
            pou_name: format!("{}_Tags", program.name),
            groups: vec![(VarKind::Var, program.tags.decls)],
            body: String::new(),
            types: Vec::new(),
        });
    }
}
