import type { HostForm } from "./types";

/** Free: Status variables stream live for this many selected variables. Trends need Pro. */
export const FREE_LIVE_SVS = 10;
export const FREE_PROFILE_LIMIT = 1;
export const PROFILES_KEY = "secsgem-explorer-conn-profiles";

export const HOST_DEFAULTS: HostForm = {
  mode: "active",
  host: "127.0.0.1",
  port: 5000,
  deviceId: 0,
  t3: 45,
  t5: 10,
  t6: 5,
  t7: 10,
  t8: 5,
  controlSvid: 2,
};

export interface ConnProfile extends HostForm {
  name: string;
}

const num = (v: unknown, d: number, min = 0, max = Number.MAX_SAFE_INTEGER) =>
  typeof v === "number" && Number.isFinite(v) && v >= min && v <= max ? v : d;

export function normalizeProfile(raw: unknown): ConnProfile | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.name !== "string" || !r.name) return null;
  const d = HOST_DEFAULTS;
  return {
    name: r.name,
    mode: r.mode === "passive" ? "passive" : "active",
    host: typeof r.host === "string" ? r.host : d.host,
    port: num(r.port, d.port, 1, 65535),
    deviceId: num(r.deviceId, d.deviceId, 0, 65535),
    t3: num(r.t3, d.t3, 1),
    t5: num(r.t5, d.t5, 1),
    t6: num(r.t6, d.t6, 1),
    t7: num(r.t7, d.t7, 1),
    t8: num(r.t8, d.t8, 1),
    controlSvid: num(r.controlSvid, d.controlSvid),
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

/** Whether `count` selected status variables may all update live in this tier. */
export function liveSelectionAllowed(count: number, pro: boolean): boolean {
  return pro || count <= FREE_LIVE_SVS;
}
