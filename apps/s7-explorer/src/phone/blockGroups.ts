import type { BlockListEntry } from "../useBlocks";

export interface BlockGroup {
  type: string;
  name: string;
  blocks: BlockListEntry[];
}

const NAMES: Record<string, string> = {
  OB: "Organization blocks",
  FC: "Functions",
  FB: "Function blocks",
  DB: "Data blocks",
  SDB: "System data blocks",
  SFC: "System functions",
  SFB: "System function blocks",
};

const SINGULAR: Record<string, string> = {
  OB: "Organization block",
  FC: "Function",
  FB: "Function block",
  DB: "Data block",
  SDB: "System data block",
  SFC: "System function",
  SFB: "System function block",
};

const ORDER = ["OB", "FC", "FB", "DB", "SDB", "SFC", "SFB"];

export const blockTypeOf = (b: BlockListEntry) => b.block_type.split("/")[0];

export const blockTypeName = (type: string) => NAMES[type] ?? type;

export const blockKindName = (type: string) => SINGULAR[type] ?? type;

/** Blocks grouped by type in the order OB, FC, FB, DB, then the rest; numbers ascending inside a group. */
export function groupBlocks(blocks: readonly BlockListEntry[]): BlockGroup[] {
  const byType = new Map<string, BlockListEntry[]>();
  for (const b of blocks) {
    const type = blockTypeOf(b);
    const list = byType.get(type);
    if (list) list.push(b);
    else byType.set(type, [b]);
  }
  const rank = (t: string) => {
    const i = ORDER.indexOf(t);
    return i === -1 ? ORDER.length : i;
  };
  return [...byType.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([type, list]) => ({ type, name: blockTypeName(type), blocks: [...list].sort((a, b) => a.block_number - b.block_number) }));
}

/** The 16 byte addresses of a data block window, `DB10.DBB0`..`DB10.DBB15`. */
export function dbWindowAddresses(dbNumber: number, start: number, length = 16): string[] {
  return Array.from({ length }, (_, i) => `DB${dbNumber}.DBB${start + i}`);
}

/** Bytes as rows of four: `[offset, "00 1F 3C A0"]`. A byte that did not read shows as `--`. */
export function byteRows(start: number, values: ReadonlyArray<unknown>, perRow = 4): Array<[number, string]> {
  const rows: Array<[number, string]> = [];
  for (let i = 0; i < values.length; i += perRow) {
    const cells = values.slice(i, i + perRow).map((v) => (typeof v === "number" ? v.toString(16).padStart(2, "0").toUpperCase() : "--"));
    rows.push([start + i, cells.join(" ")]);
  }
  return rows;
}
