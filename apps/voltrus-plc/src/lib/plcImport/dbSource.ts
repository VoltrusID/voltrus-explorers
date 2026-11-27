// Data block and PLC data type sources (`.db`, `.udt`: the text the engineering software
// generates from blocks, and classic STL DB sources): parse the declarations, compute the byte
// offsets of a DB with standard (non-optimized) access, and emit ST types.
//
// Standard access layout (S7-300/400 rules, also used by S7-1200/1500 for non-optimized DBs):
//  - BOOLs are packed bit by bit; anything else starts at the next byte.
//  - Types of 2 bytes or more, STRING, ARRAY and STRUCT start at an even byte.
//  - ARRAY, STRUCT and STRING occupy an even number of bytes (padded).
//  - STRING[n] takes n + 2 bytes (maximum length, actual length, characters); n defaults to 254.
import { mapElementaryType, toIdentifier, watchTypeFor } from "./common";

export type DeclType =
  | { kind: "elementary"; name: string }
  | { kind: "array"; dims: [number, number][]; of: DeclType }
  | { kind: "struct"; members: Member[] }
  | { kind: "udt"; name: string };

export interface Member {
  name: string;
  type: DeclType;
  initial?: string;
  comment: string;
}

export interface SourceBlock {
  kind: "db" | "type";
  name: string;
  /** DB number when the source names it as `DB 5`. */
  number?: number;
  /** `S7_Optimized_Access := 'TRUE'`: offsets do not apply. */
  optimized: boolean;
  members: Member[];
  /** Instance or UDT-typed DB: the type it is made of. */
  ofType?: string;
  /** Actual values from the BEGIN section, by member path. */
  actual: Record<string, string>;
}

// ── Tokenizer ──

type Tok = { t: "id" | "qid" | "str" | "num" | "sym"; v: string; line: number };

