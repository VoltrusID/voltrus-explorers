export interface Dnp3Point {
  name: string;
  group: number;
  variation: number;
  index: number;
}

export const POINT_TYPES: { value: string; label: string }[] = [
  { value: "30:4", label: "Analog input (float32)" },
  { value: "30:1", label: "Analog input (int32)" },
  { value: "1:2", label: "Binary input" },
  { value: "10:2", label: "Binary output status" },
  { value: "20:1", label: "Counter (uint32)" },
  { value: "32:1", label: "Analog output status (int32)" },
];

export const MAX_POINT_INDEX = 65535;

const HEADER = "name,group,variation,index";
export function pointsToCsv(points: Dnp3Point[]): string {
  const rows = points.map((p) => {
    const name = /[",\r\n]/.test(p.name) ? `"${p.name.replace(/"/g, '""')}"` : p.name;
    return [name, p.group, p.variation, p.index].join(",");
  });
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

export interface PointsImport {
  points: Dnp3Point[];
  /** One message per rejected line, with its line number. */
  errors: string[];
}

/** Parse the file written by [`pointsToCsv`]. The first line is the header. */
export function parsePointsCsv(text: string): PointsImport {
  const lines = text.replace(/^\uFEFF/, "").split(/\r?\n/);
  const points: Dnp3Point[] = [];
  const errors: string[] = [];
  const names = new Set<string>();
  lines.forEach((line, i) => {
    if (i === 0 || line.trim() === "") return;
    const lineNo = i + 1;
    const parts = splitRecord(line);
    if (parts.length < 4) {
      errors.push(`Line ${lineNo}: expected name,group,variation,index`);
      return;
    }
    const [name, g, v, idx] = parts;
    const group = toNumber(g);
    const variation = toNumber(v);
    const index = toNumber(idx);
    if (!name) {
      errors.push(`Line ${lineNo}: the point name is empty`);
    } else if (names.has(name)) {
      errors.push(`Line ${lineNo}: duplicate point name '${name}'`);
    } else if (!POINT_TYPES.some((t) => t.value === `${group}:${variation}`)) {
      errors.push(`Line ${lineNo}: unsupported object g${g}v${v}`);
    } else if (!Number.isInteger(index) || index < 0 || index > MAX_POINT_INDEX) {
      errors.push(`Line ${lineNo}: index must be a whole number from 0 to ${MAX_POINT_INDEX}`);
    } else {
      names.add(name);
      points.push({ name, group, variation, index });
    }
  });
  return { points, errors };
}
