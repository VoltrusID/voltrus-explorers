import { describe, it, expect } from "vitest";
import { transpileFbdToSt } from "../components/Editor/FbdEditor/transpiler/fbd-to-st";
import { topologicalSort } from "../components/Editor/FbdEditor/transpiler/topological-sort";
import { BLOCK_LIBRARY, getBlockDef } from "../types/fbd";
import type { FbdProgram } from "../types/fbd";

describe("FBD transpiler", () => {
  it("should produce empty program for empty diagram", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "Empty",
      declarations: [],
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.source).toContain("PROGRAM Empty");
    expect(result.source).toContain("END_PROGRAM");
    expect(result.diagnostics).toHaveLength(0);
  });

  it("should transpile a simple AND gate", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "AndTest",
      declarations: [],
      nodes: [
        { id: "in1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "a", pinValues: {} } },
        { id: "in2", type: "input_pin", position: { x: 0, y: 100 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "b", pinValues: {} } },
        { id: "and1", type: "block", position: { x: 200, y: 50 }, data: { blockType: "AND", label: "AND", pinValues: {} } },
        { id: "out1", type: "output_pin", position: { x: 400, y: 50 }, data: { blockType: "OUTPUT_PIN", label: "Output", variableName: "result", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "in1", target: "and1", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e2", source: "in2", target: "and1", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e3", source: "and1", target: "out1", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("PROGRAM AndTest");
    expect(result.source).toContain("VAR_INPUT");
    expect(result.source).toContain("a : BOOL");
    expect(result.source).toContain("b : BOOL");
    expect(result.source).toContain("VAR_OUTPUT");
    expect(result.source).toContain("result : BOOL");
    expect(result.source).toContain("result := a AND b");
    expect(result.source).toContain("END_PROGRAM");
  });

  it("should transpile a TON timer with FB instance", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "TimerTest",
      declarations: [],
      nodes: [
        { id: "in1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "start", pinValues: {} } },
        { id: "ton1", type: "timer", position: { x: 200, y: 0 }, data: { blockType: "TON", label: "TON", instanceName: "delay", pinValues: { PT: "T#500MS" } } },
        { id: "out1", type: "output_pin", position: { x: 400, y: 0 }, data: { blockType: "OUTPUT_PIN", label: "Output", variableName: "done", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "in1", target: "ton1", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
        { id: "e2", source: "ton1", target: "out1", sourceHandle: "Q", targetHandle: "IN", data: { sourcePin: "Q", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("delay : TON");
    expect(result.source).toContain("delay(IN := start, PT := T#500MS)");
    expect(result.source).toContain("done := delay.Q");
  });

  it("should detect cycles and report error", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "CycleTest",
      declarations: [],
      nodes: [
        { id: "n1", type: "block", position: { x: 0, y: 0 }, data: { blockType: "AND", label: "AND", pinValues: {} } },
        { id: "n2", type: "block", position: { x: 200, y: 0 }, data: { blockType: "OR", label: "OR", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "n1", target: "n2", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e2", source: "n2", target: "n1", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("cycle"))).toBe(true);
    expect(result.source).toBe("");
  });

  it("should warn about unconnected input pins", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "UnconnectedTest",
      declarations: [],
      nodes: [
        { id: "and1", type: "block", position: { x: 0, y: 0 }, data: { blockType: "AND", label: "AND", pinValues: {} } },
      ],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    const warnings = result.diagnostics.filter((d) => d.severity === "warning");
    expect(warnings.length).toBeGreaterThanOrEqual(1);
    expect(warnings.some((w) => w.message.includes("IN1"))).toBe(true);
  });
});

describe("topological sort", () => {
  it("should sort nodes in dependency order", () => {
    const nodes = [
      { id: "a", position: { x: 0, y: 0 }, data: {} },
      { id: "b", position: { x: 100, y: 0 }, data: {} },
      { id: "c", position: { x: 200, y: 0 }, data: {} },
    ];
    const edges = [
      { id: "e1", source: "a", target: "b" },
      { id: "e2", source: "b", target: "c" },
    ];
    const { order, cycle } = topologicalSort(nodes, edges);
    expect(cycle).toBe(false);
    expect(order.indexOf("a")).toBeLessThan(order.indexOf("b"));
    expect(order.indexOf("b")).toBeLessThan(order.indexOf("c"));
  });

  it("should detect cycles", () => {
    const nodes = [
      { id: "a", position: { x: 0, y: 0 }, data: {} },
      { id: "b", position: { x: 100, y: 0 }, data: {} },
    ];
    const edges = [
      { id: "e1", source: "a", target: "b" },
      { id: "e2", source: "b", target: "a" },
    ];
    const { cycle } = topologicalSort(nodes, edges);
    expect(cycle).toBe(true);
  });
});

describe("BLOCK_LIBRARY", () => {
  it("should have all required fields", () => {
    for (const block of BLOCK_LIBRARY) {
      expect(block.type).toBeTruthy();
      expect(block.label).toBeTruthy();
      expect(block.category).toBeTruthy();
      expect(block.pins.length).toBeGreaterThan(0);
    }
  });

  it("should have matching getBlockDef", () => {
    for (const block of BLOCK_LIBRARY) {
      const found = getBlockDef(block.type);
      expect(found).toBe(block);
    }
  });

  it("should return undefined for unknown block type", () => {
    expect(getBlockDef("NONEXISTENT")).toBeUndefined();
  });
});

describe("FBD transpiler — review fixes", () => {
  it("should declare INT temp variables for arithmetic blocks", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "ArithTest",
      declarations: [],
      nodes: [
        { id: "in1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "a", variableType: "INT", pinValues: {} } },
        { id: "in2", type: "input_pin", position: { x: 0, y: 100 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "b", variableType: "INT", pinValues: {} } },
        { id: "add1", type: "block", position: { x: 200, y: 50 }, data: { blockType: "ADD", label: "ADD", pinValues: {} } },
        { id: "gt1", type: "block", position: { x: 400, y: 50 }, data: { blockType: "GT", label: "GT", pinValues: {} } },
        { id: "out1", type: "output_pin", position: { x: 600, y: 50 }, data: { blockType: "OUTPUT_PIN", label: "Output", variableName: "too_big", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "in1", target: "add1", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e2", source: "in2", target: "add1", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e3", source: "add1", target: "gt1", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e4", source: "in2", target: "gt1", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e5", source: "gt1", target: "out1", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toMatch(/__t1\s*:\s*INT/);
    // GT output temp or output variable should be BOOL
    expect(result.source).toContain("too_big : BOOL");
    expect(result.source).toContain("too_big := __t1 > b");
  });

  it("should reject invalid variable names", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "InjectTest",
      declarations: [],
      nodes: [
        { id: "in1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "a; (* inject *)", pinValues: {} } },
      ],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("Invalid variable name"))).toBe(true);
  });

  it("should reject invalid instance names", () => {
    const program: FbdProgram = {
      version: "1.0",
      programName: "InstanceTest",
      declarations: [],
      nodes: [
        { id: "ton1", type: "timer", position: { x: 0, y: 0 }, data: { blockType: "TON", label: "TON", instanceName: "1bad name!", pinValues: { PT: "T#100MS" } } },
      ],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    };
    const result = transpileFbdToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("Invalid instance name"))).toBe(true);
  });
});
