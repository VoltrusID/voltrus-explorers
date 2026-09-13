//! FBD → ST port of `fbd-to-st.ts` + `topological-sort.ts`. Every behavior —
//! including temp-variable numbering order and diagnostic ordering — mirrors
//! the TypeScript exactly (see module docs in `mod.rs`).

use serde::Deserialize;
use std::collections::HashMap;

use super::{is_iec_identifier, or_named, Diag, TranspileResult};
use crate::diagram::blocks::{block_def, PinDef};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FbdProgram {
    #[serde(default)]
    pub version: String,
    pub program_name: String,
    #[serde(default)]
    pub nodes: Vec<FbdNode>,
    #[serde(default)]
    pub edges: Vec<FbdEdge>,
}

#[derive(Deserialize, Debug)]
pub struct FbdNode {
    pub id: String,
    pub data: FbdNodeData,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FbdNodeData {
    pub block_type: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub variable_name: Option<String>,
    #[serde(default)]
    pub variable_type: Option<String>,
    #[serde(default)]
    pub instance_name: Option<String>,
    #[serde(default)]
    pub pin_values: HashMap<String, String>,
    /// Tolerate unknown/extra fields (older formats, editor runtime leftovers).
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FbdEdge {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub source_handle: Option<String>,
    #[serde(default)]
    pub target_handle: Option<String>,
    #[serde(default)]
    pub data: Option<WireData>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WireData {
    #[serde(default)]
    pub source_pin: Option<String>,
    #[serde(default)]
    pub target_pin: Option<String>,
}

/// Kahn topological sort, insertion-order-faithful port of `topologicalSort`.
/// Returns node indices in emission order plus the cycle flag.
fn topological_sort(nodes: &[FbdNode], edges: &[FbdEdge]) -> (Vec<usize>, bool) {
    // JS `new Set(nodes.map(n => n.id))` keeps first-occurrence order.
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    let mut ids: Vec<&str> = Vec::new();
    for node in nodes {
        if !index_of.contains_key(node.id.as_str()) {
            index_of.insert(node.id.as_str(), ids.len());
            ids.push(node.id.as_str());
        }
    }

    let mut in_degree = vec![0usize; ids.len()];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];

    for edge in edges {
        if let (Some(&s), Some(&t)) = (
            index_of.get(edge.source.as_str()),
            index_of.get(edge.target.as_str()),
        ) {
            adj[s].push(t);
            in_degree[t] += 1;
        }
    }

    let mut order: Vec<usize> = Vec::new();
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (id, deg) in in_degree.iter().enumerate() {
        if *deg == 0 {
            queue.push_back(id);
        }
    }
    while let Some(current) = queue.pop_front() {
        order.push(current);
        for &neighbor in &adj[current] {
            in_degree[neighbor] -= 1;
            if in_degree[neighbor] == 0 {
                queue.push_back(neighbor);
            }
        }
    }

    let cyclic = order.len() != ids.len();
    (order, cyclic)
}

struct Ctx<'a> {
    nodes: &'a [FbdNode],
    edges: &'a [FbdEdge],
    node_map: HashMap<&'a str, usize>,
    /// target node id → (sourceNode, sourcePin, targetPin), in edge order.
    edge_targets: HashMap<&'a str, Vec<(&'a str, &'a str, &'a str)>>,
    /// nodeId → (pin → temp name), both insertion-ordered like the JS Maps.
    signal_names: Vec<(&'a str, Vec<(&'a str, String)>)>,
    signal_types: HashMap<String, String>,
    temp_counter: u32,
}

