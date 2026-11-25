// Import from engineering-software exports: decide what a file is and turn it into project files
// (ST) plus, where addresses are known, rows for the Online watch table.
import { dbWatchRows, layoutDb, parseDbSource, sourceToSt, type Layout, type SourceBlock } from "./dbSource";
import { sclToSt } from "./scl";
import { parseTagTableXlsx, parseTagTableXml, tagTableToSt, tagTableWatchRows, type ImportedWatchRow, type TagTable } from "./tagTable";
import { toIdentifier } from "./common";

export type { ImportedWatchRow } from "./tagTable";

export interface ImportedFile {
  name: string;
  content: string;
}

/** A DB whose rows depend on a DB number the user may have to enter. */
export interface ImportedDb {
  name: string;
  number?: number;
  optimized: boolean;
  layout: Layout;
  block: SourceBlock;
}

export interface ImportPlan {
  kind: "tag-table" | "db-source" | "scl";
  summary: string;
  files: ImportedFile[];
  watchRows: ImportedWatchRow[];
  /** Tag tables as read, for the device's tag-table documents. */
  tagTables: TagTable[];
  dbs: ImportedDb[];
  warnings: string[];
}

export const ACCEPTED_EXTENSIONS = ".xml,.xlsx,.db,.udt,.scl,.awl,.txt";

function extensionOf(name: string) {
  return name.toLowerCase().match(/\.([a-z0-9]+)$/)?.[1] ?? "";
}

/** Read one exported file. `bytes` is the raw file; text formats are decoded as UTF-8. */
export async function planImport(fileName: string, bytes: Uint8Array): Promise<ImportPlan> {
  const ext = extensionOf(fileName);
  if (ext === "xlsx") {
    return tagTablesPlan(await parseTagTableXlsx(bytes));
  }
  const text = new TextDecoder().decode(bytes).replace(/^\uFEFF/, "");
  if (ext === "xml" || text.trimStart().startsWith("<")) {
    return tagTablesPlan(parseTagTableXml(text));
  }
  const looksLikeCode = /^\s*(FUNCTION_BLOCK|FUNCTION|ORGANIZATION_BLOCK)\b/im.test(text) && !/^\s*DATA_BLOCK\b/im.test(text);
  if (ext === "scl" || looksLikeCode) {
    const r = sclToSt(text);
    const base = r.pous[0] ?? toIdentifier(fileName.replace(/\.[^.]+$/, ""));
    return {
      kind: "scl",
      summary: `${r.pous.length} block(s): ${r.pous.join(", ") || "none"}`,
      files: [{ name: `${base}.st`, content: r.st.endsWith("\n") ? r.st : `${r.st}\n` }],
      watchRows: [],
      tagTables: [],
      dbs: [],
      warnings: r.warnings,
    };
  }
  return dbSourcePlan(fileName, text);
}

function tagTablesPlan(tables: Awaited<ReturnType<typeof parseTagTableXlsx>>): ImportPlan {
  const files: ImportedFile[] = [];
  const warnings: string[] = [];
  const watchRows: ImportedWatchRow[] = [];
  let tagCount = 0;
  for (const t of tables) {
    const st = tagTableToSt(t);
    files.push({ name: st.fileName, content: st.st });
    warnings.push(...st.warnings);
    watchRows.push(...tagTableWatchRows(t));
    tagCount += t.tags.length + t.constants.length;
  }
  return {
    kind: "tag-table",
    summary: `${tables.length} tag table(s), ${tagCount} tag(s), ${watchRows.length} with an address`,
    files,
    watchRows,
    tagTables: tables,
    dbs: [],
    warnings,
  };
}

function dbSourcePlan(fileName: string, text: string): ImportPlan {
  const blocks = parseDbSource(text);
  const st = sourceToSt(blocks, fileName);
  const warnings = [...st.warnings];
  const dbs: ImportedDb[] = blocks
    .filter((b) => b.kind === "db")
    .map((b) => {
      const layout = layoutDb(b, blocks);
      if (b.optimized) {
        warnings.push(`${b.name} uses optimized access: its members have no fixed offsets and cannot be read over PUT/GET`);
      } else {
        warnings.push(...layout.warnings);
      }
      return { name: b.name, number: b.number, optimized: b.optimized, layout, block: b };
    });
  const types = blocks.length - dbs.length;
  return {
    kind: "db-source",
    summary: `${dbs.length} data block(s), ${types} data type(s)`,
    files: [{ name: st.fileName, content: st.st }],
    watchRows: [],
    tagTables: [],
    dbs,
    warnings,
  };
}

/** Watch rows of the non-optimized DBs, with the DB numbers the user confirmed. */
export function dbRows(dbs: ImportedDb[], numbers: Record<string, number>): ImportedWatchRow[] {
  return dbs.flatMap((db) => {
    const n = numbers[db.name] ?? db.number;
    if (db.optimized || n === undefined || !Number.isInteger(n) || n < 1) return [];
    return dbWatchRows(db.block, db.layout, n);
  });
}
