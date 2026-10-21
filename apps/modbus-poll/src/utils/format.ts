import type { DisplayFormat } from "../types";

export function formatValue(raw: number, type: string, displayFormat: DisplayFormat): string {
  if (isNaN(raw)) return "---";
  if (type === "string") return String(raw);
  const is64 = type === "float64";
  switch (displayFormat) {
    case "hex": return "0x" + (is64 ? ((raw | 0) >>> 0).toString(16).toUpperCase().padStart(8, "0") : (type === "float32" || type === "int32" ? ((raw | 0) >>> 0).toString(16).toUpperCase().padStart(8, "0") : ((raw | 0) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0")));
    case "bin": return (is64 ? ((raw | 0) >>> 0).toString(2).padStart(32, "0") : type === "float32" || type === "int32" ? ((raw | 0) >>> 0).toString(2).padStart(32, "0") : ((raw | 0) & 0xFFFF).toString(2).padStart(16, "0"));
    default: return raw % 1 !== 0 ? raw.toPrecision(is64 ? 15 : 4) : raw.toString();
  }
}

export function displayAddr(addr: number, area: string, useOneBased: boolean): string {
  const offset = useOneBased ? 1 : 0;
  const base = addr + offset;
  if (!useOneBased) return String(base);
  switch (area) {
    case "holding": return String(40000 + base);
    case "input": return String(30000 + base);
    case "coil": return String(base);
    case "discrete": return String(10000 + base);
    default: return String(base);
  }
}

export function extractRaw(v: unknown): number {
  if (typeof v === "number") return v;
  if (typeof v === "string") return Number(v) || 0;
  if (typeof v === "object" && v !== null) {
    for (const key of Object.keys(v)) {
      const val = (v as Record<string, unknown>)[key];
      if (typeof val === "number") return val;
    }
  }
  return NaN;
}