impl<'a> Ctx<'a> {
    fn data(&self, node_id: &str) -> Option<&'a FbdNodeData> {
        self.node_map.get(node_id).map(|&i| &self.nodes[i].data)
    }

    /// Faithful port of the TS `getSignalName` closure.
    fn get_signal_name(&mut self, node_id: &'a str, pin: &'a str) -> String {
        let Some(data) = self.data(node_id) else {
            return "__unknown".to_string();
        };

        if data.block_type == "INPUT_PIN" {
            let name = or_named(data.variable_name.as_deref(), format!("__input_{node_id}"));
            self.signal_types.insert(
                name.clone(),
                data.variable_type.clone().unwrap_or_else(|| "BOOL".into()),
            );
            return name;
        }

        if let Some(def) = block_def(&data.block_type) {
            if def.is_function_block {
                let instance = or_named(data.instance_name.as_deref(), format!("__fb_{node_id}"));
                let sig = format!("{instance}.{pin}");
                if let Some(pin_def) = def.pin(pin) {
                    self.signal_types.insert(sig.clone(), pin_def.data_type.to_string());
                }
                return sig;
            }
        }

        // Check if output connects directly to an OUTPUT_PIN.
        for edge in self.edges {
            if edge.source != node_id {
                continue;
            }
            let matches = edge.source_handle.as_deref() == Some(pin) || pin == "OUT";
            if !matches {
                continue;
            }
            if let Some(target) = self.data(&edge.target) {
                if target.block_type == "OUTPUT_PIN" {
                    let name = or_named(
                        target.variable_name.as_deref(),
                        format!("__output_{}", edge.target),
                    );
                    self.signal_types.insert(
                        name.clone(),
                        target.variable_type.clone().unwrap_or_else(|| "BOOL".into()),
                    );
                    return name;
                }
            }
        }

        // Internal wire → temp variable with correct type.
        if self.signal_names.iter().all(|(id, _)| *id != node_id) {
            self.signal_names.push((node_id, Vec::new()));
        }
        let entry = self
            .signal_names
            .iter_mut()
            .find(|(id, _)| *id == node_id)
            .expect("just inserted");
        if let Some((_, name)) = entry.1.iter().find(|(p, _)| *p == pin) {
            return name.clone();
        }
        self.temp_counter += 1;
        let temp_name = format!("__t{}", self.temp_counter);
        entry.1.push((pin, temp_name.clone()));
        let pin_type = block_def(&data.block_type)
            .and_then(|def| def.pin(pin))
            .map(|p: &PinDef| p.data_type)
            .unwrap_or("BOOL");
        self.signal_types.insert(temp_name.clone(), pin_type.to_string());
        temp_name
    }
}

