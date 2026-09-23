import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// bridgeMode is read at render time from a mutable hoisted box so a single
// mock serves both cloud-mode and tauri-mode assertions.
const { bridgeModeBox } = vi.hoisted(() => ({ bridgeModeBox: { value: "cloud" as string } }));

vi.mock("../api/bridge", () => ({
  get bridgeMode() {
    return bridgeModeBox.value;
  },
  invoke: vi.fn(),
  listen: vi.fn().mockResolvedValue(() => {}),
  // ai-bridge.ts retries cloud-command registration on a timer; a no-op
  // keeps that retry loop quiet in tests.
  registerCloudCommands: vi.fn(),
  BridgeError: class BridgeError extends Error {
    constructor(
      public code: string,
      public cmd: string,
      message?: string,
    ) {
      super(message ?? `${code} (${cmd})`);
      this.name = "BridgeError";
    }
  },
}));

// Node ≥22's localStorage global shadows jsdom's; install memory stubs before
// the (dynamic) imports below — the cloud store reads vplc_* keys at load.
function memStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (k: string) => (map.has(k) ? map.get(k)! : null),
    key: (i: number) => Array.from(map.keys())[i] ?? null,
    removeItem: (k: string) => map.delete(k),
    setItem: (k: string, v: string) => map.set(k, String(v)),
  } as Storage;
}

vi.stubGlobal("localStorage", memStorage());
vi.stubGlobal("sessionStorage", memStorage());

const { useCloudStore } = await import("../store/cloud");
const { default: AuthScreen } = await import("../components/Cloud/AuthScreen");

const USER = { id: "u-1", email: "tech@plant.io" };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

let container: HTMLElement;
let root: Root;

const renderNode = (node: React.ReactNode) =>
  act(async () => {
    root.render(node);
  });

const click = (el: Element) =>
  act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });

const setInputValue = (el: Element, value: string) =>
  act(async () => {
    (el as HTMLInputElement).value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });

const buttonByText = (text: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  useCloudStore.getState().logout();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.stubGlobal("localStorage", memStorage());
  vi.stubGlobal("sessionStorage", memStorage());
  bridgeModeBox.value = "cloud";
});

describe("AuthScreen", () => {
  it("renders sign-in form in cloud mode when signed out", async () => {
    await renderNode(<AuthScreen />);
    expect(container.querySelector<HTMLInputElement>('input[placeholder="Email"]')).toBeTruthy();
    expect(buttonByText("Sign in")).toBeTruthy();
    expect(buttonByText("Continue without an account — edit locally")).toBeTruthy();
  });

  it("renders nothing in tauri mode", async () => {
    bridgeModeBox.value = "tauri";
    await renderNode(<AuthScreen />);
    expect(container.querySelector('input[placeholder="Email"]')).toBeNull();
  });

  it("renders nothing when a token is already present", async () => {
    localStorage.setItem("vplc_token", "tok-1");
    useCloudStore.setState({ token: "tok-1" });
    await renderNode(<AuthScreen />);
    expect(container.querySelector('input[placeholder="Email"]')).toBeNull();
  });

  it("toggles to register mode", async () => {
    await renderNode(<AuthScreen />);
    await click(buttonByText("Register")!);
    expect(buttonByText("Create account")).toBeTruthy();
    expect(container.querySelector('input[autocomplete="new-password"]')).toBeTruthy();
  });

  it("signs in and stores the session token", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login")) return jsonResponse({ token: "tok-2", user: USER });
      if (url.endsWith("/api/projects")) return jsonResponse([]);
      if (url.endsWith("/api/billing/plan")) {
        return jsonResponse({
          plan: "free",
          limits: { maxProjects: 1, maxFilesPerProject: 25 },
          checkout: { solo: null, team: null },
        });
      }
      return jsonResponse({ error: "not_found" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);

    await renderNode(<AuthScreen />);
    await setInputValue(container.querySelector('input[placeholder="Email"]')!, "tech@plant.io");
    await setInputValue(container.querySelector('input[placeholder="Password"]')!, "hunter2");
    await click(container.querySelector('.auth-submit')!);

    expect(useCloudStore.getState().token).toBe("tok-2");
    expect(useCloudStore.getState().user?.email).toBe(USER.email);
    expect(localStorage.getItem("vplc_token")).toBe("tok-2");
    expect(container.querySelector('input[placeholder="Email"]')).toBeNull();
  });

  it("shows server errors without clearing the form", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login")) return jsonResponse({ error: "invalid credentials" }, 401);
      return jsonResponse({ error: "not_found" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);

    await renderNode(<AuthScreen />);
    await setInputValue(container.querySelector('input[placeholder="Email"]')!, "tech@plant.io");
    await setInputValue(container.querySelector('input[placeholder="Password"]')!, "wrong");
    await click(container.querySelector('.auth-submit')!);

    expect(container.querySelector('[role="alert"]')?.textContent).toBe("invalid credentials");
    expect(useCloudStore.getState().token).toBeNull();
  });

  it("skip stores the session flag and hides the screen", async () => {
    await renderNode(<AuthScreen />);
    await click(buttonByText("Continue without an account — edit locally")!);
    expect(sessionStorage.getItem("vplc_auth_skipped")).toBe("1");
    expect(container.querySelector('input[placeholder="Email"]')).toBeNull();
  });

  it("stays hidden after a skip on reload", async () => {
    sessionStorage.setItem("vplc_auth_skipped", "1");
    await renderNode(<AuthScreen />);
    expect(container.querySelector('input[placeholder="Email"]')).toBeNull();
  });
});
