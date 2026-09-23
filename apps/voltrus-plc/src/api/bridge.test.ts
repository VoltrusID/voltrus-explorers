import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { FileEntry } from "../types";

// vi.mock is hoisted, so these apply to every (re)import of ./bridge below.
const tauriInvokeMock = vi.hoisted(() => vi.fn());
const tauriListenMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauriInvokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauriListenMock }));

// bridge.ts computes bridgeMode at module load, so each mode needs a fresh
// import after toggling the Tauri global — this test intentionally exercises
// the module-loading boundary.
async function loadBridge(tauri: boolean) {
  const win = window as unknown as Record<string, unknown>;
  if (tauri) {
    win.__TAURI_INTERNALS__ = {};
  } else {
    delete win.__TAURI_INTERNALS__;
  }
  vi.resetModules();
  return import("./bridge");
}

const BASE = "http://test-api";
const PID = "proj-1";

function setCloudEnv() {
  localStorage.setItem("vplc_base", BASE);
  localStorage.setItem("vplc_project", PID);
  localStorage.setItem("vplc_token", "tok-1");
}

function fetchJson(body: unknown, status = 200) {
  return vi.fn(async (_url: string, _init?: RequestInit) => ({
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  }));
}

// Node/vitest provide no localStorage (browsers always do), so cloud-mode
// tests run against an in-memory stub.
function memStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (k) => map.get(k) ?? null,
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (k) => {
      map.delete(k);
    },
    setItem: (k, v) => {
      map.set(k, String(v));
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("localStorage", memStorage());
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("bridge in tauri mode", () => {
  it("delegates invoke to @tauri-apps/api/core", async () => {
    const bridge = await loadBridge(true);
    expect(bridge.bridgeMode).toBe("tauri");
    tauriInvokeMock.mockResolvedValueOnce({ state: "paused" });
    await expect(bridge.invoke("continue_execution", { extra: 1 })).resolves.toEqual({
      state: "paused",
    });
    expect(tauriInvokeMock).toHaveBeenCalledWith("continue_execution", { extra: 1 });
  });

  it("delegates listen and returns the tauri unlisten fn", async () => {
    const bridge = await loadBridge(true);
    const unlisten = vi.fn();
    const cb = vi.fn();
    tauriListenMock.mockResolvedValueOnce(unlisten);
    await expect(bridge.listen("breakpoint-hit", cb)).resolves.toBe(unlisten);
    expect(tauriListenMock).toHaveBeenCalledWith("breakpoint-hit", cb);
  });
});

describe("bridge in cloud mode", () => {
  it("save_file PUTs JSON content to the flat project file URL", async () => {
    setCloudEnv();
    const fetchMock = fetchJson(null, 204);
    vi.stubGlobal("fetch", fetchMock);
    const bridge = await loadBridge(false);
    await expect(
      bridge.invoke("save_file", { path: "sub/Main.st", content: "PROGRAM" }),
    ).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${BASE}/api/projects/${PID}/files/sub%2FMain.st`);
    expect(init?.method).toBe("PUT");
    expect(init?.headers).toEqual({
      Authorization: "Bearer tok-1",
      "Content-Type": "application/json",
    });
    expect(JSON.parse(init?.body as string)).toEqual({ content: "PROGRAM" });
  });

  it("list_project_files GETs and maps rows to the Tauri FileEntry shape", async () => {
    setCloudEnv();
    const fetchMock = fetchJson([
      { path: "b.st", size: 3, updatedAt: "2026-01-02T00:00:00Z" },
      { path: "a.st", size: 1, updatedAt: "2026-01-01T00:00:00Z" },
      { path: ".hidden.st", size: 1, updatedAt: "2026-01-01T00:00:00Z" },
    ]);
    vi.stubGlobal("fetch", fetchMock);
    const bridge = await loadBridge(false);
    const files = await bridge.invoke<FileEntry[]>("list_project_files", { dir: "" });
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${BASE}/api/projects/${PID}/files`);
    expect(init?.method).toBeUndefined();
    expect(init?.headers).toEqual({ Authorization: "Bearer tok-1" });
    expect(files).toEqual([
      { name: "a.st", path: "a.st", is_dir: false },
      { name: "b.st", path: "b.st", is_dir: false },
    ]);
  });

  it("list_project_files filters by dir prefix to immediate children", async () => {
    setCloudEnv();
    vi.stubGlobal(
      "fetch",
      fetchJson([
        { path: "Main.st", size: 1, updatedAt: "2026-01-01T00:00:00Z" },
        { path: "sub/x.st", size: 1, updatedAt: "2026-01-01T00:00:00Z" },
        { path: "sub/deep/y.st", size: 1, updatedAt: "2026-01-01T00:00:00Z" },
      ]),
    );
    const bridge = await loadBridge(false);
    const files = await bridge.invoke<FileEntry[]>("list_project_files", { dir: "sub" });
    expect(files).toEqual([{ name: "x.st", path: "sub/x.st", is_dir: false }]);
  });

  it("open_file GETs the file and unwraps { content }", async () => {
    setCloudEnv();
    const fetchMock = fetchJson({ content: "PROGRAM Main" });
    vi.stubGlobal("fetch", fetchMock);
    const bridge = await loadBridge(false);
    await expect(bridge.invoke<string>("open_file", { path: "Main.st" })).resolves.toBe(
      "PROGRAM Main",
    );
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${BASE}/api/projects/${PID}/files/Main.st`);
    expect(init?.headers).toEqual({ Authorization: "Bearer tok-1" });
  });

  it("maps 401 to unauthorized and 404 to not_found", async () => {
    setCloudEnv();
    vi.stubGlobal("fetch", fetchJson(null, 401));
    const bridge = await loadBridge(false);
    type BridgeErr = InstanceType<typeof bridge.BridgeError>;
    const unauthorized = (await bridge
      .invoke("open_file", { path: "Main.st" })
      .catch((e) => e)) as BridgeErr;
    expect(unauthorized).toBeInstanceOf(bridge.BridgeError);
    expect(unauthorized.code).toBe("unauthorized");
    expect(unauthorized.cmd).toBe("open_file");

    vi.stubGlobal("fetch", fetchJson(null, 404));
    const notFound = (await bridge
      .invoke("open_file", { path: "Nope.st" })
      .catch((e) => e)) as BridgeErr;
    expect(notFound).toBeInstanceOf(bridge.BridgeError);
    expect(notFound.code).toBe("not_found");

    vi.stubGlobal("fetch", fetchJson(null, 400));
    const badRequest = (await bridge
      .invoke("open_file", { path: "a..b.st" })
      .catch((e) => e)) as BridgeErr;
    expect(badRequest).toBeInstanceOf(bridge.BridgeError);
    expect(badRequest.code).toBe("bad_request");
  });

  it("throws cloud_unavailable for commands with no cloud mapping", async () => {
    setCloudEnv();
    const bridge = await loadBridge(false);
    const err = (await bridge.invoke("compile_source", { source: "x" }).catch((e) => e)) as InstanceType<
      typeof bridge.BridgeError
    >;
    expect(err).toBeInstanceOf(bridge.BridgeError);
    expect(err.code).toBe("cloud_unavailable");
    expect(err.cmd).toBe("compile_source");
  });

  it("listen registers locally and never fires in wave 1", async () => {
    setCloudEnv();
    const bridge = await loadBridge(false);
    const cb = vi.fn();
    const unlisten = await bridge.listen("breakpoint-hit", cb);
    expect(typeof unlisten).toBe("function");
    expect(() => unlisten()).not.toThrow();
    expect(cb).not.toHaveBeenCalled();
  });
});
