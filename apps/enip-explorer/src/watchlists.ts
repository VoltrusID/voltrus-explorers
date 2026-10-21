// Saved tag watch lists, persisted in localStorage, with CSV export and import.
//
// A watch list is a named set of tag names. Applying one makes the scanner poll those tags first
// (and only those, in Pro); every other tag stays readable on demand. Lists are matched by tag
// name, so one list can be reused across PLCs that share tag names.

export interface WatchList {
  name: string;
  tags: string[];
}

const KEY = "enip-explorer:watchlists:v1";

/** Saved lists the free tier may keep; more needs Pro. */
export const FREE_WATCHLIST_LIMIT = 1;

export function loadWatchLists(): WatchList[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr
      .filter((l): l is WatchList => !!l && typeof l.name === "string" && Array.isArray(l.tags))
      .map((l) => ({ name: l.name, tags: l.tags.filter((t: unknown): t is string => typeof t === "string") }));
  } catch {
    return [];
  }
}

function persist(lists: WatchList[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(lists));
  } catch {
    // Ignore quota / private-mode errors: watch lists are a convenience.
  }
}

/** Insert or replace by name, keeping the list order stable. */
export function mergeWatchLists(current: WatchList[], incoming: WatchList[]): WatchList[] {
  const next = current.map((l) => ({ ...l }));
  for (const l of incoming) {
    const i = next.findIndex((x) => x.name === l.name);
    if (i >= 0) next[i] = { name: l.name, tags: [...l.tags] };
    else next.push({ name: l.name, tags: [...l.tags] });
  }
  return next;
}

export function upsertWatchLists(lists: WatchList[]): WatchList[] {
  const next = mergeWatchLists(loadWatchLists(), lists);
  persist(next);
  return next;
}

export function deleteWatchList(name: string): WatchList[] {
  const next = loadWatchLists().filter((l) => l.name !== name);
  persist(next);
  return next;
}

export function sameTags(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((t, i) => t === b[i]);
}

// ── CSV ──

function esc(v: string): string {
  return /[",\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

/** One row per tag: `list,tag`. Lists with no tags are skipped. */
export function watchListsToCsv(lists: WatchList[]): string {
  const rows = ["list,tag"];
  for (const l of lists) for (const t of l.tags) rows.push(`${esc(l.name)},${esc(t)}`);
  return rows.join("\n") + "\n";
}

/** RFC 4180 style parser: quoted fields, doubled quotes, CRLF or LF, embedded newlines. */
export function parseCsvRows(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  const src = text.replace(/^\uFEFF/, "");
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (quoted) {
      if (c === '"') {
        if (src[i + 1] === '"') { field += '"'; i++; } else quoted = false;
      } else field += c;
    } else if (c === '"' && field === "") {
      quoted = true;
    } else if (c === ",") {
      row.push(field); field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && src[i + 1] === "\n") i++;
      row.push(field); field = "";
      rows.push(row); row = [];
    } else field += c;
  }
  if (field !== "" || row.length > 0) { row.push(field); rows.push(row); }
  return rows.filter((r) => r.some((c) => c.trim() !== ""));
}

/**
 * Read watch lists from CSV text.
 *
 * Accepted shapes: a `list,tag` file (what Export writes, header optional), a file with a `tag`
 * column and an optional `list` column, or a plain column of tag names. Rows without a list name go
 * to `fallbackName`. Tags are trimmed, de-duplicated and kept in file order.
 */
export function parseWatchListCsv(text: string, fallbackName: string): WatchList[] {
  const rows = parseCsvRows(text);
  if (rows.length === 0) return [];

  let listCol = -1;
  let tagCol = 0;
  let start = 0;
  const head = rows[0].map((c) => c.trim().toLowerCase());
  if (head.includes("tag") || head.includes("tag name") || head.includes("name")) {
    tagCol = head.findIndex((c) => c === "tag" || c === "tag name");
    if (tagCol < 0) tagCol = head.indexOf("name");
    listCol = head.indexOf("list");
    if (listCol < 0) listCol = head.indexOf("list name");
    start = 1;
  } else if (head.includes("list")) {
    // Header with a list column but no recognisable tag column: treat the other column as the tag.
    listCol = head.indexOf("list");
    tagCol = head.findIndex((_, i) => i !== listCol);
    if (tagCol < 0) return [];
    start = 1;
  } else if (rows[0].length >= 2) {
    listCol = 0;
    tagCol = 1;
  }

  const order: string[] = [];
  const byName = new Map<string, string[]>();
  for (const r of rows.slice(start)) {
    const tag = (r[tagCol] ?? "").trim();
    if (!tag) continue;
    const name = (listCol >= 0 ? (r[listCol] ?? "").trim() : "") || fallbackName;
    let tags = byName.get(name);
    if (!tags) { tags = []; byName.set(name, tags); order.push(name); }
    if (!tags.includes(tag)) tags.push(tag);
  }
  return order.map((name) => ({ name, tags: byName.get(name)! }));
}
