//! LD → ST port of `ld-to-st.ts`. Power-flow semantics: contacts within a
//! path AND, paths of a rung OR; last coil/fb_block encountered wins as the
//! rung output; X-register contacts are VAR_INPUT, Y-register coils
//! VAR_OUTPUT, everything else VAR; edge-detect contacts hoist R_TRIG/F_TRIG
//! instances whose CLK calls are inserted at the front of the body via
//! `unshift` in a loop (i.e. reverse insertion order — replicated here).

use serde::Deserialize;
use std::collections::HashMap;

use super::{Diag, TranspileResult, is_iec_identifier, or_named};
use crate::diagram::blocks::block_def;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LdProgram {
    #[serde(default)]
    pub version: String,
    pub program_name: String,
    #[serde(default)]
    pub rungs: Vec<LdRung>,
}

#[derive(Deserialize, Debug)]
pub struct LdRung {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub paths: Vec<LdPath>,
}

#[derive(Deserialize, Debug)]
pub struct LdPath {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub elements: Vec<LdElement>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LdElement {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub variable: Option<String>,
    #[serde(default)]
    pub variable_type: Option<String>,
    #[serde(default)]
    pub register_type: Option<String>,
    #[serde(default)]
    pub block_type: Option<String>,
    #[serde(default)]
    pub instance_name: Option<String>,
    #[serde(default)]
    pub pin_values: HashMap<String, String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn is_contact(kind: &str) -> bool {
    matches!(
        kind,
        "contact_no" | "contact_nc" | "contact_re" | "contact_fe"
    )
}

fn is_coil(kind: &str) -> bool {
    matches!(kind, "coil" | "coil_set" | "coil_reset" | "coil_toggle")
}

/// Insertion-ordered `Map<String, String>` with JS `Map.set` semantics
/// (update value in place, keep original position).
#[derive(Default)]
struct OrderedMap {
    entries: Vec<(String, String)>,
}

impl OrderedMap {
    fn upsert(&mut self, key: String, value: String) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    fn push_unique(&mut self, key: String, value: String) {
        if !self.contains(&key) {
            self.entries.push((key, value));
        }
    }
}

struct Ctx {
    var_inputs: Vec<String>,
    var_outputs: Vec<String>,
    var_inouts: OrderedMap,
    fb_instances: OrderedMap,
    edge_instances: OrderedMap,
    edge_counter: u32,
}

impl Ctx {
    fn contact_expr(&mut self, el: &LdElement) -> String {
        let v = or_named(el.variable.as_deref(), format!("__var_{}", el.id));
        match el.kind.as_str() {
            "contact_no" => v,
            "contact_nc" => format!("NOT {v}"),
            "contact_re" => {
                self.edge_counter += 1;
                let inst = format!("__rtrig_{}", self.edge_counter);
                self.edge_instances.upsert(inst.clone(), v);
                format!("{inst}.Q")
            }
            "contact_fe" => {
                self.edge_counter += 1;
                let inst = format!("__ftrig_{}", self.edge_counter);
                self.edge_instances.upsert(inst.clone(), v);
                format!("{inst}.Q")
            }
            _ => v,
        }
    }

