import type { LocalProfile, ProfileData } from "@mactools/ui";

/** A saved BACnet target: the device to connect to and the objects to watch on it. */
export interface BacnetProfileData {
  device: { ip: string; port: number; device_id: number };
  watch: { type: string; instance: number; name: string; source: "poll" | "cov" }[];
}

const STORAGE_KEY = "bacnet-explorer-profiles";

const asObject = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

/** Profiles also arrive from other devices and teammates: keep only well-formed fields. */
export function normalizeData(data: ProfileData): BacnetProfileData {
  const d = asObject(data.device);
  const watch = Array.isArray(data.watch) ? data.watch : [];
  return {
    device: {
      ip: typeof d.ip === "string" ? d.ip : "",
      port: Number.isInteger(d.port) ? (d.port as number) : 47808,
      device_id: Number.isInteger(d.device_id) ? (d.device_id as number) : 0,
    },
    watch: watch
      .map(asObject)
      .filter((w) => typeof w.type === "string" && Number.isInteger(w.instance))
      .map((w) => ({
        type: w.type as string,
        instance: w.instance as number,
        name: typeof w.name === "string" ? w.name : `${w.type}:${w.instance}`,
        source: w.source === "cov" ? ("cov" as const) : ("poll" as const),
      })),
  };
}

export function loadProfiles(): LocalProfile[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((p) => p && typeof p.name === "string") : [];
  } catch {
    return [];
  }
}

const listeners = new Set<() => void>();

export function saveProfiles(profiles: LocalProfile[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles));
  listeners.forEach((l) => l());
}

export function onProfilesChanged(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Names are the keys, as in Modbus Poll: unique per device. */
export function upsertLocal(list: LocalProfile[], key: string | undefined, name: string, data: ProfileData): LocalProfile[] {
  const entry: LocalProfile = { key: name, name, data: { ...normalizeData(data) } };
  const at = key === undefined ? -1 : list.findIndex((p) => p.key === key);
  const rest = list.filter((p) => p.key !== name && p.key !== key);
  if (at < 0) return [...rest, entry];
  const position = list.slice(0, at).filter((p) => p.key !== name).length;
  return [...rest.slice(0, position), entry, ...rest.slice(position)];
}
