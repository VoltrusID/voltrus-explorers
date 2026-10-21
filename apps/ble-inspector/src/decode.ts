export type FormatId =
  | "auto"
  | "uint8"
  | "int8"
  | "uint16le"
  | "uint16be"
  | "int16le"
  | "int16be"
  | "uint32le"
  | "uint32be"
  | "int32le"
  | "int32be"
  | "float32le"
  | "float32be"
  | "float64le"
  | "float64be"
  | "utf8"
  | "ascii"
  | "hex";

export interface FormatInfo {
  id: FormatId;
  label: string;
  /** Bytes consumed per value; 0 for text, hex and auto. */
  size: number;
  kind: "number" | "text" | "hex" | "auto";
}

export const FORMATS: readonly FormatInfo[] = [
  { id: "auto", label: "Auto", size: 0, kind: "auto" },
  { id: "uint8", label: "uint8", size: 1, kind: "number" },
  { id: "int8", label: "int8", size: 1, kind: "number" },
  { id: "uint16le", label: "uint16 LE", size: 2, kind: "number" },
  { id: "uint16be", label: "uint16 BE", size: 2, kind: "number" },
  { id: "int16le", label: "int16 LE", size: 2, kind: "number" },
  { id: "int16be", label: "int16 BE", size: 2, kind: "number" },
  { id: "uint32le", label: "uint32 LE", size: 4, kind: "number" },
  { id: "uint32be", label: "uint32 BE", size: 4, kind: "number" },
  { id: "int32le", label: "int32 LE", size: 4, kind: "number" },
  { id: "int32be", label: "int32 BE", size: 4, kind: "number" },
  { id: "float32le", label: "float32 LE", size: 4, kind: "number" },
  { id: "float32be", label: "float32 BE", size: 4, kind: "number" },
  { id: "float64le", label: "float64 LE", size: 8, kind: "number" },
  { id: "float64be", label: "float64 BE", size: 8, kind: "number" },
  { id: "utf8", label: "UTF-8 text", size: 0, kind: "text" },
  { id: "ascii", label: "ASCII text", size: 0, kind: "text" },
  { id: "hex", label: "Hex bytes", size: 0, kind: "hex" },
];

export const NUMERIC_FORMATS: readonly FormatInfo[] = FORMATS.filter((f) => f.kind === "number");

const BY_ID = new Map(FORMATS.map((f) => [f.id, f]));

export function formatInfo(id: FormatId): FormatInfo {
  return BY_ID.get(id) ?? FORMATS[0];
}

export function isFormatId(value: unknown): value is FormatId {
  return typeof value === "string" && BY_ID.has(value as FormatId);
}

/** Parse the hex strings used by the backend and the write box ("0x" and spaces allowed). Null if invalid. */
export function hexToBytes(hex: string): Uint8Array | null {
  const clean = hex.trim().replace(/0x/gi, "").replace(/\s+/g, "");
  if (clean.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(clean)) return null;
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.substring(i * 2, i * 2 + 2), 16);
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}

export function textToHex(text: string): string {
  return bytesToHex(new TextEncoder().encode(text));
}

/** Read one number at `offset`. Null if the format is not numeric or the value is too short. */
export function decodeNumber(bytes: Uint8Array, id: FormatId, offset = 0): number | null {
  const info = formatInfo(id);
  if (info.kind !== "number" || !Number.isInteger(offset) || offset < 0 || offset + info.size > bytes.length) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const le = id.endsWith("le");
  if (id === "uint8") return view.getUint8(offset);
  if (id === "int8") return view.getInt8(offset);
  if (id.startsWith("uint16")) return view.getUint16(offset, le);
  if (id.startsWith("int16")) return view.getInt16(offset, le);
  if (id.startsWith("uint32")) return view.getUint32(offset, le);
  if (id.startsWith("int32")) return view.getInt32(offset, le);
  if (id.startsWith("float32")) return view.getFloat32(offset, le);
  return view.getFloat64(offset, le);
}

export function decodeText(bytes: Uint8Array, id: "utf8" | "ascii"): string {
  if (id === "utf8") return new TextDecoder("utf-8").decode(bytes).replace(/\0+$/, "");
  let s = "";
  for (const b of bytes) {
    if (b === 0) break;
    s += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
  }
  return s;
}

export function spacedHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");
}

export function isPrintableAscii(bytes: Uint8Array): boolean {
  if (bytes.length === 0) return false;
  for (const b of bytes) if (b < 0x20 || b >= 0x7f) return false;
  return true;
}

/** Number to a short display string (7 significant digits, no trailing zeros, NaN/Infinity kept). */
export function formatNumber(v: number): string {
  if (!Number.isFinite(v)) return String(v);
  return String(Number(v.toPrecision(7)));
}

export interface DecodeOptions {
  format: FormatId;
  /** Byte offset of the number to read (numeric formats only). */
  offset: number;
  /** Multiplier applied to numeric results (for example 0.01 for hundredths of a degree). */
  scale: number;
  /** Format to use when `format` is "auto" (a known characteristic default, if any). */
  fallback?: FormatId;
}

/** Resolve "auto": a known default, else printable text as UTF-8, else hex bytes. */
export function resolveFormat(bytes: Uint8Array, format: FormatId, fallback?: FormatId): FormatId {
  if (format !== "auto") return format;
  if (fallback && fallback !== "auto") return fallback;
  return isPrintableAscii(bytes) ? "utf8" : "hex";
}

/** Human-readable decode of a whole value. */
export function describeValue(bytes: Uint8Array, opts: DecodeOptions): string {
  const format = resolveFormat(bytes, opts.format, opts.fallback);
  const info = formatInfo(format);
  if (info.kind === "text") return decodeText(bytes, format as "utf8" | "ascii");
  if (info.kind === "hex") return spacedHex(bytes);
  const n = decodeNumber(bytes, format, opts.offset);
  if (n === null) return `needs ${opts.offset + info.size} bytes, got ${bytes.length}`;
  return formatNumber(n * opts.scale);
}

/** Numeric value for plotting, or null when the format is not numeric or the value is too short. */
export function plotValue(bytes: Uint8Array, opts: DecodeOptions): number | null {
  const format = resolveFormat(bytes, opts.format, opts.fallback);
  const n = decodeNumber(bytes, format, opts.offset);
  if (n === null) return null;
  const v = n * opts.scale;
  return Number.isFinite(v) ? v : null;
}

/** RFC 4180 CSV, CRLF line endings; cells starting with = + - @ are prefixed so spreadsheets do not run them as formulas. */
export function toCsv(rows: (string | number)[][]): string {
  const cell = (c: string | number): string => {
    let s = String(c);
    if (typeof c === "string" && /^[=+\-@\t\r]/.test(s)) s = `'${s}`;
    return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return rows.map((r) => r.map(cell).join(",")).join("\r\n") + "\r\n";
}

/** Hex string to send for a preset value, or null if a hex value is malformed. */
export function presetToHex(mode: "hex" | "text", value: string): string | null {
  if (mode === "text") return textToHex(value);
  const bytes = hexToBytes(value);
  return bytes ? bytesToHex(bytes) : null;
}
