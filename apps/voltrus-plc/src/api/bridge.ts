import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FileEntry, RuntimeConfig } from "../types";
import { getSimHost, setSimEventHandler } from "../wasm/sim-host";
import "./relay-bridge";
import "./ai-bridge";

export type { UnlistenFn };

export type BridgeMode = "tauri" | "cloud";

export const bridgeMode: BridgeMode = "__TAURI_INTERNALS__" in window ? "tauri" : "cloud";

export type BridgeErrorCode =
  | "cloud_unavailable"
  | "unauthorized"
  | "not_found"
  | "bad_request"
  | "network";

export class BridgeError extends Error {
  constructor(
    public code: BridgeErrorCode,
    public cmd: string,
    message?: string,
  ) {
    super(message ?? `${code} (${cmd})`);
    this.name = "BridgeError";
  }
}

// Production serves the API from the same origin as the SPA (one worker);
// localhost dev targets the local wrangler dev server. vplc_base overrides.
const DEFAULT_BASE = /^localhost$|^127\.0\.0\.1$/.test(location.hostname)
  ? "http://localhost:8787"
  : "";

function cloudBase(): string {
  return localStorage.getItem("vplc_base") ?? DEFAULT_BASE;
}

function requireProjectId(cmd: string): string {
  const pid = localStorage.getItem("vplc_project");
  if (!pid) {
    throw new BridgeError("unauthorized", cmd, "No active cloud project (vplc_project is not set)");
  }
  return pid;
}

function authHeaders(): Record<string, string> {
  const token = localStorage.getItem("vplc_token");
  if (!token) return {};
  return { Authorization: `Bearer ${token}` };
}

function httpError(cmd: string, status: number): BridgeError {
  if (status === 401 || status === 403) return new BridgeError("unauthorized", cmd, `HTTP ${status}`);
  if (status === 404) return new BridgeError("not_found", cmd, `HTTP ${status} for ${cmd}`);
  if (status === 400) return new BridgeError("bad_request", cmd, `HTTP ${status} for ${cmd}`);
  return new BridgeError("network", cmd, `HTTP ${status} for ${cmd}`);
}

async function cloudFetch(cmd: string, url: string, init?: RequestInit): Promise<Response> {
  let res: Response;
  try {
    res = await fetch(url, init);
  } catch (e) {
    throw new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
  }
  if (!res.ok) throw httpError(cmd, res.status);
  return res;
}

function strArg(args: Record<string, unknown> | undefined, key: string, cmd: string): string {
  const v = args?.[key];
  if (typeof v !== "string") {
    throw new BridgeError("cloud_unavailable", cmd, `Command ${cmd} requires a string '${key}' argument`);
  }
  return v;
}

interface CloudFileRow {
  path: string;
  size: number;
  updatedAt: string;
}

// Mirrors commands::files::FileEntry { name, path, is_dir }: immediate children of
// `dir` only (prefix match with no deeper segment), dotfiles skipped, sorted
// dirs-first then case-insensitive by name. Cloud storage is flat, so is_dir is
// always false.
function toFileEntries(rows: CloudFileRow[], dir: string): FileEntry[] {
  const prefix = dir && !dir.endsWith("/") ? `${dir}/` : dir;
  return rows
    .filter((r) => {
      if (!prefix) return true;
      if (!r.path.startsWith(prefix)) return false;
      return !r.path.slice(prefix.length).includes("/");
    })
    .filter((r) => {
      const name = r.path.split("/").pop() ?? r.path;
      return !name.startsWith(".");
    })
    .map((r) => ({
      name: r.path.split("/").pop() ?? r.path,
      path: r.path,
      is_dir: false,
    }))
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}

async function cloudInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case "save_file": {
      const path = strArg(args, "path", cmd);
      const content = strArg(args, "content", cmd);
      const pid = encodeURIComponent(requireProjectId(cmd));
      await cloudFetch(
        cmd,
        `${cloudBase()}/api/projects/${pid}/files/${encodeURIComponent(path)}`,
        {
          method: "PUT",
          headers: { ...authHeaders(), "Content-Type": "application/json" },
          body: JSON.stringify({ content }),
        },
      );
      return undefined as T;
    }
    case "list_project_files": {
      const pid = encodeURIComponent(requireProjectId(cmd));
      const res = await cloudFetch(
        cmd,
        `${cloudBase()}/api/projects/${pid}/files`,
        { headers: authHeaders() },
      );
      const rows = (await res.json()) as CloudFileRow[];
      const dir = typeof args?.dir === "string" ? args.dir : "";
      return toFileEntries(rows, dir) as T;
    }
    case "open_file": {
      const path = strArg(args, "path", cmd);
      const pid = encodeURIComponent(requireProjectId(cmd));
      const res = await cloudFetch(
        cmd,
        `${cloudBase()}/api/projects/${pid}/files/${encodeURIComponent(path)}`,
        { headers: authHeaders() },
      );
      const body = (await res.json()) as { content: string };
      return body.content as T;
    }
    default:
      throw new BridgeError("cloud_unavailable", cmd, `Command '${cmd}' has no cloud mapping`);
  }
}

