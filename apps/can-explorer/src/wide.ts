import type { DbcSignalView } from "./types";

/**
 * Integer signals wider than a double's 53-bit mantissa, unscaled: their values travel as exact
 * decimal text (the Rust side decodes them to `label` text and encodes from a string).
 */
export function isWideInteger(s: Pick<DbcSignalView, "length" | "valueType" | "factor" | "offset">): boolean {
  return (s.valueType === "unsigned" || s.valueType === "signed") && s.length > 53 && s.factor === 1 && s.offset === 0;
}

const INTEGER = /^\s*[-+]?\d+\s*$/;

/**
 * What `dbc_encode` gets for one edited value: a number, or for wide integer signals the text
 * itself when it is a whole number (so no digit is lost). `null` when the text is not a number.
 */
export function encodeValue(s: Pick<DbcSignalView, "length" | "valueType" | "factor" | "offset">, text: string): number | string | null {
  if (isWideInteger(s) && INTEGER.test(text)) return text.trim();
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}

/** The editor's starting text for a signal: the exact integer of a wide signal when the frame carried one. */
export function exactText(s: Pick<DbcSignalView, "length" | "valueType" | "factor" | "offset">, label: string | null | undefined): string | null {
  return isWideInteger(s) && label != null && INTEGER.test(label) ? label.trim() : null;
}
