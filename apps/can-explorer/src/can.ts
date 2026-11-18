import type { BusStatus, Dir, FrameIn, FrameView, InterfaceConfig, InterfaceKind } from "./types";

// ── Tiers ──

export const FREE_PROFILE_LIMIT = 1;
/** Frames kept in the trace (both tiers); older ones scroll out. */
export const TRACE_MAX = 20000;

// ── Identifiers and bytes ──

/** Map key keeping 11-bit and 29-bit identifiers apart. */
export const keyOf = (id: number, ext: boolean) => (ext ? `x${id}` : `s${id}`);

export const idText = (id: number, ext: boolean) =>
  ext ? id.toString(16).toUpperCase().padStart(8, "0") : id.toString(16).toUpperCase().padStart(3, "0");

export const hex = (data: number[]) => data.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join(" ");

export const ascii = (data: number[]) => data.map((b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".")).join("");

/**
 * Reads an identifier typed by the user: hex, with or without `0x`. More than three digits or a value
 * above 0x7FF means 29-bit, as cansend decides.
 */
export function parseId(text: string): { id: number; ext: boolean } | string {
  const t = text.trim().replace(/^0x/i, "");
  if (!/^[0-9a-f]{1,8}$/i.test(t)) return "The identifier is 1 to 8 hex digits, e.g. 7DF or 18FEF100.";
  const id = parseInt(t, 16);
  if (id > 0x1fffffff) return "A 29-bit identifier is at most 1FFFFFFF.";
  return { id, ext: t.length > 3 || id > 0x7ff };
}

const FD_LENGTHS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/** Hex bytes typed by the user: `DE AD BE EF`, `DEADBEEF`, `de:ad`. */
export function parseData(text: string): number[] | string {
  const t = text.replace(/[\s:,.]/g, "").replace(/^0x/i, "");
  if (t.length % 2 !== 0) return "Data needs an even number of hex digits.";
  if (t && !/^[0-9a-f]+$/i.test(t)) return "Data is hex bytes, e.g. 02 01 0C.";
  const out: number[] = [];
  for (let i = 0; i < t.length; i += 2) out.push(parseInt(t.slice(i, i + 2), 16));
  return out;
}

/** The problem with a frame, or null if it can be sent. */
export function frameProblem(f: FrameIn): string | null {
  const data = parseData(f.data);
  if (typeof data === "string") return data;
  if (f.id > (f.ext ? 0x1fffffff : 0x7ff)) return `The identifier does not fit in ${f.ext ? 29 : 11} bits.`;
  if (f.fd) {
    if (f.rtr) return "CAN FD has no remote frames.";
    if (!FD_LENGTHS.includes(data.length)) return "A CAN FD frame carries 0 to 8, 12, 16, 20, 24, 32, 48 or 64 bytes.";
  } else {
    if (f.brs) return "Bit rate switch needs a CAN FD frame.";
    if (data.length > 8) return "A classic CAN frame carries at most 8 bytes. Tick CAN FD for more.";
    if (f.rtr && data.length > 0) return "A remote frame carries no data.";
  }
  return null;
}

/** cansend syntax: `123#DEADBEEF`, `123#R`, `123##1AABB` (CAN FD flags digit: 1 = bit rate switch, 2 = error state indicator). */
export function cansendText(f: { id: number; ext: boolean; rtr: boolean; fd: boolean; brs: boolean; esi?: boolean; data: number[]; dlc?: number }): string {
  const id = idText(f.id, f.ext);
  const d = f.data.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join("");
  if (f.rtr) return `${id}#R${f.dlc ? f.dlc : ""}`;
  if (f.fd) return `${id}##${(f.brs ? 1 : 0) | (f.esi ? 2 : 0)}${d}`;
  return `${id}#${d}`;
}

// ── Monitor (one row per identifier) ──

export interface MonitorRow {
  key: string;
  id: number;
  ext: boolean;
  dir: Dir;
  fd: boolean;
  brs: boolean;
  esi: boolean;
  rtr: boolean;
  dlc: number;
  count: number;
  firstTs: number;
  lastTs: number;
  /** Estimated period in ms, null until a frame with a later timestamp. */
  periodMs: number | null;
  data: number[];
  /** When each byte last changed (µs), for highlighting. */
  changedAt: number[];
  /** Latest of `changedAt` (µs). */
  lastChange: number;
  /** Start (µs) and frame count of the current period window. */
  winTs: number;
  winN: number;
  msg?: string;
  sigs?: FrameView["sigs"];
  label?: string;
}

