import type { TagConfig } from "./types";

/** A tag read from a file, before it gets an id. */
export type ImportedTag = Omit<TagConfig, "id">;

export interface TagCsvImport {
  tags: ImportedTag[];
  /** One message per rejected row, with its line number. */
  errors: string[];
  /** Things the user should know about the rows that were accepted. */
  notes: string[];
}

const EXPORT_HEADER = "Name,Address,Type";

/** Quote a CSV field when it holds a delimiter, quote or line break. */
export function csvField(field: string): string {
  return /[",;\t\r\n]/.test(field) ? `"${field.replace(/"/g, '""')}"` : field;
}

/** Write the tag list as a simple `Name,Address,Type` CSV that [`parseTagCsv`] reads back. */
export function tagsToCsv(tags: ImportedTag[]): string {
  const rows = tags.map((t) => [csvField(t.name), csvField(t.address), t.data_type].join(","));
  return [EXPORT_HEADER, ...rows].join("\n") + "\n";
}

// ── Addresses ──

interface AddressInfo {
  /** The address in the form the S7 client parses, e.g. `DB1.DBD0`, `MW10`, `I0.0`. */
  address: string;
  /** The type the address itself implies. */
  type: "Bool" | "Byte" | "Word" | "DWord";
}

const SIZE_TYPE = { X: "Bool", B: "Byte", W: "Word", D: "DWord" } as const;

/**
 * Turn an address as written by TIA Portal (`%MW10`, `%I0.0`), classic STEP 7 (`MW    10`,
 * `DB1.DBD    0`) or by hand (`DB1.DBD0`) into the form the S7 client accepts.
 * Returns `null` when the address is not a DB, I, Q or M address the client can read.
 */
export function normalizeAddress(raw: string): AddressInfo | null {
  const a = raw.replace(/\s+/g, "").replace(/^%/, "").toUpperCase();

  const db = /^DB(\d{1,5})\.DB([XBWD])(\d{1,5})(?:\.([0-7]))?$/.exec(a);
  if (db) {
    const [, n, size, offset, bit] = db;
    if (Number(n) > 65535 || Number(offset) > 65535) return null;
    if (size === "X") return { address: `DB${Number(n)}.DBX${Number(offset)}.${bit ?? "0"}`, type: "Bool" };
    if (bit !== undefined) return null;
    return { address: `DB${Number(n)}.DB${size}${Number(offset)}`, type: SIZE_TYPE[size as "B" | "W" | "D"] };
  }

  const area = /^([IQM])(?:([XBWD])(\d{1,5})(?:\.([0-7]))?|(\d{1,5})\.([0-7]))$/.exec(a);
  if (area) {
    const [, letter, size, sizedOffset, sizedBit, bitOffset, bit] = area;
    if (size === undefined) {
      if (Number(bitOffset) > 65535) return null;
      return { address: `${letter}${Number(bitOffset)}.${bit}`, type: "Bool" };
    }
    if (Number(sizedOffset) > 65535) return null;
    if (size === "X") return { address: `${letter}${Number(sizedOffset)}.${sizedBit ?? "0"}`, type: "Bool" };
    if (sizedBit !== undefined) return null;
    return { address: `${letter}${size}${Number(sizedOffset)}`, type: SIZE_TYPE[size as "B" | "W" | "D"] };
  }
  return null;
}

// ── Data types ──

/** TIA Portal and STEP 7 type names mapped to the types the S7 client can read. */
const TYPE_MAP: Record<string, string> = {
  bool: "Bool", bit: "Bool",
  byte: "Byte", sint: "Byte", usint: "Byte", char: "Byte", uint8: "Byte", int8: "Byte",
  word: "Word", int: "Word", uint: "Word", uint16: "Word", int16: "Word",
  dword: "DWord", dint: "DWord", udint: "DWord", time: "DWord", uint32: "DWord", int32: "DWord",
  real: "Real", float: "Real", float32: "Real",
  string: "String", str: "String",
};

const SIGNED_TYPES: Record<string, true> = { sint: true, int: true, dint: true, int8: true, int16: true, int32: true };

// ── CSV text ──

/** Pick the delimiter that appears most often in the header line, outside quotes. */
function detectDelimiter(text: string): string {
  const counts: Record<string, number> = { ",": 0, ";": 0, "\t": 0 };
  let quoted = false;
  for (const c of text) {
    if (c === '"') quoted = !quoted;
    else if (!quoted && (c === "\n" || c === "\r")) break;
    else if (!quoted && c in counts) counts[c]++;
  }
  return [";", "\t"].reduce((best, d) => (counts[d] > counts[best] ? d : best), ",");
}

interface CsvRecord {
  /** 1-based line the record starts on. */
  line: number;
  fields: string[];
}

/** Split CSV text into records, honouring quoted fields with `""` escapes and line breaks inside quotes. */
function parseRecords(text: string, delimiter: string): CsvRecord[] {
  const records: CsvRecord[] = [];
  let fields: string[] = [];
  let field = "";
  let quoted = false;
  let line = 1;
  let startLine = 1;
  const endRecord = () => {
    fields.push(field.trim());
    if (fields.some((f) => f !== "")) records.push({ line: startLine, fields });
    fields = [];
    field = "";
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') quoted = false;
      else { field += c; if (c === "\n") line++; }
    } else if (c === '"') quoted = true;
    else if (c === delimiter) { fields.push(field.trim()); field = ""; }
    else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      endRecord();
      line++;
      startLine = line;
    } else field += c;
  }
  endRecord();
  return records;
}

const NAME_HEADERS = ["name", "symbol", "tag", "tagname", "symbolname", "variable"];
const ADDRESS_HEADERS = ["logicaladdress", "address", "absoluteaddress", "addr"];
const TYPE_HEADERS = ["datatype", "type"];

/** Index of the first header cell whose lowercase letters-and-digits form is in `names`, or -1. */
const columnOf = (header: string[], names: string[]) =>
  header.findIndex((h) => names.includes(h.toLowerCase().replace(/[^a-z0-9]/g, "")));

/**
 * Read a tag list from CSV text. Accepts the `Name,Address,Type` file written by [`tagsToCsv`] and
 * the columns of a TIA Portal PLC tag table export (`Name`, `Path`, `Data Type`, `Logical Address`,
 * `Comment`, ...) saved from Excel as CSV. Column order does not matter, extra columns are ignored,
 * and the delimiter may be a comma, semicolon or tab. The first row must be the header.
 */
export function parseTagCsv(text: string): TagCsvImport {
  const body = text.replace(/^\uFEFF/, "");
  const records = parseRecords(body, detectDelimiter(body));
  const result: TagCsvImport = { tags: [], errors: [], notes: [] };
  if (records.length === 0) {
    result.errors.push("The file is empty");
    return result;
  }

  const header = records[0].fields;
  const nameCol = columnOf(header, NAME_HEADERS);
  const addressCol = columnOf(header, ADDRESS_HEADERS);
  const typeCol = columnOf(header, TYPE_HEADERS);
  if (nameCol < 0 || addressCol < 0) {
    result.errors.push('Line 1: the first row must be a header with "Name" and "Address" (or "Logical Address") columns');
    return result;
  }

  let signed = 0;
  for (const { line, fields } of records.slice(1)) {
    const name = fields[nameCol] ?? "";
    const rawAddress = fields[addressCol] ?? "";
    const rawType = typeCol >= 0 ? fields[typeCol] ?? "" : "";
    if (name === "") { result.errors.push(`Line ${line}: missing name`); continue; }
    if (rawAddress === "") { result.errors.push(`Line ${line}: "${name}" has no address`); continue; }

    const info = normalizeAddress(rawAddress);
    if (!info) {
      result.errors.push(`Line ${line}: "${name}" has an address S7 Explorer cannot read (${rawAddress}). Use DB, I, Q or M addresses.`);
      continue;
    }

    let dataType = info.type as string;
    if (rawType !== "") {
      const key = rawType.toLowerCase();
      const mapped = Object.prototype.hasOwnProperty.call(TYPE_MAP, key) ? TYPE_MAP[key] : undefined;
      if (!mapped) {
        result.errors.push(`Line ${line}: "${name}" has data type ${rawType}, which S7 Explorer cannot read`);
        continue;
      }
      if ((mapped === "Bool") !== (info.type === "Bool")) {
        result.errors.push(`Line ${line}: "${name}" is ${rawType} but ${rawAddress} is ${info.type === "Bool" ? "a bit" : "not a bit"} address`);
        continue;
      }
      if (Object.prototype.hasOwnProperty.call(SIGNED_TYPES, key)) signed++;
      dataType = mapped;
    }
    result.tags.push({ name, address: info.address, data_type: dataType });
  }

  if (signed > 0) {
    result.notes.push(`${signed} signed tag(s) (SInt, Int, DInt) were imported as unsigned Byte, Word or DWord, so negative values show as large positive numbers.`);
  }
  return result;
}