function tokenize(src: string): { toks: Tok[]; comments: Map<number, string> } {
  const toks: Tok[] = [];
  const comments = new Map<number, string>();
  let i = 0;
  let line = 1;
  while (i < src.length) {
    const c = src[i];
    if (c === "\n") { line++; i++; continue; }
    if (/\s/.test(c)) { i++; continue; }
    if (c === "/" && src[i + 1] === "/") {
      const end = src.indexOf("\n", i);
      const text = src.slice(i + 2, end < 0 ? undefined : end).trim();
      if (text) comments.set(line, text);
      i = end < 0 ? src.length : end;
      continue;
    }
    if (c === "(" && src[i + 1] === "*") {
      const end = src.indexOf("*)", i + 2);
      const body = src.slice(i, end < 0 ? undefined : end + 2);
      line += body.split("\n").length - 1;
      i = end < 0 ? src.length : end + 2;
      continue;
    }
    if (c === "{") {
      // Attributes: keep S7_Optimized_Access as a pseudo-token, drop the rest.
      const end = src.indexOf("}", i);
      const body = src.slice(i, end < 0 ? undefined : end + 1);
      const opt = body.match(/S7_Optimized_Access\s*:=\s*'(\w+)'/i);
      if (opt) toks.push({ t: "sym", v: `@optimized=${opt[1].toUpperCase()}`, line });
      line += body.split("\n").length - 1;
      i = end < 0 ? src.length : end + 1;
      continue;
    }
    if (c === '"') {
      const end = src.indexOf('"', i + 1);
      toks.push({ t: "qid", v: src.slice(i + 1, end), line });
      i = end + 1;
      continue;
    }
    if (c === "'") {
      let j = i + 1;
      while (j < src.length && !(src[j] === "'" && src[j - 1] !== "$")) j++;
      toks.push({ t: "str", v: src.slice(i, j + 1), line });
      i = j + 1;
      continue;
    }
    const two = src.slice(i, i + 2);
    if (two === ":=" || two === "..") { toks.push({ t: "sym", v: two, line }); i += 2; continue; }
    const word = src.slice(i).match(/^[A-Za-z_][A-Za-z0-9_]*(#[A-Za-z0-9_.:+\-#]+)?/);
    if (word) { toks.push({ t: "id", v: word[0], line }); i += word[0].length; continue; }
    const num = src.slice(i).match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/);
    if (num) { toks.push({ t: "num", v: num[0], line }); i += num[0].length; continue; }
    toks.push({ t: "sym", v: c, line });
    i++;
  }
  return { toks, comments };
}

class Parser {
  i = 0;
  constructor(private toks: Tok[], private comments: Map<number, string>) {}
  peek(o = 0) { return this.toks[this.i + o]; }
  next() { return this.toks[this.i++]; }
  is(v: string) { const t = this.peek(); return !!t && (t.t === "id" ? t.v.toUpperCase() === v : t.v === v); }
  accept(v: string) { if (this.is(v)) { this.i++; return true; } return false; }
  expect(v: string) {
    if (!this.accept(v)) {
      const t = this.peek();
      throw new Error(`Line ${t?.line ?? "end"}: expected ${v}, found ${t ? t.v : "end of file"}`);
    }
  }
  done() { return this.i >= this.toks.length; }
  comment(line: number) { return this.comments.get(line) ?? ""; }

  name(): string {
    const t = this.next();
    if (!t || (t.t !== "id" && t.t !== "qid")) throw new Error(`Line ${t?.line ?? "end"}: expected a name`);
    return t.v;
  }

  type(): DeclType {
    if (this.accept("STRUCT")) {
      const members = this.members("END_STRUCT");
      this.expect("END_STRUCT");
      return { kind: "struct", members };
    }
    if (this.accept("ARRAY")) {
      this.expect("[");
      const dims: [number, number][] = [];
      do {
        const lo = Number(this.next().v);
        this.expect("..");
        const hi = Number(this.next().v);
        if (!Number.isFinite(lo) || !Number.isFinite(hi)) throw new Error("Array bounds must be numbers");
        dims.push([lo, hi]);
      } while (this.accept(","));
      this.expect("]");
      this.expect("OF");
      return { kind: "array", dims, of: this.type() };
    }
    const t = this.next();
    if (!t) throw new Error("Unexpected end of file in a type");
    if (t.t === "qid") return { kind: "udt", name: t.v };
    let name = t.v;
    if (/^W?STRING$/i.test(name) && this.accept("[")) {
      name = `${name}[${this.next().v}]`;
      this.expect("]");
    }
    return { kind: "elementary", name };
  }

  members(end: string): Member[] {
    const out: Member[] = [];
    while (!this.done() && !this.is(end)) {
      const startLine = this.peek().line;
      const name = this.name();
      this.expect(":");
      const type = this.type();
      let initial: string | undefined;
      if (this.accept(":=")) {
        const parts: string[] = [];
        while (!this.done() && !this.is(";")) parts.push(this.next().v);
        initial = parts.join("");
      }
      const endLine = this.peek()?.line ?? startLine;
      this.expect(";");
      out.push({ name, type, initial, comment: this.comment(type.kind === "struct" ? startLine : endLine) || this.comment(startLine) });
    }
    return out;
  }
}

/** Parse every DATA_BLOCK and TYPE in a source file. */
export function parseDbSource(src: string): SourceBlock[] {
  const { toks, comments } = tokenize(src);
  const p = new Parser(toks, comments);
  const blocks: SourceBlock[] = [];
  while (!p.done()) {
    if (p.accept("TYPE")) {
      const name = p.name();
      let optimized = false;
      while (!p.is("STRUCT") && !p.done()) {
        const t = p.next();
        if (t.v.startsWith("@optimized=")) optimized = t.v.endsWith("TRUE");
      }
      p.expect("STRUCT");
      const members = p.members("END_STRUCT");
      p.expect("END_STRUCT");
      p.accept(";");
      p.expect("END_TYPE");
      blocks.push({ kind: "type", name, optimized, members, actual: {} });
    } else if (p.accept("DATA_BLOCK")) {
      let name: string;
      let number: number | undefined;
      if (p.is("DB") && p.peek(1)?.t === "num") {
        p.next();
        number = Number(p.next().v);
        name = `DB${number}`;
      } else {
        name = p.name();
        const m = name.match(/^DB\s*(\d+)$/i);
        if (m) number = Number(m[1]);
      }
      let optimized = false;
      let members: Member[] = [];
      let ofType: string | undefined;
      // Header: attributes, TITLE, VERSION, AUTHOR, NON_RETAIN... until STRUCT, a type name or BEGIN.
      while (!p.done()) {
        const t = p.peek();
        if (t.v.startsWith("@optimized=")) { optimized = t.v.endsWith("TRUE"); p.next(); continue; }
        if (p.is("STRUCT")) {
          p.next();
          members = p.members("END_STRUCT");
          p.expect("END_STRUCT");
          p.accept(";");
          break;
        }
        if (p.is("BEGIN") || p.is("END_DATA_BLOCK")) break;
        if (t.t === "qid" || (t.t === "id" && /^(FB|UDT|SFB)\s*\d+$/i.test(t.v))) { ofType = t.v; p.next(); continue; }
        if (p.is("TITLE") || p.is("VERSION") || p.is("AUTHOR") || p.is("FAMILY") || p.is("NAME")) {
          // `TITLE = text` runs to the end of its line; `VERSION : 0.1`
          const line = t.line;
          while (!p.done() && p.peek().line === line) p.next();
          continue;
        }
        p.next(); // NON_RETAIN, READ_ONLY, KNOW_HOW_PROTECT, ...
      }
      const actual: Record<string, string> = {};
      if (p.accept("BEGIN")) {
        while (!p.done() && !p.is("END_DATA_BLOCK")) {
          const path: string[] = [];
          while (!p.done() && !p.is(":=")) path.push(p.next().v);
          p.expect(":=");
          const value: string[] = [];
          while (!p.done() && !p.is(";")) value.push(p.next().v);
          p.expect(";");
          actual[path.join("")] = value.join("");
        }
      }
      p.expect("END_DATA_BLOCK");
      blocks.push({ kind: "db", name, number, optimized, members, ofType, actual });
    } else {
      const t = p.next();
      if (/^(FUNCTION|FUNCTION_BLOCK|ORGANIZATION_BLOCK)$/i.test(t.v)) {
        throw new Error(`Line ${t.line}: this is program code; import it as an .scl source`);
      }
      // Anything else between blocks (stray semicolons, comments already dropped) is skipped.
    }
  }
  if (blocks.length === 0) throw new Error("No DATA_BLOCK or TYPE found in the source");
  return blocks;
}

// ── Layout ──

export interface LaidOutMember {
  /** `Motor.Speed`, `Values[3]`. */
  path: string;
  type: string;
  /** Byte offset, and bit for BOOL. */
  byte: number;
  bit?: number;
  size: number;
  comment: string;
}

export interface Layout {
  size: number;
  members: LaidOutMember[];
  warnings: string[];
}

const ELEMENT_SIZE: Record<string, number> = {
  BYTE: 1, CHAR: 1, SINT: 1, USINT: 1,
  WORD: 2, INT: 2, UINT: 2, DATE: 2, S5TIME: 2, WCHAR: 2,
  DWORD: 4, DINT: 4, UDINT: 4, REAL: 4, TIME: 4, TIME_OF_DAY: 4, TOD: 4,
  LWORD: 8, LINT: 8, ULINT: 8, LREAL: 8, LTIME: 8, DATE_AND_TIME: 8, DT: 8, LTOD: 8, LDT: 8,
  DTL: 12,
};

/** Arrays are expanded member by member up to this many elements; longer ones get one row. */
const MAX_EXPANDED = 64;

const even = (n: number) => n + (n & 1);

class Layouter {
  members: LaidOutMember[] = [];
  warnings: string[] = [];
  constructor(private udts: Map<string, Member[]>) {}

  /**
   * Place `type` at `pos`; returns the position after it, or null if a size is unknown. Records
   * rows when `record` is set.
   */
  place(type: DeclType, pos: { byte: number; bit: number }, path: string, record: boolean, comment = ""): { byte: number; bit: number } | null {
    const upper = type.kind === "elementary" ? type.name.toUpperCase() : "";
    // Leave a run of bits for anything that is not a BOOL.
    const afterBits = () => (pos.bit > 0 ? { byte: pos.byte + 1, bit: 0 } : { ...pos });

    if (type.kind === "elementary" && upper === "BOOL") {
      if (record) this.members.push({ path, type: "Bool", byte: pos.byte, bit: pos.bit, size: 0, comment });
      return pos.bit === 7 ? { byte: pos.byte + 1, bit: 0 } : { byte: pos.byte, bit: pos.bit + 1 };
    }
    if (type.kind === "elementary") {
      const str = upper.match(/^(W?)STRING(?:\[(\d+)\])?$/);
      let size: number | undefined;
      let padded = false;
      if (str) {
        const n = Number(str[2] ?? 254);
        size = str[1] ? 2 * n + 4 : n + 2;
        padded = true;
      } else {
        size = ELEMENT_SIZE[upper];
      }
      if (size === undefined) {
        this.warnings.push(`${path}: size of ${type.name} unknown, offsets stop here`);
        return null;
      }
      let start = afterBits().byte;
      if (size >= 2 || padded) start = even(start);
      if (record) this.members.push({ path, type: type.name, byte: start, size, comment });
      const end = start + (padded ? even(size) : size);
      return { byte: end, bit: 0 };
    }
    if (type.kind === "udt") {
      const members = this.udts.get(type.name);
      if (!members) {
        this.warnings.push(`${path}: data type "${type.name}" is not in this source, offsets stop here`);
        return null;
      }
      return this.place({ kind: "struct", members }, pos, path, record, comment);
    }
    if (type.kind === "struct") {
      const start = even(afterBits().byte);
      let cur: { byte: number; bit: number } | null = { byte: start, bit: 0 };
      for (const m of type.members) {
        cur = this.place(m.type, cur, path ? `${path}.${m.name}` : m.name, record, m.comment);
        if (!cur) return null;
      }
      const end = cur.bit > 0 ? cur.byte + 1 : cur.byte;
      return { byte: start + even(end - start), bit: 0 };
    }
    // Array: elements one after another; BOOL elements are bit-packed.
    const counts = type.dims.map(([lo, hi]) => hi - lo + 1);
    const total = counts.reduce((a, b) => a * b, 1);
    if (total <= 0) {
      this.warnings.push(`${path}: empty array`);
      return null;
    }
    const start = even(afterBits().byte);
    const indexOf = (k: number) => {
      const idx: number[] = [];
      for (let d = type.dims.length - 1; d >= 0; d--) {
        idx.unshift(type.dims[d][0] + (k % counts[d]));
        k = Math.floor(k / counts[d]);
      }
      return idx.join(",");
    };
    const isBool = type.of.kind === "elementary" && type.of.name.toUpperCase() === "BOOL";
    let cur: { byte: number; bit: number } | null = { byte: start, bit: 0 };
    const expand = record && total <= MAX_EXPANDED;
    if (record && !expand) {
      this.members.push({ path: `${path}[${indexOf(0)}..]`, type: `Array of ${typeLabel(type.of)}`, byte: start, size: 0, comment });
    }
    for (let k = 0; k < total; k++) {
      cur = this.place(type.of, cur, `${path}[${indexOf(k)}]`, expand, k === 0 ? comment : "");
      if (!cur) return null;
      // Each element of a struct array is padded by `place`; scalar elements follow each other.
      if (!isBool && cur.bit > 0) cur = { byte: cur.byte + 1, bit: 0 };
    }
    const end = cur.bit > 0 ? cur.byte + 1 : cur.byte;
    return { byte: start + even(end - start), bit: 0 };
  }
}

function typeLabel(t: DeclType): string {
  switch (t.kind) {
    case "elementary": return t.name;
    case "udt": return `"${t.name}"`;
    case "struct": return "Struct";
    case "array": return `Array[${t.dims.map(([a, b]) => `${a}..${b}`).join(",")}] of ${typeLabel(t.of)}`;
  }
}

/** The byte offsets of a DB with standard access. UDTs it uses must be in `blocks`. */
export function layoutDb(db: SourceBlock, blocks: SourceBlock[]): Layout {
  const udts = new Map(blocks.filter((b) => b.kind === "type").map((b) => [b.name, b.members]));
  const l = new Layouter(udts);
  let members = db.members;
  if (members.length === 0 && db.ofType) {
    const udt = udts.get(db.ofType);
    if (!udt) {
      return { size: 0, members: [], warnings: [`${db.name} is an instance of ${db.ofType}, whose declaration is not in this source`] };
    }
    members = udt;
  }
  const end = l.place({ kind: "struct", members }, { byte: 0, bit: 0 }, "", true);
  return { size: end?.byte ?? 0, members: l.members, warnings: l.warnings };
}

/** An S7 address for a laid-out member: `DB5.DBX12.3`, `DB5.DBW12`, ... */
export function memberAddress(dbNumber: number, m: LaidOutMember): string {
  if (m.bit !== undefined) return `DB${dbNumber}.DBX${m.byte}.${m.bit}`;
  if (/^W?STRING/i.test(m.type) || m.size === 1) return `DB${dbNumber}.DBB${m.byte}`;
  if (m.size === 2) return `DB${dbNumber}.DBW${m.byte}`;
  return `DB${dbNumber}.DBD${m.byte}`;
}

export function dbWatchRows(db: SourceBlock, layout: Layout, dbNumber: number) {
  return layout.members.flatMap((m) => {
    const dataType = watchTypeFor(m.type);
    if (!dataType || m.path.endsWith("..]")) return [];
    return [{ name: `${db.name}.${m.path}`, address: memberAddress(dbNumber, m), dataType }];
  });
}

// ── ST output ──

function stType(t: DeclType, warnings: string[], path: string, indent: string): string {
  switch (t.kind) {
    case "elementary": {
      const m = mapElementaryType(t.name);
      if (m.warning) warnings.push(`${path}: ${m.warning}`);
      return m.st;
    }
    case "udt":
      return toIdentifier(t.name);
    case "array":
      return `ARRAY[${t.dims.map(([a, b]) => `${a}..${b}`).join(", ")}] OF ${stType(t.of, warnings, path, indent)}`;
    case "struct":
      return `STRUCT\n${stMembers(t.members, warnings, path, indent + "  ")}${indent}END_STRUCT`;
  }
}

function stMembers(members: Member[], warnings: string[], path: string, indent: string, actual: Record<string, string> = {}): string {
  return members.map((m) => {
    const p = path ? `${path}.${m.name}` : m.name;
    const id = toIdentifier(m.name);
    const init = actual[m.name] ?? actual[`"${m.name}"`] ?? m.initial;
    const comment = [id !== m.name ? `"${m.name}"` : "", m.comment].filter(Boolean).join(": ");
    return `${indent}${id} : ${stType(m.type, warnings, p, indent)}${init !== undefined && m.type.kind !== "struct" ? ` := ${init}` : ""};${comment ? ` (* ${comment.replace(/\*\)/g, "* )")} *)` : ""}\n`;
  }).join("");
}

