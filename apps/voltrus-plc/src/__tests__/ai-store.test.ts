import { describe, it, expect, beforeEach, vi } from "vitest";

// AI key management store actions (store/cloud.ts) against the real
// api/ai-bridge.ts REST helpers, with fetch routed like the cloud worker.

// Hoisted so the vi.mock factory exports the same BridgeError class that
// store/cloud.ts imports — instanceof checks must match at runtime.
const { BridgeError } = vi.hoisted(() => {
  class HoistedBridgeError extends Error {
    constructor(
      public code: string,
      public cmd: string,
      message?: string,
    ) {
      super(message ?? `${code} (${cmd})`);
      this.name = "BridgeError";
    }
  }
  return { BridgeError: HoistedBridgeError };
});

vi.mock("../api/bridge", () => ({
  invoke: vi.fn(),
  listen: vi.fn().mockResolvedValue(() => {}),
  emitLocalEvent: vi.fn(),
  registerCloudCommands: vi.fn(),
  BridgeError,
}));

// Node ≥22 exposes a localStorage global that stays undefined without
// --localstorage-file and shadows jsdom's; the cloud store reads vplc_* keys
// at module load, hence the stub before the dynamic import below.
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

vi.stubGlobal("localStorage", memStorage());
localStorage.setItem("vplc_base", "http://test-api");

const { useCloudStore } = await import("../store/cloud");

const BASE = "http://test-api";
const TOKEN = "tok-1";
const KEY_ROW = { provider: "claude", masked: "sk-ant-…cdef", updatedAt: "2026-09-23T10:00:00Z" };

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

// Minimal URL/method router for the cloud worker's /api/ai/keys surface.
function fetchRouter(routes: Record<string, (init?: RequestInit) => Response>) {
  return vi.fn((url: string | URL, init?: RequestInit) => {
    const method = init?.method ?? "GET";
    const path = String(url).replace(BASE, "");
    const handler = routes[`${method} ${path}`];
    if (!handler) throw new Error(`unexpected fetch: ${method} ${path}`);
    return handler(init);
  });
}

function resetStore(token: string | null) {
  useCloudStore.setState({
    user: null,
    token,
    projectId: null,
    projects: [],
    cloudFiles: [],
    aiKeys: [],
    authRequired: false,
    busy: false,
    loadingProjects: false,
    filesLoading: false,
    openingPath: null,
    error: null,
    unavailable: false,
  });
}

beforeEach(() => {
  resetStore(TOKEN);
});

describe("fetchAiKeys", () => {
  it("loads the masked key list with the session token", async () => {
    const fetchMock = fetchRouter({
      "GET /api/ai/keys": (init) => {
        expect((init?.headers as Record<string, string>).Authorization).toBe(`Bearer ${TOKEN}`);
        return jsonResponse([KEY_ROW]);
      },
    });
    vi.stubGlobal("fetch", fetchMock);

    await useCloudStore.getState().fetchAiKeys();

    expect(useCloudStore.getState().aiKeys).toEqual([KEY_ROW]);
    expect(useCloudStore.getState().error).toBeNull();
  });

  it("flags the session as dead on 401", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({ "GET /api/ai/keys": () => jsonResponse({ error: "bad token" }, 401) }),
    );

    await useCloudStore.getState().fetchAiKeys();

    expect(useCloudStore.getState().authRequired).toBe(true);
    expect(useCloudStore.getState().error).toBe("bad token");
  });

  it("demands sign-in without a token", async () => {
    resetStore(null);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    await useCloudStore.getState().fetchAiKeys();

    expect(fetchMock).not.toHaveBeenCalled();
    expect(useCloudStore.getState().authRequired).toBe(true);
  });
});

describe("saveAiKey", () => {
  it("PUTs the provider and key, then refetches the masked list", async () => {
    const updated = [KEY_ROW, { provider: "ollama", masked: "olk-…a1", updatedAt: "2026-09-24T09:00:00Z" }];
    const bodies: unknown[] = [];
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "PUT /api/ai/keys": (init) => {
          bodies.push(JSON.parse(init?.body as string));
          expect((init?.headers as Record<string, string>).Authorization).toBe(`Bearer ${TOKEN}`);
          return jsonResponse({ provider: "ollama", masked: "olk-…a1" });
        },
        "GET /api/ai/keys": () => jsonResponse(updated),
      }),
    );

    await useCloudStore.getState().saveAiKey("ollama", "secret-ollama-key");

    expect(bodies).toEqual([{ provider: "ollama", key: "secret-ollama-key" }]);
    expect(useCloudStore.getState().aiKeys).toEqual(updated);
    expect(useCloudStore.getState().error).toBeNull();
  });

  it("surfaces plan-gate errors as text and leaves the list untouched", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "PUT /api/ai/keys": () => jsonResponse({ error: "plan_required", requiredPlan: "solo" }, 402),
      }),
    );

    await useCloudStore.getState().saveAiKey("claude", "sk-new");

    expect(useCloudStore.getState().error).toBe("plan_required");
    expect(useCloudStore.getState().aiKeys).toEqual([]);
    expect(useCloudStore.getState().unavailable).toBe(false);
  });
});

describe("deleteAiKey", () => {
  it("DELETEs the provider and refetches", async () => {
    const fetchMock = fetchRouter({
      "DELETE /api/ai/keys/claude": (init) => {
        expect((init?.headers as Record<string, string>).Authorization).toBe(`Bearer ${TOKEN}`);
        return jsonResponse({ ok: true });
      },
      "GET /api/ai/keys": () => jsonResponse([]),
    });
    vi.stubGlobal("fetch", fetchMock);
    useCloudStore.setState({ aiKeys: [KEY_ROW] });

    await useCloudStore.getState().deleteAiKey("claude");

    expect(fetchMock).toHaveBeenCalledWith(`${BASE}/api/ai/keys/claude`, expect.anything());
    expect(useCloudStore.getState().aiKeys).toEqual([]);
    expect(useCloudStore.getState().error).toBeNull();
  });

  it("surfaces a 404 no_key from a stale delete as text", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "DELETE /api/ai/keys/claude": () => jsonResponse({ error: "no_key" }, 404),
      }),
    );

    await useCloudStore.getState().deleteAiKey("claude");

    expect(useCloudStore.getState().error).toBe("no_key");
    expect(useCloudStore.getState().authRequired).toBe(false);
  });
});
