// Project documents for the Siemens device: watch tables (`.s7watch`) and tag tables (`.s7tags`,
// PLC tags or the layout of a standard-access DB). Both are JSON files saved with the project and
// opened as editor tabs; the Online & diagnostics view is a tab without a file.
import { watchTypeFor, s7WatchAddress } from "./plcImport/common";
import { layoutDb, memberAddress, parseDbSource, sourceToSt, type DeclType, type Layout, type Member, type SourceBlock } from "./plcImport/dbSource";
import type { TagTable } from "./plcImport/tagTable";
import { defaultTypeFor } from "./s7";

export const WATCH_EXT = ".s7watch";
export const TAGS_EXT = ".s7tags";
/** Path of the Online & diagnostics tab. Paths starting with `s7:` are never saved. */
export const ONLINE_PATH = "s7:online";

export const isWatchDoc = (path: string) => path.endsWith(WATCH_EXT);
export const isTagDoc = (path: string) => path.endsWith(TAGS_EXT);
export const isVirtualDoc = (path: string) => path.startsWith("s7:");
/** Device documents are not program source: the compiler and the Project list skip them. */
export const isDeviceDoc = (path: string) => isVirtualDoc(path) || isWatchDoc(path) || isTagDoc(path) || path.endsWith(".s7hw");

/** File name without folder and extension. */
export function docTitle(path: string): string {
  return (path.split("/").pop() ?? path).replace(/\.(s7watch|s7tags|s7hw)$/, "");
}

// ── Watch tables ──

export interface WatchDocRow {
  /** Symbol from a tag table, if the row was added by name. */
  name?: string;
  address: string;
  dataType: string;
  /** Value written by Modify. */
  modify: string;
  /** Ticked rows are written by "Modify now" and by permanent modify. */
  modifyOn?: boolean;
  comment?: string;
}

export interface WatchDoc {
  format: "voltrus-s7-watch";
  version: 1;
  rows: WatchDocRow[];
}

export function emptyWatchDoc(rows: WatchDocRow[] = []): WatchDoc {
  return { format: "voltrus-s7-watch", version: 1, rows };
}

export function parseWatchDoc(text: string): WatchDoc {
  if (!text.trim()) return emptyWatchDoc();
  const raw = JSON.parse(text);
  const rows = Array.isArray(raw?.rows) ? raw.rows : [];
  return emptyWatchDoc(
    rows
      .filter((r: unknown): r is Record<string, unknown> => !!r && typeof r === "object")
      .map((r: Record<string, unknown>) => ({
        name: typeof r.name === "string" && r.name ? r.name : undefined,
        address: String(r.address ?? ""),
        dataType: String(r.dataType ?? ""),
        modify: String(r.modify ?? ""),
        modifyOn: r.modifyOn === true ? true : undefined,
        comment: typeof r.comment === "string" && r.comment ? r.comment : undefined,
      })),
  );
}

export function serializeWatchDoc(doc: WatchDoc): string {
  return `${JSON.stringify(doc, null, 2)}\n`;
}

// ── Tag tables ──

export interface TagDocRow {
  name: string;
  /** Data type as written in the source (`Bool`, `Int`, `String[20]`). */
  dataType: string;
  /** Absolute address (`I0.0`, `MW10`, `DB5.DBD4`), or "" when it has none. */
  address: string;
  comment: string;
  /** DB members: byte offset, bit for BOOL, size in bytes. Addresses follow from the DB number. */
  byte?: number;
  bit?: number;
  size?: number;
}

export interface TagDocDb {
  name: string;
  number?: number;
  optimized: boolean;
  size: number;
}

/** A top-level member of a DB as declared: the type is source text (`Int`, `Array[0..9] of Bool`). */
export interface DeclRow {
  name: string;
  type: string;
  initial?: string;
  comment: string;
}

export interface TagDoc {
  format: "voltrus-s7-tags";
  version: 1;
  kind: "tags" | "db";
  db?: TagDocDb;
  /** DBs: the declaration the rows are laid out from (editable), and the UDT sources it uses. */
  decl?: DeclRow[];
  types?: string;
  rows: TagDocRow[];
}

export function emptyTagDoc(): TagDoc {
  return { format: "voltrus-s7-tags", version: 1, kind: "tags", rows: [] };
}

const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : undefined);

