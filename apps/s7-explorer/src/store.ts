import { create } from "zustand";
import type { LogEntry } from "@mactools/ui";
import type { Device, MainTab, PhoneTab, TagConfig } from "./types";
import { loadDevices, newDevice, saveDevices } from "./helpers";

interface SimState {
  running: boolean;
  template: string | null;
  port: number | null; // bound port reported by backend
}

/** What the phone header and tab bar show about the active device. */
export interface LiveState {
  connected: boolean;
  latency: number;
  pollCount: number;
  error: string | null;
  trendCount: number;
}

interface AppState {
  // Devices
  devices: Device[];
  activeDeviceId: string;
  setDevices: (d: Device[] | ((prev: Device[]) => Device[])) => void;
  addDevice: () => string; // returns new device id
  removeDevice: (id: string) => void;
  renameDevice: (id: string, label: string) => void;
  setActiveDevice: (id: string) => void;
  updateDevice: (id: string, patch: Partial<Device>) => void;
  setDeviceTags: (id: string, tags: TagConfig[] | ((prev: TagConfig[]) => TagConfig[])) => void;

  // Layout
  mainTab: MainTab;
  setMainTab: (t: MainTab) => void;
  /** The phone shell's current tab. */
  phoneTab: PhoneTab;
  setPhoneTab: (t: PhoneTab) => void;
  /** Connection state of the active device, for the phone header and tab bar (the device panels own the real state). */
  live: LiveState;
  setLive: (s: LiveState) => void;

  // Log (global, across all devices)
  log: LogEntry[];
  addLog: (direction: LogEntry["direction"], message: string) => void;
  clearLog: () => void;

  // Sim
  sim: SimState;
  setSim: (s: Partial<SimState>) => void;
}

export const useStore = create<AppState>((set, get) => {
  const persist = (next: Device[]) => saveDevices(next);

  // Load once: a second loadDevices() call would mint fresh UUIDs for the
  // default device set, leaving activeDeviceId pointing at a nonexistent id.
  const initialDevices = loadDevices();

  return {
    devices: initialDevices,
    activeDeviceId: initialDevices[0]?.id ?? "",

    setDevices: (dOrFn) => {
      const next = typeof dOrFn === "function" ? dOrFn(get().devices) : dOrFn;
      set({ devices: next });
      persist(next);
    },

    addDevice: () => {
      const dev = newDevice();
      const next = [...get().devices, dev];
      set({ devices: next, activeDeviceId: dev.id });
      persist(next);
      return dev.id;
    },

    removeDevice: (id) => {
      const prev = get().devices;
      if (prev.length <= 1) return; // keep at least one tab
      const next = prev.filter((d) => d.id !== id);
      const nextActive = get().activeDeviceId === id ? next[0].id : get().activeDeviceId;
      set({ devices: next, activeDeviceId: nextActive });
      persist(next);
    },

    renameDevice: (id, label) => {
      const next = get().devices.map((d) => (d.id === id ? { ...d, label } : d));
      set({ devices: next });
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

    // Layout
    mainTab: "tags",
    setMainTab: (t) => set({ mainTab: t }),
    phoneTab: "connect",
    setPhoneTab: (t) => set({ phoneTab: t }),
    live: { connected: false, latency: 0, pollCount: 0, error: null, trendCount: 0 },
    setLive: (live) => set({ live }),

    // Log
    log: [],
    addLog: (direction, message) =>
      set((s) => ({
        log: [...s.log, { timestamp: Date.now(), direction, message }].slice(-1000),
      })),
    clearLog: () => set({ log: [] }),

    // Sim
    sim: { running: false, template: null, port: null },
    setSim: (s) => set((state) => ({ sim: { ...state.sim, ...s } })),
  };
});
