import type { DisplayFormat } from "../types";

const EXCEPTION_02 = /Illegal Data Address \(0x02\) reading (holding|input|coil|discrete) (\d+)\.\.(\d+)/gi;

/** The register table a master should try when the one it asked has nothing at the address. */
const OTHER_AREA: Record<string, string> = { holding: "input", input: "holding", coil: "discrete", discrete: "coil" };

/**
 * Adds to an "Illegal Data Address (0x02) reading <area> <from>..<to>" message what to check: the device answered, but has
 * nothing in that range. The usual causes are an address that is off by one (manuals count registers from 0 or from 1) and a
 * different register table (holding vs input). Other messages, and every part of a "; "-joined list, are handled separately.
 */
export function explainModbusError(message: string): string {
  return message.replace(EXCEPTION_02, (found, area: string, from: string) => {
    const a = Number(from);
    const lower = area.toLowerCase();
    const alternatives = a > 0 ? `${a + 1} or ${a - 1}` : `${a + 1}`;
    const what = lower === "coil" || lower === "discrete" ? "bits" : "registers";
    return `${found}: the device has no ${what} in that range (check the start address, since manuals count from 0 or from 1, so try ${alternatives}, and the register area, so try ${OTHER_AREA[lower]} instead of ${lower})`;
  });
}

export function formatValue(raw: number, type: string, displayFormat: DisplayFormat): string {
  if (isNaN(raw)) return "---";
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

/** Numeric payload of a serialized `ModbusValue` (`{ Float64: 1.5 }`), or NaN if it has none. */
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

/** Message of a serialized `ModbusValue::Failed` (`{ Failed: "Illegal Data Address (0x02) reading …" }`). */
export function extractError(v: unknown): string | undefined {
  if (typeof v === "object" && v !== null) {
    const failed = (v as Record<string, unknown>).Failed;
    if (typeof failed === "string") return failed;
  }
  return undefined;
}