export interface SourceSt {
  fileName: string;
  st: string;
  warnings: string[];
}

/**
 * Every block as ST: TYPEs as STRUCT types, each DB as a STRUCT type `<Name>_T` (initial values
 * from the BEGIN section where there is one) plus a note on how to declare its instance.
 */
export function sourceToSt(blocks: SourceBlock[], sourceName: string): SourceSt {
  const warnings: string[] = [];
  const parts: string[] = [`(* Imported from ${sourceName} *)`];
  for (const b of blocks) {
    const id = toIdentifier(b.name);
    if (b.kind === "type") {
      parts.push(`TYPE ${id} :\nSTRUCT\n${stMembers(b.members, warnings, b.name, "  ")}END_STRUCT\nEND_TYPE`);
      continue;
    }
    if (b.members.length === 0) {
      if (b.ofType) {
        parts.push(`(* ${b.name} is an instance of ${b.ofType} *)`);
      } else {
        warnings.push(`${b.name}: no declarations`);
      }
      continue;
    }
    parts.push(
      `(* Data block ${b.name}${b.number !== undefined ? ` (DB${b.number})` : ""} *)\n` +
      `TYPE ${id}_T :\nSTRUCT\n${stMembers(b.members, warnings, b.name, "  ", b.actual)}END_STRUCT\nEND_TYPE`,
    );
  }
  const base = toIdentifier(sourceName.replace(/\.[^.]+$/, ""));
  // The data blocks themselves, as instances of their types. A source file also needs a POU to
  // compile, so this is emitted even when there are only TYPEs.
  const instances = blocks
    .filter((b) => b.kind === "db" && (b.members.length > 0 || b.ofType))
    .map((b) => `  ${toIdentifier(b.name)} : ${b.members.length > 0 ? `${toIdentifier(b.name)}_T` : toIdentifier(b.ofType!)};`);
  parts.push(`PROGRAM ${base}_Data${instances.length ? `\nVAR\n${instances.join("\n")}\nEND_VAR` : ""}\nEND_PROGRAM`);
  return { fileName: `${base}.st`, st: parts.join("\n\n") + "\n", warnings };
}
