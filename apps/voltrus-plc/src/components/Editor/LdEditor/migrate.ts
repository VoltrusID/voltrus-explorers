import type { LdProgram, LdElement, LdPath, LdRung } from "../../../types/ld";
import { nextLdId, isContactType, isCoilType } from "../../../types/ld";

interface LegacyNode {
  id: string;
  type?: string;
  data: {
    elementType: string;
    variableName?: string;
    variableType?: string;
    registerType?: string;
    registerNumber?: number;
    blockType?: string;
    instanceName?: string;
    pinValues?: Record<string, string>;
  };
  position?: { x: number; y: number };
}

interface LegacyEdge {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string;
  targetHandle?: string;
}

interface LegacyProgram {
  version: string;
  programName: string;
  nodes: LegacyNode[];
  edges: LegacyEdge[];
}

export function migrateLdProgram(raw: unknown): LdProgram {
  if (raw && typeof raw === "object" && "rungs" in raw) {
    return raw as LdProgram;
  }
  if (raw && typeof raw === "object" && "nodes" in raw && "edges" in raw) {
    return convertLegacy(raw as LegacyProgram);
  }
  return { version: "2.0", programName: "Unknown", rungs: [] };
}

function convertLegacy(old: LegacyProgram): LdProgram {
  const nodes = old.nodes;
  const edges = old.edges;

  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const outgoing = new Map<string, LegacyEdge[]>();
  const incoming = new Map<string, LegacyEdge[]>();
  for (const e of edges) {
    const oList = outgoing.get(e.source) ?? [];
    oList.push(e);
    outgoing.set(e.source, oList);
    const iList = incoming.get(e.target) ?? [];
    iList.push(e);
    incoming.set(e.target, iList);
  }

  const leftRails = nodes.filter((n) => n.data.elementType === "left_rail");
  const rightRailIds = new Set(nodes.filter((n) => n.data.elementType === "right_rail").map((n) => n.id));

  // Find all paths from left rails to right rails
  interface PathResult { path: string[] }
  function findPaths(start: string, visited: Set<string>): PathResult[] {
    if (rightRailIds.has(start)) return [{ path: [] }];
    const results: PathResult[] = [];
    for (const e of outgoing.get(start) ?? []) {
      if (visited.has(e.target)) continue;
      const sub = findPaths(e.target, new Set([...visited, e.target]));
      for (const s of sub) results.push({ path: [e.target, ...s.path] });
    }
    return results;
  }

  // Group paths by output element
  const rungMap = new Map<string, PathResult[]>();
  for (const lr of leftRails) {
    const paths = findPaths(lr.id, new Set([lr.id]));
    for (const pr of paths) {
      for (const nid of pr.path) {
        const node = nodeMap.get(nid);
        if (node && (isCoilType(node.data.elementType as any) || node.data.elementType === "fb_block")) {
          const list = rungMap.get(nid) ?? [];
          list.push(pr);
          rungMap.set(nid, list);
          break;
        }
      }
    }
  }

  function toElement(node: LegacyNode): LdElement {
    return {
      id: nextLdId("e"),
      type: node.data.elementType as LdElement["type"],
      variable: node.data.variableName,
      variableType: node.data.variableType as any,
      registerType: node.data.registerType as any,
      registerNumber: node.data.registerNumber,
      blockType: node.data.blockType,
      instanceName: node.data.instanceName,
      pinValues: node.data.pinValues,
    };
  }

  const rungs: LdRung[] = [];
  for (const [outputId, paths] of rungMap) {
    const ldPaths: LdPath[] = [];
    for (const pr of paths) {
      const elements: LdElement[] = [];
      for (const nid of pr.path) {
        if (rightRailIds.has(nid)) continue;
        const node = nodeMap.get(nid);
        if (!node || node.data.elementType === "left_rail" || node.data.elementType === "right_rail") continue;
        elements.push(toElement(node));
      }
      if (elements.length > 0) ldPaths.push({ id: nextLdId("p"), elements });
    }
    if (ldPaths.length > 0) rungs.push({ id: nextLdId("r"), paths: ldPaths });
  }

  return { version: "2.0", programName: old.programName, rungs };
}
