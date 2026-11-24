// @vitest-environment node
//
// Real-wasm round trip, no Worker: transpile the MotorStartStop FBD sample to
// ST, compile it with the wasm mt-stc, run the module on the wasm PLC runtime,
// force an input, assert outputs and a breakpoint hit. This is the same
// compiler+runtime pair the sim worker drives in the browser.
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import initStc, { wasm_compile_project } from "@voltrus/stc-wasm";
import initRuntime, { WasmPlcRuntime } from "@voltrus/runtime-wasm";
import { transpileFbdToSt } from "../components/Editor/FbdEditor/transpiler/fbd-to-st";
import type { FbdProgram } from "../types/fbd";

const PKG_STC = "../../../../crates/mt-stc-wasm/pkg";
const PKG_RUNTIME = "../../../../crates/mt-plc-runtime-wasm/pkg";

const fbdSource = readFileSync(new URL("../../samples/MotorStartStop.fbd", import.meta.url), "utf-8");
const stcBytes = readFileSync(new URL(`${PKG_STC}/mt_stc_wasm_bg.wasm`, import.meta.url));
const runtimeBytes = readFileSync(
  new URL(`${PKG_RUNTIME}/mt_plc_runtime_wasm_bg.wasm`, import.meta.url),
);

// MotorStartStop.fbd is Y0 := (X0 OR M0) AND X1.
const program = JSON.parse(fbdSource) as FbdProgram;
const st = transpileFbdToSt(program);
const compileOut = await initStc({ module_or_path: stcBytes }).then(() => {
  const out = wasm_compile_project(JSON.stringify([["MotorStartStop.st", st.source]]));
  return {
    success: out.success,
    diagnostics: JSON.parse(out.diagnostics || "[]") as Array<{ severity: string; message: string }>,
    moduleJson: out.module_json,
    lineMap: JSON.parse(out.line_map || "[]") as [number, number][],
  };
});

await initRuntime({ module_or_path: runtimeBytes });

describe("wasm compile + simulate round trip (MotorStartStop)", () => {
  it("compiles the transpiled FBD sample to a module with a line map", () => {
    expect(st.diagnostics.filter((d) => d.severity === "error")).toEqual([]);
    expect(compileOut.success).toBe(true);
    expect(compileOut.diagnostics.filter((d) => d.severity === "error")).toEqual([]);
    expect(compileOut.moduleJson).not.toBe("");
    expect(compileOut.lineMap.length).toBeGreaterThan(0);
    const module = JSON.parse(compileOut.moduleJson) as { name: string };
    expect(module.name).toBe("MotorStartStop");
  });

  it("runs cycles with a forced input and drives the output", () => {
    const plc = new WasmPlcRuntime(
      compileOut.moduleJson,
      JSON.stringify({ scan_cycle_ms: 10, max_cycle_ms: 1000 }),
    );
    plc.force_variable("X0", JSON.stringify({ Bool: true }));
    plc.force_variable("X1", JSON.stringify({ Bool: true }));

    plc.execute_cycle();
    plc.execute_cycle();

    expect(JSON.parse(plc.read_variable("Y0")!)).toEqual({ Bool: true });

    // Breaking the force turns the AND gate off on the next scan.
    plc.unforce_variable("X1");
    plc.execute_cycle();
    expect(JSON.parse(plc.read_variable("Y0")!)).toEqual({ Bool: false });
    plc.free();
  });

  it("pauses on a breakpoint line set from the compile line map", () => {
    const plc = new WasmPlcRuntime(
      compileOut.moduleJson,
      JSON.stringify({ scan_cycle_ms: 10, max_cycle_ms: 1000 }),
    );
    plc.force_variable("X0", JSON.stringify({ Bool: true }));
    plc.force_variable("X1", JSON.stringify({ Bool: true }));

    const [, line] = compileOut.lineMap[compileOut.lineMap.length - 1];
    plc.set_breakpoint_line(line);

    const hit = JSON.parse(plc.continue_execution()) as Record<string, unknown>;
    expect(hit).toHaveProperty("BreakpointHit");
    expect(plc.current_pc_line()).toBe(line);

    plc.clear_breakpoint_line(line);
    const done = JSON.parse(plc.continue_execution());
    expect(done).toBe("Done");
    expect(JSON.parse(plc.read_variable("Y0")!)).toEqual({ Bool: true });

    const watch = JSON.parse(plc.watch_variables()) as Array<{
      name: string;
      quality: string;
      type_name: string;
    }>;
    expect(watch.find((v) => v.name === "X0")?.quality).toBe("forced");
    expect(watch.find((v) => v.name === "Y0")?.type_name).toBe("BOOL");
    plc.free();
  });
});
