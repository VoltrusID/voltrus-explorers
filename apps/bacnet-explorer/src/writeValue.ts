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
