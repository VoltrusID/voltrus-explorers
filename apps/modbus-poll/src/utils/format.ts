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

/**
 * The bits a decoded value came from, most significant first (the word order is already undone by the backend), and how
 * many there are: 16 for one register, 32 for int32/uint32/float32, 64 for float64. Floats give their IEEE 754 bits.
 */
export function registerBits(raw: number, type: string): { bits: bigint; width: 16 | 32 | 64 } {
  const view = new DataView(new ArrayBuffer(8));
  switch (type) {
    case "float64":
      view.setFloat64(0, raw);
      return { bits: view.getBigUint64(0), width: 64 };
    case "float32":
      view.setFloat32(0, raw);
      return { bits: BigInt(view.getUint32(0)), width: 32 };
    case "int32":
    case "uint32":
      return { bits: BigInt((raw | 0) >>> 0), width: 32 };
    default:
      return { bits: BigInt((raw | 0) & 0xffff), width: 16 };
  }
}

/** Two characters per register, high byte first; bytes outside printable ASCII show as "·". */
function asciiText(bits: bigint, width: number): string {
  let text = "";
  for (let shift = width - 8; shift >= 0; shift -= 8) {
    const byte = Number((bits >> BigInt(shift)) & 0xffn);
    text += byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : "·";
  }
  return `"${text}"`;
}

/** Packed BCD: one decimal digit per nibble (0x1234 is 1234). A nibble above 9 is not BCD. */
function bcdText(bits: bigint, width: number): string {
  const digits = bits.toString(16).padStart(width / 4, "0");
  return /[a-f]/.test(digits) ? "not BCD" : digits.replace(/^0+(?=\d)/, "");
}

export function formatValue(raw: number, type: string, displayFormat: DisplayFormat): string {
  if (isNaN(raw)) return "---";
  if (displayFormat === "dec") return raw % 1 !== 0 ? raw.toPrecision(type === "float64" ? 15 : 4) : raw.toString();
  const { bits, width } = registerBits(raw, type);
  switch (displayFormat) {
    case "hex": return "0x" + bits.toString(16).toUpperCase().padStart(width / 4, "0");
    case "bin": return bits.toString(2).padStart(width, "0");
    case "ascii": return asciiText(bits, width);
    case "bcd": return bcdText(bits, width);
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