/** Unique identifiers the monitor keeps; past it the least recently seen one makes room. */
export const MONITOR_MAX_IDS = 5000;

/** The period is measured over windows of about this long (µs). */
const PERIOD_WINDOW_US = 1_000_000;

const evictions = new WeakMap<Map<string, MonitorRow>, number>();

/** How many identifiers were dropped from `rows` to stay within the cap. */
export const monitorEvicted = (rows: Map<string, MonitorRow>) => evictions.get(rows) ?? 0;

/**
 * Period estimate: frames counted over a window of about a second, divided into the time it spans. Frames sharing one
 * timestamp (an adapter or driver that stamps a whole read at once) count as frames but add no time, so they no longer
 * drag the period down. The first estimate comes with the first frame at a later timestamp; later estimates far from the
 * current one (the ID was quiet, or changed rate) replace it, close ones are smoothed in.
 */
export function nextPeriod(r: Pick<MonitorRow, "periodMs" | "winTs" | "winN">, ts: number): Pick<MonitorRow, "periodMs" | "winTs" | "winN"> {
  if (ts < r.winTs) return { periodMs: r.periodMs, winTs: ts, winN: 0 };
  const winN = r.winN + 1;
  const elapsed = ts - r.winTs;
  if (elapsed <= 0 || (r.periodMs != null && elapsed < PERIOD_WINDOW_US)) return { periodMs: r.periodMs, winTs: r.winTs, winN };
  const est = elapsed / winN / 1000;
  const prev = r.periodMs;
  const periodMs = prev == null || est > prev * 2 || est < prev / 2 ? est : prev * 0.5 + est * 0.5;
  return { periodMs, winTs: ts, winN: 0 };
}

/**
 * Folds frames into the monitor table; returns whether anything changed. A changed row is replaced by a new object, so
 * unchanged rows keep their identity (and are not re-rendered), and moves to the end of the map: the first entry is
 * always the least recently seen identifier, the one dropped when a new identifier would exceed `maxIds`.
 */
export function foldMonitor(rows: Map<string, MonitorRow>, frames: FrameView[], maxIds = MONITOR_MAX_IDS): boolean {
  for (const f of frames) {
    const key = keyOf(f.id, f.ext) + (f.dir === "tx" ? ":tx" : "");
    const r = rows.get(key);
    if (!r) {
      if (rows.size >= maxIds) {
        const oldest = rows.keys().next().value;
        if (oldest !== undefined) {
          rows.delete(oldest);
          evictions.set(rows, monitorEvicted(rows) + 1);
        }
      }
      rows.set(key, {
        key, id: f.id, ext: f.ext, dir: f.dir, fd: f.fd, brs: f.brs, esi: !!f.esi, rtr: f.rtr, dlc: f.dlc, count: 1, firstTs: f.ts, lastTs: f.ts, periodMs: null,
        data: f.data, changedAt: f.data.map(() => f.ts), lastChange: f.ts, winTs: f.ts, winN: 0, msg: f.msg, sigs: f.sigs, label: f.label,
      });
      continue;
    }
    let lastChange = f.data.length !== r.data.length ? f.ts : r.lastChange;
    const changedAt = f.data.map((b, i) => {
      if (r.data[i] !== b) { lastChange = f.ts; return f.ts; }
      return r.changedAt[i] ?? f.ts;
    });
    rows.delete(key);
    rows.set(key, {
      ...r, ...nextPeriod(r, f.ts), count: r.count + 1, lastTs: f.ts, data: f.data, changedAt, lastChange, dlc: f.dlc, fd: f.fd, brs: f.brs,
      esi: !!f.esi, rtr: f.rtr, msg: f.msg, sigs: f.sigs, label: f.label,
    });
  }
  return frames.length > 0;
}

export type MonitorSort = "id" | "count" | "rate" | "recent";

