import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

// Node ≥22's localStorage global is undefined without --localstorage-file and
// shadows jsdom's; the cloud store reads vplc_* keys at module load, so the
// stub is installed before the (dynamic) imports below.
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
const { default: AiKeysPanel } = await import("../components/Cloud/AiKeysPanel");

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

let container: HTMLElement;
let root: Root;

const renderNode = (node: React.ReactNode) =>
  act(async () => {
    root.render(node);
  });

// Flush the store's async action chain (fetch → store → render) inside one
// act scope: each yield runs one hop of the mocked promise chain, so a fixed
// drain covers the whole chain deterministically (no timers).
const DRAIN = 8;
const flush = () =>
  act(async () => {
    for (let i = 0; i < DRAIN; i++) await Promise.resolve();
  });

const click = (el: Element) =>
  act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    for (let i = 0; i < DRAIN; i++) await Promise.resolve();
  });

const setInputValue = (el: Element, value: string) => {
  // React's value tracker ignores direct .value writes, so go through the
  // native setter (same as cloud-components.test.tsx).
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(el, value);
  act(() => {
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const buttonByText = (text: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  resetStore(TOKEN);
});

afterEach(() =>
  act(async () => {
    root.unmount();
    container.remove();
  }),
);

describe("AiKeysPanel", () => {
  it("shows the sign-in hint without a token", async () => {
    resetStore(null);
    await renderNode(<AiKeysPanel />);
    expect(container.textContent).toContain("Sign in to manage cloud AI keys.");
  });

  it("lists stored keys with their masked values and a delete action", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({ "GET /api/ai/keys": () => jsonResponse([KEY_ROW]) }),
    );
    await renderNode(<AiKeysPanel />);
    await flush();

    expect(container.textContent).toContain("claude");
    expect(container.textContent).toContain("sk-ant-…cdef");
    expect(buttonByText("Delete")).toBeDefined();
  });

  it("shows the empty hint when no keys are stored", async () => {
    vi.stubGlobal("fetch", fetchRouter({ "GET /api/ai/keys": () => jsonResponse([]) }));
    await renderNode(<AiKeysPanel />);
    await flush();

    expect(container.textContent).toContain("No keys stored");
    expect(buttonByText("Delete")).toBeUndefined();
  });

  it("saves a key through the store and clears the input", async () => {
    const saved = { provider: "claude", masked: "sk-ant-…new1", updatedAt: "2026-09-24T09:00:00Z" };
    // Stateful stand-in for the worker's ai_keys table: PUT replaces the row,
    // GET lists what's stored, so the store's post-save refetch is truthful.
    let stored: typeof saved | null = null;
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string | URL, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        if (String(url).replace(BASE, "") !== "/api/ai/keys") {
          throw new Error(`unexpected fetch: ${method} ${url}`);
        }
        if (method === "GET") return jsonResponse(stored ? [stored] : []);
        if (method === "PUT") {
          const body = JSON.parse(init?.body as string) as { provider: string; key: string };
          expect(body).toEqual({ provider: "claude", key: "sk-new" });
          stored = saved;
          return jsonResponse({ provider: saved.provider, masked: saved.masked });
        }
        throw new Error(`unexpected method: ${method}`);
      }),
    );
    await renderNode(<AiKeysPanel />);
    await flush();

    const input = container.querySelector('input[type="password"]');
    expect(input).toBeDefined();
    await setInputValue(input as Element, "sk-new");
    await click(buttonByText("Save") as Element);
    await flush();

    expect(useCloudStore.getState().aiKeys).toEqual([saved]);
    expect((input as HTMLInputElement).value).toBe("");
  });

  it("deletes a key", async () => {
    let stored: typeof KEY_ROW | null = KEY_ROW;
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string | URL, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const path = String(url).replace(BASE, "");
        if (path !== "/api/ai/keys" && !path.startsWith("/api/ai/keys/")) {
          throw new Error(`unexpected fetch: ${method} ${url}`);
        }
        if (method === "GET") return jsonResponse(stored ? [stored] : []);
        if (method === "DELETE") {
          expect(path).toBe("/api/ai/keys/claude");
          stored = null;
          return jsonResponse({ ok: true });
        }
        throw new Error(`unexpected method: ${method}`);
      }),
    );
    await renderNode(<AiKeysPanel />);
    await flush();

    await click(buttonByText("Delete") as Element);
    await flush();

    expect(useCloudStore.getState().aiKeys).toEqual([]);
    expect(buttonByText("Delete")).toBeUndefined();
  });

  it("surfaces plan-gate errors from the keys fetch as alert text", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "GET /api/ai/keys": () => jsonResponse({ error: "plan_required", requiredPlan: "solo" }, 402),
      }),
    );
    await renderNode(<AiKeysPanel />);
    await flush();

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe("plan_required");
  });
});