pub fn transpile(program: &FbdProgram) -> TranspileResult {
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

    let (order, cycle) = topological_sort(nodes, edges);
    if cycle {
        diagnostics.push(Diag {
            message: "Diagram contains a cycle — FBD execution order must be acyclic".into(),
            severity: "error".into(),
        });
        return TranspileResult { source: String::new(), diagnostics };
    }

    let mut node_map: HashMap<&str, usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        node_map.insert(node.id.as_str(), i);
    }

    let mut edge_targets: HashMap<&str, Vec<(&str, &str, &str)>> = HashMap::new();
    for edge in edges {
        let target_pin = edge
            .data
            .as_ref()
            .and_then(|d| d.target_pin.as_deref())
            .or(edge.target_handle.as_deref())
            .unwrap_or("");
        let source_pin = edge
            .data
            .as_ref()
            .and_then(|d| d.source_pin.as_deref())
            .or(edge.source_handle.as_deref())
            .unwrap_or("OUT");
        edge_targets
            .entry(edge.target.as_str())
            .or_default()
            .push((edge.source.as_str(), source_pin, target_pin));
    }

    let mut ctx = Ctx {
        nodes,
        edges,
        node_map,
        edge_targets,
        signal_names: Vec::new(),
        signal_types: HashMap::new(),
        temp_counter: 0,
    };

    // Validate variable names.
    for node in nodes {
        let data = &node.data;
        if (data.block_type == "INPUT_PIN" || data.block_type == "OUTPUT_PIN")
            && data.variable_name.as_deref().is_some_and(|v| !v.is_empty())
        {
            let name = data.variable_name.as_deref().expect("checked");
            if !is_iec_identifier(name) {
                diagnostics.push(Diag {
                    message: format!(
                        "Invalid variable name: \"{name}\" — must match [A-Za-z_][A-Za-z0-9_]*"
                    ),
                    severity: "error".into(),
                });
            }
        }
        if data.instance_name.as_deref().is_some_and(|v| !v.is_empty())
            && !is_iec_identifier(data.instance_name.as_deref().expect("checked"))
        {
            diagnostics.push(Diag {
                message: format!(
                    "Invalid instance name: \"{}\" — must match [A-Za-z_][A-Za-z0-9_]*",
                    data.instance_name.as_deref().expect("checked")
                ),
                severity: "error".into(),
            });
        }
    }

    if diagnostics.iter().any(|d| d.severity == "error") {
        return TranspileResult { source: String::new(), diagnostics };
    }

    // Collect declarations.
    let mut var_inputs: Vec<String> = Vec::new();
    let mut var_outputs: Vec<String> = Vec::new();
    let mut var_fb_instances: Vec<String> = Vec::new();

    for node in nodes {
        let data = &node.data;
        let Some(def) = block_def(&data.block_type) else {
            continue;
        };
        if data.block_type == "INPUT_PIN" && data.variable_name.as_deref().is_some_and(|v| !v.is_empty())
        {
            var_inputs.push(format!(
                "    {} : {};",
                data.variable_name.as_deref().expect("checked"),
                data.variable_type.clone().unwrap_or_else(|| "BOOL".into())
            ));
        } else if data.block_type == "OUTPUT_PIN"
            && data.variable_name.as_deref().is_some_and(|v| !v.is_empty())
        {
            var_outputs.push(format!(
                "    {} : {};",
                data.variable_name.as_deref().expect("checked"),
                data.variable_type.clone().unwrap_or_else(|| "BOOL".into())
            ));
        } else if def.is_function_block && data.instance_name.as_deref().is_some_and(|v| !v.is_empty())
        {
            var_fb_instances.push(format!(
                "    {} : {};",
                data.instance_name.as_deref().expect("checked"),
                data.block_type
            ));
        }
    }

    // Generate ST body (also populates signalNames/signalTypes).
    let mut body: Vec<String> = Vec::new();

    for id in &order {
        let node = &nodes[*id];
        let data = &node.data;
        let Some(def) = block_def(&data.block_type) else {
            continue;
        };

        if data.block_type == "INPUT_PIN" || data.block_type == "OUTPUT_PIN" {
            continue;
        }

        // Resolve input pin values from incoming edges (keyed by targetPin).
        let mut pin_values: HashMap<&str, String> = HashMap::new();
        let incoming: Vec<(&str, &str, &str)> = ctx
            .edge_targets
            .get(node.id.as_str())
            .cloned()
            .unwrap_or_default();
        for (source_node, source_pin, target_pin) in &incoming {
            pin_values.insert(target_pin, ctx.get_signal_name(source_node, source_pin));
        }

        // Apply constant pin values.
        for (k, v) in &data.pin_values {
            if !v.is_empty() && !pin_values.contains_key(k.as_str()) {
                pin_values.insert(k.as_str(), v.clone());
            }
        }

        if def.is_function_block {
            let instance = or_named(data.instance_name.as_deref(), format!("__fb_{}", node.id));
            let args = def
                .pins
                .iter()
                .filter(|p| p.input)
                .map(|p| match pin_values.get(p.name) {
                    Some(val) if !val.is_empty() => format!("{} := {}", p.name, val),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            body.push(format!("    {instance}({args});"));

            // Copy FB outputs connected to output pins.
            for out_pin in def.pins.iter().filter(|p| !p.input) {
                for edge in edges {
                    if edge.source != node.id.as_str()
                        || edge.source_handle.as_deref() != Some(out_pin.name)
                    {
                        continue;
                    }
                    if let Some(target) = ctx.data(&edge.target) {
                        if target.block_type == "OUTPUT_PIN"
                            && target.variable_name.as_deref().is_some_and(|v| !v.is_empty())
                        {
                            body.push(format!(
                                "    {} := {}.{};",
                                target.variable_name.as_deref().expect("checked"),
                                instance,
                                out_pin.name
                            ));
                        }
                    }
                }
            }
        } else {
            if !def.pins.iter().any(|p| !p.input) {
                continue;
            }

            // Build all placeholder values (output pins first, then inputs).
            let mut placeholders: Vec<(&str, String)> = Vec::new();
            for pin in def.pins.iter().filter(|p| !p.input) {
                placeholders.push((pin.name, ctx.get_signal_name(&node.id, pin.name)));
            }
            for pin in def.pins.iter().filter(|p| p.input) {
                placeholders.push((
                    pin.name,
                    pin_values
                        .get(pin.name)
                        .cloned()
                        .unwrap_or_else(|| "__unconnected".into()),
                ));
            }

            let mut stmt = def.st_template.to_string();
            for (key, val) in &placeholders {
                stmt = stmt.replace(&format!("{{{{{key}}}}}"), val);
            }
            body.push(format!("    {stmt}"));
        }
    }

    // Collect temp variables with types (AFTER body generation).
    let mut temp_decls: Vec<String> = Vec::new();
    for (_, pin_map) in &ctx.signal_names {
        for (_, name) in pin_map {
            let dt = ctx
                .signal_types
                .get(name)
                .map(String::as_str)
                .unwrap_or("BOOL");
            temp_decls.push(format!("    {name} : {dt};"));
        }
    }

    // Check for unconnected pins (warnings only).
    for node in nodes {
        let data = &node.data;
        let Some(def) = block_def(&data.block_type) else {
            continue;
        };
        if data.block_type == "INPUT_PIN" || data.block_type == "OUTPUT_PIN" {
            continue;
        }
        let incoming = ctx.edge_targets.get(node.id.as_str()).cloned().unwrap_or_default();
        let connected_target_pins: std::collections::HashSet<&str> =
            incoming.iter().map(|(_, _, tp)| *tp).collect();
        for pin in def.pins.iter().filter(|p| p.input) {
            let has_connection = connected_target_pins.contains(pin.name)
                || data.pin_values.get(pin.name).is_some_and(|v| !v.is_empty());
            if !has_connection {
                diagnostics.push(Diag {
                    message: format!(
                        "Block \"{}\" pin \"{}\" is not connected",
                        data.label, pin.name
                    ),
                    severity: "warning".into(),
                });
            }
        }
    }

    // Assemble.
    let mut sections: Vec<String> = vec![format!("PROGRAM {}", program.program_name)];
    if !var_inputs.is_empty() {
        sections.push("VAR_INPUT".into());
        sections.extend(var_inputs);
        sections.push("END_VAR".into());
    }
    if !var_outputs.is_empty() {
        sections.push("VAR_OUTPUT".into());
        sections.extend(var_outputs);
        sections.push("END_VAR".into());
    }
    let var_body: Vec<String> = var_fb_instances
        .into_iter()
        .chain(temp_decls)
        .collect();
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

    TranspileResult { source: sections.join("\n"), diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_is_minimal() {
        let program = FbdProgram {
            version: "1.1".into(),
            program_name: "Empty".into(),
            nodes: vec![],
            edges: vec![],
        };
        let result = transpile(&program);
        assert_eq!(result.source, "PROGRAM Empty\nEND_PROGRAM");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn cycle_errors_with_ts_message() {
        let json = r#"{
            "version": "1.1", "programName": "Loop",
            "nodes": [
                { "id": "a", "data": { "blockType": "AND", "label": "AND", "pinValues": {} } },
                { "id": "b", "data": { "blockType": "AND", "label": "AND", "pinValues": {} } }
            ],
            "edges": [
                { "source": "a", "target": "b", "sourceHandle": "OUT", "targetHandle": "IN1", "data": { "sourcePin": "OUT", "targetPin": "IN1" } },
                { "source": "b", "target": "a", "sourceHandle": "OUT", "targetHandle": "IN1", "data": { "sourcePin": "OUT", "targetPin": "IN1" } }
            ]
        }"#;
        let program: FbdProgram = serde_json::from_str(json).unwrap();
        let result = transpile(&program);
        assert_eq!(result.source, "");
        assert_eq!(
            result.diagnostics,
            vec![Diag {
                message: "Diagram contains a cycle — FBD execution order must be acyclic".into(),
                severity: "error".into(),
            }]
        );
    }
}
