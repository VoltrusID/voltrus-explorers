import type { BacnetValue } from "./types";

/**
 * BACnet present values are typed by object class (ASHRAE 135 clause 12): analog
 * objects take Real, binary objects take Enumerated (inactive = 0, active = 1)
 * and multi-state objects take Unsigned (states are numbered from 1). A device
 * answers any other application tag with an invalid-data-type error.
 */
export type WriteKind = "real" | "bool" | "unsigned";

export function writeKindFor(objectType: string): WriteKind {
  if (objectType.startsWith("binary")) return "bool";
  if (objectType.startsWith("multiState")) return "unsigned";
  return "real";
}

/** Type dropdown contents for `ValueEditor`: the one type the object accepts. */
export const WRITE_KIND_TYPES: Record<WriteKind, { value: WriteKind; label: string }[]> = {
  real: [{ value: "real", label: "Real" }],
  bool: [{ value: "bool", label: "Binary (inactive / active)" }],
  unsigned: [{ value: "unsigned", label: "Unsigned (state number)" }],
};

const MAX_REAL = 3.4028234663852886e38;

/** Parse the text entered in `ValueEditor` into the typed value to write. Throws `Error` with a user-readable message. */
export function parseWriteValue(kind: WriteKind, input: string): BacnetValue {
  const text = input.trim();
  switch (kind) {
    case "bool":
      if (text === "true") return { Enum: 1 };
      if (text === "false") return { Enum: 0 };
      throw new Error("Choose inactive (false) or active (true).");
    case "unsigned": {
      if (!/^\d+$/.test(text)) throw new Error(`"${input}" is not a whole number.`);
      const n = Number(text);
      if (n < 1 || n > 0xffffffff) throw new Error("A multi-state value must be between 1 and 4294967295.");
      return { Unsigned: n };
    }
    case "real": {
      const n = Number(text);
      if (text === "" || !Number.isFinite(n)) throw new Error(`"${input}" is not a number.`);
      if (Math.abs(n) > MAX_REAL) throw new Error("Value is outside the range of a BACnet Real.");
      return { Real: n };
    }
  }
}

/** Current value as the editor's initial text. */
export function writeInitialText(kind: WriteKind, value: BacnetValue): string {
  if (kind === "bool") {
    const active = "Boolean" in value ? value.Boolean : "Enum" in value ? value.Enum !== 0 : false;
    return active ? "true" : "false";
  }
  if ("Real" in value) return String(value.Real);
  if ("Float64" in value) return String(value.Float64);
  if ("Double" in value) return String(value.Double);
  if ("Unsigned" in value) return String(value.Unsigned);
  if ("Enum" in value) return String(value.Enum);
  return "";
}

/** The value that clears a command priority slot: a WriteProperty carrying Null (ASHRAE 135 clause 19.2). */
export const RELINQUISH_VALUE: BacnetValue = { Null: null };

/** Priorities that have a standard meaning (ASHRAE 135 table 19-1); the others are "available". 1 is the highest. */
const PRIORITY_NAMES: Record<number, string> = {
  1: "Manual life safety",
  2: "Automatic life safety",
  5: "Critical equipment control",
  6: "Minimum on/off",
  8: "Manual operator",
  16: "Lowest, the usual default",
};

/** Picker contents for the command priority: the device default first, then 1 to 16. */
export const PRIORITY_OPTIONS: { value: string; label: string; hint?: string }[] = [
  { value: "", label: "Device default", hint: "No priority is sent; a commandable object uses 16" },
  ...Array.from({ length: 16 }, (_, i) => ({ value: String(i + 1), label: String(i + 1), hint: PRIORITY_NAMES[i + 1] })),
];

/** Command priority from the picker's text: `null` for "device default", otherwise 1 to 16. Throws on anything else. */
export function parsePriority(text: string): number | null {
  if (text === "") return null;
  const n = Number(text);
  if (!Number.isInteger(n) || n < 1 || n > 16) throw new Error(`Priority "${text}" is not between 1 and 16.`);
  return n;
}
