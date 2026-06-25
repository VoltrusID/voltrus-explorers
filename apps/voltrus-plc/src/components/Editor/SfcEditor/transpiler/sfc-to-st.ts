import type { Node, Edge } from "@xyflow/react";
import type { SfcNodeData, SfcProgram, SfcAction } from "../../../../types/sfc";

interface TranspileResult {
  source: string;
  diagnostics: Array<{ message: string; severity: "error" | "warning" }>;
}

const IEC_IDENTIFIER_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

function isValidIecIdentifier(name: string): boolean {
  return IEC_IDENTIFIER_RE.test(name);
}

export function transpileSfcToSt(program: SfcProgram): TranspileResult {
  const diagnostics: TranspileResult["diagnostics"] = [];
  const nodes = program.nodes as Node<SfcNodeData>[];
  const edges = program.edges;

  if (nodes.length === 0) {
    return { source: `PROGRAM ${program.programName}\nEND_PROGRAM`, diagnostics };
  }

  if (!isValidIecIdentifier(program.programName)) {
    diagnostics.push({ message: `Invalid program name: "${program.programName}"`, severity: "error" });
  }

  if (diagnostics.some((d) => d.severity === "error")) {
    return { source: "", diagnostics };
  }

  // Build adjacency maps
  const nodeMap = new Map<string, Node<SfcNodeData>>(nodes.map((n) => [n.id, n as Node<SfcNodeData>]));
  const outgoing = new Map<string, Edge[]>();
  const incoming = new Map<string, Edge[]>();
  for (const edge of edges) {
    const oList = outgoing.get(edge.source) ?? [];
    oList.push(edge);
    outgoing.set(edge.source, oList);
    const iList = incoming.get(edge.target) ?? [];
    iList.push(edge);
    incoming.set(edge.target, iList);
  }

  // Classify nodes
  const isStepLike = (n: Node<SfcNodeData>) => n.data.elementType === "step" || n.data.elementType === "initial_step";
  const isTransition = (n: Node<SfcNodeData>) => n.data.elementType === "transition";
  const isOrDiverge = (n: Node<SfcNodeData>) => n.data.elementType === "or_diverge";
  const isOrConverge = (n: Node<SfcNodeData>) => n.data.elementType === "or_converge";
  const isJump = (n: Node<SfcNodeData>) => n.data.elementType === "jump";

  // Find initial step(s)
  const initialSteps = nodes.filter((n) => n.data.elementType === "initial_step");
  if (initialSteps.length === 0) {
    diagnostics.push({ message: "SFC must have exactly one initial step", severity: "error" });
  }
  if (initialSteps.length > 1) {
    diagnostics.push({ message: `Found ${initialSteps.length} initial steps — exactly one required`, severity: "error" });
  }
  if (diagnostics.some((d) => d.severity === "error")) {
    return { source: "", diagnostics };
  }

  // Validate step names
  for (const node of nodes) {
    if (isStepLike(node) && node.data.stepName && !isValidIecIdentifier(node.data.stepName)) {
      diagnostics.push({ message: `Invalid step name: "${node.data.stepName}"`, severity: "error" });
    }
  }

  // Assign step numbers via BFS from initial step
  const stepVarMap = new Map<string, string>(); // nodeId -> _sN
  const stepOrder: string[] = []; // ordered node IDs of steps
  const visited = new Set<string>();
  const queue: string[] = [initialSteps[0].id];
  visited.add(initialSteps[0].id);
  let stepCounter = 0;

  // Also find all steps not reachable via BFS (disconnected)
  const allStepIds = new Set(nodes.filter(isStepLike).map((n) => n.id));

  while (queue.length > 0) {
    const current = queue.shift()!;
    const node = nodeMap.get(current);
    if (!node) continue;

    if (isStepLike(node)) {
      stepCounter++;
      stepVarMap.set(current, `_s${stepCounter}`);
      stepOrder.push(current);
    }

    // Follow edges to discover more nodes
    const outEdges = outgoing.get(current) ?? [];
    for (const edge of outEdges) {
      if (!visited.has(edge.target)) {
        visited.add(edge.target);
        queue.push(edge.target);
      }
    }
  }

  // Assign step vars for disconnected steps
  for (const nodeId of allStepIds) {
    if (!stepVarMap.has(nodeId)) {
      stepCounter++;
      stepVarMap.set(nodeId, `_s${stepCounter}`);
      stepOrder.push(nodeId);
      diagnostics.push({ message: `Step "${nodeMap.get(nodeId)?.data.stepName ?? nodeId}" is not reachable from the initial step`, severity: "warning" });
    }
  }

  // Validate transitions
  for (const node of nodes) {
    if (isTransition(node)) {
      if (!node.data.condition?.trim()) {
        diagnostics.push({ message: `Transition at node ${node.id} has no condition`, severity: "error" });
      }
    }
    if (isJump(node)) {
      const target = node.data.targetStepName?.trim();
      if (!target) {
        diagnostics.push({ message: `Jump at node ${node.id} has no target step name`, severity: "error" });
      } else {
        const targetExists = nodes.some((n) => isStepLike(n) && n.data.stepName === target);
        if (!targetExists) {
          diagnostics.push({ message: `Jump target "${target}" does not exist`, severity: "error" });
        }
      }
    }
  }

  if (diagnostics.some((d) => d.severity === "error")) {
    return { source: "", diagnostics };
  }

  // Warn about steps with no actions
  for (const stepId of stepOrder) {
    const node = nodeMap.get(stepId);
    if (node && (!node.data.actions || node.data.actions.length === 0)) {
      diagnostics.push({ message: `Step "${node.data.stepName ?? stepId}" has no actions`, severity: "warning" });
    }
  }

  // Build step name → var mapping for jumps
  const stepNameToVar = new Map<string, string>();
  for (const stepId of stepOrder) {
    const node = nodeMap.get(stepId);
    if (node?.data.stepName) {
      stepNameToVar.set(node.data.stepName, stepVarMap.get(stepId)!);
    }
  }

  // === Generate ST ===

  // Collect all variables used in actions
  const actionVars = new Map<string, string>(); // name -> type (default BOOL)
  for (const stepId of stepOrder) {
    const node = nodeMap.get(stepId);
    if (!node?.data.actions) continue;
    for (const act of node.data.actions) {
      if (act.variableName && !actionVars.has(act.variableName)) {
        actionVars.set(act.variableName, "BOOL");
      }
    }
  }

  // VAR declarations
  const varLines: string[] = [];
  for (const stepId of stepOrder) {
    const v = stepVarMap.get(stepId)!;
    const isInit = nodeMap.get(stepId)?.data.elementType === "initial_step";
    varLines.push(`    ${v} : BOOL${isInit ? " := TRUE" : ""};`);
  }
  for (const stepId of stepOrder) {
    const v = stepVarMap.get(stepId)!;
    varLines.push(`    ${v}_prev : BOOL;`);
  }
  for (const [name, type] of actionVars) {
    varLines.push(`    ${name} : ${type};`);
  }

  // Body
  const body: string[] = [];

  // Store previous states
  for (const stepId of stepOrder) {
    const v = stepVarMap.get(stepId)!;
    body.push(`    ${v}_prev := ${v};`);
  }

  // Resolve what comes after a node (handle OR divergence)
  function resolveDownstream(nodeId: string): Array<{ targetStepId: string; condition: string }> {
    const results: Array<{ targetStepId: string; condition: string }> = [];
    const node = nodeMap.get(nodeId);
    if (!node) return results;

    const outEdges = outgoing.get(nodeId) ?? [];

    if (isStepLike(node)) {
      // Step → transition(s) or divergence or jump
      for (const edge of outEdges) {
        const target = nodeMap.get(edge.target);
        if (!target) continue;

        if (isTransition(target)) {
          // Step → Transition → (resolve further)
          const further = resolveDownstream(target.id);
          results.push(...further);
        } else if (isOrDiverge(target)) {
          // Step → OR Diverge → multiple transitions
          const further = resolveDownstream(target.id);
          results.push(...further);
        } else if (isJump(target)) {
          const targetStepName = target.data.targetStepName ?? "";
          const targetVar = stepNameToVar.get(targetStepName);
          if (targetVar) {
            results.push({ targetStepId: targetVar, condition: "TRUE" });
          }
        } else if (isStepLike(target)) {
          // Direct step-to-step (unusual, treat as always-true transition)
          results.push({ targetStepId: target.id, condition: "TRUE" });
        }
      }
    } else if (isTransition(node)) {
      // Transition → step(s) or convergence or jump
      for (const edge of outEdges) {
        const target = nodeMap.get(edge.target);
        if (!target) continue;

        if (isStepLike(target)) {
          results.push({ targetStepId: target.id, condition: node.data.condition ?? "TRUE" });
        } else if (isOrConverge(target)) {
          const further = resolveDownstream(target.id);
          results.push(...further);
        } else if (isJump(target)) {
          const targetStepName = target.data.targetStepName ?? "";
          const targetVar = stepNameToVar.get(targetStepName);
          if (targetVar) {
            results.push({ targetStepId: targetVar, condition: node.data.condition ?? "TRUE" });
          }
        }
      }
    } else if (isOrDiverge(node)) {
      // OR Diverge → multiple transitions (priority ordered)
      for (const edge of outEdges) {
        const further = resolveDownstream(edge.target);
        results.push(...further);
      }
    } else if (isOrConverge(node)) {
      // OR Converge → single output
      for (const edge of outEdges) {
        const further = resolveDownstream(edge.target);
        results.push(...further);
      }
    }

    return results;
  }

  // Generate transitions for each step
  for (const stepId of stepOrder) {
    const sourceVar = stepVarMap.get(stepId)!;
    const downstreams = resolveDownstream(stepId);

    for (const ds of downstreams) {
      const targetVar = stepVarMap.get(ds.targetStepId) ?? ds.targetStepId;
      body.push(`    IF ${sourceVar} AND (${ds.condition}) THEN`);
      body.push(`        ${sourceVar} := FALSE;`);
      body.push(`        ${targetVar} := TRUE;`);
      body.push(`    END_IF;`);
    }
  }

  // Generate actions for each step
  for (const stepId of stepOrder) {
    const node = nodeMap.get(stepId);
    if (!node?.data.actions) continue;
    const v = stepVarMap.get(stepId)!;

    for (const act of node.data.actions) {
      if (!act.variableName) continue;

      switch (act.qualifier) {
        case "N":
          body.push(`    ${act.variableName} := ${v};`);
          break;
        case "S":
          body.push(`    IF ${v} AND NOT ${v}_prev THEN ${act.variableName} := TRUE; END_IF;`);
          break;
        case "R":
          body.push(`    IF ${v} AND NOT ${v}_prev THEN ${act.variableName} := FALSE; END_IF;`);
          break;
        case "P1": {
          const expr = act.expression ?? `${act.variableName} := TRUE`;
          body.push(`    IF ${v} AND NOT ${v}_prev THEN ${expr}; END_IF;`);
          break;
        }
        case "P0": {
          const expr = act.expression ?? `${act.variableName} := TRUE`;
          body.push(`    IF NOT ${v} AND ${v}_prev THEN ${expr}; END_IF;`);
          break;
        }
      }
    }
  }

  // Assemble
  const sections: string[] = [`PROGRAM ${program.programName}`];
  if (varLines.length > 0) {
    sections.push("VAR");
    sections.push(...varLines);
    sections.push("END_VAR");
  }
  if (body.length > 0) {
    sections.push("");
    sections.push(...body);
  }
  sections.push("END_PROGRAM");

  return { source: sections.join("\n"), diagnostics };
}
