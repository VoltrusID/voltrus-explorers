import type { ConnType } from "./types";

export const FREE_LIVE_ITEMS = 10;
export const FREE_PROFILE_LIMIT = 1;
export const PROFILES_KEY = "cnc-explorer-conn-profiles";

export interface ConnProfile {
  name: string;
  type: ConnType;
  /** MTConnect agent base URL. */
  url: string;
  /** MTConnect device name (empty: every device). */
  device: string;
  /** FANUC host. */
  host: string;
  port: number;
  /** Connect timeout, ms. */
  timeout: number;
}

export function normalizeProfile(raw: unknown): ConnProfile | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.name !== "string" || !r.name) return null;
  return {
    name: r.name,
    type: r.type === "fanuc" ? "fanuc" : "mtconnect",
    url: typeof r.url === "string" ? r.url : "http://localhost:5050",
    device: typeof r.device === "string" ? r.device : "",
    host: typeof r.host === "string" ? r.host : "192.168.1.100",
    port: typeof r.port === "number" ? r.port : 8193,
    timeout: typeof r.timeout === "number" ? r.timeout : 10000,
  };
}

export function loadProfiles(storage: Pick<Storage, "getItem"> = localStorage): ConnProfile[] {
  try {
    const raw = storage.getItem(PROFILES_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.map(normalizeProfile).filter((p): p is ConnProfile => p !== null) : [];
  } catch {
    return [];
  }
}

export function saveProfiles(profiles: ConnProfile[], storage: Pick<Storage, "setItem"> = localStorage) {
  storage.setItem(PROFILES_KEY, JSON.stringify(profiles));
}

/** Free builds keep one saved profile; saving under an existing name replaces it. */
export function needsPro(profiles: ConnProfile[], name: string, pro: boolean): boolean {
  return !pro && !profiles.some((p) => p.name === name) && profiles.length >= FREE_PROFILE_LIMIT;
}
