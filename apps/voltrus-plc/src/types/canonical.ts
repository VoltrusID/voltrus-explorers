// Canonical, git-friendly serialization for diagram programs (.fbd / .sfc / .ld) — PLC-5.
//
// Every save routes through this module so two logically identical programs produce
// byte-identical files, and an edit touches only the fields that actually changed:
//
// - FBD/SFC nodes are sorted by a stable key: (instanceName, else variableName,
//   else step-ish name, then blockType/elementType, then id). Node ids are
//   session-scoped counters ("node-17") and churn across sessions; the user-facing
//   names are what the transpiler keys on and are stable. Unnamed nodes (pure
//   logic blocks, transitions, branches) fall back to blockType then id, which is
//   still deterministic for a fixed diagram.
// - Edges are sorted by (source, sourceHandle, target, targetHandle).
// - All object keys (pinValues, params, node data, ...) are serialized with
//   recursively sorted keys.
// - Node positions are quantized to a 10px grid so pixel-level drags don't leak
//   into diffs.
// - viewport is in-memory only: the canvases render with ReactFlow defaults and
//   never read the persisted viewport, so it is excluded from saved content.
// - ReactFlow runtime fields (selected, dragging, measured, ...) are stripped:
//   they change on every interaction and carry no program meaning.
// - LD rung/path/element array order IS ladder semantics (network order, contact
//   column order) and is preserved; only key order and the version stamp apply.
//
// Versions: FBD/SFC save as "1.1" (was "1.0"). LD stays at "2.0" — it is already
// past the 1.x line (see components/Editor/LdEditor/migrate.ts for the legacy
// node-graph format it absorbed). Loaders accept any object-shaped program and
// migrate old files simply by canonicalizing on load/save.

import type { FbdProgram, FbdNode, FbdEdge } from "./fbd";
import type { SfcProgram, SfcNode, SfcEdge } from "./sfc";
import type { LdProgram } from "./ld";
/** Format version written by FBD/SFC canonicalization. Loaders also accept "1.0". */
export const DIAGRAM_FORMAT_VERSION = "1.1";
/** Format version written by LD canonicalization (unchanged by PLC-5). */
export const LD_FORMAT_VERSION = "2.0";

const POSITION_GRID_PX = 10;
const DEFAULT_VIEWPORT = { x: 0, y: 0, zoom: 1 };


// Derived by ReactFlow during interaction; meaningless on disk.
const EPHEMERAL_NODE_FIELDS: Record<string, true> = {
  selected: true,
  dragging: true,
  resizing: true,
  measured: true,
  positionAbsolute: true,
  internals: true,
};

/** Locale-independent string compare (plain code-unit order, same everywhere). */
function cmpStr(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function cmpKeys(a: string[], b: string[]): number {
  for (let i = 0; i < a.length; i++) {
    const c = cmpStr(a[i], b[i]);
    if (c !== 0) return c;
  }
  return 0;
}

/** Snap a coordinate to the canonical 10px grid. */
export function quantize(v: number): number {
  return Math.round(v / POSITION_GRID_PX) * POSITION_GRID_PX;
}

/** Recursively sort object keys; array order is preserved. */
export function sortKeysDeep<T>(value: T): T {
  if (Array.isArray(value)) {
    return value.map(sortKeysDeep) as T;
  }
  if (value !== null && typeof value === "object") {
    const source = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(source).sort(cmpStr)) {
      out[key] = sortKeysDeep(source[key]);
    }
    return out as T;
  }
  return value;
}

function fbdNodeKey(node: FbdNode): string[] {
  const data = node.data ?? ({} as Record<string, unknown>);
  const name = data.instanceName ?? data.variableName ?? "";
  return [String(name), String(data.blockType ?? ""), node.id];
}

function sfcNodeKey(node: SfcNode): string[] {
  const data = node.data ?? ({} as Record<string, unknown>);
  const name = data.stepName ?? data.label ?? data.targetStepName ?? "";
  return [String(name), String(data.elementType ?? ""), node.id];
}

function edgeKey(edge: FbdEdge | SfcEdge): string[] {
  return [
    edge.source,
    edge.sourceHandle ?? "",
    edge.target,
    edge.targetHandle ?? "",
  ];
}

