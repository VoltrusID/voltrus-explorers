import { create } from "zustand";
import { invoke, BridgeError } from "../api/bridge";
import { useStore } from "../store";
import {
  CloudApiError,
  PlanRequiredError,
  createProject as apiCreateProject,
  deleteProject as apiDeleteProject,
  listProjects,
  login as apiLogin,
  register as apiRegister,
  whoami as apiWhoami,
  type CloudProject,
  type CloudUser,
} from "../api/cloud-auth";
import { getPlan, openBillingPortal, type PlanInfo } from "../api/billing";
import {
  listAiKeys as apiListAiKeys,
  putAiKey as apiPutAiKey,
  removeAiKey as apiRemoveAiKey,
  type AiProvider,
  type CloudAiKey,
} from "../api/ai-bridge";
import type { FileEntry } from "../types";

export type { CloudProject, CloudUser };

interface CloudState {
  user: CloudUser | null;
  token: string | null;
  projectId: string | null;
  projects: CloudProject[];
  // File list of the active cloud project; also mirrored into the main
  // store's projectFiles while no local folder is open.
  cloudFiles: FileEntry[];

  // Server-side AI provider keys (masked) managed by the cloud worker; the
  // chat panels prefer these over local key inputs in cloud mode.
  aiKeys: CloudAiKey[];

  // Billing: current entitlement (null = not fetched yet), the paywall overlay
  // flag (any 402 plan_required or explicit user request), and portal state.
  plan: PlanInfo | null;
  planGate: boolean;
  portalBusy: boolean;

  // Status fields
  authRequired: boolean; // 401 seen → AuthDialog auto-shows
  busy: boolean; // login/register in flight
  loadingProjects: boolean;
  filesLoading: boolean;
  openingPath: string | null;
  error: string | null;
  unavailable: boolean; // last error was cloud_unavailable → desktop-only hint

