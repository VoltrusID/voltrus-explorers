import type { FanucPoll } from "./types";

export function fmt(v: number | null | undefined, digits = 3): string {
  return v == null || !Number.isFinite(v) ? "\u2014" : v.toFixed(digits);
}

/** Alarm type from the Rust enum: a snake_case name or `{ unknown: n }`. */
export function alarmTypeName(t: unknown): string {
  if (typeof t === "string") return t.replace(/_/g, " ");
  if (t && typeof t === "object") {
    const [k, v] = Object.entries(t as Record<string, unknown>)[0] ?? [];
    return k ? `${k} ${String(v)}` : "?";
  }
  return "?";
}

export function pmcHex(value: number, bytes: number): string {
  return "0x" + value.toString(16).toUpperCase().padStart(bytes * 2, "0");
}

export interface TrendRow {
  name: string;
  value: number;
}

/** What the FANUC trend can plot: feed, spindle, then servo load and absolute position per axis. */
export function fanucTrendRows(p: FanucPoll | null): TrendRow[] {
  if (!p) return [];
  const rows: TrendRow[] = [];
  const add = (name: string, value: number | null) => {
    if (value != null && Number.isFinite(value)) rows.push({ name, value });
  };
  add("Feed rate", p.feed_rate);
  add("Spindle speed", p.spindle_speed);
  for (const a of p.axes) add(`Servo load ${a.name}`, a.servo_load);
  for (const a of p.axes) add(`Absolute ${a.name}`, a.absolute);
  return rows;
}
