import type { ReadBlock } from "./types";
import { validateBlock } from "./helpers";

const HEADER = "name,address,quantity,type,area";

export function quote(field: string): string {
  return /[",\r\n]/.test(field) ? `"${field.replace(/"/g, '""')}"` : field;
}

export function blocksToCsv(blocks: ReadBlock[]): string {
  const rows = blocks.map((b) => [quote(b.name), b.address, b.quantity, b.type, b.area].join(","));
  return [HEADER, ...rows].join("\n") + "\n";
}

/** Split one CSV record into fields, honouring double-quoted fields with `""` escapes. */
function splitRecord(line: string): string[] {
  const fields: string[] = [];
  let field = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"') quoted = true;
    else if (c === ",") { fields.push(field); field = ""; }
    else field += c;
  }
  fields.push(field);
  return fields.map((f) => f.trim());
}

/** An empty cell is a missing number, not zero. */
const toNumber = (cell: string) => (cell === "" ? NaN : Number(cell));

export interface CsvImport {
  blocks: ReadBlock[];
  /** One message per rejected line, with its line number. */
  errors: string[];
}

/** Parse the file written by [`blocksToCsv`]. The first line is the header. */
export function parseBlocksCsv(text: string): CsvImport {
  const lines = text.replace(/^\uFEFF/, "").split(/\r?\n/);
  const blocks: ReadBlock[] = [];
  const errors: string[] = [];
  lines.forEach((line, index) => {
    if (index === 0 || line.trim() === "") return;
    const lineNo = index + 1;
    const parts = splitRecord(line);
    if (parts.length < 5) {
      errors.push(`Line ${lineNo}: expected name,address,quantity,type,area`);
      return;
    }
    const block: ReadBlock = {
      name: parts[0],
      address: toNumber(parts[1]),
      quantity: toNumber(parts[2]),
      type: parts[3],
      area: parts[4],
    };
    const problem = validateBlock(block);
    if (problem) errors.push(`Line ${lineNo}: ${problem}`);
    else blocks.push(block);
  });
  return { blocks, errors };
}
