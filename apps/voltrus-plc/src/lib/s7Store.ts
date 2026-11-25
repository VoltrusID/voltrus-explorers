// The connection to the Siemens CPU, shared by the Online & diagnostics tab, watch and tag tables,
// the device tree, the title bar and the Online side panel.
import { create } from "zustand";
import { bridgeMode } from "../api/bridge";
import { loadTarget, s7, saveTarget, type BlockEntry, type OnlineInfo, type S7Target } from "./s7";

const STATUS_POLL_MS = 2000;

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

interface S7State {
  target: S7Target;
  /** Set while online. */
  info: OnlineInfo | null;
  mode: string | null;
  busy: boolean;
  error: string | null;
  /** Blocks in the CPU, read when going online (null if not read or refused). */
  blocks: BlockEntry[] | null;
  blocksError: string | null;
  /** Watch and tag tables that are monitoring, by path; kept across tab switches. */
  monitoring: Record<string, boolean>;
  confirm: null | "run" | "stop";
  scanOpen: boolean;

  setTarget: (t: S7Target) => void;
  goOnline: (t?: S7Target) => Promise<boolean>;
  goOffline: () => Promise<void>;
  requestMode: (action: "run" | "stop") => void;
  cancelMode: () => void;
  setMode: (run: boolean, cold: boolean) => Promise<void>;
  readBlocks: () => Promise<void>;
  setMonitoring: (path: string, on: boolean) => void;
  setScanOpen: (open: boolean) => void;
}

let pollId: number | undefined;

function stopPolling() {
  if (pollId !== undefined) window.clearInterval(pollId);
  pollId = undefined;
}

function startPolling() {
  stopPolling();
  pollId = window.setInterval(async () => {
    try {
      const st = await s7.status();
      if (!st.online) {
        stopPolling();
        useS7.setState({
          info: null,
          mode: null,
          blocks: null,
          monitoring: {},
          error: st.error ?? "Connection to the CPU was lost",
        });
      } else if (st.mode) {
        useS7.setState({ mode: st.mode });
      }
    } catch { /* next poll */ }
  }, STATUS_POLL_MS);
}

export const useS7 = create<S7State>((set, get) => ({
  target: loadTarget(),
  info: null,
  mode: null,
  busy: false,
  error: null,
  blocks: null,
  blocksError: null,
  monitoring: {},
  confirm: null,
  scanOpen: false,

  setTarget: (t) => set({ target: t }),

  goOnline: async (t) => {
    const target = t ?? get().target;
    set({ busy: true, error: null, target });
    try {
      const info = await s7.goOnline(target);
      saveTarget(target);
      set({ info, mode: info.mode, busy: false });
      startPolling();
      void get().readBlocks();
      return true;
    } catch (e) {
      set({ busy: false, error: errorText(e) });
      return false;
    }
  },

  goOffline: async () => {
    stopPolling();
    await s7.goOffline().catch(() => {});
    set({ info: null, mode: null, blocks: null, blocksError: null, monitoring: {}, error: null });
  },

  requestMode: (action) => set({ confirm: action }),
  cancelMode: () => set({ confirm: null }),

  setMode: async (run, cold) => {
    set({ confirm: null, busy: true, error: null });
    try {
      const mode = await s7.setMode(run, cold);
      set({ mode, busy: false });
    } catch (e) {
      set({ busy: false, error: errorText(e) });
    }
  },

  readBlocks: async () => {
    try {
      set({ blocks: await s7.listBlocks(), blocksError: null });
    } catch (e) {
      set({ blocks: null, blocksError: errorText(e) });
    }
  },

  setMonitoring: (path, on) => set((s) => ({ monitoring: { ...s.monitoring, [path]: on } })),
  setScanOpen: (open) => set({ scanOpen: open }),
}));

/** Pick up a connection the backend still holds (after a reload of the window). */
export async function initS7() {
  if (bridgeMode !== "tauri") return;
  try {
    const st = await s7.status();
    if (st.online && st.target) {
      useS7.setState({
        target: st.target,
        info: { target: st.target, pdu_size: 0, cpu: null, cpu_error: null, mode: st.mode },
        mode: st.mode,
      });
      startPolling();
      void useS7.getState().readBlocks();
    }
  } catch { /* offline */ }
}

/** A short name for the CPU: its station name when online, else the address. */
export function deviceLabel(info: OnlineInfo | null, target: S7Target): string {
  return info?.cpu?.as_name || info?.cpu?.module_name || (target.host ? "PLC_1" : "No device");
}
