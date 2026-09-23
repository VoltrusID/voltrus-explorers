import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FileEntry } from "../types";

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

const DEFAULT_BASE = "http://localhost:8787";

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

export async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (bridgeMode === "tauri") return tauriInvoke<T>(cmd, args);
  return cloudInvoke<T>(cmd, args);
}

// Cloud wave 1 has no event transport: listeners register locally and simply
// never fire.
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