/** Strip ReactFlow runtime fields and quantize the position of a flow node. */
function canonicalFlowNode(node: FbdNode | SfcNode): Record<string, unknown> {
  const source = node as unknown as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(source)) {
    if (EPHEMERAL_NODE_FIELDS[key]) continue;
    out[key] = source[key];
  }
  if (out.position !== null && typeof out.position === "object") {
    const pos = out.position as { x: number; y: number };
    out.position = { x: quantize(pos.x), y: quantize(pos.y) };
  }
  return out;
}

export type CanonicalFbdProgram = Omit<FbdProgram, "viewport">;
export type CanonicalSfcProgram = Omit<SfcProgram, "viewport">;

export function canonicalizeFbd(program: FbdProgram): CanonicalFbdProgram {
  const nodes = [...program.nodes]
    .map(canonicalFlowNode)
    .sort((a, b) => cmpKeys(fbdNodeKey(a as FbdNode), fbdNodeKey(b as FbdNode)));
  const edges = [...program.edges].sort((a, b) => cmpKeys(edgeKey(a), edgeKey(b)));
  return sortKeysDeep({
    version: DIAGRAM_FORMAT_VERSION,
    programName: program.programName,
    declarations: program.declarations,
    nodes,
    edges,
  }) as CanonicalFbdProgram;
}

export function canonicalizeSfc(program: SfcProgram): CanonicalSfcProgram {
  const nodes = [...program.nodes]
    .map(canonicalFlowNode)
    .sort((a, b) => cmpKeys(sfcNodeKey(a as SfcNode), sfcNodeKey(b as SfcNode)));
  const edges = [...program.edges].sort((a, b) => cmpKeys(edgeKey(a), edgeKey(b)));
  return sortKeysDeep({
    version: DIAGRAM_FORMAT_VERSION,
    programName: program.programName,
    nodes,
    edges,
  }) as CanonicalSfcProgram;
}

export function canonicalizeLd(program: LdProgram): LdProgram {
  return sortKeysDeep({
    version: LD_FORMAT_VERSION,
    programName: program.programName,
    rungs: program.rungs,
  });
}

export function serializeFbd(program: FbdProgram): string {
  return JSON.stringify(canonicalizeFbd(program), null, 2);
}

export function serializeSfc(program: SfcProgram): string {
  return JSON.stringify(canonicalizeSfc(program), null, 2);
}

export function serializeLd(program: LdProgram): string {
  return JSON.stringify(canonicalizeLd(program), null, 2);
}

/**
 * Parse a stored FBD program. Accepts "1.0" and "1.1" (any object-shaped program,
 * actually): canonicalization happens on the next serialize, which is the migrate.
 * The viewport is re-materialized in memory (it is not persisted anymore).
 */
export function parseFbdProgram(value: string): FbdProgram {
  const parsed: unknown = JSON.parse(value);
  assertProgramObject(parsed, "FBD");
  const program = parsed as FbdProgram;
  return { ...program, viewport: program.viewport ?? { ...DEFAULT_VIEWPORT } };
}

/** Parse a stored SFC program; same rules as parseFbdProgram. */
export function parseSfcProgram(value: string): SfcProgram {
  const parsed: unknown = JSON.parse(value);
  assertProgramObject(parsed, "SFC");
  const program = parsed as SfcProgram;
  return { ...program, viewport: program.viewport ?? { ...DEFAULT_VIEWPORT } };
}

function assertProgramObject(parsed: unknown, kind: string): void {
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${kind} file does not contain a program object`);
  }
}

/**
 * Canonicalize the content of a file being saved to disk. Non-diagram files and
 * unparseable content pass through untouched. Legacy .ld files in the old
 * node-graph format (no "rungs" array) also pass through — the LD editor owns
 * migrating them on load.
 */
export function canonicalizeFileContent(fileName: string, content: string): string {
  if (fileName.endsWith(".fbd")) {
    return rewriteDiagramJson(content, (p) => serializeFbd(p as FbdProgram), "nodes");
  }
  if (fileName.endsWith(".sfc")) {
    return rewriteDiagramJson(content, (p) => serializeSfc(p as SfcProgram), "nodes");
  }
  if (fileName.endsWith(".ld")) {
    return rewriteDiagramJson(content, (p) => serializeLd(p as LdProgram), "rungs");
  }
  return content;
}

function rewriteDiagramJson(
  content: string,
  serialize: (program: unknown) => string,
  arrayKey: string,
): string {
  try {
    const parsed: unknown = JSON.parse(content);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      return content;
    }
    if (!Array.isArray((parsed as Record<string, unknown>)[arrayKey])) {
      return content;
    }
    return serialize(parsed);
  } catch {
    return content;
  }
}
