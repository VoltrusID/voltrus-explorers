import { describe, it, expect } from "vitest";
import { transpileLdToSt } from "../components/Editor/LdEditor/transpiler/ld-to-st";
import type { LdProgram } from "../types/ld";
import { makeLdElement, makeLdPath, makeLdRung } from "../types/ld";

function makeProgram(name: string, rungs: LdProgram["rungs"]): LdProgram {
  return { version: "2.0", programName: name, rungs };
}

describe("LD transpiler (grid model)", () => {
  it("should produce empty program for empty rungs", () => {
    const program = makeProgram("Empty", []);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("PROGRAM Empty");
    expect(result.source).toContain("END_PROGRAM");
    expect(result.diagnostics).toHaveLength(0);
  });

  it("should transpile single contact → coil", () => {
    const program = makeProgram("Simple", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.diagnostics.filter((d) => d.severity === "error")).toHaveLength(0);
    expect(result.source).toContain("Y0 := X0");
    expect(result.source).toContain("X0 : BOOL");
    expect(result.source).toContain("Y0 : BOOL");
  });

  it("should transpile AND logic (two contacts)", () => {
    const program = makeProgram("AndLogic", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
          makeLdElement("contact_no", { variable: "X1", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("X0 AND X1");
  });

  it("should transpile NC contact as NOT", () => {
    const program = makeProgram("NcContact", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_nc", { variable: "X0", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("NOT X0");
  });

  it("should transpile OR logic (parallel paths)", () => {
    const program = makeProgram("OrLogic", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
        makeLdPath([
          makeLdElement("contact_no", { variable: "X1", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("X0 OR X1");
  });

  it("should transpile SET and RESET coils", () => {
    const program = makeProgram("SetReset", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
          makeLdElement("coil_set", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X1", registerType: "X" as const }),
          makeLdElement("coil_reset", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("IF X0 THEN Y0 := TRUE; END_IF;");
    expect(result.source).toContain("IF X1 THEN Y0 := FALSE; END_IF;");
  });

  it("should transpile TON timer", () => {
    const program = makeProgram("Timer", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
          makeLdElement("fb_block", { blockType: "TON", instanceName: "T1", pinValues: { PT: "T#500ms" } }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("T1(IN := X0, PT := T#500ms)");
    expect(result.source).toContain("T1 : TON");
  });

  it("should warn about rungs with no output", () => {
    const program = makeProgram("NoOutput", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_no", { variable: "X0", registerType: "X" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "warning" && d.message.includes("no output"))).toBe(true);
  });

  it("should reject invalid program names", () => {
    const program = makeProgram("Bad Name!", [
      makeLdRung([makeLdPath([makeLdElement("coil", { variable: "Y0" })])]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.diagnostics.some((d) => d.severity === "error" && d.message.includes("Invalid program name"))).toBe(true);
  });

  it("should handle rising edge contact", () => {
    const program = makeProgram("RisingEdge", [
      makeLdRung([
        makeLdPath([
          makeLdElement("contact_re", { variable: "X0", registerType: "X" as const }),
          makeLdElement("coil", { variable: "Y0", registerType: "Y" as const }),
        ]),
      ]),
    ]);
    const result = transpileLdToSt(program);
    expect(result.source).toContain("R_TRIG");
    expect(result.source).toContain("__rtrig_1.Q");
  });
});
