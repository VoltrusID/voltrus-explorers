import { describe, it, expect, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useStore } from "../store";
import { transpileLdToSt } from "../components/Editor/LdEditor/transpiler/ld-to-st";
import {
  canonicalizeFileContent,
  parseFbdProgram,
  parseSfcProgram,
  serializeFbd,
  serializeLd,
  serializeSfc,
  DIAGRAM_FORMAT_VERSION,
  LD_FORMAT_VERSION,
} from "../types/canonical";
import type { FbdProgram, FbdNode } from "../types/fbd";
import type { SfcProgram } from "../types/sfc";
import type { LdProgram } from "../types/ld";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const invokeMock = vi.mocked(invoke);

function buildFbdProgram(): FbdProgram {
  return {
    version: "1.0",
    programName: "Canonical",
    declarations: [],
    nodes: [
      { id: "n_and", type: "block", position: { x: 250, y: 50 }, data: { blockType: "AND", label: "AND", pinValues: { IN1: "X0" } } },
      { id: "n_x1", type: "input_pin", position: { x: 10, y: 120 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "X1", variableType: "BOOL", pinValues: {} } },
      { id: "n_x0", type: "input_pin", position: { x: 10, y: 20 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "X0", variableType: "BOOL", pinValues: {} } },
      { id: "n_y0", type: "output_pin", position: { x: 500, y: 50 }, data: { blockType: "OUTPUT_PIN", label: "Output", variableName: "Y0", variableType: "BOOL", pinValues: {} } },
    ],
    edges: [
      { id: "e2", source: "n_x1", target: "n_and", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
      { id: "e1", source: "n_x0", target: "n_and", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
      { id: "e3", source: "n_and", target: "n_y0", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
    ],
    viewport: { x: 0, y: 0, zoom: 1 },
  };
}

function buildSfcProgram(): SfcProgram {
  return {
    version: "1.0",
    programName: "SfcCanonical",
    nodes: [
      { id: "s1", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Idle", actions: [], pinValues: {} } },
      { id: "t1", type: "transition", position: { x: 200, y: 120 }, data: { elementType: "transition", condition: "Start", pinValues: {} } },
      { id: "s2", type: "step", position: { x: 200, y: 240 }, data: { elementType: "step", stepName: "Run", actions: [{ qualifier: "N", variableName: "Y0" }], pinValues: {} } },
    ],
    edges: [
      { id: "e2", source: "t1", target: "s2" },
      { id: "e1", source: "s1", target: "t1" },
    ],
    viewport: { x: 0, y: 0, zoom: 1 },
  };
}

function buildLdProgram(): LdProgram {
  return {
    version: "2.0",
    programName: "LdCanonical",
    rungs: [
      {
        id: "r_1",
        label: "Motor",
        paths: [
          {
            id: "p_1",
            elements: [
              { id: "e_1", type: "contact_no", variable: "X0" },
              { id: "e_2", type: "coil", variable: "Y0" },
              { id: "e_3", type: "fb_block", blockType: "TON", instanceName: "t1", pinValues: { PT: "T#1S" } },
            ],
          },
        ],
      },
      { id: "r_2", paths: [{ id: "p_2", elements: [
        { id: "e_4", type: "contact_no", variable: "X1" },
        { id: "e_5", type: "coil", variable: "Y1" },
      ] }] },
    ],
  };
}

// The same ladder as buildLdProgram(), but "built" in a different session:
// elements created in reverse order, unrelated session ids, scrambled key
// insertion order. Semantic rung/branch/series order is untouched — that part
// is the ladder, not its history.
function buildLdProgramShuffled(): LdProgram {
  // Created in reverse order with unrelated session ids — but placed in the
  // same semantic (rung, path, column) slots as buildLdProgram().
  const coilY1 = { id: "q77", type: "coil", variable: "Y1" };
  const contactX1 = { id: "z12", type: "contact_no", variable: "X1" };
  const ton = { id: "a5", type: "fb_block", blockType: "TON", instanceName: "t1", pinValues: { PT: "T#1S" } };
  const coilY0 = { id: "m2", type: "coil", variable: "Y0" };
  const contactX0 = { id: "k9", type: "contact_no", variable: "X0" };
  const rung2 = { id: "rung-two", label: undefined, paths: [{ id: "path-two", elements: [contactX1, coilY1] }] };
  const rung1 = { id: "rung-one", label: "Motor", paths: [{ id: "path-one", elements: [contactX0, coilY0, ton] }] };
  return reverseKeysOnly({ rungs: [rung1, rung2], version: "2.0", programName: "LdCanonical" }) as LdProgram;
}

// Clone with every object's keys inserted in reverse order and every array
// reversed — the "different insertion order" twin of a program.
function reverseKeyOrder(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(reverseKeyOrder).reverse();
  if (value !== null && typeof value === "object") {
    const source = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(source).reverse()) {
      out[key] = reverseKeyOrder(source[key]);
    }
    return out;
  }
  return value;
}

// Key-order-only scramble for LD, where array order is ladder semantics.
function reverseKeysOnly(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(reverseKeysOnly);
  if (value !== null && typeof value === "object") {
    const source = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(source).reverse()) {
      out[key] = reverseKeysOnly(source[key]);
    }
    return out;
  }
  return value;
}

interface LeafDiff {
  path: string;
  before: unknown;
  after: unknown;
}

function diffLeafPaths(before: unknown, after: unknown, path = "", out: LeafDiff[] = []): LeafDiff[] {
  if (Array.isArray(before) && Array.isArray(after)) {
    const len = Math.max(before.length, after.length);
    for (let i = 0; i < len; i++) {
      diffLeafPaths(before[i], after[i], `${path}[${i}]`, out);
    }
    return out;
  }
  if (before !== null && after !== null && typeof before === "object" && typeof after === "object") {
    const a = before as Record<string, unknown>;
    const b = after as Record<string, unknown>;
    const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort();
    for (const key of keys) {
      diffLeafPaths(a[key], b[key], path ? `${path}.${key}` : key, out);
    }
    return out;
  }
  if (before !== after) {
    out.push({ path, before, after });
  }
  return out;
}

function legacyFbdContent(): string {
  return JSON.stringify({
    version: "1.0",
    programName: "Legacy",
    viewport: { x: 120, y: -40, zoom: 1.25 },
    declarations: [],
    nodes: [
      { selected: true, id: "and", type: "block", position: { x: 313.7, y: 95.3 }, data: { pinValues: { IN1: "X0" }, blockType: "AND", label: "AND" } },
      { id: "in2", type: "input_pin", position: { x: 11.2, y: 188.4 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "X9", variableType: "BOOL", pinValues: {} } },
      { id: "in1", type: "input_pin", position: { x: 11.2, y: 8.8 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "X0", variableType: "BOOL", pinValues: {} } },
    ],
    edges: [
      { id: "e2", source: "in2", target: "and", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
      { id: "e1", source: "in1", target: "and", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
    ],
  });
}

describe("canonical serialization (PLC-5)", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    useStore.setState({ openFiles: [], activeFileIndex: 0 });
  });

  describe("order invariance", () => {
    it("produces identical FBD files for diagrams built in different orders", () => {
      const ordered = buildFbdProgram();
      const reordered = reverseKeyOrder({
        ...ordered,
        nodes: [...ordered.nodes].reverse(),
        edges: [...ordered.edges].reverse(),
      }) as FbdProgram;
      // shift one node 3px off-grid in the reordered twin (253 -> 250 after quantize)
      (reordered.nodes[0] as FbdNode).position = { x: 253, y: 50 };
      expect(serializeFbd(reordered)).toBe(serializeFbd(ordered));
    });

    it("produces identical SFC files for programs built in different orders", () => {
      const ordered = buildSfcProgram();
      const reordered = reverseKeyOrder({
        ...ordered,
        nodes: [...ordered.nodes].reverse(),
        edges: [...ordered.edges].reverse(),
      }) as SfcProgram;
      expect(serializeSfc(reordered)).toBe(serializeSfc(ordered));
    });

    it("produces identical LD files for ladders built in different sessions", () => {
      const natural = buildLdProgram();
      const shuffled = buildLdProgramShuffled();
      expect(serializeLd(shuffled)).toBe(serializeLd(natural));
    });

    it("assigns canonical LD ids from the semantic traversal, not construction history", () => {
      const out = JSON.parse(serializeLd(buildLdProgram())) as LdProgram;
      expect(out.rungs.map((r) => r.id)).toEqual(["r0", "r1"]);
      expect(out.rungs[0].paths.map((p) => p.id)).toEqual(["p0_0"]);
      expect(out.rungs[0].paths[0].elements.map((e) => e.id)).toEqual(["e0_0", "e0_1", "e0_2"]);
      expect(out.rungs[1].paths[0].elements.map((e) => e.id)).toEqual(["e1_0", "e1_1"]);
    });

    it("keeps LD rung and element order (ladder semantics) untouched", () => {
      const out = JSON.parse(serializeLd(buildLdProgram())) as LdProgram;
      expect(out.rungs[0].paths[0].elements.map((e) => e.variable ?? e.instanceName)).toEqual(["X0", "Y0", "t1"]);
      expect(out.rungs[1].paths[0].elements.map((e) => e.variable)).toEqual(["X1", "Y1"]);
    });
    it("transpiles differently-built ladders to identical ST", () => {
      const natural = transpileLdToSt(buildLdProgram());
      const shuffled = transpileLdToSt(buildLdProgramShuffled());
      expect(natural.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
      expect(shuffled.source).toBe(natural.source);
      expect(shuffled.diagnostics).toEqual(natural.diagnostics);
    });
  });

  describe("minimal diffs", () => {
    it("changes exactly one field when a single FBD pin value changes", () => {
      const before = JSON.parse(serializeFbd(buildFbdProgram()));
      const program = buildFbdProgram();
      const andNode = program.nodes.find((n) => n.data.blockType === "AND")!;
      andNode.data.pinValues = { ...andNode.data.pinValues, IN1: "M5" };
      const after = JSON.parse(serializeFbd(program));

      const diffs = diffLeafPaths(before, after);
      expect(diffs).toHaveLength(1);
      expect(diffs[0].path).toBe("nodes[0].data.pinValues.IN1");
      expect(diffs[0].before).toBe("X0");
      expect(diffs[0].after).toBe("M5");
    });

    it("serializes FBD programs idempotently", () => {
      const once = serializeFbd(buildFbdProgram());
      const twice = serializeFbd(parseFbdProgram(once));
      expect(twice).toBe(once);
    });

    it("changes exactly one field when one LD operand changes", () => {
      const before = JSON.parse(serializeLd(buildLdProgram()));
      const program = buildLdProgram();
      program.rungs[0].paths[0].elements[0].variable = "X5";
      const after = JSON.parse(serializeLd(program));

      const diffs = diffLeafPaths(before, after);
      expect(diffs).toHaveLength(1);
      expect(diffs[0].path).toBe("rungs[0].paths[0].elements[0].variable");
      expect(diffs[0].before).toBe("X0");
      expect(diffs[0].after).toBe("X5");
    });

    it("serializes SFC programs idempotently", () => {
      const once = serializeSfc(buildSfcProgram());
      const twice = serializeSfc(JSON.parse(once) as SfcProgram);
      expect(twice).toBe(once);
    });

    it("serializes LD programs idempotently", () => {
      const once = serializeLd(buildLdProgram());
      const twice = serializeLd(JSON.parse(once) as LdProgram);
      expect(twice).toBe(once);
    });
  });

  describe("v1.0 migration", () => {
    it("loads a v1.0 FBD fixture and canonicalizes it", () => {
      const program = parseFbdProgram(legacyFbdContent());
      // viewport survives in memory only
      expect(program.viewport).toEqual({ x: 120, y: -40, zoom: 1.25 });

      const out = JSON.parse(serializeFbd(program));
      expect(out.version).toBe(DIAGRAM_FORMAT_VERSION);
      expect(JSON.stringify(out)).not.toContain("viewport");
      // sorted by (variableName, blockType, id): the unnamed AND block sorts first
      expect(out.nodes.map((n: FbdNode) => n.id)).toEqual(["and", "in1", "in2"]);
      expect(out.edges.map((e: { id: string }) => e.id)).toEqual(["e1", "e2"]);
      for (const node of out.nodes as FbdNode[]) {
        expect(node.position.x % 10).toBe(0);
        expect(node.position.y % 10).toBe(0);
        expect("selected" in node).toBe(false);
        expect("dragging" in node).toBe(false);
      }
    });

    it("loads a v1.0 SFC fixture and canonicalizes it", () => {
      const legacySfc = JSON.stringify({
        version: "1.0",
        programName: "LegacySfc",
        viewport: { x: 5, y: 5, zoom: 0.8 },
        nodes: [
          { id: "s_run", type: "step", position: { x: 203, y: 240 }, data: { elementType: "step", stepName: "Run", actions: [], pinValues: {} } },
          { id: "s_idle", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Idle", actions: [], pinValues: {} } },
        ],
        edges: [],
      });

      const out = JSON.parse(serializeSfc(parseSfcProgram(legacySfc)));
      expect(out.version).toBe(DIAGRAM_FORMAT_VERSION);
      expect(out.viewport).toBeUndefined();
      // sorted by (stepName, elementType, id)
      expect(out.nodes.map((n: { id: string }) => n.id)).toEqual(["s_idle", "s_run"]);
    });

    it("migrates a v2.0 LD file with session ids to canonical v2.1", () => {
      const legacy = JSON.stringify({
        version: "2.0",
        programName: "OldLd",
        rungs: [
          {
            id: "r_7",
            label: "Legacy rung",
            paths: [
              {
                id: "p_3",
                elements: [
                  { id: "e_42", type: "contact_no", variable: "X0" },
                  { id: "e_43", type: "coil", variable: "Y0" },
                ],
              },
            ],
          },
        ],
      });
      const out = JSON.parse(serializeLd(JSON.parse(legacy) as LdProgram)) as LdProgram;
      expect(out.version).toBe(LD_FORMAT_VERSION);
      expect(out.rungs[0].id).toBe("r0");
      expect(out.rungs[0].label).toBe("Legacy rung");
      expect(out.rungs[0].paths[0].id).toBe("p0_0");
      expect(out.rungs[0].paths[0].elements.map((e) => e.id)).toEqual(["e0_0", "e0_1"]);
      // round-trip stable: reopened canonical file keeps its ids
      expect(serializeLd(out)).toBe(JSON.stringify(out, null, 2));
    });
  });

  describe("store persistence paths", () => {
    it("canonicalizes diagram content when saving to disk", async () => {
      useStore.setState({
        openFiles: [{ name: "Legacy.fbd", path: "Legacy.fbd", content: legacyFbdContent(), modified: true }],
        activeFileIndex: 0,
      });
      await useStore.getState().saveFile(0);

      expect(invokeMock).toHaveBeenCalledTimes(1);
      const [command, payload] = invokeMock.mock.calls[0] as [string, { path: string; content: string }];
      expect(command).toBe("save_file");
      expect(payload.path).toBe("Legacy.fbd");
      const saved = JSON.parse(payload.content);
      expect(saved.version).toBe("1.1");
      expect(saved.viewport).toBeUndefined();
      expect(saved.nodes.map((n: FbdNode) => n.id)).toEqual(["and", "in1", "in2"]);
      // in-memory copy synced with what was written, file marked clean
      expect(useStore.getState().openFiles[0].content).toBe(payload.content);
      expect(useStore.getState().openFiles[0].modified).toBe(false);
    });

    it("creates new FBD/SFC files as canonical v1.1 and LD as v2.1", () => {
      useStore.getState().newFbdFile("Main");
      const fbd = JSON.parse(useStore.getState().openFiles[0].content);
      expect(fbd.version).toBe(DIAGRAM_FORMAT_VERSION);
      expect(fbd.viewport).toBeUndefined();
      expect(fbd.nodes).toEqual([]);

      useStore.getState().newSfcFile("Seq");
      const sfc = JSON.parse(useStore.getState().openFiles[1].content);
      expect(sfc.version).toBe(DIAGRAM_FORMAT_VERSION);
      expect(sfc.viewport).toBeUndefined();

      useStore.getState().newLdFile("Ladder");
      const ld = JSON.parse(useStore.getState().openFiles[2].content);
      expect(ld.version).toBe(LD_FORMAT_VERSION);
    });

    it("serializes loaded samples canonically", () => {
      useStore.getState().loadFbdSample({ name: "Sample", program: buildFbdProgram() });
      const content = useStore.getState().openFiles[0].content;
      expect(content).toBe(serializeFbd(buildFbdProgram()));
    });

    it("leaves non-diagram and unparseable content untouched", () => {
      const st = "PROGRAM Main END_PROGRAM";
      expect(canonicalizeFileContent("Main.st", st)).toBe(st);
      const broken = "{not json";
      expect(canonicalizeFileContent("Broken.fbd", broken)).toBe(broken);
    });
  });
});
