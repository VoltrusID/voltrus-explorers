import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore } from "../store";
import type { FileEntry } from "../types";

// Hoisted so the vi.mock factory below can export the same BridgeError class
// that store/cloud.ts imports — instanceof checks must match at runtime.
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
  bridgeMode: "cloud",
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  // Pulled in transitively: store/cloud.ts imports api/ai-bridge, which
  // registers cloud commands against these on load.
  emitLocalEvent: vi.fn(),
  registerCloudCommands: vi.fn(),
  BridgeError,
}));

// Node ≥22 exposes a localStorage global that stays undefined without
// --localstorage-file and shadows jsdom's, so — like api/bridge.test.ts — we
// run against an in-memory stub. The cloud store reads vplc_* keys at module
// load, hence the dynamic import below only after the stub is in place.
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

const BASE = "http://test-api";
const USER = { id: "u-1", email: "tech@plant.io" };
const TOKEN = "tok-1";
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

// Minimal URL/method router for the cloud worker's REST surface.
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

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("vplc_base", BASE);
  vi.stubGlobal("fetch", vi.fn());
  resetCloudStore();
  useStore.setState({
    openFiles: [],
    activeFileIndex: 0,
    projectDir: null,
    projectFiles: [],
  });
  invokeMock.mockReset();
});

describe("cloud store auth", () => {
  it("login persists vplc_token, loads the user and refreshes projects", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        ["POST /api/auth/login"]: () => jsonResponse({ token: TOKEN, user: USER }),
        ["GET /api/projects"]: () => jsonResponse([PROJECT]),
      }),
    );

    await useCloudStore.getState().login(USER.email, "password123");

    expect(localStorage.getItem("vplc_token")).toBe(TOKEN);
    const s = useCloudStore.getState();
    expect(s.token).toBe(TOKEN);
    expect(s.user).toEqual(USER);
    expect(s.projects).toEqual([PROJECT]);
    expect(s.authRequired).toBe(false);
  });

  it("login failure surfaces the server error and writes no token", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        ["POST /api/auth/login"]: () => jsonResponse({ error: "invalid credentials" }, 401),
      }),
    );

    await useCloudStore.getState().login(USER.email, "wrong-password");

    expect(useCloudStore.getState().error).toBe("invalid credentials");
    expect(useCloudStore.getState().token).toBeNull();
    expect(localStorage.getItem("vplc_token")).toBeNull();
  });

  it("register duplicate email surfaces the 409 server message", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        ["POST /api/auth/register"]: () => jsonResponse({ error: "email already registered" }, 409),
      }),
    );
    useCloudStore.setState({ authRequired: true });

    await useCloudStore.getState().register(USER.email, "password123");

    const s = useCloudStore.getState();
    expect(s.error).toBe("email already registered");
    // Dialog stays open (authRequired untouched) and no session is created.
    expect(s.authRequired).toBe(true);
    expect(s.token).toBeNull();
    expect(localStorage.getItem("vplc_token")).toBeNull();
  });

  it("logout clears localStorage keys and cloud state", async () => {
    localStorage.setItem("vplc_token", TOKEN);
    localStorage.setItem("vplc_project", PROJECT.id);
    useCloudStore.setState({
      token: TOKEN,
      user: USER,
      projectId: PROJECT.id,
      projects: [PROJECT],
      cloudFiles: [{ name: "Main.st", path: "Main.st", is_dir: false }],
    });

    useCloudStore.getState().logout();

    expect(localStorage.getItem("vplc_token")).toBeNull();
    expect(localStorage.getItem("vplc_project")).toBeNull();
    const s = useCloudStore.getState();
    expect(s.token).toBeNull();
    expect(s.user).toBeNull();
    expect(s.projectId).toBeNull();
    expect(s.projects).toEqual([]);
    expect(s.cloudFiles).toEqual([]);
  });

  it("whoami restores the user for a stored token; 401 drops the session", async () => {
    // whoami also refreshes the entitlement since billing landed, so stub by
    // route (call-count for /me) instead of a fragile call-order queue.
    let meCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/auth/me")) {
          meCalls += 1;
          return meCalls === 1
            ? jsonResponse({ user: USER })
            : jsonResponse({ error: "token expired" }, 401);
        }
        if (url.includes("/api/billing/plan")) {
          return jsonResponse({
            plan: "free",
            limits: { maxProjects: 1, maxFilesPerProject: 25 },
            checkout: { solo: null, team: null },
          });
        }
        return jsonResponse({ error: "not_found" }, 404);
      }),
    );

    useCloudStore.setState({ token: TOKEN });
    await useCloudStore.getState().whoami();
    expect(useCloudStore.getState().user).toEqual(USER);
    // refreshPlan is fire-and-forget; flush it before asserting.
    await vi.waitFor(() => expect(useCloudStore.getState().plan?.plan).toBe("free"));

    useCloudStore.setState({ token: TOKEN, user: USER });
    await useCloudStore.getState().whoami();
    const s = useCloudStore.getState();
    expect(s.user).toBeNull();
    expect(s.token).toBeNull();
    expect(s.authRequired).toBe(true);
  });
});