// Commands served locally by the wasm simulation worker in cloud mode.
// compile_source mirrors the Tauri `compile_source` command; plc_* and the
// debug/step/force/breakpoint/watch commands mirror commands::runtime and
// commands::watch. Everything else falls through to cloudInvoke.
const SIM_COMMANDS: Record<string, true> = {
  compile_source: true,
  plc_run: true,
  plc_stop: true,
  plc_step: true,
  continue_execution: true,
  step_instruction: true,
  force_variable: true,
  unforce_variable: true,
  set_breakpoint: true,
  clear_breakpoint: true,
  set_remote_breakpoint: true,
  clear_remote_breakpoint: true,
  get_watched_variables: true,
};

async function simInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const host = getSimHost();
  switch (cmd) {
    case "compile_source": {
      const source = strArg(args, "source", cmd);
      const fileName = typeof args?.fileName === "string" ? args.fileName : "untitled.st";
      const sources: Record<string, string> = { [fileName]: source };
      const extras = args?.extraSources;
      if (Array.isArray(extras)) {
        for (const [path, content] of extras as [string, string][]) sources[path] = content;
      }
      return host.compile(sources) as Promise<T>;
    }
    case "plc_run": {
      const raw = (args?.config ?? {}) as Partial<RuntimeConfig>;
      const config: RuntimeConfig = {
        scan_cycle_ms: Number(raw.scan_cycle_ms ?? 10),
        max_cycle_ms: Number(raw.max_cycle_ms ?? 1000),
      };
      return host.run(config) as Promise<T>;
    }
    case "plc_stop":
      return host.stop() as Promise<T>;
    case "plc_step":
      return host.step() as Promise<T>;
    case "continue_execution":
      return host.continueExecution() as Promise<T>;
    case "step_instruction":
      return host.stepInstruction() as Promise<T>;
    case "force_variable": {
      const name = strArg(args, "name", cmd);
      return host.force(name, args?.value) as Promise<T>;
    }
    case "unforce_variable":
      return host.unforce(strArg(args, "name", cmd)) as Promise<T>;
    case "set_breakpoint":
    case "set_remote_breakpoint":
      return host.setBreakpoint(Number(args?.line)) as Promise<T>;
    case "clear_breakpoint":
    case "clear_remote_breakpoint":
      return host.clearBreakpoint(Number(args?.line)) as Promise<T>;
    case "get_watched_variables":
      return host.readWatch() as Promise<T>;
    default:
      throw new BridgeError("cloud_unavailable", cmd, `Command '${cmd}' has no cloud mapping`);
  }
}

// Extension point for feature modules (relay, ai) that add cloud-mode command
// handlers without editing this file's switch. Register from your module:
//   registerCloudCommands({ my_cmd: (cmd, args) => ... })
// and side-effect-import the module here so it registers on load.
const cloudCommandHandlers = new Map<
  string,
  (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
>();

export function registerCloudCommands(
  handlers: Record<string, (cmd: string, args?: Record<string, unknown>) => Promise<unknown>>,
): void {
  for (const [cmd, handler] of Object.entries(handlers)) cloudCommandHandlers.set(cmd, handler);
}

export async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (bridgeMode === "tauri") return tauriInvoke<T>(cmd, args);
  if (cmd in SIM_COMMANDS) return simInvoke<T>(cmd, args);
  const extension = cloudCommandHandlers.get(cmd);
  if (extension) return extension(cmd, args) as Promise<T>;
  return cloudInvoke<T>(cmd, args);
}

// Cloud events: Tauri events ride the native bus; cloud-mode listeners
// register locally and are fed by the wasm sim worker (watch-update /
// runtime-state / breakpoint-hit) plus nothing else — remote transports have
// no event mapping in this wave.
const cloudListeners = new Map<string, Set<(e: { payload: unknown }) => void>>();

export function listen<T = unknown>(
  event: string,
  cb: (e: { payload: T }) => void,
): Promise<() => void> {
  if (bridgeMode === "tauri") return tauriListen<T>(event, cb);
  let set = cloudListeners.get(event);
  if (!set) {
    set = new Set();
    cloudListeners.set(event, set);
  }
  const handler = cb as (e: { payload: unknown }) => void;
  set.add(handler);
  return Promise.resolve(() => {
    set.delete(handler);
  });
}

// Delivers a local event to cloud-mode listen() subscribers.
export function emitLocalEvent(event: string, payload: unknown): void {
  const set = cloudListeners.get(event);
  if (!set) return;
  for (const handler of set) handler({ payload });
}

// Worker events (watch-update / runtime-state / breakpoint-hit) ride the same
// listener registry Tauri events use, so store.ts and App.tsx subscribers work
// unchanged in cloud mode.
setSimEventHandler((msg) => {
  emitLocalEvent(msg.event, msg.payload);
});
