import type { CompileResult, RuntimeConfig, RuntimeStateInfo, WatchedVar } from "../types";

// ── Requests (host → worker) ──
//
// Every request carries a monotonically increasing id; the worker echoes it on
// the matching response so concurrent commands can be correlated.

export type DebugStepInfo = {
  state: "paused" | "stopped" | "running";
  current_line: number | null;
};

// Same shape the Tauri `compile_source` command returns, plus the raw
// instruction→source-line map used by debugger views.
export type SimCompileResult = CompileResult & {
  lineMap: [number, number][];
};

export type SimRequestType =
  | "compile"
  | "run"
  | "stop"
  | "step"
  | "continue"
  | "stepInstruction"
  | "setBreakpoint"
  | "clearBreakpoint"
  | "readWatch"
  | "write"
  | "force"
  | "unforce"
  | "dispose";

export type SimRequest =
  | { id: number; type: "compile"; sources: Record<string, string> }
  | { id: number; type: "run"; config: RuntimeConfig }
  | { id: number; type: "stop" }
  | { id: number; type: "step" }
  | { id: number; type: "continue" }
  | { id: number; type: "stepInstruction" }
  | { id: number; type: "setBreakpoint"; line: number }
  | { id: number; type: "clearBreakpoint"; line: number }
  | { id: number; type: "readWatch" }
  | { id: number; type: "write"; name: string; value: unknown }
  | { id: number; type: "force"; name: string; value: unknown }
  | { id: number; type: "unforce"; name: string }
  | { id: number; type: "dispose" };

// Requests without the correlation id — the host assigns ids on send.
export type SimRequestBody = SimRequest extends infer T
  ? T extends { id: number }
    ? Omit<T, "id">
    : never
  : never;

// ── Responses (worker → host) ──

export type SimResultMap = {
  compile: SimCompileResult;
  run: null;
  stop: null;
  step: null;
  continue: DebugStepInfo;
  stepInstruction: DebugStepInfo;
  setBreakpoint: null;
  clearBreakpoint: null;
  readWatch: WatchedVar[];
  write: null;
  force: null;
  unforce: null;
  dispose: null;
};

export type SimOk<M extends SimRequestType = SimRequestType> = {
  id: number;
  type: M;
  ok: true;
  result: SimResultMap[M];
};

export type SimErr = {
  id: number;
  type: SimRequestType;
  ok: false;
  error: string;
};

export type SimResponse = {
  [M in SimRequestType]: SimOk<M>;
}[SimRequestType]
  | SimErr;

// ── Events (worker → host, unsolicited) ──
//
// Same payloads the Tauri backend emits as app events; the host forwards them
// into the local listener registry so the store needs no cloud-specific code.

export type SimEventMessage =
  | { type: "event"; event: "watch-update"; payload: WatchedVar[] }
  | { type: "event"; event: "runtime-state"; payload: RuntimeStateInfo }
  | { type: "event"; event: "breakpoint-hit"; payload: DebugStepInfo };

export type SimWorkerOutbound = SimResponse | SimEventMessage;
