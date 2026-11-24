// @vitest-environment node
//
// Real-wasm round trip over the voltrus-lsp core — the same pair the LSP
// worker (src/wasm/lsp.worker.ts) drives in the browser. Mirrors the native
// round trip in crates/voltrus-lsp/tests/roundtrip.rs: a declared Motor FB,
// a deliberately broken Main (`boom` is never declared), diagnostics with
// range/message, hover, completion, and cross-file definition.
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import init, { WasmLsp } from "@voltrus/lsp-wasm";
import type { LspCompletionItem, LspDiagnostic, LspHoverInfo, LspLocation } from "./protocol";

const wasmBytes = readFileSync(
  new URL("../../../../crates/voltrus-lsp-wasm/pkg/voltrus_lsp_wasm_bg.wasm", import.meta.url),
);

await init({ module_or_path: wasmBytes });

const MOTOR_URI = "file:///Motor.st";
const MAIN_URI = "file:///Main.st";

const MOTOR_ST = "\
FUNCTION_BLOCK Motor\n\
VAR_INPUT\n\
    speed : INT;\n\
END_VAR\n\
END_FUNCTION_BLOCK\n\
";

// Deliberately broken: `boom` is never declared, so mt-stc's codegen reports
// it at the use site on line index 4 (`boom := 1;`).
const BROKEN_MAIN_ST = "\
PROGRAM Main\n\
VAR\n\
    m : Motor;\n\
END_VAR\n\
    boom := 1;\n\
END_PROGRAM\n\
";

const lsp = new WasmLsp();

describe("wasm lsp core round trip (Motor + broken Main)", () => {
  it("reports the undeclared variable with an LSP range and message", () => {
    lsp.sync(MOTOR_URI, MOTOR_ST);
    const diags = JSON.parse(lsp.sync(MAIN_URI, BROKEN_MAIN_ST)) as LspDiagnostic[];
    const boom = diags.find((d) => d.message.includes("boom"));
    expect(boom).toBeDefined();
    expect(boom!.severity).toBe(1);
    expect(boom!.range.start.line).toBe(4);
    expect(boom!.range.start.character).toBe(4);
    expect(boom!.range.end.character).toBeGreaterThan(4);
  });

  it("resolves hover for `m` to the instance and for `Motor` cross-file", () => {
    const m = JSON.parse(lsp.hover(MAIN_URI, 2, 4)) as LspHoverInfo | null;
    expect(m).not.toBeNull();
    expect(m!.name).toBe("m");
    expect(m!.type_name).toBe("Motor");
    expect(m!.range.start.line).toBe(2);
    expect(m!.uri).toBe(MAIN_URI);

    const motor = JSON.parse(lsp.hover(MAIN_URI, 2, 9)) as LspHoverInfo | null;
    expect(motor).not.toBeNull();
    expect(motor!.name).toBe("Motor");
    expect(motor!.uri).toBe(MOTOR_URI);
    expect(motor!.decl_line).toBe(1);
  });

  it("offers project symbols and IEC keywords in completion", () => {
    const items = JSON.parse(lsp.completion(MAIN_URI, 4, 8)) as LspCompletionItem[];
    const labels = items.map((i) => i.label);
    expect(labels).toContain("Motor");
    expect(labels).toContain("speed");
    expect(labels).toContain("Main");
    expect(labels).toContain("PROGRAM");
    const motor = items.find((i) => i.label === "Motor");
    expect(motor!.kind).toBeDefined();
  });

  it("jumps to the cross-file declaration via definition", () => {
    const loc = JSON.parse(lsp.definition(MAIN_URI, 2, 9)) as LspLocation | null;
    expect(loc).not.toBeNull();
    expect(loc!.uri).toBe(MOTOR_URI);
    expect(loc!.range.start.line).toBe(0); // FUNCTION_BLOCK Motor header
  });

  it("clears error diagnostics once the document is fixed", () => {
    const fixed = BROKEN_MAIN_ST.replace("boom := 1;", "m.speed := 1;");
    const diags = JSON.parse(lsp.sync(MAIN_URI, fixed)) as LspDiagnostic[];
    expect(diags.filter((d) => d.severity === 1)).toEqual([]);
  });
});