export function sortMonitor(rows: MonitorRow[], by: MonitorSort): MonitorRow[] {
  const out = [...rows];
  const ext = (r: MonitorRow) => (r.ext ? 1 : 0);
  if (by === "id") out.sort((a, b) => ext(a) - ext(b) || a.id - b.id || a.dir.localeCompare(b.dir));
  if (by === "count") out.sort((a, b) => b.count - a.count);
  if (by === "rate") out.sort((a, b) => (a.periodMs ?? Infinity) - (b.periodMs ?? Infinity));
  if (by === "recent") out.sort((a, b) => b.lastTs - a.lastTs);
  return out;
}

// ── Monitor layout (virtual scrolling) ──

/** Height of a monitor row with up to 16 data bytes, px. */
export const MON_ROW_H = 30;
/** Extra height per further line of 16 bytes (CAN FD), px. */
export const MON_LINE_H = 20;
/** Height of one line of signals under an opened row, px. */
export const MON_SIG_H = 22;
/** Minimum width of a signal cell, px; decides how many fit on a line. */
export const MON_SIG_W = 240;

/** Columns of the signal grid under an opened row of a table `width` px wide. */
export const sigColumns = (width: number) => Math.max(1, Math.floor((width - 48) / MON_SIG_W));

/** Height of a row, px: one line per 16 data bytes, plus its signals when open. */
export function monitorRowHeight(r: Pick<MonitorRow, "data" | "sigs">, open: boolean, cols: number): number {
  const lines = Math.max(1, Math.ceil(r.data.length / 16));
  const base = MON_ROW_H + (lines - 1) * MON_LINE_H;
  const sigs = open ? (r.sigs?.length ?? 0) : 0;
  return sigs > 0 ? base + Math.ceil(sigs / cols) * MON_SIG_H + 6 : base;
}

/** Top offset and height of every row, and the total height. */
export function monitorLayout(rows: MonitorRow[], isOpen: (key: string) => boolean, width: number): { tops: number[]; heights: number[]; total: number } {
  const cols = sigColumns(width);
  const tops = new Array<number>(rows.length);
  const heights = new Array<number>(rows.length);
  let y = 0;
  for (let i = 0; i < rows.length; i++) {
    const h = monitorRowHeight(rows[i], isOpen(rows[i].key), cols);
    tops[i] = y;
    heights[i] = h;
    y += h;
  }
  return { tops, heights, total: y };
}

/** Rows `[first, last)` that overlap the viewport, with `overscan` more on either side. */
export function visibleRange(tops: number[], heights: number[], scrollTop: number, viewHeight: number, overscan = 6): [number, number] {
  const n = tops.length;
  if (n === 0) return [0, 0];
  let lo = 0;
  let hi = n - 1;
  // First row whose bottom is below scrollTop.
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (tops[mid] + heights[mid] <= scrollTop) lo = mid + 1;
    else hi = mid;
  }
  let last = lo;
  const bottom = scrollTop + viewHeight;
  while (last < n && tops[last] < bottom) last++;
  return [Math.max(0, lo - overscan), Math.min(n, last + overscan)];
}

// ── Filter ──

/** Placeholder and help of the identifier filter. */
export const ID_FILTER_PLACEHOLDER = "IDs: 7E8, 700-7FF, 18FEF100, !100";
export const ID_FILTER_HELP =
  "Identifiers in hex, single or as ranges, separated by commas or spaces; ! in front hides them. Up to 3 digits is an 11-bit ID " +
  "(100-1FF), 4 to 8 digits or an x at the end a 29-bit one (18FEF100, 100x, 0-FFFFx). Search message names in Search.";

type IdFrame = Pick<FrameView, "id" | "ext">;

export interface ParsedFilter<F> {
  test: (f: F) => boolean;
  /** What could not be read; those terms are left out. */
  error: string | null;
}

/** An identifier as typed in the filter: up to 3 hex digits is 11-bit; 4 to 8 digits, above 7FF or an `x` suffix is 29-bit. */
function filterId(t: string): { id: number; ext: boolean } | null {
  const m = /^(?:0x)?([0-9a-f]{1,8})(x)?$/i.exec(t);
  if (!m) return null;
  const id = parseInt(m[1], 16);
  if (id > 0x1fffffff) return null;
  return { id, ext: !!m[2] || m[1].length > 3 || id > 0x7ff };
}

