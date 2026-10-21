// User-defined CIP struct decoders (persisted in localStorage).

export type FieldType =
  | "bool"
  | "sint"
  | "int"
  | "uint16"
  | "dint"
  | "uint32"
  | "real"
  | "lint"
  | "bytes";

export interface DecoderField {
  name: string;
  type: FieldType;
  size: number; // only used for "bytes"
}

export interface Decoder {
  id: string;
  name: string;
  fields: DecoderField[];
}

export interface DecodedField {
  name: string;
  type: FieldType;
  offset: number;
  size: number;
  value: string;
}

export const FIELD_TYPES: Array<{ value: FieldType; label: string; size: number | null }> = [
  { value: "bool", label: "BOOL (1)", size: 1 },
  { value: "sint", label: "SINT i8 (1)", size: 1 },
  { value: "int", label: "INT i16 (2)", size: 2 },
  { value: "uint16", label: "UINT u16 (2)", size: 2 },
  { value: "dint", label: "DINT i32 (4)", size: 4 },
  { value: "uint32", label: "UDINT u32 (4)", size: 4 },
  { value: "real", label: "REAL f32 (4)", size: 4 },
  { value: "lint", label: "LINT i64 (8)", size: 8 },
  { value: "bytes", label: "BYTES (n)", size: null },
];

const KEY = "enip.decoders";

export function loadDecoders(): Decoder[] {
  try {
    const raw = localStorage.getItem(KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function saveDecoders(decoders: Decoder[]): void {
  localStorage.setItem(KEY, JSON.stringify(decoders));
}

export function fieldSize(f: DecoderField): number {
  if (f.type === "bytes") return Math.max(0, Math.floor(f.size || 0));
  const meta = FIELD_TYPES.find((t) => t.value === f.type);
  return meta?.size ?? 0;
}

export function decodeStruct(bytes: number[], fields: DecoderField[]): DecodedField[] {
  const dv = new DataView(new Uint8Array(bytes).buffer);
  const out: DecodedField[] = [];
  let off = 0;
  for (const f of fields) {
    const sz = fieldSize(f);
    let value = "—";
    try {
      switch (f.type) {
        case "bool":
          value = bytes[off] ? "true" : "false";
          break;
        case "sint":
          value = String(dv.getInt8(off));
          break;
        case "int":
          value = String(dv.getInt16(off, true));
          break;
        case "uint16":
          value = String(dv.getUint16(off, true));
          break;
        case "dint":
          value = String(dv.getInt32(off, true));
          break;
        case "uint32":
          value = String(dv.getUint32(off, true));
          break;
        case "real":
          value = String(dv.getFloat32(off, true));
          break;
        case "lint":
          value = dv.getBigInt64(off, true).toString();
          break;
        case "bytes":
          value = bytes
            .slice(off, off + sz)
            .map((b) => b.toString(16).padStart(2, "0"))
            .join(" ");
          break;
      }
    } catch {
      value = "—";
    }
    out.push({ name: f.name || `(offset ${off})`, type: f.type, offset: off, size: sz, value });
    off += sz;
  }
  return out;
}

export function newDecoder(name: string): Decoder {
  return {
    id: `dec_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`,
    name,
    fields: [{ name: "field1", type: "dint", size: 0 }],
  };
}
