//! SFC → ST port of `sfc-to-st.ts`: one BOOL per step (initial `:= TRUE`)
//! plus `_prev` shadows, transitions as
//! `IF <source> AND (<condition>) THEN <source> := FALSE; <target> := TRUE;`,
//! OR diverge/converge flattened by recursive `resolveDownstream`, jumps to
//! `TRUE`-condition transitions on the target step variable, and N/S/R/P1/P0
//! action qualifiers.

use serde::Deserialize;
use std::collections::HashMap;

use super::{is_iec_identifier, Diag, TranspileResult};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfcProgram {
    #[serde(default)]
    pub version: String,
    pub program_name: String,
    #[serde(default)]
    pub nodes: Vec<SfcNode>,
    #[serde(default)]
    pub edges: Vec<SfcEdge>,
}

#[derive(Deserialize, Debug)]
pub struct SfcNode {
    pub id: String,
    pub data: SfcNodeData,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfcNodeData {
    #[serde(rename = "elementType")]
    pub element_type: String,
    #[serde(default)]
    pub step_name: Option<String>,
    #[serde(default)]
    pub actions: Vec<SfcAction>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub target_step_name: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub pin_values: HashMap<String, String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfcAction {
    #[serde(default)]
    pub qualifier: String,
    #[serde(default)]
    pub variable_name: Option<String>,
    #[serde(default)]
    pub expression: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct SfcEdge {
    pub source: String,
    pub target: String,
}

fn is_step_like(data: &SfcNodeData) -> bool {
    data.element_type == "step" || data.element_type == "initial_step"
}

pub fn transpile(program: &SfcProgram) -> TranspileResult {
    let mut diagnostics: Vec<Diag> = Vec::new();
    let nodes = &program.nodes;
    let edges = &program.edges;

    if nodes.is_empty() {
        return TranspileResult::empty_program(&program.program_name);
    }

    if !is_iec_identifier(&program.program_name) {
        diagnostics.push(Diag {
            message: format!("Invalid program name: \"{}\"", program.program_name),
            severity: "error".into(),
        });
    }
    if diagnostics.iter().any(|d| d.severity == "error") {
        return TranspileResult { source: String::new(), diagnostics };
    }

    // Adjacency maps.
    let mut node_map: HashMap<&str, usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        node_map.insert(node.id.as_str(), i);
    }
    let mut outgoing: HashMap<&str, Vec<&SfcEdge>> = HashMap::new();
    let mut incoming: HashMap<&str, Vec<&SfcEdge>> = HashMap::new();
    for edge in edges {
        outgoing.entry(edge.source.as_str()).or_default().push(edge);
        incoming.entry(edge.target.as_str()).or_default().push(edge);
    }

    // Find initial step(s).
    let initial_steps: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.data.element_type == "initial_step")
        .map(|(i, _)| i)
        .collect();
    if initial_steps.is_empty() {
        diagnostics.push(Diag {
            message: "SFC must have exactly one initial step".into(),
            severity: "error".into(),
        });
    }
    if initial_steps.len() > 1 {
        diagnostics.push(Diag {
            message: format!(
                "Found {} initial steps — exactly one required",
                initial_steps.len()
            ),
            severity: "error".into(),
        });
    }
    if diagnostics.iter().any(|d| d.severity == "error") {
        return TranspileResult { source: String::new(), diagnostics };
    }

    // Validate step names.
    for node in nodes {
        if is_step_like(&node.data) {
            if let Some(name) = node.data.step_name.as_deref() {
                if !is_iec_identifier(name) {
                    diagnostics.push(Diag {
                        message: format!("Invalid step name: \"{name}\""),
                        severity: "error".into(),
                    });
                }
            }
        }
    }

    // Assign step numbers via BFS from the initial step.
    let mut step_var_map: HashMap<&str, String> = HashMap::new();
    let mut step_order: Vec<&str> = Vec::new();
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<&str> = std::collections::VecDeque::new();
    queue.push_back(nodes[initial_steps[0]].id.as_str());
    visited.insert(nodes[initial_steps[0]].id.as_str());
    let mut step_counter: u32 = 0;

    let all_step_ids: Vec<&str> = nodes
        .iter()
        .filter(|n| is_step_like(&n.data))
        .map(|n| n.id.as_str())
        .collect();

    while let Some(current) = queue.pop_front() {
        let Some(&idx) = node_map.get(current) else {
            continue;
        };
        let node = &nodes[idx];

        if is_step_like(&node.data) {
            step_counter += 1;
            step_var_map.insert(current, format!("_s{step_counter}"));
            step_order.push(current);
        }

        if let Some(out_edges) = outgoing.get(current) {
            for edge in out_edges {
                if !visited.contains(edge.target.as_str()) {
                    visited.insert(edge.target.as_str());
                    queue.push_back(edge.target.as_str());
                }
            }
        }
    }

    // Disconnected steps.
    for node_id in &all_step_ids {
        if !step_var_map.contains_key(node_id) {
            step_counter += 1;
            step_var_map.insert(node_id, format!("_s{step_counter}"));
            step_order.push(node_id);
            let name = node_map
                .get(node_id)
                .and_then(|&i| nodes[i].data.step_name.as_deref())
                .unwrap_or(node_id);
            diagnostics.push(Diag {
                message: format!("Step \"{name}\" is not reachable from the initial step"),
                severity: "warning".into(),
            });
        }
    }

    // Validate transitions and jumps.
    for node in nodes {
        if node.data.element_type == "transition" {
            if node.data.condition.as_deref().map_or(true, |c| c.trim().is_empty()) {
                diagnostics.push(Diag {
                    message: format!("Transition at node {} has no condition", node.id),
                    severity: "error".into(),
                });
            }
        }
        if node.data.element_type == "jump" {
            let target = node.data.target_step_name.as_deref().map(str::trim);
            match target.filter(|t| !t.is_empty()) {
                None => diagnostics.push(Diag {
                    message: format!("Jump at node {} has no target step name", node.id),
                    severity: "error".into(),
                }),
                Some(target) => {
                    let target_exists = nodes
                        .iter()
                        .any(|n| is_step_like(&n.data) && n.data.step_name.as_deref() == Some(target));
                    if !target_exists {
                        diagnostics.push(Diag {
                            message: format!("Jump target \"{target}\" does not exist"),
                            severity: "error".into(),
                        });
                    }
                }
            }
        }
    }

    if diagnostics.iter().any(|d| d.severity == "error") {
        return TranspileResult { source: String::new(), diagnostics };
    }

    // Warn about steps with no actions.
    for step_id in &step_order {
        let idx = node_map[step_id];
        let data = &nodes[idx].data;
        if data.actions.is_empty() {
            let name = data.step_name.as_deref().unwrap_or(step_id);
            diagnostics.push(Diag {
                message: format!("Step \"{name}\" has no actions"),
                severity: "warning".into(),
            });
        }
    }

    // Step name → var mapping for jumps.
    let mut step_name_to_var: HashMap<&str, &str> = HashMap::new();
    for step_id in &step_order {
        let idx = node_map[step_id];
        if let Some(name) = nodes[idx].data.step_name.as_deref() {
            step_name_to_var.insert(name, &step_var_map[step_id]);
        }
    }

    // === Generate ST ===

    // Action variables.
    let mut action_vars: Vec<(String, String)> = Vec::new();
    for step_id in &step_order {
        let idx = node_map[step_id];
        for act in &nodes[idx].data.actions {
            if let Some(name) = act.variable_name.as_deref().filter(|n| !n.is_empty()) {
                if !action_vars.iter().any(|(n, _)| n == name) {
                    action_vars.push((name.to_string(), "BOOL".to_string()));
                }
            }
        }
    }

    // VAR declarations.
    let mut var_lines: Vec<String> = Vec::new();
    for step_id in &step_order {
        let v = &step_var_map[step_id];
        let idx = node_map[step_id];
        let is_init = nodes[idx].data.element_type == "initial_step";
        var_lines.push(format!("    {v} : BOOL{};", if is_init { " := TRUE" } else { "" }));
    }
    for step_id in &step_order {
        let v = &step_var_map[step_id];
        var_lines.push(format!("    {v}_prev : BOOL;"));
    }
    for (name, ty) in &action_vars {
        var_lines.push(format!("    {name} : {ty};"));
    }

    // Resolve what comes after a node (handles OR divergence). Free function
    // so the recursion can borrow the immutable maps.
    fn resolve_downstream(
        node_map: &HashMap<&str, usize>,
        nodes: &[SfcNode],
        outgoing: &HashMap<&str, Vec<&SfcEdge>>,
        step_name_to_var: &HashMap<&str, &str>,
        node_id: &str,
    ) -> Vec<(String, String)> {
        let mut results: Vec<(String, String)> = Vec::new();
        let Some(&idx) = node_map.get(node_id) else {
            return results;
        };
        let data = &nodes[idx].data;
        let Some(out_edges) = outgoing.get(node_id) else {
            return results;
        };

        if is_step_like(data) {
            for edge in out_edges {
                let Some(&t_idx) = node_map.get(edge.target.as_str()) else {
                    continue;
                };
                let target = &nodes[t_idx].data;
                if target.element_type == "transition" || target.element_type == "or_diverge" {
                    results.extend(resolve_downstream(
                        node_map,
                        nodes,
                        outgoing,
                        step_name_to_var,
                        &edge.target,
                    ));
                } else if target.element_type == "jump" {
                    let target_step_name =
                        target.target_step_name.as_deref().unwrap_or("");
                    if let Some(target_var) = step_name_to_var.get(target_step_name) {
                        results.push((target_var.to_string(), "TRUE".into()));
                    }
                } else if is_step_like(target) {
                    results.push((edge.target.clone(), "TRUE".into()));
                }
            }
        } else if data.element_type == "transition" {
            for edge in out_edges {
                let Some(&t_idx) = node_map.get(edge.target.as_str()) else {
                    continue;
                };
                let target = &nodes[t_idx].data;
                let condition = data.condition.clone().unwrap_or_else(|| "TRUE".into());
                if is_step_like(target) {
                    results.push((edge.target.clone(), condition));
                } else if target.element_type == "or_converge" {
                    results.extend(resolve_downstream(
                        node_map,
                        nodes,
                        outgoing,
                        step_name_to_var,
                        &edge.target,
                    ));
                } else if target.element_type == "jump" {
                    let target_step_name =
                        target.target_step_name.as_deref().unwrap_or("");
                    if let Some(target_var) = step_name_to_var.get(target_step_name) {
                        results.push((target_var.to_string(), condition));
                    }
                }
            }
        } else if data.element_type == "or_diverge" || data.element_type == "or_converge" {
            for edge in out_edges {
                results.extend(resolve_downstream(
                    node_map,
                    nodes,
                    outgoing,
                    step_name_to_var,
                    &edge.target,
                ));
            }
        }

        results
    }

    // Body.
    let mut body: Vec<String> = Vec::new();

    // Store previous states.
    for step_id in &step_order {
        let v = &step_var_map[step_id];
        body.push(format!("    {v}_prev := {v};"));
    }

    // Transitions.
    for step_id in &step_order {
        let source_var = &step_var_map[step_id];
        let downstreams = resolve_downstream(
            &node_map,
            nodes,
            &outgoing,
            &step_name_to_var,
            step_id,
        );

        for (target_step_id, condition) in downstreams {
            let target_var = step_var_map
                .get(target_step_id.as_str())
                .cloned()
                .unwrap_or(target_step_id);
            body.push(format!("    IF {source_var} AND ({condition}) THEN"));
            body.push(format!("        {source_var} := FALSE;"));
            body.push(format!("        {target_var} := TRUE;"));
            body.push("    END_IF;".into());
        }
    }

    // Actions.
    for step_id in &step_order {
        let idx = node_map[step_id];
        let data = &nodes[idx].data;
        let v = &step_var_map[step_id];

        for act in &data.actions {
            let Some(name) = act.variable_name.as_deref().filter(|n| !n.is_empty()) else {
                continue;
            };
            match act.qualifier.as_str() {
                "N" => body.push(format!("    {name} := {v};")),
                "S" => body.push(format!(
                    "    IF {v} AND NOT {v}_prev THEN {name} := TRUE; END_IF;"
                )),
                "R" => body.push(format!(
                    "    IF {v} AND NOT {v}_prev THEN {name} := FALSE; END_IF;"
                )),
                "P1" => {
                    let expr = act
                        .expression
                        .clone()
                        .unwrap_or_else(|| format!("{name} := TRUE"));
                    body.push(format!("    IF {v} AND NOT {v}_prev THEN {expr}; END_IF;"));
                }
                "P0" => {
                    let expr = act
                        .expression
                        .clone()
                        .unwrap_or_else(|| format!("{name} := TRUE"));
                    body.push(format!("    IF NOT {v} AND {v}_prev THEN {expr}; END_IF;"));
                }
                _ => {}
            }
        }
    }

    // Assemble.
    let mut sections: Vec<String> = vec![format!("PROGRAM {}", program.program_name)];
    if !var_lines.is_empty() {
        sections.push("VAR".into());
        sections.extend(var_lines);
        sections.push("END_VAR".into());
    }
    if !body.is_empty() {
        sections.push(String::new());
        sections.extend(body);
    }
    sections.push("END_PROGRAM".into());

    TranspileResult { source: sections.join("\n"), diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_is_minimal() {
        let program = SfcProgram {
            version: "1.1".into(),
            program_name: "Empty".into(),
            nodes: vec![],
            edges: vec![],
        };
        let result = transpile(&program);
        assert_eq!(result.source, "PROGRAM Empty\nEND_PROGRAM");
    }

    #[test]
    fn missing_initial_step_errors() {
        let json = r#"{
            "version": "1.1", "programName": "NoInit",
            "nodes": [
                { "id": "s1", "data": { "elementType": "step", "stepName": "A", "pinValues": {} } }
            ],
            "edges": []
        }"#;
        let program: SfcProgram = serde_json::from_str(json).unwrap();
        let result = transpile(&program);
        assert_eq!(result.source, "");
        assert_eq!(
            result.diagnostics,
            vec![Diag {
                message: "SFC must have exactly one initial step".into(),
                severity: "error".into(),
            }]
        );
    }
}
