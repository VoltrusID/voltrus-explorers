import type { S7Value } from "./useS7Device";

/** A polled value as the Watch list and the desktop table show it. */
export function formatS7Value(v: S7Value | undefined): string {
  if (v === undefined || v === null) return "—";
  if (typeof v === "boolean") return v ? "On (1)" : "Off (0)";
  if (typeof v === "number") return Number.isInteger(v) ? String(v) : v.toFixed(3);
  return String(v);
}