    fn track_var(&mut self, el: &LdElement, is_input: bool) {
        let Some(variable) = el.variable.as_deref().filter(|v| !v.is_empty()) else {
            return;
        };
        let reg = el.register_type.as_deref();
        if reg == Some("X") && is_input {
            if !self.var_inputs.iter().any(|v| v == variable) {
                self.var_inputs.push(variable.to_string());
            }
        } else if reg == Some("Y") && !is_input {
            if !self.var_outputs.iter().any(|v| v == variable) {
                self.var_outputs.push(variable.to_string());
            }
        } else {
            self.var_inouts.push_unique(
                variable.to_string(),
                el.variable_type.clone().unwrap_or_else(|| "BOOL".into()),
            );
        }
    }
}

pub fn transpile(program: &LdProgram) -> TranspileResult {
    let mut diagnostics: Vec<Diag> = Vec::new();

    if program.rungs.is_empty() {
        return TranspileResult::empty_program(&program.program_name);
    }

    if !is_iec_identifier(&program.program_name) {
        diagnostics.push(Diag {
            message: format!("Invalid program name: \"{}\"", program.program_name),
            severity: "error".into(),
        });
    }

    // Validate names.
    for rung in &program.rungs {
        for path in &rung.paths {
            for el in &path.elements {
                if el.variable.as_deref().is_some_and(|v| !v.is_empty())
                    && !is_iec_identifier(el.variable.as_deref().expect("checked"))
                {
                    diagnostics.push(Diag {
                        message: format!(
                            "Invalid variable name: \"{}\"",
                            el.variable.as_deref().expect("checked")
                        ),
                        severity: "error".into(),
                    });
                }
                if el.instance_name.as_deref().is_some_and(|v| !v.is_empty())
                    && !is_iec_identifier(el.instance_name.as_deref().expect("checked"))
                {
                    diagnostics.push(Diag {
                        message: format!(
                            "Invalid instance name: \"{}\"",
                            el.instance_name.as_deref().expect("checked")
                        ),
                        severity: "error".into(),
                    });
                }
            }
        }
    }

    if diagnostics.iter().any(|d| d.severity == "error") {
        return TranspileResult {
            source: String::new(),
            diagnostics,
        };
    }

    let mut ctx = Ctx {
        var_inputs: Vec::new(),
        var_outputs: Vec::new(),
        var_inouts: OrderedMap::default(),
        fb_instances: OrderedMap::default(),
        edge_instances: OrderedMap::default(),
        edge_counter: 0,
    };

    // Generate body.
    let mut body: Vec<String> = Vec::new();

    for rung in &program.rungs {
        let mut path_exprs: Vec<String> = Vec::new();
        let mut output_el: Option<&LdElement> = None;

        for path in &rung.paths {
            let mut contacts: Vec<String> = Vec::new();
            for el in &path.elements {
                if is_contact(&el.kind) {
                    ctx.track_var(el, true);
                    contacts.push(ctx.contact_expr(el));
                } else if is_coil(&el.kind) {
                    output_el = Some(el);
                    ctx.track_var(el, false);
                } else if el.kind == "fb_block" {
                    output_el = Some(el);
                }
            }
            if !contacts.is_empty() {
                path_exprs.push(contacts.join(" AND "));
            }
        }

        let rung_expr = if path_exprs.is_empty() {
            "TRUE".to_string()
        } else if path_exprs.len() == 1 {
            path_exprs[0].clone()
        } else {
            format!("({})", path_exprs.join(" OR "))
        };

        let Some(output_el) = output_el else {
            diagnostics.push(Diag {
                message: format!(
                    "Rung \"{}\" has no output",
                    rung.label.as_deref().unwrap_or(rung.id.as_str())
                ),
                severity: "warning".into(),
            });
            continue;
        };

        if is_coil(&output_el.kind) {
            let out_var = or_named(
                output_el.variable.as_deref(),
                format!("__coil_{}", output_el.id),
            );
            match output_el.kind.as_str() {
                "coil" => body.push(format!("    {out_var} := {rung_expr};")),
                "coil_set" => body.push(format!(
                    "    IF {rung_expr} THEN {out_var} := TRUE; END_IF;"
                )),
                "coil_reset" => body.push(format!(
                    "    IF {rung_expr} THEN {out_var} := FALSE; END_IF;"
                )),
                "coil_toggle" => {
                    body.push(format!(
                        "    IF {rung_expr} THEN {out_var} := NOT {out_var}; END_IF;"
                    ));
                    diagnostics.push(Diag {
                        message: format!(
                            "Toggle coil \"{out_var}\" toggles every scan cycle — consider using R_TRIG"
                        ),
                        severity: "warning".into(),
                    });
                }
                _ => {}
            }
        } else if output_el.kind == "fb_block" {
            let block_type = output_el.block_type.as_deref().unwrap_or("TON");
            if let Some(def) = block_def(block_type) {
                let instance = or_named(
                    output_el.instance_name.as_deref(),
                    format!("__fb_{}", output_el.id),
                );
                ctx.fb_instances
                    .upsert(instance.clone(), block_type.to_string());

                let mut fb_inputs: Vec<(String, String)> = vec![("IN".into(), rung_expr.clone())];
                for (k, v) in &output_el.pin_values {
                    if !v.is_empty() && !fb_inputs.iter().any(|(name, _)| name == k) {
                        fb_inputs.push((k.clone(), v.clone()));
                    }
                }

                let args = def
                    .pins
                    .iter()
                    .filter(|p| p.input)
                    .map(|p| {
                        let val = fb_inputs
                            .iter()
                            .find(|(name, _)| name == p.name)
                            .map(|(_, v)| v.as_str())
                            .unwrap_or("");
                        if val.is_empty() {
                            String::new()
                        } else {
                            format!("{} := {val}", p.name)
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                body.push(format!("    {instance}({args});"));
            }
        }
    }

    // Edge detection instances (JS `body.unshift` in a loop → reverse order).
    for (inst, var_name) in &ctx.edge_instances.entries {
        body.insert(0, format!("    {inst}(CLK := {var_name});"));
    }

    // Assemble.
    let mut sections: Vec<String> = vec![format!("PROGRAM {}", program.program_name)];
    if !ctx.var_inputs.is_empty() {
        sections.push("VAR_INPUT".into());
        sections.extend(ctx.var_inputs.iter().map(|v| format!("    {v} : BOOL;")));
        sections.push("END_VAR".into());
    }
    if !ctx.var_outputs.is_empty() {
        sections.push("VAR_OUTPUT".into());
        sections.extend(ctx.var_outputs.iter().map(|v| format!("    {v} : BOOL;")));
        sections.push("END_VAR".into());
    }

    let mut var_body: Vec<String> = Vec::new();
    for (name, ty) in &ctx.var_inouts.entries {
        var_body.push(format!("    {name} : {ty};"));
    }
    for (inst, block_type) in &ctx.fb_instances.entries {
        var_body.push(format!("    {inst} : {block_type};"));
    }
    for (inst, _) in &ctx.edge_instances.entries {
        let ty = if inst.contains("ftrig") {
            "F_TRIG"
        } else {
            "R_TRIG"
        };
        var_body.push(format!("    {inst} : {ty};"));
    }
    if !var_body.is_empty() {
        sections.push("VAR".into());
        sections.extend(var_body);
        sections.push("END_VAR".into());
    }
    if !body.is_empty() {
        sections.push(String::new());
        sections.extend(body);
    }
    sections.push("END_PROGRAM".into());

    TranspileResult {
        source: sections.join("\n"),
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_is_minimal() {
        let program = LdProgram {
            version: "2.0".into(),
            program_name: "Empty".into(),
            rungs: vec![],
        };
        let result = transpile(&program);
        assert_eq!(result.source, "PROGRAM Empty\nEND_PROGRAM");
    }
}
