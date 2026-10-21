import { VIRTUAL_PORT } from "./constants";
import type { AppState } from "./store";

/**
 * Everything that belongs to one master connection. The active connection's copy lives at the
 * root of the store (so every panel keeps reading `useStore()` as before); the others are parked
 * in `sessions[].fields` and swapped in by `switchSession`.
 */
export const SESSION_KEYS = [
  "blocks", "activeBlockTab",
  "log", "trendSeries",
  "transport", "tcpFormValues",
  "rtuPort", "rtuPortManual", "rtuBaud", "rtuDataBits", "rtuStopBits", "rtuParity", "rtuUnitId", "rtuTimeout", "rtuPollInterval",
  "endianness",
  "busScanning", "busScanResults", "showBusScanner",
  "activeConnProfile", "profileDirty",
  "newName", "newAddr", "newQty", "newType", "newArea", "editingBlock",
  "editingTag",
] as const;

export type SessionKey = (typeof SESSION_KEYS)[number];
export type SessionFields = Pick<AppState, SessionKey>;

export interface Session {
  id: string;
  /** The connection's state while another one is active; null for the active one. */
  fields: SessionFields | null;
}

export function pickSessionFields(state: SessionFields): SessionFields {
  const out = {} as Record<string, unknown>;
  for (const key of SESSION_KEYS) out[key] = state[key];
  return out as SessionFields;
}

/** A connection with nothing configured yet: local TCP defaults and no read blocks. */
export function newSessionFields(): SessionFields {
  return {
    blocks: [], activeBlockTab: 0,
    log: [], trendSeries: new Map(),
    transport: "tcp",
    tcpFormValues: { host: "127.0.0.1", port: 502, unitId: 1, pollInterval: 1000 },
    rtuPort: "", rtuPortManual: false, rtuBaud: 9600, rtuDataBits: 8, rtuStopBits: 1, rtuParity: "none",
    rtuUnitId: 1, rtuTimeout: 5000, rtuPollInterval: 1000,
    endianness: "big",
    busScanning: false, busScanResults: [], showBusScanner: false,
    activeConnProfile: null, profileDirty: false,
    newName: "", newAddr: "", newQty: "", newType: "uint16", newArea: "holding", editingBlock: null,
    editingTag: null,
  };
}

/** What a tab shows for a connection: the saved profile's name, else where it points. */
export function sessionLabel(f: Pick<SessionFields, "transport" | "tcpFormValues" | "rtuPort" | "activeConnProfile">): string {
  if (f.activeConnProfile) return f.activeConnProfile;
  if (f.transport === "rtu") {
    if (f.rtuPort === VIRTUAL_PORT) return "Virtual RTU";
    return f.rtuPort.split("/").pop() || "RTU";
  }
  return `${f.tcpFormValues.host}:${f.tcpFormValues.port}`;
}

/** Fields of connection `id`, from the store root when it is the active one. */
export function fieldsOf(state: Pick<AppState, "sessions" | "activeSessionId"> & SessionFields, id: string): SessionFields | null {
  if (id === state.activeSessionId) return pickSessionFields(state);
  return state.sessions.find((s) => s.id === id)?.fields ?? null;
}
