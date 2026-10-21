// Saved MQTT connection profiles, persisted in localStorage.
// Passwords are stored locally (this is a desktop tool); do not enable this
// flow for shared hosts without a secrets backend.

export interface ConnProfile {
  name: string;
  host: string;
  port: number;
  clientId: string;
  username?: string;
  password?: string;
  tls: boolean;
}

const KEY = "mqtt-explorer:profiles:v1";

export function loadProfiles(): ConnProfile[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter(
      (p): p is ConnProfile =>
        !!p && typeof p.name === "string" && typeof p.host === "string",
    );
  } catch {
    return [];
  }
}

function persist(list: ConnProfile[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    // Ignore quota / private-mode errors — profiles are a convenience.
  }
}

export function upsertProfile(p: ConnProfile): ConnProfile[] {
  const list = loadProfiles().filter((x) => x.name !== p.name);
  list.push(p);
  persist(list);
  return list;
}

export function deleteProfile(name: string): ConnProfile[] {
  const list = loadProfiles().filter((x) => x.name !== name);
  persist(list);
  return list;
}

/** Saved profiles the free tier may keep; more needs Pro. */
export const FREE_PROFILE_LIMIT = 1;
