import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface TrafficEntry {
  timestamp: number;
  direction: string;
  function_code: number;
  address: number;
  data: string;
  length: number;
}

export interface TcpEvent {
  timestamp: number;
  event_type: string;
  detail: string;
}

const MAX_ENTRIES = 500;

interface TrafficState {
  entries: TrafficEntry[];
  tcpEvents: TcpEvent[];
  clear: () => void;
}

/** Frame and connection history. Lives for the whole session, whether or not the Traffic panel is shown. */
export const useTrafficStore = create<TrafficState>((set) => ({
  entries: [],
  tcpEvents: [],
  clear: () => set({ entries: [], tcpEvents: [] }),
}));

const capped = <T>(list: T[], item: T): T[] => {
  const next = [...list, item];
  return next.length > MAX_ENTRIES ? next.slice(-MAX_ENTRIES) : next;
};

/** Start recording `traffic-log` / `tcp-event` into the store. Call once at app start; returns the stop function. */
export function startTrafficFeed(): () => void {
  let stopped = false;
  const unlisteners: UnlistenFn[] = [];
  const track = (pending: Promise<UnlistenFn>) => {
    pending.then((unlisten) => {
      if (stopped) unlisten();
      else unlisteners.push(unlisten);
    });
  };
  track(listen<TrafficEntry>("traffic-log", (event) => {
    useTrafficStore.setState((s) => ({ entries: capped(s.entries, event.payload) }));
  }));
  track(listen<TcpEvent>("tcp-event", (event) => {
    useTrafficStore.setState((s) => ({ tcpEvents: capped(s.tcpEvents, event.payload) }));
  }));
  return () => {
    stopped = true;
    unlisteners.forEach((unlisten) => unlisten());
  };
}