export function parseTagDoc(text: string): TagDoc {
  if (!text.trim()) return emptyTagDoc();
  const raw = JSON.parse(text);
  const kind = raw?.kind === "db" ? "db" : "tags";
  const rows: TagDocRow[] = (Array.isArray(raw?.rows) ? raw.rows : [])
    .filter((r: unknown): r is Record<string, unknown> => !!r && typeof r === "object")
    .map((r: Record<string, unknown>) => ({
      name: String(r.name ?? ""),
      dataType: String(r.dataType ?? ""),
      address: String(r.address ?? ""),
      comment: String(r.comment ?? ""),
      byte: num(r.byte),
      bit: num(r.bit),
      size: num(r.size),
    }));
  const db = raw?.db && typeof raw.db === "object"
    ? {
        name: String(raw.db.name ?? ""),
        number: num(raw.db.number),
        optimized: raw.db.optimized === true,
        size: num(raw.db.size) ?? 0,
      }
    : undefined;
  const decl: DeclRow[] | undefined = kind === "db" && Array.isArray(raw?.decl)
    ? raw.decl
        .filter((r: unknown): r is Record<string, unknown> => !!r && typeof r === "object")
        .map((r: Record<string, unknown>) => ({
          name: String(r.name ?? ""),
          type: String(r.type ?? ""),
          initial: typeof r.initial === "string" && r.initial ? r.initial : undefined,
          comment: String(r.comment ?? ""),
        }))
    : undefined;
  const types = kind === "db" && typeof raw?.types === "string" && raw.types ? raw.types : undefined;
  return { format: "voltrus-s7-tags", version: 1, kind, db: kind === "db" ? db : undefined, decl, types, rows };
}

export function serializeTagDoc(doc: TagDoc): string {
  // Leave out undefined fields so the files stay small and diff cleanly.
  return `${JSON.stringify(doc, (_k, v) => (v === undefined ? undefined : v), 2)}\n`;
}

/** The offset column of a DB member: `12.3` for a BOOL, `12.0` otherwise. */
export function offsetText(r: TagDocRow): string {
  return r.byte === undefined ? "" : `${r.byte}.${r.bit ?? 0}`;
}

/** Give every DB member the address its offset has in DB `number` (none without a number). */
export function withDbNumber(doc: TagDoc, number: number | undefined): TagDoc {
  if (doc.kind !== "db" || !doc.db) return doc;
  const valid = number !== undefined && Number.isInteger(number) && number > 0;
  return {
    ...doc,
    db: { ...doc.db, number: valid ? number : undefined },
    rows: doc.rows.map((r) => ({
      ...r,
      address: valid && !doc.db!.optimized && r.byte !== undefined
        ? memberAddress(number, { path: r.name, type: r.dataType.toUpperCase(), byte: r.byte, bit: r.bit, size: r.size ?? 0, comment: "" })
        : "",
    })),
  };
}

export function tagDocFromTable(t: TagTable): TagDoc {
  return {
    format: "voltrus-s7-tags",
    version: 1,
    kind: "tags",
    rows: t.tags.map((tag) => ({
      name: tag.name,
      dataType: tag.dataType,
      address: tag.address ? (s7WatchAddress(tag.address) ?? tag.address.replace(/^%/, "")) : "",
      comment: tag.comment,
    })),
  };
}

export function tagDocFromDb(
  db: { name: string; number?: number; optimized: boolean; layout: Layout; block?: SourceBlock; types?: SourceBlock[] },
  number?: number,
): TagDoc {
  const doc: TagDoc = {
    format: "voltrus-s7-tags",
    version: 1,
    kind: "db",
    db: { name: db.name, number: undefined, optimized: db.optimized, size: db.layout.size },
    decl: db.block && !db.block.ofType
      ? db.block.members.map((m) => ({ name: m.name, type: declTypeText(m.type), initial: db.block!.actual[m.name] ?? m.initial, comment: m.comment }))
      : undefined,
    types: db.types?.length ? db.types.map(typeBlockText).join("\n") : undefined,
    rows: db.layout.members.map((m) => ({
      name: m.path,
      dataType: m.type,
      address: "",
      comment: m.comment,
      byte: db.optimized ? undefined : m.byte,
      bit: db.optimized ? undefined : m.bit,
      size: m.size,
    })),
  };
  return withDbNumber(doc, number ?? db.number);
}

/** The watch display type for a tag: from its declared type, else from its address. */
export function tagWatchType(r: TagDocRow): string {
  return watchTypeFor(r.dataType) || defaultTypeFor(r.address);
}

/** The symbol a watch row or tag is known by: `Motor_On`, or `Motor_DB.Speed` for DB members. */
export function symbolOf(doc: TagDoc, r: TagDocRow): string {
  return doc.kind === "db" && doc.db ? `${doc.db.name}.${r.name}` : r.name;
}

export interface Symbol {
  name: string;
  address: string;
  dataType: string;
}

/** Every tag with an address, by symbol, from a set of tag tables. */
export function symbolTable(docs: TagDoc[]): Map<string, Symbol> {
  const out = new Map<string, Symbol>();
  for (const doc of docs) {
    for (const r of doc.rows) {
      if (!r.address) continue;
      const name = symbolOf(doc, r);
      const key = name.toUpperCase();
      if (!out.has(key)) out.set(key, { name, address: r.address, dataType: tagWatchType(r) });
    }
  }
  return out;
}