describe("cloud store 401 handling", () => {
  it("expired token on refreshProjects sets authRequired", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({ ["GET /api/projects"]: () => jsonResponse({ error: "token expired" }, 401) }),
    );
    useCloudStore.setState({ token: "expired" });

    await useCloudStore.getState().refreshProjects();

    const s = useCloudStore.getState();
    expect(s.authRequired).toBe(true);
    expect(s.error).toBe("token expired");
  });

  it("bridge unauthorized during selectProject sets authRequired", async () => {
    useCloudStore.setState({ token: TOKEN });
    invokeMock.mockRejectedValue(new BridgeError("unauthorized", "list_project_files", "HTTP 401"));

    await useCloudStore.getState().selectProject(PROJECT.id);

    const s = useCloudStore.getState();
    expect(s.authRequired).toBe(true);
    expect(s.filesLoading).toBe(false);
  });
});

describe("cloud store project flow", () => {
  it("login -> create -> select -> files load -> openCloudFile end to end", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        ["POST /api/auth/login"]: () => jsonResponse({ token: TOKEN, user: USER }),
        ["GET /api/projects"]: () => jsonResponse([PROJECT]),
        ["POST /api/projects"]: () =>
          jsonResponse({ id: "p-2", name: "Line 2", createdAt: PROJECT.createdAt, updatedAt: PROJECT.updatedAt }, 201),
      }),
    );
    const files: FileEntry[] = [
      { name: "Main.st", path: "Main.st", is_dir: false },
      { name: "Conveyor.st", path: "Conveyor.st", is_dir: false },
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_project_files") return Promise.resolve(files);
      if (cmd === "open_file") return Promise.resolve("PROGRAM Main END_PROGRAM");
      throw new Error(`unexpected command ${cmd}`);
    });

    const cloud = useCloudStore.getState();
    await cloud.login(USER.email, "password123");
    await cloud.createProject("Line 2");
    await cloud.selectProject("p-1");
    await cloud.openCloudFile("Conveyor.st");

    // create prepends the new project
    expect(useCloudStore.getState().projects.map((p) => p.id)).toEqual(["p-2", "p-1"]);

    // select wrote the bridge's project key and loaded files
    expect(localStorage.getItem("vplc_project")).toBe("p-1");
    expect(useCloudStore.getState().cloudFiles).toEqual(files);
    // files surfaced to the main store while no local folder is open
    expect(useStore.getState().projectFiles).toEqual(files);

    // openCloudFile routed the file into the main store's editor tabs
    expect(invokeMock).toHaveBeenCalledWith("open_file", { path: "Conveyor.st" });
    const main = useStore.getState();
    const opened = main.openFiles.find((f) => f.path === "Conveyor.st");
    expect(opened?.content).toBe("PROGRAM Main END_PROGRAM");
    expect(main.openFiles[main.activeFileIndex]?.path).toBe("Conveyor.st");
    expect(useCloudStore.getState().openingPath).toBeNull();
  });

  it("selectProject does not clobber an open local folder's file list", async () => {
    useCloudStore.setState({ token: TOKEN });
    useStore.setState({ projectDir: "/local", projectFiles: [{ name: "Local.st", path: "/local/Local.st", is_dir: false }] });
    invokeMock.mockResolvedValue([{ name: "Main.st", path: "Main.st", is_dir: false }]);

    await useCloudStore.getState().selectProject(PROJECT.id);

    expect(useCloudStore.getState().cloudFiles).toHaveLength(1);
    expect(useStore.getState().projectFiles).toEqual([
      { name: "Local.st", path: "/local/Local.st", is_dir: false },
    ]);
  });

  it("delete removes the project; deleting the active one clears the selection", async () => {
    vi.stubGlobal(
      "fetch",
      fetchRouter({
        ["DELETE /api/projects/p-1"]: () => jsonResponse(null, 204),
        ["DELETE /api/projects/p-2"]: () => jsonResponse(null, 204),
      }),
    );
    useCloudStore.setState({
      token: TOKEN,
      projectId: "p-1",
      projects: [PROJECT, { ...PROJECT, id: "p-2", name: "Line 2" }],
      cloudFiles: [{ name: "Main.st", path: "Main.st", is_dir: false }],
    });
    localStorage.setItem("vplc_project", "p-1");

    await useCloudStore.getState().deleteProject("p-1");

    let s = useCloudStore.getState();
    expect(s.projects.map((p) => p.id)).toEqual(["p-2"]);
    expect(s.projectId).toBeNull();
    expect(s.cloudFiles).toEqual([]);
    expect(localStorage.getItem("vplc_project")).toBeNull();

    await useCloudStore.getState().deleteProject("p-2");
    s = useCloudStore.getState();
    expect(s.projects).toEqual([]);
    // non-active delete never touches the (already cleared) selection
    expect(s.projectId).toBeNull();
  });

  it("cloud_unavailable from the bridge flags the desktop-only hint", async () => {
    useCloudStore.setState({ token: TOKEN });
    invokeMock.mockRejectedValue(
      new BridgeError("cloud_unavailable", "list_project_files", "Command 'list_project_files' has no cloud mapping"),
    );

    await useCloudStore.getState().selectProject(PROJECT.id);

    const s = useCloudStore.getState();
    expect(s.unavailable).toBe(true);
    expect(s.authRequired).toBe(false);
    expect(s.error).toBe("Command 'list_project_files' has no cloud mapping");
  });
});

