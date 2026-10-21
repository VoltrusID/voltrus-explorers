// The user's protobuf schema and topic rules, kept in localStorage.

export interface ProtoRule {
  filter: string;
  message: string;
}

export interface ProtoSettings {
  source: string;
  rules: ProtoRule[];
}

const KEY = "mqtt-explorer:proto:v1";

export const EMPTY_PROTO: ProtoSettings = { source: "", rules: [] };

export function loadProto(): ProtoSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return EMPTY_PROTO;
    const v: unknown = JSON.parse(raw);
    if (!v || typeof v !== "object" || !("source" in v) || typeof v.source !== "string") return EMPTY_PROTO;
    const rules =
      "rules" in v && Array.isArray(v.rules)
        ? v.rules.filter(
            (r: unknown): r is ProtoRule =>
              !!r &&
              typeof r === "object" &&
              "filter" in r &&
              "message" in r &&
              typeof r.filter === "string" &&
              typeof r.message === "string",
          )
        : [];
    return { source: v.source, rules };
  } catch {
    return EMPTY_PROTO;
  }
}

export function saveProto(p: ProtoSettings): void {
  try {
    if (p.source.trim() === "") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // Quota or private mode: the schema just will not persist.
  }
}