/** Look up `"Motor_On"`, `Motor_On` or `Motor_DB.Speed` (quotes and case ignored). */
export function resolveSymbol(text: string, symbols: Map<string, Symbol>): Symbol | undefined {
  const name = text.trim().replace(/"/g, "");
  return name ? symbols.get(name.toUpperCase()) : undefined;
}

/** Watch rows for tags, skipping ones without an address or with a type that cannot be shown. */
export function watchRowsForTags(doc: TagDoc, rows: TagDocRow[]): WatchDocRow[] {
  return rows.flatMap((r) => {
    const dataType = tagWatchType(r);
    return r.address && dataType ? [{ name: symbolOf(doc, r), address: r.address, dataType, modify: "" }] : [];
  });
}

/** Add rows to a watch table, skipping addresses it already watches with the same type. */
export function appendToWatchDoc(doc: WatchDoc, rows: WatchDocRow[]): { doc: WatchDoc; added: number } {
  const have = new Set(doc.rows.map((r) => `${r.address.toUpperCase()}|${r.dataType}`));
  const added: WatchDocRow[] = [];
  for (const r of rows) {
    const key = `${r.address.toUpperCase()}|${r.dataType}`;
    if (have.has(key)) continue;
    have.add(key);
    added.push(r);
  }
  return { doc: { ...doc, rows: [...doc.rows, ...added] }, added: added.length };
}

/** `Watch table_1`, `Watch table_2`, ...: the first name not taken. */
export function nextDocName(base: string, ext: string, taken: string[]): string {
  const names = new Set(taken.map((t) => (t.split("/").pop() ?? t).toLowerCase()));
  for (let i = 1; ; i++) {
    const name = `${base}_${i}${ext}`;
    if (!names.has(name.toLowerCase())) return name;
  }
}

// ── DB declarations ──

/** A declared type back as source text. */
export function declTypeText(t: DeclType): string {
  switch (t.kind) {
    case "elementary": return t.name;
    case "udt": return `"${t.name}"`;
    case "array": return `Array[${t.dims.map(([a, b]) => `${a}..${b}`).join(", ")}] of ${declTypeText(t.of)}`;
    case "struct": return `Struct ${t.members.map(memberText).join(" ")} End_Struct`;
  }
}

function memberText(m: Member): string {
  const name = /^[A-Za-z_][A-Za-z0-9_]*$/.test(m.name) ? m.name : `"${m.name}"`;
  return `${name} : ${declTypeText(m.type)}${m.initial !== undefined ? ` := ${m.initial}` : ""};`;
}

function typeBlockText(b: SourceBlock): string {
  return `TYPE "${b.name}"\n  STRUCT\n${b.members.map((m) => `    ${memberText(m)}${m.comment ? ` // ${m.comment}` : ""}`).join("\n")}\n  END_STRUCT;\nEND_TYPE\n`;
}

/** The DB as a source file: its UDTs, then the DATA_BLOCK with the declaration. */
export function dbSourceText(doc: TagDoc): string {
  const db = doc.db!;
  const lines = (doc.decl ?? []).map((r) => {
    const name = /^[A-Za-z_][A-Za-z0-9_]*$/.test(r.name) ? r.name : `"${r.name}"`;
    const comment = r.comment.replace(/\n/g, " ");
    return `      ${name} : ${r.type}${r.initial ? ` := ${r.initial}` : ""};${comment ? `   // ${comment}` : ""}`;
  });
  return `${doc.types ?? ""}
DATA_BLOCK "${db.name}"
{ S7_Optimized_Access := '${db.optimized ? "TRUE" : "FALSE"}' }
VERSION : 0.1
   STRUCT
${lines.join("\n")}
   END_STRUCT;
BEGIN
END_DATA_BLOCK
`;
}

/**
 * Lay a DB out again from its declaration: new rows with offsets (and addresses, with a DB
 * number). On a declaration that does not parse, the rows stay as they were and the error is given.
 */
export function relayoutDb(doc: TagDoc): { doc: TagDoc; error: string | null } {
  if (doc.kind !== "db" || !doc.db || !doc.decl) return { doc, error: null };
  try {
    const blocks = parseDbSource(dbSourceText(doc));
    const block = blocks.find((b) => b.kind === "db");
    if (!block) return { doc, error: "No data block in the declaration" };
    const layout = layoutDb(block, blocks);
    const next: TagDoc = {
      ...doc,
      db: { ...doc.db, size: layout.size },
      rows: layout.members.map((m) => ({
        name: m.path,
        dataType: m.type,
        address: "",
        comment: m.comment,
        byte: doc.db!.optimized ? undefined : m.byte,
        bit: doc.db!.optimized ? undefined : m.bit,
        size: m.size,
      })),
    };
    return { doc: withDbNumber(next, doc.db.number), error: layout.warnings[0] ?? null };
  } catch (e) {
    return { doc, error: e instanceof Error ? e.message : String(e) };
  }
}

/** ST for the DB (types plus a program declaring it), as the DB-source import writes it. */
export function dbToSt(doc: TagDoc): { fileName: string; st: string } {
  const st = sourceToSt(parseDbSource(dbSourceText(doc)), `${doc.db?.name ?? "DB"}.db`);
  return { fileName: st.fileName, st: st.st };
}

/** A new, empty DB with standard access. */
export function newDbDoc(name: string): TagDoc {
  return relayoutDb({
    format: "voltrus-s7-tags",
    version: 1,
    kind: "db",
    db: { name, optimized: false, size: 0 },
    decl: [{ name: "Value", type: "Int", comment: "" }],
    rows: [],
  }).doc;
}
