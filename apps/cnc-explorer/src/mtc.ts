import type { Component, DataItem, Devices, ObsValue, Observation } from "./types";

export interface Item {
  id: string;
  name: string;
  item: DataItem;
  /** Component path, e.g. `Axes/Linear/X`. */
  path: string;
}

/** Every data item of every non-agent device in document order. Duplicate ids are kept once. */
export function flattenItems(devices: Devices): Item[] {
  const out: Item[] = [];
  const seen = new Set<string>();
  const add = (item: DataItem, path: string) => {
    if (seen.has(item.id)) return;
    seen.add(item.id);
    out.push({ id: item.id, name: item.name ?? item.id, item, path });
  };
  const walk = (c: Component, prefix: string) => {
    const path = prefix ? `${prefix}/${c.name ?? c.id}` : c.name ?? c.id;
    c.dataItems.forEach((d) => add(d, path));
    c.components.forEach((child) => walk(child, path));
  };
  for (const d of devices.devices) {
    d.dataItems.forEach((i) => add(i, d.name));
    d.components.forEach((c) => walk(c, d.name));
  }
  return out;
}

export function formatValue(v: ObsValue): string {
  switch (v.kind) {
    case "Unavailable":
      return "UNAVAILABLE";
    case "Number":
      return String(v.value);
    case "Text":
      return v.value;
    case "TimeSeries":
      return v.value.join(" ");
    case "DataSet":
      return v.value.filter((e) => !e.removed).map((e) => `${e.key}=${e.value}`).join(" ");
  }
}

export function numericValue(o: Observation | undefined): number | null {
  return o && o.category === "SAMPLE" && o.value.kind === "Number" ? o.value.value : null;
}

/**
 * Folds observations into the latest-by-data-item map. `replace` (snapshot, resync, refresh) accepts every
 * item; otherwise only items for which `live` holds are updated (the free tier's live item limit).
 */
export function applyObservations(
  prev: Map<string, Observation>,
  incoming: Observation[],
  live: (id: string) => boolean,
  replace: boolean,
): Map<string, Observation> {
  const next = replace && incoming.length > 0 ? new Map<string, Observation>() : new Map(prev);
  for (const o of incoming) {
    if (!replace && !live(o.dataItemId)) continue;
    const old = next.get(o.dataItemId);
    // Conditions: one observation per data item in the table; keep the newest.
    if (old && old.sequence > o.sequence) continue;
    next.set(o.dataItemId, o);
  }
  return next;
}

/** Ids that update live in the free tier: the first `limit` non-condition items. */
export function liveIdSet(items: Item[], limit: number): Set<string> {
  const ids = new Set<string>();
  for (const i of items) {
    if (i.item.category === "CONDITION") continue;
    if (ids.size >= limit) break;
    ids.add(i.id);
  }
  return ids;
}

/** Trend labels: the item name, or `name [id]` when two numeric items share a name. */
export function trendLabels(items: Item[]): Map<string, string> {
  const counts = new Map<string, number>();
  items.forEach((i) => counts.set(i.name, (counts.get(i.name) ?? 0) + 1));
  return new Map(items.map((i) => [i.id, (counts.get(i.name) ?? 0) > 1 ? `${i.name} [${i.id}]` : i.name]));
}

const LEVEL_ORDER = { Fault: 0, Warning: 1, Unavailable: 2, Normal: 3 } as const;

export function sortConditions(rows: Observation[]): Observation[] {
  return [...rows].sort(
    (a, b) =>
      LEVEL_ORDER[a.condition?.level ?? "Unavailable"] - LEVEL_ORDER[b.condition?.level ?? "Unavailable"] ||
      a.dataItemId.localeCompare(b.dataItemId),
  );
}

export function csvCell(v: unknown): string {
  const s = String(v ?? "");
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(header: string[], rows: unknown[][]): string {
  return [header, ...rows].map((r) => r.map(csvCell).join(",")).join("\n") + "\n";
}

/** Indents an XML string for display. Returns the input unchanged when it has no tags. */
export function prettyXml(xml: string): string {
  const tokens = xml.replace(/>\s+</g, "><").trim().match(/<[^>]+>|[^<]+/g);
  if (!tokens) return xml;
  const out: string[] = [];
  let depth = 0;
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    const isOpen = /^<[^/!?]/.test(t) && !t.endsWith("/>");
    // `<a>text</a>` stays on one line.
    if (isOpen && i + 2 < tokens.length && !tokens[i + 1].startsWith("<") && tokens[i + 2].startsWith("</")) {
      out.push("  ".repeat(depth) + t + tokens[i + 1].trim() + tokens[i + 2]);
      i += 2;
      continue;
    }
    if (t.startsWith("</")) depth = Math.max(0, depth - 1);
    out.push("  ".repeat(depth) + t.trim());
    if (isOpen) depth++;
  }
  return out.join("\n");
}
