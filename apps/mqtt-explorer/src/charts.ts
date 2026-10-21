// Numeric values found in message payloads, kept as time series for the charts.

import type { Decoded } from "./payload";

export interface NumericSample {
  /** "" for a bare number payload, otherwise a JSON path (`a.b[2]`) or Sparkplug metric name. */
  path: string;
  value: number;
}

const MAX_LEAVES = 64;
const MAX_DEPTH = 5;
const MAX_ARRAY = 32;
const PLAIN_NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;

function flatten(v: unknown, path: string, depth: number, out: NumericSample[]): void {
  if (out.length >= MAX_LEAVES) return;
  if (typeof v === "number") {
    if (Number.isFinite(v)) out.push({ path, value: v });
  } else if (typeof v === "boolean") {
    out.push({ path, value: v ? 1 : 0 });
  } else if (depth < MAX_DEPTH && Array.isArray(v)) {
    v.slice(0, MAX_ARRAY).forEach((x, i) => flatten(x, `${path}[${i}]`, depth + 1, out));
  } else if (depth < MAX_DEPTH && v !== null && typeof v === "object") {
    for (const [k, x] of Object.entries(v)) flatten(x, path ? `${path}.${k}` : k, depth + 1, out);
  }
}

interface SparkplugMetric {
  name?: string;
  alias?: number | string;
  value?: unknown;
  is_null?: boolean;
}

function sparkplugMetrics(metrics: SparkplugMetric[], prefix: string, depth: number, out: NumericSample[]): void {
  for (const m of metrics) {
    if (out.length >= MAX_LEAVES) return;
    if (m.is_null) continue;
    const label = m.name ?? (m.alias !== undefined ? `alias ${m.alias}` : undefined);
    if (label === undefined) continue;
    const name = prefix ? `${prefix}/${label}` : label;
    const v = m.value;
    if (typeof v === "number" && Number.isFinite(v)) out.push({ path: name, value: v });
    else if (typeof v === "boolean") out.push({ path: name, value: v ? 1 : 0 });
    else if (depth < MAX_DEPTH && v && typeof v === "object" && "metrics" in v && Array.isArray(v.metrics)) {
      // Template value: its metrics have the same shape as top-level ones.
      const inner = v.metrics as SparkplugMetric[];
      sparkplugMetrics(inner, name, depth + 1, out);
    }
  }
}

/**
 * Numbers a message carries: Sparkplug metrics, numeric leaves of a decoded
 * protobuf message, a bare number payload, or numeric leaves of a JSON payload.
 */
export function extractNumeric(text: string, decoded?: Decoded): NumericSample[] {
  const out: NumericSample[] = [];
  if (decoded) {
    if (decoded.value === undefined) return out;
    if (decoded.kind === "sparkplug") {
      // The backend builds this tree in sparkplug.rs, so the shape is known.
      const tree = (decoded.value ?? {}) as { metrics?: SparkplugMetric[] };
      sparkplugMetrics(tree.metrics ?? [], "", 0, out);
    } else {
      flatten(decoded.value, "", 0, out);
    }
    return out;
  }
  const t = text.trim();
  if (t === "") return out;
  if (PLAIN_NUMBER.test(t)) {
    const n = Number(t);
    if (Number.isFinite(n)) out.push({ path: "", value: n });
    return out;
  }
  const c = t[0];
  if (c === "{" || c === "[") {
    try {
      flatten(JSON.parse(t), "", 0, out);
    } catch {
      /* not JSON */
    }
  }
  return out;
}

export interface Point {
  t: number;
  v: number;
}

export interface Series {
  key: string;
  conn: string;
  topic: string;
  path: string;
  points: Point[];
  last: number;
}

export const MAX_POINTS = 2000;
export const MAX_SERIES = 300;

export function seriesKey(conn: string, topic: string, path: string): string {
  return `${conn}\u0001${topic}\u0001${path}`;
}

/** `topic` or `topic · path`. */
export function seriesLabel(s: Pick<Series, "topic" | "path">): string {
  return s.path ? `${s.topic} · ${s.path}` : s.topic;
}

/**
 * Time series per connection, topic and value path. Mutable on purpose: it
 * is fed once per received message, so it lives outside React state and the
 * chart view polls `version`.
 */
export class SeriesStore {
  private readonly byKey = new Map<string, Series>();
  /** Bumped whenever any point is added or series are removed. */
  version = 0;

  ingest(conn: string, topic: string, text: string, decoded: Decoded | undefined, ts: number): void {
    for (const { path, value } of extractNumeric(text, decoded)) {
      const key = seriesKey(conn, topic, path);
      let s = this.byKey.get(key);
      if (!s) {
        // Past the cap new series are ignored rather than evicting ones the user may be charting.
        if (this.byKey.size >= MAX_SERIES) continue;
        s = { key, conn, topic, path, points: [], last: value };
        this.byKey.set(key, s);
      }
      s.points.push({ t: ts, v: value });
      s.last = value;
      // Trim in batches so a busy topic does not shift the array on every message.
      if (s.points.length > MAX_POINTS + 200) s.points.splice(0, s.points.length - MAX_POINTS);
      this.version++;
    }
  }

  get(key: string): Series | undefined {
    return this.byKey.get(key);
  }

  list(conn: string): Series[] {
    const out: Series[] = [];
    for (const s of this.byKey.values()) if (s.conn === conn) out.push(s);
    return out.sort((a, b) => seriesLabel(a).localeCompare(seriesLabel(b), undefined, { numeric: true }));
  }

  /** Drop the points of one connection (or all) but keep nothing else around. */
  clear(conn?: string): void {
    for (const [k, s] of this.byKey) if (conn === undefined || s.conn === conn) this.byKey.delete(k);
    this.version++;
  }
}
