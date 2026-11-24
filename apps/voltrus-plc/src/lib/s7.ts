// Siemens S7 online functions (desktop only): types and calls into the Tauri commands in
// src-tauri/src/commands/siemens, plus the watch-table rows kept between sessions.
import { invoke } from "../api/bridge";

export interface S7Target {
  host: string;
  port: number;
  rack: number;
  slot: number;
}

export interface CpuInfo {
  module_type: string;
  serial_number: string;
  as_name: string;
  module_name: string;
  firmware_version: string;
  order_code: string;
  copyright: string;
}

export interface OnlineInfo {
  target: S7Target;
  pdu_size: number;
  cpu: CpuInfo | null;
  cpu_error: string | null;
  mode: string | null;
}

export interface OnlineStatus {
  online: boolean;
  target: S7Target | null;
  mode: string | null;
  error: string | null;
}

export interface FoundDevice {
  ip: string;
  port: number;
  rack: number | null;
  slot: number | null;
  module_type: string | null;
  order_code: string | null;
  station_name: string | null;
  module_name: string | null;
  firmware: string | null;
  note: string | null;
}

export interface DiagEntry {
  event_number: number;
  timestamp: string;
  event_text: string;
  priority_class: number;
  ob_number: number;
}

export interface DiagBuffer {
  entry_count: number;
  entries: DiagEntry[];
}

export interface BlockEntry {
  block_type: string;
  block_number: number;
  language: string;
}

export interface WatchRow {
  id: string;
  /** Symbol name, for rows that came from an imported tag table or DB. */
  name?: string;
  address: string;
  dataType: string;
  /** Value to write with "Modify". */
  modify: string;
}

export interface WatchResult {
  value: string | null;
  error: string | null;
}

export const WATCH_TYPES = [
  "BOOL", "BYTE", "WORD", "DWORD", "LWORD",
  "SINT", "USINT", "INT", "UINT", "DINT", "UDINT", "LINT", "ULINT",
  "REAL", "LREAL", "TIME", "CHAR", "STRING", "S5TIME", "COUNTER",
] as const;

export const s7 = {
  goOnline: (t: S7Target) => invoke<OnlineInfo>("s7_go_online", { host: t.host, port: t.port, rack: t.rack, slot: t.slot }),
  goOffline: () => invoke<void>("s7_go_offline"),
  status: () => invoke<OnlineStatus>("s7_status"),
  diagBuffer: () => invoke<DiagBuffer>("s7_diag_buffer"),
  listBlocks: () => invoke<BlockEntry[]>("s7_list_blocks"),
  setMode: (run: boolean, cold = false) => invoke<string>("s7_set_mode", { run, cold }),
  readWatch: (rows: WatchRow[]) =>
    invoke<WatchResult[]>("s7_read_watch", {
      items: rows.map((r) => ({ address: r.address, data_type: r.dataType || null })),
    }),
  writeWatch: (row: WatchRow, value: string) =>
    invoke<void>("s7_write_watch", { address: row.address, dataType: row.dataType || null, value }),
  scan: (subnet: string) => invoke<FoundDevice[]>("s7_scan", { subnet }),
  suggestSubnet: () => invoke<string | null>("s7_suggest_subnet"),
};

/**
 * The display type an address implies when the user has not picked one: the bit, byte, word or
 * double word in it (mirrors `DisplayType::default_for` in values.rs).
 */
export function defaultTypeFor(address: string): string {
  const a = address.trim().replace(/^%/, "").toUpperCase();
  if (/^T\d+$/.test(a)) return "S5TIME";
  if (/^C\d+$/.test(a)) return "COUNTER";
  if (/^DB\d+\.DBX\d+\.\d+$/.test(a) || /^[IQM]\d+\.\d+$/.test(a)) return "BOOL";
  if (/^DB\d+\.DBB\d+$/.test(a) || /^[IQM]B?\d+$/.test(a)) return "BYTE";
  if (/^DB\d+\.DBW\d+$/.test(a) || /^[IQM]W\d+$/.test(a)) return "INT";
  if (/^DB\d+\.DBD\d+$/.test(a) || /^[IQM]D\d+$/.test(a)) return "DINT";
  return "";
}

export function modeTone(mode: string | null | undefined): "success" | "warn" | "danger" | "neutral" {
  switch (mode) {
    case "RUN": return "success";
    case "STOP": return "warn";
    case "DEFECT": return "danger";
    default: return "neutral";
  }
}

const TARGET_KEY = "vplc_s7_target";
const WATCH_KEY = "vplc_s7_watch";

export function loadTarget(): S7Target {
  try {
    const t = JSON.parse(localStorage.getItem(TARGET_KEY) ?? "null");
    if (t && typeof t.host === "string") {
      return { host: t.host, port: Number(t.port) || 102, rack: Number(t.rack) || 0, slot: Number(t.slot) || 0 };
    }
  } catch { /* fall through */ }
  return { host: "", port: 102, rack: 0, slot: 1 };
}

export function saveTarget(t: S7Target) {
  try { localStorage.setItem(TARGET_KEY, JSON.stringify(t)); } catch { /* session only */ }
}

export function loadWatchRows(): WatchRow[] {
  try {
    const rows = JSON.parse(localStorage.getItem(WATCH_KEY) ?? "[]");
    if (Array.isArray(rows)) {
      return rows
        .filter((r) => r && typeof r.address === "string")
        .map((r) => ({
          id: String(r.id ?? crypto.randomUUID()),
          name: typeof r.name === "string" && r.name ? r.name : undefined,
          address: r.address,
          dataType: String(r.dataType ?? ""),
          modify: String(r.modify ?? ""),
        }));
    }
  } catch { /* fall through */ }
  return [];
}

export function saveWatchRows(rows: WatchRow[]) {
  try { localStorage.setItem(WATCH_KEY, JSON.stringify(rows)); } catch { /* session only */ }
}

/** Fired on window when the saved watch rows change outside the Online panel (an import). */
export const WATCH_ROWS_EVENT = "s7-watch-rows-changed";

/**
 * Add rows to the saved watch table, skipping addresses already in it. Returns how many were
 * added.
 */
export function appendWatchRows(rows: { name?: string; address: string; dataType: string }[]): number {
  const current = loadWatchRows();
  const have = new Set(current.map((r) => `${r.address.toUpperCase()}|${r.dataType}`));
  const added: WatchRow[] = [];
  for (const r of rows) {
    const key = `${r.address.toUpperCase()}|${r.dataType}`;
    if (have.has(key)) continue;
    have.add(key);
    added.push({ id: crypto.randomUUID(), name: r.name, address: r.address, dataType: r.dataType, modify: "" });
  }
  if (added.length > 0) {
    saveWatchRows([...current, ...added]);
    window.dispatchEvent(new Event(WATCH_ROWS_EVENT));
  }
  return added.length;
}
