import { create } from "zustand";
import type { LogEntry } from "@mactools/ui";
import type { Device, Protocol, TagConfig } from "./types";
import { loadDevices, newDevice, saveDevices } from "./helpers";

interface SimState {
  running: boolean;
  mcPort: number | null;
  finsPort: number | null;
  finsNode: number | null;
}

interface AppState {
  devices: Device[];
  activeDeviceId: string;
  addDevice: (protocol?: Protocol) => string;
  removeDevice: (id: string) => void;
  setActiveDevice: (id: string) => void;
  updateDevice: (id: string, patch: Partial<Device>) => void;
  setDeviceTags: (id: string, tags: TagConfig[] | ((prev: TagConfig[]) => TagConfig[])) => void;

  /** Global comm log across devices. */
  log: LogEntry[];
  addLog: (direction: LogEntry["direction"], message: string) => void;
  /** Several entries in one update (frames arrive in bursts; one render per burst). */
  addLogs: (entries: LogEntry[]) => void;
  clearLog: () => void;
  /** Show every frame (hex and decoded) in the comm log. */
  showFrames: boolean;
  setShowFrames: (on: boolean) => void;

  sim: SimState;
  setSim: (s: Partial<SimState>) => void;
}

const FRAMES_KEY = "mc-fins-explorer-frames";

export const useStore = create<AppState>((set, get) => {
  const persist = (next: Device[]) => saveDevices(next);
  const initialDevices = loadDevices();

  return {
    devices: initialDevices,
    activeDeviceId: initialDevices[0]?.id ?? "",

    addDevice: (protocol = "mc") => {
      const dev = newDevice(protocol, get().devices.length + 1);
      const next = [...get().devices, dev];
      set({ devices: next, activeDeviceId: dev.id });
      persist(next);
      return dev.id;
    },

    removeDevice: (id) => {
      const prev = get().devices;
      if (prev.length <= 1) return;
      const next = prev.filter((d) => d.id !== id);
      const nextActive = get().activeDeviceId === id ? next[0].id : get().activeDeviceId;
      set({ devices: next, activeDeviceId: nextActive });
      persist(next);
    },

    setActiveDevice: (id) => set({ activeDeviceId: id }),

    updateDevice: (id, patch) => {
      const next = get().devices.map((d) => (d.id === id ? { ...d, ...patch } : d));
      set({ devices: next });
      persist(next);
    },

    setDeviceTags: (id, tagsOrFn) => {
      const next = get().devices.map((d) => {
        if (d.id !== id) return d;
        const tags = typeof tagsOrFn === "function" ? tagsOrFn(d.tags) : tagsOrFn;
        return { ...d, tags };
      });
      set({ devices: next });
      persist(next);
    },

    log: [],
    addLog: (direction, message) =>
      set((s) => ({ log: [...s.log, { timestamp: Date.now(), direction, message }].slice(-1000) })),
    addLogs: (entries) => {
      if (entries.length > 0) set((s) => ({ log: [...s.log, ...entries].slice(-1000) }));
    },
    clearLog: () => set({ log: [] }),
    showFrames: localStorage.getItem(FRAMES_KEY) !== "0",
    setShowFrames: (on) => {
      localStorage.setItem(FRAMES_KEY, on ? "1" : "0");
      set({ showFrames: on });
    },

    sim: { running: false, mcPort: null, finsPort: null, finsNode: null },
    setSim: (s) => set((state) => ({ sim: { ...state.sim, ...s } })),
  };
});
