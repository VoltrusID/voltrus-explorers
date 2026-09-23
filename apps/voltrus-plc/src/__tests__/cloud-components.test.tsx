import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { FileEntry } from "../types";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// Hoisted so the vi.mock factory can export the same BridgeError class that
// store/cloud.ts imports — instanceof checks must match at runtime.
const { invokeMock, BridgeError } = vi.hoisted(() => {
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
  return { invokeMock: vi.fn(), BridgeError: HoistedBridgeError };
});

vi.mock("../api/bridge", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
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

const { useCloudStore } = await import("../store/cloud");
const { default: AuthDialog } = await import("../components/Cloud/AuthDialog");
const { default: CloudProjectPanel } = await import("../components/Cloud/CloudProjectPanel");
const { default: AccountBadge } = await import("../components/Cloud/AccountBadge");
const { useStore } = await import("../store");

const BASE = "http://test-api";
const USER = { id: "u-1", email: "tech@plant.io" };
const PROJECT = {
  id: "p-1",
  name: "Line 1",
  createdAt: "2026-09-23T00:00:00Z",
  updatedAt: "2026-09-23T00:00:00Z",
  fileCount: 2,
};

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function fetchRouter(routes: Record<string, (init?: RequestInit) => Response>) {
  return vi.fn((url: string | URL, init?: RequestInit) => {
    const path = String(url).replace(BASE, "");
    const key = `${init?.method ?? "GET"} ${path}`;
    const handler = routes[key];
    if (!handler) throw new Error(`unexpected fetch: ${key}`);
    return handler(init);
  });
}

function resetCloudStore() {
  useCloudStore.setState({
    user: null,
    token: null,
    projectId: null,
    projects: [],
    cloudFiles: [],
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

const click = (el: Element) =>
  act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });

const setInputValue = (el: Element, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(el, value);
  act(() => {
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const buttonByText = (text: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("vplc_base", BASE);
  vi.stubGlobal("fetch", vi.fn());
  resetCloudStore();
  useStore.setState({ openFiles: [], activeFileIndex: 0, projectDir: null, projectFiles: [] });
  invokeMock.mockReset();

  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
});

describe("AuthDialog", () => {
  it("renders only when authRequired is set, and dismisses", async () => {
    await renderNode(React.createElement(AuthDialog));
    expect(container.querySelector('[role="dialog"]')).toBeNull();

    await act(async () => {
      useCloudStore.setState({ authRequired: true });
    });
    expect(container.querySelector('[role="dialog"]')).not.toBeNull();
    expect(container.textContent).toContain("Sign in to Voltrus Cloud");

    await click(buttonByText("Cancel")!);
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it("login from the dialog persists vplc_token and closes on success", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "POST /api/auth/login": () => jsonResponse({ token: "tok-ui", user: USER }),
        "GET /api/projects": () => jsonResponse([]),
      }),
    );
    await act(async () => {
      useCloudStore.setState({ authRequired: true });
    });
    await renderNode(React.createElement(AuthDialog));

    setInputValue(container.querySelector('input[aria-label="Email"]')!, USER.email);
    setInputValue(container.querySelector('input[aria-label="Password"]')!, "password123");
    await click(buttonByText("Sign in")!);

    expect(localStorage.getItem("vplc_token")).toBe("tok-ui");
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(useCloudStore.getState().user).toEqual(USER);
  });

  it("register with duplicate email shows the server error verbatim", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        "POST /api/auth/register": () => jsonResponse({ error: "email already registered" }, 409),
      }),
    );
    await act(async () => {
      useCloudStore.setState({ authRequired: true });
    });
    await renderNode(React.createElement(AuthDialog));

    await click(buttonByText("Need an account? Register")!);
    setInputValue(container.querySelector('input[aria-label="Email"]')!, USER.email);
    setInputValue(container.querySelector('input[aria-label="Password"]')!, "password123");
    await click(buttonByText("Register")!);

    expect(container.querySelector('[role="alert"]')?.textContent).toBe("email already registered");
    expect(localStorage.getItem("vplc_token")).toBeNull();
  });
});

describe("CloudProjectPanel", () => {
  const files: FileEntry[] = [
    { name: "Main.st", path: "Main.st", is_dir: false },
    { name: "Conveyor.st", path: "Conveyor.st", is_dir: false },
  ];

  it("Open selects the project, pulls files, and surfaces them to the main store", async () => {
    invokeMock.mockResolvedValue(files);
    await act(async () => {
      useCloudStore.setState({ token: "tok-1", projects: [PROJECT] });
    });
    await renderNode(React.createElement(CloudProjectPanel));

    expect(container.textContent).toContain("Line 1");
    expect(container.textContent).toContain("No active project");

    await click(buttonByText("Open")!);
    await act(async () => {});

    expect(localStorage.getItem("vplc_project")).toBe("p-1");
    expect(container.textContent).toContain("Active: Line 1");
    expect(container.textContent).toContain("Main.st");
    expect(container.textContent).toContain("Conveyor.st");
    expect(useStore.getState().projectFiles).toEqual(files);
  });

  it("shows a desktop-only hint when the bridge has no cloud mapping", async () => {
    invokeMock.mockRejectedValue(
      new BridgeError("cloud_unavailable", "list_project_files", "no cloud mapping"),
    );
    await act(async () => {
      useCloudStore.setState({ token: "tok-1", projects: [PROJECT] });
    });
    await renderNode(React.createElement(CloudProjectPanel));

    await click(buttonByText("Open")!);
    await act(async () => {});

    expect(container.textContent).toContain("Desktop-only feature");
  });

  it("notes unsaved changes from the main store", async () => {
    await renderNode(React.createElement(CloudProjectPanel));
    expect(container.textContent).not.toContain("unsaved");

    await act(async () => {
      useStore.setState({
        openFiles: [
          { name: "Main.st", path: "Main.st", content: "a", modified: true },
          { name: "B.st", path: "B.st", content: "b", modified: false },
        ],
        activeFileIndex: 0,
      });
    });

    expect(container.textContent).toContain("1 unsaved change");
  });

  it("delete removes the project row", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({ "DELETE /api/projects/p-1": () => jsonResponse(null, 204) }),
    );
    await act(async () => {
      useCloudStore.setState({ token: "tok-1", projects: [PROJECT] });
    });
    await renderNode(React.createElement(CloudProjectPanel));

    await click(buttonByText("Delete")!);
    await act(async () => {});

    expect(useCloudStore.getState().projects).toEqual([]);
    expect(container.textContent).toContain("No cloud projects yet.");
  });
});

describe("AccountBadge", () => {
  it("restores the session via whoami and logs out", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({ "GET /api/auth/me": () => jsonResponse({ user: USER }) }),
    );
    await act(async () => {
      useCloudStore.setState({ token: "tok-1" });
    });
    await renderNode(React.createElement(AccountBadge));

    // whoami resolves within the render act, so the badge lands on the email.
    expect(container.textContent).toContain(USER.email);

    await click(buttonByText("Log out")!);

    expect(localStorage.getItem("vplc_token")).toBeNull();
    expect(container.textContent).toContain("Sign in to Voltrus Cloud");
  });

  it("offers sign-in when there is no session", async () => {
    await renderNode(React.createElement(AccountBadge));

    await click(buttonByText("Sign in to Voltrus Cloud")!);
    expect(useCloudStore.getState().authRequired).toBe(true);
  });
});
