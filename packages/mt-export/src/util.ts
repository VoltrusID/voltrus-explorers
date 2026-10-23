import type { ModbusArea, ModbusTag, ModbusType } from "./model";

/** RFC 4180 field: quoted when it holds a comma, quote or line break. */
export function csvField(value: string | number | boolean): string {
  const s = String(value);
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export const csvLine = (fields: ReadonlyArray<string | number | boolean>): string => fields.map(csvField).join(",");

/** Lowercase ASCII identifier: letters, digits and underscores, never empty. */
export function slug(s: string, fallback = "device"): string {
  const out = s
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return out || fallback;
}

/** A file-name stem: keeps case, swaps anything awkward for `_`. */
export function fileStem(s: string, fallback = "tags"): string {
  const out = s.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
  return out || fallback;
}

/** Registers one value of `type` occupies. */
export const typeWidth = (t: ModbusType): number => (t === "float64" ? 4 : t === "float32" || t === "int32" ? 2 : 1);

export const isBitArea = (a: ModbusArea): boolean => a === "coil" || a === "discrete";

export const isRegisterArea = (a: ModbusArea): boolean => !isBitArea(a);

/** Stable JSON: two-space indent and a trailing newline. */
export const toJson = (v: unknown): string => JSON.stringify(v, null, 2) + "\n";

/** Shortest decimal text of a number, so `0.1` stays `0.1`. */
export const num = (n: number): string => String(n);

/** Reading a number out of a tag with `scale`, as the scaled value rather than the raw one. */
export const isScaled = (t: ModbusTag): boolean => t.scale !== 1;

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
