import type { ConnProfile } from "./types";
import type { LocalProfile, ProfileData } from "@mactools/ui";

/** The part of a saved connection that goes to Voltrus Connect; the name travels beside it. */
export function profileToData(p: ConnProfile): ProfileData {
  return { transport: p.transport, tcp: p.tcp, rtu: p.rtu, blocks: p.blocks };
}

export function toLocal(p: ConnProfile): LocalProfile {
  return { key: p.name, name: p.name, data: profileToData(p) };
}

const TCP_DEFAULTS: ConnProfile["tcp"] = { host: "127.0.0.1", port: 502, unitId: 1, pollInterval: 1000 };
const RTU_DEFAULTS: ConnProfile["rtu"] = { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1 };

const asObject = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

/**
 * Profiles arrive from the account, written by other devices or teammates: fill what is missing instead of
 * trusting the shape. Blocks that can not be polled are filtered out when the profile is opened.
 */
export function fromData(name: string, data: ProfileData): ConnProfile {
  return {
    name,
    transport: data.transport === "rtu" ? "rtu" : "tcp",
    tcp: { ...TCP_DEFAULTS, ...asObject(data.tcp) },
    rtu: { ...RTU_DEFAULTS, ...asObject(data.rtu) },
    blocks: Array.isArray(data.blocks) ? (data.blocks.filter((b) => b && typeof b === "object") as ConnProfile["blocks"]) : [],
  };
}

/** Replaces the profile stored under `key` (or adds a new one) and drops any other profile with the same name. */
export function upsertProfile(list: ConnProfile[], key: string | undefined, next: ConnProfile): ConnProfile[] {
  const at = key === undefined ? -1 : list.findIndex((p) => p.name === key);
  const rest = list.filter((p) => p.name !== next.name && p.name !== key);
  if (at < 0) return [...rest, next];
  // Keep the profile where it was in the list.
  const position = list.slice(0, at).filter((p) => p.name !== next.name).length;
  return [...rest.slice(0, position), next, ...rest.slice(position)];
}