/**
 * Identifier filter: `7E8`, `700-7FF`, `18FEF100`, `100x`, `18FE0000-18FEFFFF`, several separated by commas or spaces,
 * `!` in front to hide. Matches identifiers only, never names. A term is 11-bit or 29-bit by `filterId`'s rule (a range
 * is 29-bit if either end is) and only matches identifiers of that kind. Empty shows everything.
 */
export function parseFilter(text: string): ParsedFilter<IdFrame> {
  const terms = text.split(/[\s,]+/).map((t) => t.trim()).filter((t) => t && t !== "!");
  type Term = { neg: boolean; ext: boolean; lo: number; hi: number };
  const parsed: Term[] = [];
  const bad: string[] = [];
  for (const raw of terms) {
    const neg = raw.startsWith("!");
    const t = neg ? raw.slice(1) : raw;
    const dash = t.indexOf("-", 1);
    const a = filterId(dash > 0 ? t.slice(0, dash) : t);
    const b = dash > 0 ? filterId(t.slice(dash + 1)) : a;
    if (a && b) parsed.push({ neg, ext: a.ext || b.ext, lo: Math.min(a.id, b.id), hi: Math.max(a.id, b.id) });
    else bad.push(raw);
  }
  const error = bad.length === 0 ? null : `Not an identifier: ${bad.join(", ")}. The ID filter takes hex IDs and ranges; type names in Search.`;
  if (parsed.length === 0) return { test: () => true, error };
  const hit = (p: Term, f: IdFrame) => p.ext === f.ext && f.id >= p.lo && f.id <= p.hi;
  const pos = parsed.filter((p) => !p.neg);
  const neg = parsed.filter((p) => p.neg);
  return { test: (f) => (pos.length === 0 || pos.some((p) => hit(p, f))) && !neg.some((p) => hit(p, f)), error };
}

type TextFrame = Pick<FrameView, "msg" | "label">;

/** Name search: words in the DBC message name or the protocol label, `!` in front to hide. Case does not matter. */
export function parseSearch(text: string): (f: TextFrame) => boolean {
  const words = text.split(/[\s,]+/).map((t) => t.trim().toLowerCase()).filter((t) => t && t !== "!");
  if (words.length === 0) return () => true;
  const has = (f: TextFrame, w: string) => !!f.msg?.toLowerCase().includes(w) || !!f.label?.toLowerCase().includes(w);
  const pos = words.filter((w) => !w.startsWith("!"));
  const neg = words.filter((w) => w.startsWith("!")).map((w) => w.slice(1));
  return (f) => (pos.length === 0 || pos.some((w) => has(f, w))) && !neg.some((w) => has(f, w));
}

/**
 * Data pattern, matched from the first byte: two hex digits per byte, `x` or `?` for a digit that may be anything:
 * `xx 12 xx`, `01 ?F`, `xx12` (without spaces, two digits per byte). Frames shorter than the pattern do not match.
 */
export function parseDataPattern(text: string): ParsedFilter<Pick<FrameView, "data">> {
  const t = text.trim();
  if (!t) return { test: () => true, error: null };
  const tokens = /[\s,:]/.test(t) ? t.split(/[\s,:]+/).filter(Boolean) : (t.match(/.{1,2}/g) ?? []);
  const masks: number[] = [];
  const values: number[] = [];
  for (const tok of tokens) {
    if (!/^[0-9a-fx?]{2}$/i.test(tok)) {
      return { test: () => true, error: `Data pattern: "${tok}" is not a byte. Write two hex digits per byte, x for any digit: xx 12 xx.` };
    }
    let mask = 0;
    let value = 0;
    [...tok].forEach((c, i) => {
      if (c === "x" || c === "X" || c === "?") return;
      const shift = i === 0 ? 4 : 0;
      mask |= 0xf << shift;
      value |= parseInt(c, 16) << shift;
    });
    masks.push(mask);
    values.push(value);
  }
  return {
    test: (f) => {
      if (f.data.length < masks.length) return false;
      for (let i = 0; i < masks.length; i++) if ((f.data[i] & masks[i]) !== values[i]) return false;
      return true;
    },
    error: null,
  };
}