describe("bridge error surfacing (PLC-29)", () => {
  const OPEN_FILE = {
    name: "Main.st",
    path: "Main.st",
    content: "PROGRAM Main\nEND_PROGRAM",
    modified: true,
  };

  it("Ctrl+S save failure with an expired session raises authRequired", async () => {
    localStorage.setItem("vplc_token", TOKEN);
    useCloudStore.setState({ token: TOKEN, user: USER });
    useStore.setState({ openFiles: [OPEN_FILE], activeFileIndex: 0 });
    invokeMock.mockRejectedValueOnce(new BridgeError("unauthorized", "save_file"));

    await useStore.getState().saveFile(0);

    expect(useCloudStore.getState().authRequired).toBe(true);
  });

  it("Ctrl+S save failure with a paywall gate raises the upgrade screen", async () => {
    localStorage.setItem("vplc_token", TOKEN);
    useCloudStore.setState({ token: TOKEN, user: USER });
    useStore.setState({ openFiles: [OPEN_FILE], activeFileIndex: 0 });
    invokeMock.mockRejectedValueOnce(new BridgeError("plan_required", "save_file"));

    await useStore.getState().saveFile(0);

    const s = useCloudStore.getState();
    expect(s.planGate).toBe(true);
    expect(s.error).toBe("Cloud storage limit reached — upgrade for unlimited projects and files.");
  });

  it("a successful cloud save refreshes the project file list", async () => {
    localStorage.setItem("vplc_token", TOKEN);
    localStorage.setItem("vplc_project", PROJECT.id);
    useCloudStore.setState({ token: TOKEN, user: USER, projectId: PROJECT.id });
    useStore.setState({ openFiles: [OPEN_FILE], activeFileIndex: 0 });
    invokeMock.mockResolvedValueOnce(undefined); // save_file
    vi.stubGlobal("fetch", fetchRouter({ ["GET /api/projects"]: () => jsonResponse([PROJECT]) }));
    invokeMock.mockResolvedValue([{ name: "Main.st", path: "Main.st", is_dir: false }]);

    await useStore.getState().saveFile(0);

    // vplc:saved listener → refreshProjects (fileCount) + selectProject (list)
    expect(invokeMock).toHaveBeenCalledWith("list_project_files");
    expect(useCloudStore.getState().cloudFiles).toHaveLength(1);
  });
});
