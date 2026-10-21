/** Saved port settings and send macros, kept in localStorage. */

export interface PortProfile {
  name: string;
  port_name: string;
  baud_rate: number;
  data_bits: number;
  stop_bits: number;
  parity: string;
  timeout_ms: number;
}

export interface SendMacro {
  name: string;
  data: string;
  format: "hex" | "ascii";
}

/** Saved profiles the free tier may keep; more needs Pro. */
export const FREE_PROFILE_LIMIT = 1;
/** Log lines kept for free users; Pro keeps everything. */
export const FREE_LOG_LINES = 200;

const PROFILES_KEY = "serial.profiles";
const MACROS_KEY = "serial.macros";

function load<T>(key: string): T[] {
  try {
    const raw = localStorage.getItem(key);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? (parsed as T[]) : [];
  } catch {
    return [];
  }
}

function persist<T>(key: string, list: T[]): T[] {
  try {
    localStorage.setItem(key, JSON.stringify(list));
  } catch {
    // storage unavailable: keep the in-memory list only
  }
  return list;
}

export const loadProfiles = () => load<PortProfile>(PROFILES_KEY);
export const loadMacros = () => load<SendMacro>(MACROS_KEY);

export function upsertByName<T extends { name: string }>(list: T[], item: T): T[] {
  return [...list.filter((x) => x.name !== item.name), item];
}

export const saveProfiles = (list: PortProfile[]) => persist(PROFILES_KEY, list);
export const saveMacros = (list: SendMacro[]) => persist(MACROS_KEY, list);
