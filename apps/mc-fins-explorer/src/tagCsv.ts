import { DATA_TYPES } from "./types";

export interface ImportedTag {
  name: string;
  address: string;
  data_type: string;
}

export interface TagCsvImport {
  tags: ImportedTag[];
  errors: string[];
}

const EXPORT_HEADER = "Name,Address,Type";

/** Quote a CSV field when it holds a separator, quote or line break. */
export function csvField(field: string): string {
  return /[",;\t\r\n]/.test(field) ? `"${field.replace(/"/g, '""')}"` : field;
}

export function tagsToCsv(tags: ImportedTag[]): string {
  const rows = tags.map((t) => [csvField(t.name), csvField(t.address), t.data_type].join(","));
  return [EXPORT_HEADER, ...rows].join("\n") + "\n";
}

/** Split one CSV line (comma, semicolon or tab separated; double quotes escape). */
function splitLine(line: string, sep: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') { cur += '"'; i++; }
      else if (c === '"') quoted = false;
      else cur += c;
    } else if (c === '"') quoted = true;
    else if (c === sep) { out.push(cur.trim()); cur = ""; }
    else cur += c;
  }
  out.push(cur.trim());
  return out;
}

/** Type names other tools use, mapped to ours. */
const TYPE_ALIASES: Record<string, string> = {
  BOOL: "Bool", BIT: "Bool",
  INT: "Int16", INT16: "Int16", WORD: "UInt16", UINT: "UInt16", UINT16: "UInt16",
  DINT: "Int32", INT32: "Int32", DWORD: "UInt32", UDINT: "UInt32", UINT32: "UInt32",
  REAL: "Float32", FLOAT: "Float32", FLOAT32: "Float32",
  BCD: "BCD16", BCD16: "BCD16", HEX: "Hex16", HEX16: "Hex16",
};

export function normaliseType(t: string): string | null {
  const exact = DATA_TYPES.find((d) => d.toLowerCase() === t.trim().toLowerCase());
  return exact ?? TYPE_ALIASES[t.trim().toUpperCase()] ?? null;
}

/** Parse `Name,Address,Type` rows (a header row is optional; the type defaults to Int16). */
export function parseTagCsv(text: string): TagCsvImport {
  const lines = text.replace(/^﻿/, "").split(/\r?\n/).filter((l) => l.trim() !== "");
  const tags: ImportedTag[] = [];
  const errors: string[] = [];
  if (lines.length === 0) return { tags, errors: ["the file is empty"] };
  const sep = [",", ";", "\t"].sort((a, b) => lines[0].split(b).length - lines[0].split(a).length)[0];
  lines.forEach((line, i) => {
    const [name = "", address = "", type = ""] = splitLine(line, sep);
    if (i === 0 && name.toLowerCase() === "name" && address.toLowerCase() === "address") return;
    if (!address) {
      errors.push(`row ${i + 1}: no address`);
      return;
    }
    const dt = type ? normaliseType(type) : "Int16";
    if (!dt) {
      errors.push(`row ${i + 1}: unknown type '${type}'`);
      return;
    }
    tags.push({ name: name || address, address: address.toUpperCase(), data_type: dt });
  });
  return { tags, errors };
}