export interface FrameQuery {
  /** Identifier filter (`parseFilter`). */
  ids: string;
  /** Name search (`parseSearch`). */
  text: string;
  /** Data pattern (`parseDataPattern`). */
  data: string;
}

/** The identifier filter, the name search and the data pattern together. */
export function frameFilter(q: FrameQuery): ParsedFilter<Pick<FrameView, "id" | "ext" | "msg" | "label" | "data">> {
  const ids = parseFilter(q.ids);
  const text = parseSearch(q.text);
  const data = parseDataPattern(q.data);
  const error = [ids.error, data.error].filter(Boolean).join(" ");
  return { test: (f) => ids.test(f) && text(f) && data.test(f), error: error || null };
}

// ── Export ──

export function candumpLine(f: FrameView, iface: string): string {
  const s = Math.floor(f.ts / 1e6);
  const us = String(f.ts % 1e6).padStart(6, "0");
  return `(${s}.${us}) ${iface} ${cansendText(f)}`;
}

export const toCandump = (frames: FrameView[], iface: string) => frames.map((f) => candumpLine(f, iface) + "\n").join("");

export function csvCell(v: unknown): string {
  const s = String(v ?? "");
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(header: string[], rows: unknown[][]): string {
  return [header, ...rows].map((r) => r.map(csvCell).join(",")).join("\n") + "\n";
}

export function traceCsv(frames: FrameView[]): string {
  const t0 = frames[0]?.ts ?? 0;
  return toCsv(
    ["time_s", "direction", "id", "extended", "fd", "brs", "rtr", "dlc", "data", "message", "signals", "label"],
    frames.map((f) => [
      ((f.ts - t0) / 1e6).toFixed(6), f.dir, idText(f.id, f.ext), +f.ext, +f.fd, +f.brs, +f.rtr, f.dlc, hex(f.data), f.msg ?? "",
      (f.sigs ?? []).map(([n, v, u, l]) => `${n}=${l ?? fmtNum(v)}${u ? ` ${u}` : ""}`).join("; "), f.label ?? "",
    ]),
  );
}

/**
 * The latest frame of a message for the Signals tab, keeping the last value of multiplexed signals the new frame does not
 * carry (another page of the same message), so every page shows at once.
 */
export function mergeLatest(prev: FrameView | undefined, next: FrameView): FrameView {
  if (!prev?.sigs || !next.sigs) return next;
  const have = new Set(next.sigs.map(([n]) => n));
  const kept = prev.sigs.filter(([n]) => !have.has(n));
  return kept.length === 0 ? next : { ...next, sigs: [...next.sigs, ...kept] };
}

export function fmtNum(v: number): string {
  if (!Number.isFinite(v)) return String(v);
  if (Number.isInteger(v)) return String(v);
  const a = Math.abs(v);
  return a >= 1000 ? v.toFixed(1) : a >= 1 ? v.toFixed(3).replace(/\.?0+$/, "") : v.toPrecision(4).replace(/\.?0+$/, "");
}

export function pushCapped<T>(prev: T[], items: T[], max: number): T[] {
  if (items.length === 0) return prev;
  const next = prev.concat(items);
  return next.length > max ? next.slice(next.length - max) : next;
}

// ── Interface profiles ──

export const PROFILES_KEY = "can-explorer-profiles";
export const TX_KEY = "can-explorer-tx-list";

/** Every nominal bit rate the connect form offers (a saved profile may hold any of them). */
export const BITRATES = [10000, 20000, 50000, 83333, 100000, 125000, 250000, 500000, 750000, 800000, 1000000];
/** Every CAN FD data rate the connect form offers. */
export const DATA_BITRATES = [1000000, 2000000, 4000000, 5000000, 8000000];

/**
 * Bit rates per adapter kind, with a note when only some adapters have the rate. SLCAN firmware has a
 * fixed table (`S0`-`S9`) and `S7` differs: 750k on CANable, 800k on Lawicel and USBtin. gs_usb
 * computes the timing from the adapter's clock, so it takes them all.
 */
export const IFACE_BITRATES: Record<"slcan" | "gsusb", { rate: number; note?: string }[]> = {
  slcan: [
    { rate: 10000 }, { rate: 20000 }, { rate: 50000 }, { rate: 83333, note: "CANable 2 only" }, { rate: 100000 }, { rate: 125000 },
    { rate: 250000 }, { rate: 500000 }, { rate: 750000, note: "CANable only" }, { rate: 800000, note: "Lawicel, USBtin; not CANable" },
    { rate: 1000000 },
  ],
  gsusb: BITRATES.map((rate) => ({ rate })),
};

/** CAN FD data rates per adapter kind: CANable 2 slcan firmware only has 2 and 5 Mbit/s. */
export const IFACE_DATA_BITRATES: Record<"slcan" | "gsusb", number[]> = {
  slcan: [2000000, 5000000],
  gsusb: DATA_BITRATES,
};

/** Status bar badge for the controller's error state; null while it is error active with no counters to show. */
export function busStateBadge(s: BusStatus | null | undefined): { text: string; tone: "warn" | "danger" | "neutral"; title: string } | null {
  if (!s) return null;
  const counters = s.txErrors != null && s.rxErrors != null ? `TX errors ${s.txErrors}, RX errors ${s.rxErrors}` : "";
  const title = `Controller state: ${s.state.replace("-", " ")}${counters ? `; ${counters}` : ""}`;
  switch (s.state) {
    case "bus-off": return { text: "Bus off", tone: "danger", title };
    case "error-passive": return { text: "Error passive", tone: "danger", title };
    case "error-warning": return { text: "Error warning", tone: "warn", title };
    default: return counters && (s.txErrors! > 0 || s.rxErrors! > 0) ? { text: `TEC ${s.txErrors} REC ${s.rxErrors}`, tone: "neutral", title } : null;
  }
}

const rateLabel = (b: number) => (b === 83333 ? "83.3 kbit/s" : rateText(b));

/** Bit rate choices for the connect form; a saved rate this kind lacks stays listed, marked. */
export function rateOptions(kind: "slcan" | "gsusb", current: number): { value: string; label: string }[] {
  const opts = IFACE_BITRATES[kind].map(({ rate, note }) => ({ value: String(rate), label: note ? `${rateLabel(rate)} (${note})` : rateLabel(rate) }));
  if (!IFACE_BITRATES[kind].some((b) => b.rate === current)) opts.push({ value: String(current), label: `${rateLabel(current)} (not available)` });
  return opts;
}

export function dataRateOptions(kind: "slcan" | "gsusb", current: number): { value: string; label: string }[] {
  const opts = [{ value: "0", label: "Classic CAN (no FD)" }, ...IFACE_DATA_BITRATES[kind].map((b) => ({ value: String(b), label: rateText(b) }))];
  if (current > 0 && !IFACE_DATA_BITRATES[kind].includes(current)) opts.push({ value: String(current), label: `${rateText(current)} (not available)` });
  return opts;
}

export interface IfaceForm {
  kind: InterfaceKind;
  port: string;
  bitrate: number;
  /** 0 for classic CAN. */
  dataBitrate: number;
  serialBaud: number;
  listenOnly: boolean;
  ifname: string;
  /** gs_usb adapter id (serial number); empty takes the first one. */
  gsDevice: string;
  /** gs_usb channel, 0-based. */
  channel: number;
}

export const IFACE_DEFAULTS: IfaceForm = {
  kind: "slcan", port: "", bitrate: 500000, dataBitrate: 0, serialBaud: 115200, listenOnly: false, ifname: "can0", gsDevice: "", channel: 0,
};

export interface Profile extends IfaceForm {
  name: string;
}

export function configOf(f: IfaceForm): InterfaceConfig {
  if (f.kind === "virtual") return { kind: "virtual" };
  if (f.kind === "socketcan") return { kind: "socketcan", ifname: f.ifname.trim() };
  if (f.kind === "gsusb") {
    return { kind: "gsusb", device: f.gsDevice || null, channel: f.channel, bitrate: f.bitrate, dataBitrate: f.dataBitrate > 0 ? f.dataBitrate : null, listenOnly: f.listenOnly };
  }
  return { kind: "slcan", port: f.port, bitrate: f.bitrate, dataBitrate: f.dataBitrate > 0 ? f.dataBitrate : null, serialBaud: f.serialBaud, listenOnly: f.listenOnly };
}

export function ifaceProblem(f: IfaceForm): string | null {
  if (f.kind === "slcan" && !f.port) return "Pick the adapter's serial port.";
  if (f.kind === "slcan" || f.kind === "gsusb") {
    if (!IFACE_BITRATES[f.kind].some((b) => b.rate === f.bitrate)) return `Pick a bit rate this adapter type offers.`;
    if (f.dataBitrate > 0 && !IFACE_DATA_BITRATES[f.kind].includes(f.dataBitrate)) {
      return f.kind === "slcan" ? "SLCAN adapters (CANable 2) only have 2 and 5 Mbit/s CAN FD data rates." : "Pick a CAN FD data rate this adapter type offers.";
    }
  }
  if (f.kind === "socketcan" && !/^[A-Za-z0-9_.-]{1,15}$/.test(f.ifname.trim())) return "Enter the interface name, e.g. can0.";
  return null;
}

export function normalizeProfile(raw: unknown): Profile | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.name !== "string" || !r.name) return null;
  const d = IFACE_DEFAULTS;
  const kind: InterfaceKind = r.kind === "socketcan" || r.kind === "virtual" || r.kind === "gsusb" ? r.kind : "slcan";
  const pick = (v: unknown, allowed: number[], def: number) => (typeof v === "number" && allowed.includes(v) ? v : def);
  return {
    name: r.name,
    kind,
    port: typeof r.port === "string" ? r.port : d.port,
    bitrate: pick(r.bitrate, BITRATES, d.bitrate),
    dataBitrate: pick(r.dataBitrate, [0, ...DATA_BITRATES], d.dataBitrate),
    serialBaud: typeof r.serialBaud === "number" && r.serialBaud > 0 ? r.serialBaud : d.serialBaud,
    listenOnly: r.listenOnly === true,
    ifname: typeof r.ifname === "string" ? r.ifname : d.ifname,
    gsDevice: typeof r.gsDevice === "string" ? r.gsDevice : d.gsDevice,
    channel: typeof r.channel === "number" && r.channel >= 0 && r.channel < 8 ? Math.floor(r.channel) : d.channel,
  };
}

