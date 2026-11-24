import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const { invokeMock, BridgeError } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
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

vi.mock("../api/bridge", () => ({
  bridgeMode: "cloud",
  invoke: invokeMock,
  listen: vi.fn().mockResolvedValue(() => {}),
  registerCloudCommands: vi.fn(),
  BridgeError,
}));

// Memory storage must exist before the store module loads (it reads vplc_*
// keys at import time) — hence the dynamic imports below.
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

const { useCloudStore } = await import("../store/cloud");
const { default: UpgradeScreen } = await import("../components/Cloud/UpgradeScreen");
const { default: AccountBadge } = await import("../components/Cloud/AccountBadge");

const USER = { id: "u-1", email: "free@plant.io" };
const FREE_PLAN = {
  plan: "free",
  limits: { maxProjects: 1, maxFilesPerProject: 25 },
  checkout: { solo: "https://polar.sh/checkout/solo", team: null },
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// Routes the store's fetch traffic: REST + bridge endpoints under test.
function stubApi(routes: Record<string, (init?: RequestInit) => Response>) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = (init?.method ?? "GET").toUpperCase();
    for (const [key, handler] of Object.entries(routes)) {
      const [m, suffix] = key.split(" ");
      if (m === method && url.includes(suffix)) return handler(init);
    }
    return jsonResponse({ error: `unrouted: ${method} ${url}` }, 404);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
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

const buttonByText = (text: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  useCloudStore.getState().logout();
  localStorage.setItem("vplc_token", "tok-1");
  useCloudStore.setState({ token: "tok-1", user: USER });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCloudStore.getState().logout();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.stubGlobal("localStorage", memStorage());
});

describe("paywall: UpgradeScreen", () => {
  it("hidden while planGate is false", async () => {
    await renderNode(<UpgradeScreen />);
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it("project-create 402 raises the paywall with plan cards and checkout link", async () => {
    stubApi({
      "GET /api/projects": () => jsonResponse([]),
      "POST /api/projects": () =>
        jsonResponse({ error: "plan_required", requiredPlan: "solo" }, 402),
      "GET /api/billing/plan": () => jsonResponse(FREE_PLAN),
    });

    await renderNode(<UpgradeScreen />);
    await act(async () => {
      await useCloudStore.getState().createProject("Second Plant");
    });

    expect(useCloudStore.getState().planGate).toBe(true);
    expect(container.textContent).toContain("Cloud plans");
    expect(container.textContent).toContain("Unlimited cloud projects");
    expect(container.textContent).toContain("$12/mo");

    const openSpy = vi.fn();
    vi.stubGlobal("open", openSpy);
    await click(buttonByText("Upgrade to Solo")!);
    expect(openSpy).toHaveBeenCalledWith("https://polar.sh/checkout/solo", "_blank", "noopener");
  });

  it("bridge save_file 402 (plan_required code) raises the paywall", async () => {
    stubApi({
      "GET /api/billing/plan": () => jsonResponse(FREE_PLAN),
    });
    invokeMock.mockRejectedValue(new BridgeError("plan_required", "save_file"));

    await renderNode(<UpgradeScreen />);
    await act(async () => {
      await useCloudStore.getState().selectProject("p-1");
    });

    expect(useCloudStore.getState().planGate).toBe(true);
  });

  it("hides upgrade buttons for plans without a checkout URL", async () => {
    useCloudStore.setState({
      plan: { plan: "free", limits: FREE_PLAN.limits, checkout: { solo: null, team: null } },
      planGate: true,
    });
    await renderNode(<UpgradeScreen />);
    expect(buttonByText("Coming soon")).toBeTruthy();
    expect(buttonByText("Upgrade to Solo")).toBeUndefined();
  });

  it("marks the current plan and dismisses via Not now", async () => {
    useCloudStore.setState({
      plan: FREE_PLAN as never,
      planGate: true,
    });
    await renderNode(<UpgradeScreen />);
    expect(buttonByText("Current plan")).toBeTruthy();
    await click(buttonByText("Not now")!);
    expect(useCloudStore.getState().planGate).toBe(false);
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it("Manage billing opens the Polar portal", async () => {
    useCloudStore.setState({ planGate: true, plan: FREE_PLAN as never });
    const openSpy = vi.fn();
    vi.stubGlobal("open", openSpy);
    stubApi({
      "GET /api/billing/portal": () => jsonResponse({ url: "https://polar.sh/portal/x" }),
    });
    await renderNode(<UpgradeScreen />);
    await click(buttonByText("Manage billing")!);
    expect(openSpy).toHaveBeenCalledWith("https://polar.sh/portal/x", "_blank", "noopener");
  });
});

describe("paywall: AccountBadge plan chip", () => {
  it("shows the free chip and opens the paywall on click", async () => {
    useCloudStore.setState({ plan: FREE_PLAN as never });
    stubApi({
      "GET /api/billing/plan": () => jsonResponse(FREE_PLAN),
    });
    await renderNode(<AccountBadge />);
    const chip = container.querySelector<HTMLButtonElement>(".plan-chip");
    expect(chip?.textContent).toBe("free");
    await click(chip!);
    expect(useCloudStore.getState().planGate).toBe(true);
  });

  it("refreshes plan after returning from checkout (visibility)", async () => {
    useCloudStore.setState({ planGate: true, plan: FREE_PLAN as never });
    stubApi({
      "GET /api/billing/plan": () =>
        jsonResponse({
          plan: "solo",
          limits: { maxProjects: null, maxFilesPerProject: null },
          checkout: { solo: "https://polar.sh/checkout/solo", team: null },
        }),
    });
    await renderNode(<UpgradeScreen />);
    await act(async () => {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(useCloudStore.getState().plan?.plan).toBe("solo");
    expect(container.querySelector(".plan-chip-solo")?.textContent).toBe("solo");
  });
});
