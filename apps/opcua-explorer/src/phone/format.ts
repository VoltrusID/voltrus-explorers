import type { Tone } from "@mactools/ui";

/** A value as the big number of a row: numbers lose float noise, booleans read True / False, objects become JSON. */
export function fmtValue(v: unknown, dataType?: string): string {
  if (v === null || v === undefined || v === "") return "No value";
  // A Double that happens to be whole keeps its decimal point, so the digits do not jump while the value moves.
  if (typeof v === "number") return Number.isInteger(v) ? (/^(double|float)$/i.test(dataType ?? "") ? v.toFixed(1) : String(v)) : String(+v.toFixed(3));
  if (typeof v === "boolean") return v ? "True" : "False";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

export function hasValue(v: unknown): boolean {
  return v !== null && v !== undefined && v !== "";
}

/** The quality chip of a monitored value: the status of its last notification, or what is known without one. */
export function quality(status: string | undefined, value: unknown): { label: string; tone: Tone } {
  if (!hasValue(value) && !status) return { label: "Waiting", tone: "neutral" };
  const s = status ?? "Good";
  if (s.startsWith("Good")) return { label: "Good", tone: "success" };
  if (s.startsWith("Uncertain")) return { label: "Uncertain", tone: "warn" };
  return { label: "Bad", tone: "danger" };
}

export function fmtTime(ms: number): string {
  if (!ms) return "";
  return new Date(ms).toLocaleTimeString("en-US", { hour12: false });
}

export function plain(v: unknown): string {
  if (v === null || v === undefined) return "No value";
  return typeof v === "object" ? JSON.stringify(v) : String(v);
}
