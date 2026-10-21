import type { ConnProfile } from "./types";

export const typeStep = (t: string) =>
  t === "float64" ? 4 : t === "float32" || t === "int32" ? 2 : 1;

export const CONN_PROFILES_KEY = "modbus-poll-conn-profiles";

export function loadConnProfiles(): ConnProfile[] {
  try {
    const raw = localStorage.getItem(CONN_PROFILES_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveConnProfiles(profiles: ConnProfile[]) {
  localStorage.setItem(CONN_PROFILES_KEY, JSON.stringify(profiles));
}
