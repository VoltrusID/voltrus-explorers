import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface TrafficEntry {
  /** Id of the connection the frame belongs to. */
  conn: string;
  timestamp: number;
  direction: string;
  function_code: number;
  address: number;
  data: string;
  length: number;
}

export interface TcpEvent {
  conn: string;
  timestamp: number;
  event_type: string;
  detail: string;
}

const MAX_ENTRIES = 500;

/** Frames and connection events of one connection. */
export interface ConnTraffic {
  entries: TrafficEntry[];
  tcpEvents: TcpEvent[];
}

export const NO_TRAFFIC: ConnTraffic = { entries: [], tcpEvents: [] };

interface TrafficState {
  byConn: Record<string, ConnTraffic>;
  /** Empty one connection's history. */
  clear: (conn: string) => void;
}

/** Frame and connection history per connection. Lives for the whole session, whether or not the Traffic panel is shown. */
export const useTrafficStore = create<TrafficState>((set) => ({
  byConn: {},
  clear: (conn) => set((s) => ({ byConn: { ...s.byConn, [conn]: NO_TRAFFIC } })),
}));

/** Forget a closed connection's history. */
export function dropTraffic(conn: string) {
  useTrafficStore.setState((s) => {
    const byConn = { ...s.byConn };
    delete byConn[conn];
    return { byConn };
  });
}

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
    const conn = event.payload.conn;
    useTrafficStore.setState((s) => {
      const t = s.byConn[conn] ?? NO_TRAFFIC;
      return { byConn: { ...s.byConn, [conn]: { ...t, entries: capped(t.entries, event.payload) } } };
    });
  }));
  track(listen<TcpEvent>("tcp-event", (event) => {
    const conn = event.payload.conn;
    useTrafficStore.setState((s) => {
      const t = s.byConn[conn] ?? NO_TRAFFIC;
      return { byConn: { ...s.byConn, [conn]: { ...t, tcpEvents: capped(t.tcpEvents, event.payload) } } };
    });
  }));
  return () => {
    stopped = true;
    unlisteners.forEach((unlisten) => unlisten());
  };
}