type Store = Pick<Storage, "getItem" | "setItem">;

export function loadProfiles(storage: Pick<Storage, "getItem"> = localStorage): Profile[] {
  try {
    const parsed = JSON.parse(storage.getItem(PROFILES_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.map(normalizeProfile).filter((p): p is Profile => p !== null) : [];
  } catch {
    return [];
  }
}

export function saveProfiles(p: Profile[], storage: Pick<Store, "setItem"> = localStorage) {
  storage.setItem(PROFILES_KEY, JSON.stringify(p));
}

export function needsPro(profiles: Profile[], name: string, pro: boolean): boolean {
  return !pro && !profiles.some((p) => p.name === name) && profiles.length >= FREE_PROFILE_LIMIT;
}

export const sameIface = (a: IfaceForm, b: IfaceForm) =>
  a.kind === b.kind && a.port === b.port && a.bitrate === b.bitrate && a.dataBitrate === b.dataBitrate &&
  a.serialBaud === b.serialBaud && a.listenOnly === b.listenOnly && a.ifname === b.ifname &&
  a.gsDevice === b.gsDevice && a.channel === b.channel;

// ── Transmit list ──

export interface TxItem {
  key: string;
  name: string;
  /** Hex identifier as typed. */
  id: string;
  fd: boolean;
  brs: boolean;
  rtr: boolean;
  data: string;
  /** Data length code a remote frame asks for, 0 to 8. */
  rtrLen?: number;
  /** Period for cyclic sending, ms. */
  periodMs: number;
  /** Frames cyclic sending stops after; 0 or absent sends until stopped. */
  count?: number;
}

/** Longest cyclic period, ms (an hour). */
export const TX_MAX_PERIOD_MS = 3_600_000;

/** Why the cyclic period cannot be used, or null. */
export function periodProblem(ms: number): string | null {
  if (!Number.isFinite(ms) || ms < 1) return "The period is at least 1 ms.";
  if (!Number.isInteger(ms)) return "The period is in whole milliseconds.";
  if (ms > TX_MAX_PERIOD_MS) return "The period is at most 3600000 ms (an hour).";
  return null;
}

/** Why the frame count cannot be used, or null. 0 means until stopped. */
export function countProblem(n: number | undefined): string | null {
  if (n == null || n === 0) return null;
  if (!Number.isInteger(n) || n < 0) return "The count is a whole number; 0 or empty sends until stopped.";
  if (n > 1_000_000_000) return "The count is at most 1000000000.";
  return null;
}

/** Badge text of a running cyclic item. */
export const cyclicText = (t: Pick<TxItem, "periodMs" | "count">) => `every ${t.periodMs} ms${t.count ? `, ${t.count.toLocaleString("en-US")} frames` : ""}`;

export const TX_DEFAULTS: TxItem[] = [
  { key: "obd-rpm", name: "OBD-II engine speed request", id: "7DF", fd: false, brs: false, rtr: false, data: "02 01 0C 00 00 00 00 00", periodMs: 200 },
  { key: "sdo-vendor", name: "CANopen SDO read 1018sub1 (node 11)", id: "611", fd: false, brs: false, rtr: false, data: "40 18 10 01 00 00 00 00", periodMs: 1000 },
  { key: "nmt-start", name: "CANopen NMT start all nodes", id: "000", fd: false, brs: false, rtr: false, data: "01 00", periodMs: 1000 },
];

/** The data length code to start a remote frame with: the length of the data typed so far, at most 8. */
export function defaultRtrLen(data: string): number {
  const d = parseData(data);
  return typeof d === "string" ? 0 : Math.min(8, d.length);
}

export function frameInOf(t: Pick<TxItem, "id" | "fd" | "brs" | "rtr" | "data" | "rtrLen">): FrameIn | string {
  const id = parseId(t.id);
  if (typeof id === "string") return id;
  const rtrLen = t.rtr ? (t.rtrLen ?? defaultRtrLen(t.data)) : 0;
  if (t.rtr && (!Number.isInteger(rtrLen) || rtrLen < 0 || rtrLen > 8)) return "A remote frame asks for 0 to 8 bytes.";
  const f: FrameIn = { id: id.id, ext: id.ext, rtr: t.rtr, fd: t.fd, brs: t.brs, data: t.rtr ? "" : t.data, rtrLen };
  return frameProblem(f) ?? f;
}

export function loadTxList(storage: Pick<Storage, "getItem"> = localStorage): TxItem[] {
  try {
    const raw = storage.getItem(TX_KEY);
    if (raw == null) return TX_DEFAULTS;
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return TX_DEFAULTS;
    return parsed
      .filter((x) => x && typeof x === "object" && typeof x.key === "string" && typeof x.id === "string")
      .map((x) => ({
        key: x.key, name: typeof x.name === "string" ? x.name : "", id: x.id, fd: x.fd === true, brs: x.brs === true, rtr: x.rtr === true,
        data: typeof x.data === "string" ? x.data : "",
        periodMs: typeof x.periodMs === "number" && periodProblem(x.periodMs) === null ? x.periodMs : 100,
        ...(Number.isInteger(x.rtrLen) && x.rtrLen >= 0 && x.rtrLen <= 8 ? { rtrLen: x.rtrLen } : {}),
        ...(typeof x.count === "number" && x.count > 0 && countProblem(x.count) === null ? { count: x.count } : {}),
      }));
  } catch {
    return TX_DEFAULTS;
  }
}

export function saveTxList(items: TxItem[], storage: Pick<Store, "setItem"> = localStorage) {
  storage.setItem(TX_KEY, JSON.stringify(items));
}

export const rateText = (bps: number) => (bps >= 1e6 && bps % 1e6 === 0 ? `${bps / 1e6} Mbit/s` : `${bps / 1000} kbit/s`);

export function periodText(ms: number | null): string {
  if (ms == null) return "";
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  return `${ms.toFixed(ms < 10 ? 2 : 1)} ms`;
}

/** Wall-clock time of a µs timestamp. */
export const clockUs = (us: number) =>
  new Date(us / 1000).toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 });
