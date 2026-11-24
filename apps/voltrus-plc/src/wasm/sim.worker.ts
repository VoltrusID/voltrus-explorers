/// <reference lib="webworker" />
//
// Cloud-mode simulation worker: owns the wasm ST compiler and the wasm PLC
// runtime so compile/run/debug happen entirely in the browser. Cycles are
// driven with setInterval(config.scan_cycle_ms); manual stepping (step /
// continue) is deterministic and wall-clock fidelity is not a goal.
import initStc, { wasm_compile, wasm_compile_project } from "@voltrus/stc-wasm";
import initRuntime, { WasmPlcRuntime } from "@voltrus/runtime-wasm";
import type { CompiledModule, RuntimeConfig, RuntimeStateInfo, WatchedVar } from "../types";
import type {
  DebugStepInfo,
  SimCompileResult,
  SimEventMessage,
  SimRequest,
  SimResponse,
  SimWorkerOutbound,
} from "./protocol";

let stcReady: Promise<unknown> | null = null;
let runtimeReady: Promise<unknown> | null = null;

function initWasm(): Promise<unknown> {
  stcReady ??= initStc();
  runtimeReady ??= initRuntime();
  return Promise.all([stcReady, runtimeReady]);
}

let runtime: WasmPlcRuntime | null = null;
// Serde `CompiledModule` JSON from the last successful compile; the raw input
// `WasmPlcRuntime::new` deserializes.
let moduleJson: string | null = null;
let cycleTimer: ReturnType<typeof setInterval> | null = null;
// Survives runtime recreation on `run` so debug sessions keep their points.
const breakpoints = new Set<number>();

function post(msg: SimWorkerOutbound): void {
  self.postMessage(msg);
}

function postEvent(event: SimEventMessage["event"], payload: unknown): void {
  post({ type: "event", event, payload } as SimEventMessage);
}

function emitWatch(plc: WasmPlcRuntime): void {
  postEvent("watch-update", JSON.parse(plc.watch_variables()) as WatchedVar[]);
}

// CycleStats serde JSON from execute_cycle.
interface CycleStatsJson {
  cycle_count: number;
  last_cycle_ms: number;
  avg_cycle_ms: number;
  max_cycle_ms: number;
  cycle_history: number[];
}

function stateInfo(
  state: RuntimeStateInfo["state"],
  errorMessage: string | null,
  cycleCount: number,
  stats?: CycleStatsJson,
): RuntimeStateInfo {
  return {
    state,
    cycle_count: cycleCount,
    last_cycle_ms: stats?.last_cycle_ms ?? 0,
    avg_cycle_ms: stats?.avg_cycle_ms ?? 0,
    max_cycle_ms: stats?.max_cycle_ms ?? 0,
    error_message: errorMessage,
    cycle_history: stats?.cycle_history ?? [],
  };
}

// RuntimeState serde JSON: "Stopped" | "Running" | "Paused" | {"Error": msg}.
function parseRuntimeState(plc: WasmPlcRuntime): { name: RuntimeStateInfo["state"]; error: string | null } {
  const parsed = JSON.parse(plc.state()) as unknown;
  if (parsed && typeof parsed === "object" && "Error" in parsed) {
    return { name: "error", error: String((parsed as Record<string, unknown>).Error) };
  }
  const name = (typeof parsed === "string" ? parsed.toLowerCase() : "stopped") as RuntimeStateInfo["state"];
  return { name, error: null };
}

function stopCycleTimer(): void {
  if (cycleTimer !== null) {
    clearInterval(cycleTimer);
    cycleTimer = null;
  }
}

// Mirrors the Tauri `force_variable` plain-JSON → PlcValue conversion.
function toPlcValueJson(value: unknown): string {
  if (typeof value === "boolean") return JSON.stringify({ Bool: value });
  if (typeof value === "number") {
    return Number.isInteger(value) ? JSON.stringify({ Dint: value }) : JSON.stringify({ Real: value });
  }
  if (typeof value === "string") return JSON.stringify({ String: value });
  return JSON.stringify({ Dint: 0 });
}

// Frontend `CompiledModule` shape (types.ts): the serde JSON carries extra
// debugger fields (pou_table, name_constants, line_map) the UI never reads.
function toFrontendModule(json: string): CompiledModule | null {
  if (!json) return null;
  const m = JSON.parse(json) as CompiledModule;
  return { name: m.name, instructions: m.instructions, variable_slots: m.variable_slots };
}

function compile(sources: Record<string, string>): SimCompileResult {
  const entries = Object.entries(sources);
  if (entries.length === 0) throw new Error("compile requires at least one source");

  // Single source follows the Tauri `compile_source` branching: compile() keeps
  // diagnostics source attribution identical ("analysis" vs "<file>:analysis").
  const out =
    entries.length === 1 ? wasm_compile(entries[0][1]) : wasm_compile_project(JSON.stringify(entries));

  moduleJson = out.module_json;

  return {
    success: out.success,
    diagnostics: JSON.parse(out.diagnostics || "[]"),
    pou_count: out.pou_count,
    variable_count: out.variable_count,
    module: toFrontendModule(out.module_json),
    lineMap: JSON.parse(out.line_map || "[]") as [number, number][],
  };
}

