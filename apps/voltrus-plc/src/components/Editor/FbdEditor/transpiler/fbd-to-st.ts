import type { Node, Edge } from "@xyflow/react";
import type { BlockNodeData, FbdProgram, WireData, IECDataType } from "../../../../types/fbd";
import { getBlockDef } from "../../../../types/fbd";
import { topologicalSort } from "./topological-sort";

interface TranspileResult {
  source: string;
  diagnostics: Array<{ message: string; severity: "error" | "warning" }>;
}

const IEC_IDENTIFIER_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

function isValidIecIdentifier(name: string): boolean {
  return IEC_IDENTIFIER_RE.test(name);
}

export function transpileFbdToSt(program: FbdProgram): TranspileResult {
  const diagnostics: TranspileResult["diagnostics"] = [];
  const nodes = program.nodes as Node<BlockNodeData>[];
  const edges = program.edges as Edge<WireData>[];

  if (nodes.length === 0) {
    return { source: `PROGRAM ${program.programName}\nEND_PROGRAM`, diagnostics };
  }

  // Validate program name
  if (!isValidIecIdentifier(program.programName)) {
    diagnostics.push({ message: `Invalid program name: "${program.programName}"`, severity: "error" });
  }

  // Check for cycles
  const { order, cycle } = topologicalSort(nodes, edges);
  if (cycle) {
    diagnostics.push({ message: "Diagram contains a cycle. FBD execution order must be acyclic.", severity: "error" });
    return { source: "", diagnostics };
  }

  // Build lookup maps
  const nodeMap = new Map<string, Node<BlockNodeData>>(nodes.map((n: Node<BlockNodeData>) => [n.id, n]));

  // Build incoming edges map: targetNodeId → { sourceNode, sourcePin, targetPin }[]
  const edgeTargets = new Map<string, Array<{ sourceNode: string; sourcePin: string; targetPin: string }>>();
  for (const edge of edges) {
    const data = edge.data as WireData | undefined;
    const targetPin = data?.targetPin ?? edge.targetHandle ?? "";
    const sourcePin = data?.sourcePin ?? edge.sourceHandle ?? "OUT";
    const list = edgeTargets.get(edge.target) ?? [];
    list.push({ sourceNode: edge.source, sourcePin, targetPin });
    edgeTargets.set(edge.target, list);
  }

  // Track signal names and their data types
  const signalNames = new Map<string, Map<string, string>>();
  const signalTypes = new Map<string, IECDataType>();
  let tempCounter = 0;

  function getSignalName(nodeId: string, pin: string): string {
    const node = nodeMap.get(nodeId);
    if (!node) return "__unknown";

    const data = node.data;
    const def = getBlockDef(data.blockType);

    if (data.blockType === "INPUT_PIN") {
      const name = data.variableName || `__input_${nodeId}`;
      signalTypes.set(name, data.variableType || "BOOL");
      return name;
    }
    if (def?.isFunctionBlock) {
      const sig = `${data.instanceName || `__fb_${nodeId}`}.${pin}`;
      const pinDef = def.pins.find((p) => p.name === pin);
      if (pinDef) signalTypes.set(sig, pinDef.dataType);
      return sig;
    }

    // Check if output connects directly to an OUTPUT_PIN
    const outEdges = edges.filter((e: Edge<WireData>) => e.source === nodeId && (e.sourceHandle === pin || pin === "OUT"));
    for (const oe of outEdges) {
      const targetNode = nodeMap.get(oe.target);
      if (targetNode && targetNode.data.blockType === "OUTPUT_PIN") {
        const name = targetNode.data.variableName || `__output_${oe.target}`;
        signalTypes.set(name, targetNode.data.variableType || "BOOL");
        return name;
      }
    }

    // Internal wire → temp variable with correct type
    if (!signalNames.has(nodeId)) signalNames.set(nodeId, new Map());
    const nodeSignals = signalNames.get(nodeId)!;
    if (!nodeSignals.has(pin)) {
      tempCounter++;
      const tempName = `__t${tempCounter}`;
      nodeSignals.set(pin, tempName);
      // Derive type from the output pin definition
      const pinDef = def?.pins.find((p) => p.name === pin);
      signalTypes.set(tempName, pinDef?.dataType || "BOOL");
    }
    return nodeSignals.get(pin)!;
  }

  // Validate variable names
  for (const node of nodes) {
    const data = node.data;
    if ((data.blockType === "INPUT_PIN" || data.blockType === "OUTPUT_PIN") && data.variableName) {
      if (!isValidIecIdentifier(data.variableName)) {
        diagnostics.push({
          message: `Invalid variable name: "${data.variableName}"; must match [A-Za-z_][A-Za-z0-9_]*`,
          severity: "error",
        });
      }
    }
    if (data.instanceName && !isValidIecIdentifier(data.instanceName)) {
      diagnostics.push({
        message: `Invalid instance name: "${data.instanceName}"; must match [A-Za-z_][A-Za-z0-9_]*`,
        severity: "error",
      });
    }
  }

  if (diagnostics.some((d) => d.severity === "error")) {
    return { source: "", diagnostics };
  }

  // Collect declarations
  const varInputs: string[] = [];
  const varOutputs: string[] = [];
  const varFbInstances: string[] = [];

  for (const node of nodes) {
    const data = node.data;
    const def = getBlockDef(data.blockType);
    if (!def) continue;

    if (data.blockType === "INPUT_PIN" && data.variableName) {
      varInputs.push(`    ${data.variableName} : ${data.variableType || "BOOL"};`);
    } else if (data.blockType === "OUTPUT_PIN" && data.variableName) {
      varOutputs.push(`    ${data.variableName} : ${data.variableType || "BOOL"};`);
    } else if (def.isFunctionBlock && data.instanceName) {
      varFbInstances.push(`    ${data.instanceName} : ${data.blockType};`);
    }
  }

  // Generate ST body (also populates signalNames/signalTypes via getSignalName)
  const body: string[] = [];

  for (const id of order) {
    const node = nodeMap.get(id);
    if (!node) continue;
    const data = node.data;
    const def = getBlockDef(data.blockType);
    if (!def) continue;

    if (data.blockType === "INPUT_PIN" || data.blockType === "OUTPUT_PIN") continue;

    // Resolve input pin values from incoming edges (keyed by targetPin)
    const pinValues = new Map<string, string>();
    const incoming = edgeTargets.get(id) ?? [];
    for (const { sourceNode, sourcePin, targetPin } of incoming) {
      pinValues.set(targetPin, getSignalName(sourceNode, sourcePin));
    }

    // Apply constant pin values
    for (const [k, v] of Object.entries(data.pinValues)) {
      if (v && !pinValues.has(k)) pinValues.set(k, v);
    }

    if (def.isFunctionBlock) {
      const instance = data.instanceName || `__fb_${id}`;
      const args = def.pins
        .filter((p) => p.direction === "input")
        .map((p) => {
          const val = pinValues.get(p.name) ?? "";
          return val ? `${p.name} := ${val}` : "";
        })
        .filter(Boolean)
        .join(", ");
      body.push(`    ${instance}(${args});`);

      // Generate output assignments for FB outputs connected to output pins
      for (const outPin of def.pins.filter((p) => p.direction === "output")) {
        const outEdges = edges.filter((e) => e.source === id && e.sourceHandle === outPin.name);
        for (const oe of outEdges) {
          const targetNode = nodeMap.get(oe.target);
          if (targetNode && targetNode.data.blockType === "OUTPUT_PIN" && targetNode.data.variableName) {
            body.push(`    ${targetNode.data.variableName} := ${instance}.${outPin.name};`);
          }
        }
      }
    } else {
      const outputPin = def.pins.find((p) => p.direction === "output");
      if (!outputPin) continue;

      // Build all placeholder values
      const placeholders: Record<string, string> = {};
      for (const pin of def.pins.filter((p) => p.direction === "output")) {
        placeholders[pin.name] = getSignalName(id, pin.name);
      }
      for (const pin of def.pins.filter((p) => p.direction === "input")) {
        placeholders[pin.name] = pinValues.get(pin.name) || "__unconnected";
      }

      let stmt = def.stTemplate;
      for (const [key, val] of Object.entries(placeholders)) {
        stmt = stmt.replaceAll(`{{${key}}}`, val);
      }
      body.push(`    ${stmt}`);
    }
  }

  // Collect temp variables with types (AFTER body generation, since getSignalName populates these)
  const tempDecls: string[] = [];
  for (const pinMap of signalNames.values()) {
    for (const name of pinMap.values()) {
      const dt = signalTypes.get(name) || "BOOL";
      tempDecls.push(`    ${name} : ${dt};`);
    }
  }

  // Check for unconnected pins
  for (const node of nodes) {
    const data = node.data;
    const def = getBlockDef(data.blockType);
    if (!def) continue;
    if (data.blockType === "INPUT_PIN" || data.blockType === "OUTPUT_PIN") continue;

    const incoming = edgeTargets.get(node.id) ?? [];
    const connectedTargetPins = new Set(incoming.map((inc) => inc.targetPin));
    for (const pin of def.pins.filter((p) => p.direction === "input")) {
      const hasConnection = connectedTargetPins.has(pin.name) || !!data.pinValues[pin.name];
      if (!hasConnection) {
        diagnostics.push({
          message: `Block "${data.label}" pin "${pin.name}" is not connected`,
          severity: "warning",
        });
      }
    }
  }

  // Assemble
  const sections: string[] = [`PROGRAM ${program.programName}`];
  if (varInputs.length > 0) sections.push("VAR_INPUT", ...varInputs, "END_VAR");
  if (varOutputs.length > 0) sections.push("VAR_OUTPUT", ...varOutputs, "END_VAR");
  const varBody = [...varFbInstances, ...tempDecls];
  if (varBody.length > 0) sections.push("VAR", ...varBody, "END_VAR");
  if (body.length > 0) sections.push("", ...body);
  sections.push("END_PROGRAM");

  return { source: sections.join("\n"), diagnostics };
}