  register: (email: string, password: string) => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
  whoami: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  createProject: (name: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  selectProject: (id: string) => Promise<void>;
  openCloudFile: (path: string) => Promise<void>;
  fetchAiKeys: () => Promise<void>;
  saveAiKey: (provider: AiProvider, key: string) => Promise<void>;
  deleteAiKey: (provider: AiProvider) => Promise<void>;
  openAuth: () => void;
  dismissAuth: () => void;
  refreshPlan: () => Promise<void>;
  openUpgrade: () => void;
  dismissUpgrade: () => void;
  openPortal: () => Promise<void>;
}

function describeError(e: unknown): { message: string; unauthorized: boolean; unavailable: boolean; planRequired?: boolean } {
  if (e instanceof BridgeError) {
    return {
      message: e.message,
      unauthorized: e.code === "unauthorized",
      unavailable: e.code === "cloud_unavailable",
      planRequired: e.code === "plan_required",
    };
  }
  if (e instanceof CloudApiError) {
    return {
      message: e.message,
      unauthorized: e.status === 401 || e.status === 403,
      unavailable: false,
    };
  }
  return { message: e instanceof Error ? e.message : String(e), unauthorized: false, unavailable: false };
}

// Action error tail: surface the message and flag session loss on 401-class
// failures. `unavailable` drives the panel's desktop-only hint; a 402
// plan_required (REST or bridge path) raises the paywall overlay.
function applyError(e: unknown): { error: string; unavailable: boolean; authRequired?: boolean; planGate?: boolean } {
  if (e instanceof PlanRequiredError) {
    return { error: "plan_required", unavailable: false, planGate: true };
  }
  const d = describeError(e);
  return {
    error: d.message,
    unavailable: d.unavailable,
    ...(d.unauthorized ? { authRequired: true } : {}),
    ...(d.planRequired ? { planGate: true } : {}),
  };
}

export const useCloudStore = create<CloudState>((set, get) => ({
  user: null,
  token: localStorage.getItem("vplc_token"),
  projectId: localStorage.getItem("vplc_project"),
  projects: [],
  cloudFiles: [],
  aiKeys: [],
  plan: null,
  planGate: false,
  portalBusy: false,

  authRequired: false,
  busy: false,
  loadingProjects: false,
  filesLoading: false,
  openingPath: null,
  error: null,
  unavailable: false,

  register: async (email, password) => {
    set({ busy: true, error: null, unavailable: false });
    try {
      const { token, user } = await apiRegister(email, password);
      localStorage.setItem("vplc_token", token);
      set({ token, user, busy: false, authRequired: false });
      await get().refreshProjects();
      void get().refreshPlan();
    } catch (e) {
      // Keep the dialog open (authRequired untouched) and show the server message.
      set({ busy: false, error: describeError(e).message });
    }
  },

  login: async (email, password) => {
    set({ busy: true, error: null, unavailable: false });
    try {
      const { token, user } = await apiLogin(email, password);
      localStorage.setItem("vplc_token", token);
      set({ token, user, busy: false, authRequired: false });
      await get().refreshProjects();
      void get().refreshPlan();
    } catch (e) {
      // Login failures are credential errors, not session loss: keep the
      // dialog open and show the server message.
      set({ busy: false, error: describeError(e).message });
    }
  },

  logout: () => {
    localStorage.removeItem("vplc_token");
    localStorage.removeItem("vplc_project");
    set({
      user: null,
      token: null,
      projectId: null,
      projects: [],
      cloudFiles: [],
      aiKeys: [],
      plan: null,
      planGate: false,
      portalBusy: false,
      authRequired: false,
      loadingProjects: false,
      filesLoading: false,
      openingPath: null,
      error: null,
      unavailable: false,
    });
  },

  whoami: async () => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true });
      return;
    }
    try {
      const { user } = await apiWhoami(token);
      set({ user });
      void get().refreshPlan();
    } catch (e) {
      const patch = applyError(e);
      // A dead token is a logged-out session, not a transient error.
      set("authRequired" in patch ? { ...patch, user: null, token: null } : patch);
    }
  },

  refreshProjects: async () => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to list cloud projects" });
      return;
    }
    set({ loadingProjects: true, error: null, unavailable: false });
    try {
      const projects = await listProjects(token);
      set({ projects, loadingProjects: false });
    } catch (e) {
      set({ loadingProjects: false, ...applyError(e) });
    }
  },

  createProject: async (name) => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to create cloud projects" });
      return;
    }
    set({ error: null, unavailable: false });
    try {
      const dto = await apiCreateProject(token, name);
      set({ projects: [{ fileCount: 0, ...dto }, ...get().projects] });
    } catch (e) {
      set(applyError(e));
    }
  },

  deleteProject: async (id) => {
    const { token, projectId } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to manage cloud projects" });
      return;
    }
    set({ error: null, unavailable: false });
    try {
      await apiDeleteProject(token, id);
      if (projectId === id) localStorage.removeItem("vplc_project");
      set({
        projects: get().projects.filter((p) => p.id !== id),
        ...(projectId === id ? { projectId: null, cloudFiles: [] } : {}),
      });
    } catch (e) {
      set(applyError(e));
    }
  },

  selectProject: async (id) => {
    localStorage.setItem("vplc_project", id);
    set({ projectId: id, cloudFiles: [], filesLoading: true, error: null, unavailable: false });
    try {
      const files = await invoke<FileEntry[]>("list_project_files");
      set({ cloudFiles: files, filesLoading: false });
      // Surface the cloud file list to the main store, but never clobber an
      // open local folder's file listing.
      if (!useStore.getState().projectDir) useStore.setState({ projectFiles: files });
    } catch (e) {
      set({ filesLoading: false, ...applyError(e) });
    }
  },

  openCloudFile: async (path) => {
    set({ openingPath: path, error: null, unavailable: false });
    try {
      // The main store's openFile swallows bridge errors into its runtime log,
      // so auth/desktop failures must be observed here first; openFile then
      // re-reads through the same bridge to land in the editor tabs.
      await invoke<string>("open_file", { path });
    } catch (e) {
      set({ openingPath: null, ...applyError(e) });
      return;
    }
    await useStore.getState().openFile(path);
    set({ openingPath: null });
  },

  fetchAiKeys: async () => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to manage cloud AI keys" });
      return;
    }
    set({ error: null, unavailable: false });
    try {
      set({ aiKeys: await apiListAiKeys(token) });
    } catch (e) {
      set(applyError(e));
    }
  },

  saveAiKey: async (provider, key) => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to manage cloud AI keys" });
      return;
    }
    set({ error: null, unavailable: false });
    try {
      await apiPutAiKey(token, provider, key);
      // Refetch: the PUT response carries only {provider, masked}, and the
      // list should show the server's updatedAt.
      set({ aiKeys: await apiListAiKeys(token) });
    } catch (e) {
      set(applyError(e));
    }
  },

  deleteAiKey: async (provider) => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true, error: "Sign in to manage cloud AI keys" });
      return;
    }
    set({ error: null, unavailable: false });
    try {
      await apiRemoveAiKey(token, provider);
      set({ aiKeys: await apiListAiKeys(token) });
    } catch (e) {
      set(applyError(e));
    }
  },

  openAuth: () => set({ authRequired: true }),

  dismissAuth: () => set({ authRequired: false }),

  // Best-effort: a failed plan fetch never blocks editing; the paywall UI
  // treats null plan as "unknown" and still renders free-tier defaults.
  refreshPlan: async () => {
    const { token } = get();
    if (!token) return;
    try {
      set({ plan: await getPlan(token) });
    } catch {
      // keep the previous plan (or null) — billing is never a hard failure
    }
  },

  openUpgrade: () => set({ planGate: true }),
  dismissUpgrade: () => set({ planGate: false }),

  openPortal: async () => {
    const { token } = get();
    if (!token) {
      set({ authRequired: true });
      return;
    }
    set({ portalBusy: true, error: null });
    try {
      await openBillingPortal(token);
    } catch (e) {
      set({ error: describeError(e).message });
    } finally {
      set({ portalBusy: false });
    }
  },
}));