function ensureRuntime(config: RuntimeConfig): WasmPlcRuntime {
  if (!moduleJson) throw new Error("No compiled program. Click Build first.");
  const snapshot = runtime?.snapshot_variables() ?? "[]";
  runtime?.free();
  runtime = new WasmPlcRuntime(moduleJson, JSON.stringify(config));
  runtime.restore_variables(snapshot);
  for (const line of breakpoints) runtime.set_breakpoint_line(line);
  return runtime;
}

// ExecutionState serde JSON: "Running" | "Paused" | "Done" | {"BreakpointHit": pc}.
function debugStepInfo(plc: WasmPlcRuntime, execState: unknown): DebugStepInfo {
  const paused =
    execState === "Paused" || (execState !== null && typeof execState === "object" && "BreakpointHit" in execState);
  const state = paused ? "paused" : execState === "Done" ? "stopped" : "running";
  const line = plc.current_pc_line();
  return { state: state as DebugStepInfo["state"], current_line: typeof line === "number" ? line : null };
}

function requireRuntime(): WasmPlcRuntime {
  if (!runtime) throw new Error("Runtime not running");
  return runtime;
}

const handlers: { [K in SimRequest["type"]]: (req: Extract<SimRequest, { type: K }>) => unknown } = {
  compile: (req) => compile(req.sources),

  run: (req) => {
    stopCycleTimer();
    const plc = ensureRuntime(req.config);
    cycleTimer = setInterval(() => {
      try {
        const stats = JSON.parse(plc.execute_cycle()) as CycleStatsJson;
        const st = parseRuntimeState(plc);
        if (st.name === "error") {
          stopCycleTimer();
          postEvent("runtime-state", stateInfo("error", st.error, Number(plc.cycle_count()), stats));
          return;
        }
        emitWatch(plc);
        postEvent("runtime-state", stateInfo(st.name, null, Number(plc.cycle_count()), stats));
      } catch (e) {
        stopCycleTimer();
        postEvent(
          "runtime-state",
          stateInfo("error", e instanceof Error ? e.message : String(e), Number(plc.cycle_count())),
        );
      }
    }, req.config.scan_cycle_ms);
    return null;
  },

  stop: () => {
    stopCycleTimer();
    const st = runtime ? parseRuntimeState(runtime) : { name: "stopped" as const, error: null };
    postEvent("runtime-state", stateInfo(st.name, st.error, Number(runtime?.cycle_count() ?? 0)));
    return null;
  },

  // One full scan cycle; deterministic stepping — no timer involved.
  step: () => {
    const plc = runtime ?? ensureRuntime({ scan_cycle_ms: 10, max_cycle_ms: 1000 });
    const stats = JSON.parse(plc.execute_cycle()) as CycleStatsJson;
    emitWatch(plc);
    postEvent("runtime-state", stateInfo("paused", null, Number(plc.cycle_count()), stats));
    return null;
  },

  continue: () => {
    const plc = requireRuntime();
    const execState = JSON.parse(plc.continue_execution()) as unknown;
    const info = debugStepInfo(plc, execState);
    emitWatch(plc);
    if (info.state === "paused") postEvent("breakpoint-hit", info);
    return info;
  },

  stepInstruction: () => {
    const plc = requireRuntime();
    const execState = JSON.parse(plc.step_instruction()) as unknown;
    const info = debugStepInfo(plc, execState);
    emitWatch(plc);
    postEvent("breakpoint-hit", info);
    return info;
  },

  setBreakpoint: (req) => {
    if (!runtime) throw new Error("No runtime. Compile first.");
    breakpoints.add(req.line);
    runtime.set_breakpoint_line(req.line);
    return null;
  },

  clearBreakpoint: (req) => {
    if (!runtime) throw new Error("No runtime. Compile first.");
    breakpoints.delete(req.line);
    runtime.clear_breakpoint_line(req.line);
    return null;
  },

  readWatch: () => (runtime ? (JSON.parse(runtime.watch_variables()) as WatchedVar[]) : []),

  write: (req) => {
    requireRuntime().write_variable(req.name, toPlcValueJson(req.value));
    return null;
  },

  force: (req) => {
    requireRuntime().force_variable(req.name, toPlcValueJson(req.value));
    return null;
  },

  unforce: (req) => {
    requireRuntime().unforce_variable(req.name);
    return null;
  },

  dispose: () => {
    stopCycleTimer();
    runtime?.free();
    runtime = null;
    moduleJson = null;
    breakpoints.clear();
    return null;
  },
};

self.onmessage = (e: MessageEvent<SimRequest>) => {
  const req = e.data;
  initWasm()
    .then(() => handlers[req.type](req as never))
    .then((result) => post({ id: req.id, type: req.type, ok: true, result } as SimResponse))
    .catch((err: unknown) => {
      const error = err instanceof Error ? err.message : String(err);
      post({ id: req.id, type: req.type, ok: false, error } as SimResponse);
    });
};
