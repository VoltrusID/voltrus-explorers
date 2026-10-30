import type { FanucPoll, MachineState } from "./types";

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

/** The most macro variables the live view reads per poll (the capture adds the same ones to its probe). */
export const MAX_WATCHED_MACROS = 8;

/**
 * Macro numbers from text like `500, 501` or `500-503`: positive integers, in the order written, without
 * repeats, at most `max`. Anything else in the text is ignored.
 */
export function parseMacroList(text: string, max = MAX_WATCHED_MACROS): number[] {
  const out: number[] = [];
  const add = (n: number) => {
    if (Number.isSafeInteger(n) && n > 0 && out.length < max && !out.includes(n)) out.push(n);
  };
  for (const part of text.split(/[\s,;]+/)) {
    const range = /^(\d+)-(\d+)$/.exec(part);
    if (range) {
      const [from, to] = [Number(range[1]), Number(range[2])];
      for (let n = from; n <= to && out.length < max; n++) add(n);
    } else if (/^\d+$/.test(part)) {
      add(Number(part));
    }
  }
  return out;
}

export const MACHINE_STATES: { value: MachineState; label: string }[] = [
  { value: "idle", label: "Idle" },
  { value: "running", label: "Running" },
  { value: "feed-hold", label: "Feed hold" },
  { value: "alarm", label: "Alarm" },
  { value: "estop", label: "E-stop" },
  { value: "mdi", label: "MDI" },
];

/** `NAME` of `NAME.capture.json`; a name without that ending only loses a trailing `.json`. */
export function captureStem(path: string): string {
  return path.replace(/\.capture\.json$/i, "").replace(/\.json$/i, "");
}

export interface TrendRow {
  name: string;
  value: number;
}

/** What the FANUC trend can plot: feed, spindle, then servo load and absolute position per axis, then the watched macros. */
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
  for (const m of p.macros) add(`Macro #${m.number}`, m.value);
  return rows;
}
