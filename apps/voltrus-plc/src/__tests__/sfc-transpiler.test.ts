import { describe, it, expect } from "vitest";
import { transpileSfcToSt } from "../components/Editor/SfcEditor/transpiler/sfc-to-st";
import type { SfcProgram, SfcNodeData } from "../types/sfc";

function makeNode(id: string, type: string, data: Partial<SfcNodeData> & { elementType: SfcNodeData["elementType"]; pinValues: Record<string, string> }) {
  return { id, type, position: { x: 0, y: 0 }, data: { ...data, pinValues: data.pinValues } as SfcNodeData };
}

function makeProgram(name: string, nodes: ReturnType<typeof makeNode>[], edges: Array<{ id: string; source: string; target: string; sourceHandle?: string; targetHandle?: string }>): SfcProgram {
  return {
    version: "1.0",
    programName: name,
    nodes,
    edges: edges.map((e) => ({
      ...e,
      sourceHandle: e.sourceHandle ?? "B",
      targetHandle: e.targetHandle ?? "T",
    })),
    viewport: { x: 0, y: 0, zoom: 1 },
  };
}

describe("SFC transpiler", () => {
  it("should produce empty program for empty diagram", () => {
    const program = makeProgram("Empty", [], []);
    const result = transpileSfcToSt(program);
    expect(result.source).toContain("PROGRAM Empty");
    expect(result.source).toContain("END_PROGRAM");
    expect(result.diagnostics).toHaveLength(0);
  });

  it("should error when no initial step", () => {
    const program = makeProgram("NoInit", [
      makeNode("s1", "step", { elementType: "step", stepName: "Step1", actions: [], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("initial step"))).toBe(true);
  });

  it("should error when multiple initial steps", () => {
    const program = makeProgram("MultiInit", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Init1", actions: [], pinValues: {} }),
      makeNode("s2", "initial_step", { elementType: "initial_step", stepName: "Init2", actions: [], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("2 initial steps"))).toBe(true);
  });

  it("should transpile single initial step with no transitions", () => {
    const program = makeProgram("SingleStep", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Init", actions: [{ qualifier: "N", variableName: "Lamp" }], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("_s1 : BOOL := TRUE");
    expect(result.source).toContain("_s1_prev : BOOL");
    expect(result.source).toContain("Lamp := _s1");
    expect(result.source).toContain("_s1_prev := _s1");
  });

  it("should transpile linear 3-step sequence", () => {
    const program = makeProgram("Linear3", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Idle", actions: [], pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "Start", pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "Running", actions: [{ qualifier: "N", variableName: "Motor" }], pinValues: {} }),
      makeNode("t2", "transition", { elementType: "transition", condition: "Done", pinValues: {} }),
      makeNode("s3", "step", { elementType: "step", stepName: "Complete", actions: [{ qualifier: "N", variableName: "DoneLamp" }], pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "t1" },
      { id: "e2", source: "t1", target: "s2" },
      { id: "e3", source: "s2", target: "t2" },
      { id: "e4", source: "t2", target: "s3" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("_s1 : BOOL := TRUE");
    expect(result.source).toContain("_s2 : BOOL");
    expect(result.source).toContain("_s3 : BOOL");
    expect(result.source).toContain("IF _s1 AND (Start) THEN");
    expect(result.source).toContain("_s1 := FALSE");
    expect(result.source).toContain("_s2 := TRUE");
    expect(result.source).toContain("Motor := _s2");
  });

  it("should handle all action qualifiers", () => {
    const program = makeProgram("Actions", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Step1", actions: [
        { qualifier: "N", variableName: "VarN" },
        { qualifier: "S", variableName: "VarS" },
        { qualifier: "R", variableName: "VarR" },
        { qualifier: "P1", variableName: "VarP1", expression: "VarP1 := VarP1 + 1" },
        { qualifier: "P0", variableName: "VarP0", expression: "VarP0 := 0" },
      ], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    // N: direct assignment
    expect(result.source).toContain("VarN := _s1;");
    // S: set on entry
    expect(result.source).toContain("IF _s1 AND NOT _s1_prev THEN VarS := TRUE; END_IF;");
    // R: reset on entry
    expect(result.source).toContain("IF _s1 AND NOT _s1_prev THEN VarR := FALSE; END_IF;");
    // P1: pulse on entry
    expect(result.source).toContain("IF _s1 AND NOT _s1_prev THEN VarP1 := VarP1 + 1; END_IF;");
    // P0: pulse on exit
    expect(result.source).toContain("IF NOT _s1 AND _s1_prev THEN VarP0 := 0; END_IF;");
  });

  it("should transpile jump to named step", () => {
    const program = makeProgram("JumpTest", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Start", actions: [], pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "Go", pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "Middle", actions: [], pinValues: {} }),
      makeNode("t2", "transition", { elementType: "transition", condition: "Back", pinValues: {} }),
      makeNode("j1", "jump", { elementType: "jump", targetStepName: "Start", pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "t1" },
      { id: "e2", source: "t1", target: "s2" },
      { id: "e3", source: "s2", target: "t2" },
      { id: "e4", source: "t2", target: "j1" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    // Jump generates: IF _s2 AND (TRUE) THEN _s2 := FALSE; _s1 := TRUE
    expect(result.source).toContain("_s1 := TRUE");
    expect(result.source).toContain("_s2 := FALSE");
  });

  it("should handle OR divergence", () => {
    const program = makeProgram("OrBranch", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Start", actions: [], pinValues: {} }),
      makeNode("t0", "transition", { elementType: "transition", condition: "Go", pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "Decide", actions: [], pinValues: {} }),
      makeNode("od1", "or_diverge", { elementType: "or_diverge", pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "PathA", pinValues: {} }),
      makeNode("t2", "transition", { elementType: "transition", condition: "PathB", pinValues: {} }),
      makeNode("s3", "step", { elementType: "step", stepName: "A", actions: [], pinValues: {} }),
      makeNode("s4", "step", { elementType: "step", stepName: "B", actions: [], pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "t0" },
      { id: "e2", source: "t0", target: "s2" },
      { id: "e3", source: "s2", target: "od1" },
      { id: "e4", source: "od1", target: "t1", sourceHandle: "B0" },
      { id: "e5", source: "od1", target: "t2", sourceHandle: "B1" },
      { id: "e6", source: "t1", target: "s3" },
      { id: "e7", source: "t2", target: "s4" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    // Both OR paths should generate transitions from _s2 (Decide step)
    const source = result.source;
    expect(source).toContain("PathA");
    expect(source).toContain("PathB");
  });

  it("should error on empty transition condition", () => {
    const program = makeProgram("EmptyCond", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "A", actions: [], pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "", pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "B", actions: [], pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "t1" },
      { id: "e2", source: "t1", target: "s2" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("no condition"))).toBe(true);
  });

  it("should error on jump to nonexistent step", () => {
    const program = makeProgram("BadJump", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "A", actions: [], pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "Go", pinValues: {} }),
      makeNode("j1", "jump", { elementType: "jump", targetStepName: "Nonexistent", pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "t1" },
      { id: "e2", source: "t1", target: "j1" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("does not exist"))).toBe(true);
  });

  it("should warn about step with no actions", () => {
    const program = makeProgram("NoActions", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Empty", actions: [], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "warning" && d.message.includes("no actions"))).toBe(true);
  });

  it("should reject invalid program names", () => {
    const program = makeProgram("Bad Name!", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "A", actions: [], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("Invalid program name"))).toBe(true);
  });

  it("should handle OR convergence", () => {
    const program = makeProgram("OrConv", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "Start", actions: [], pinValues: {} }),
      makeNode("od1", "or_diverge", { elementType: "or_diverge", pinValues: {} }),
      makeNode("t1", "transition", { elementType: "transition", condition: "PathA", pinValues: {} }),
      makeNode("t2", "transition", { elementType: "transition", condition: "PathB", pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "A", actions: [], pinValues: {} }),
      makeNode("s3", "step", { elementType: "step", stepName: "B", actions: [], pinValues: {} }),
      makeNode("oc1", "or_converge", { elementType: "or_converge", pinValues: {} }),
      makeNode("s4", "step", { elementType: "step", stepName: "End", actions: [{ qualifier: "N", variableName: "Done" }], pinValues: {} }),
    ], [
      { id: "e1", source: "s1", target: "od1" },
      { id: "e2", source: "od1", target: "t1", sourceHandle: "B0" },
      { id: "e3", source: "od1", target: "t2", sourceHandle: "B1" },
      { id: "e4", source: "t1", target: "s2" },
      { id: "e5", source: "t2", target: "s3" },
      { id: "e6", source: "s2", target: "oc1", sourceHandle: "B", targetHandle: "T0" },
      { id: "e7", source: "s3", target: "oc1", sourceHandle: "B", targetHandle: "T1" },
      { id: "e8", source: "oc1", target: "s4" },
    ]);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("Done");
  });

  it("should warn about disconnected steps", () => {
    const program = makeProgram("Disconnected", [
      makeNode("s1", "initial_step", { elementType: "initial_step", stepName: "A", actions: [], pinValues: {} }),
      makeNode("s2", "step", { elementType: "step", stepName: "B", actions: [], pinValues: {} }),
    ], []);
    const result = transpileSfcToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "warning" && d.message.includes("not reachable"))).toBe(true);
  });
});
