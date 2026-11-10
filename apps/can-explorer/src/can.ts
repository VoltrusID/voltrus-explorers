import type { Dir, FrameIn, FrameView, InterfaceConfig, InterfaceKind } from "./types";

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

/** cansend syntax: `123#DEADBEEF`, `123#R`, `123##1AABB`. */
export function cansendText(f: { id: number; ext: boolean; rtr: boolean; fd: boolean; brs: boolean; data: number[]; dlc?: number }): string {
  const id = idText(f.id, f.ext);
  const d = f.data.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join("");
  if (f.rtr) return `${id}#R${f.dlc ? f.dlc : ""}`;
  if (f.fd) return `${id}##${f.brs ? 1 : 0}${d}`;
  return `${id}#${d}`;
}

// ── Monitor (one row per identifier) ──

export interface MonitorRow {
  key: string;
  id: number;
  ext: boolean;
  dir: Dir;
  fd: boolean;
  rtr: boolean;
  dlc: number;
  count: number;
  firstTs: number;
  lastTs: number;
  /** Smoothed period in ms, null after one frame. */
  periodMs: number | null;
  data: number[];
  /** When each byte last changed (µs), for highlighting. */
  changedAt: number[];
  msg?: string;
  sigs?: FrameView["sigs"];
  label?: string;
}

/** Folds frames into the monitor table in place; returns whether anything changed. */
export function foldMonitor(rows: Map<string, MonitorRow>, frames: FrameView[]): boolean {
  for (const f of frames) {
    const key = keyOf(f.id, f.ext) + (f.dir === "tx" ? ":tx" : "");
    const r = rows.get(key);
    if (!r) {
      rows.set(key, {
        key, id: f.id, ext: f.ext, dir: f.dir, fd: f.fd, rtr: f.rtr, dlc: f.dlc, count: 1, firstTs: f.ts, lastTs: f.ts, periodMs: null,
        data: f.data, changedAt: f.data.map(() => f.ts), msg: f.msg, sigs: f.sigs, label: f.label,
      });
      continue;
    }
    const dt = (f.ts - r.lastTs) / 1000;
    if (dt >= 0) r.periodMs = r.periodMs == null ? dt : r.periodMs * 0.8 + dt * 0.2;
    const changedAt = f.data.map((b, i) => (r.data[i] !== b ? f.ts : (r.changedAt[i] ?? f.ts)));
    Object.assign(r, { count: r.count + 1, lastTs: f.ts, data: f.data, changedAt, dlc: f.dlc, fd: f.fd, rtr: f.rtr, msg: f.msg, sigs: f.sigs, label: f.label });
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

// ── Filter ──

/**
 * Identifier filter as typed: `7E8`, `700-7FF`, `18FEF100`, a message or label word (`EEC1`,
 * `heartbeat`), several separated by commas or spaces, `!` in front to hide. Empty shows everything.
 */
export function parseFilter(text: string): (f: Pick<FrameView, "id" | "ext" | "msg" | "label">) => boolean {
  const terms = text.split(/[\s,]+/).map((t) => t.trim()).filter(Boolean);
  if (terms.length === 0) return () => true;
  type Term = { neg: boolean; test: (f: Pick<FrameView, "id" | "ext" | "msg" | "label">) => boolean };
  const parsed: Term[] = terms.map((raw) => {
    const neg = raw.startsWith("!");
    const t = neg ? raw.slice(1) : raw;
    const range = /^(?:0x)?([0-9a-f]{1,8})-(?:0x)?([0-9a-f]{1,8})$/i.exec(t);
    if (range) {
      const lo = parseInt(range[1], 16);
      const hi = parseInt(range[2], 16);
      return { neg, test: (f) => f.id >= Math.min(lo, hi) && f.id <= Math.max(lo, hi) };
    }
    const word = t.toLowerCase();
    if (/^(?:0x)?[0-9a-f]{1,8}$/i.test(t)) {
      const id = parseInt(t.replace(/^0x/i, ""), 16);
      return { neg, test: (f) => f.id === id || !!f.msg?.toLowerCase().includes(word) || !!f.label?.toLowerCase().includes(word) };
    }
    return { neg, test: (f) => !!f.msg?.toLowerCase().includes(word) || !!f.label?.toLowerCase().includes(word) };
  });
  const pos = parsed.filter((p) => !p.neg);
  const neg = parsed.filter((p) => p.neg);
  return (f) => (pos.length === 0 || pos.some((p) => p.test(f))) && !neg.some((p) => p.test(f));
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

export const BITRATES = [10000, 20000, 50000, 100000, 125000, 250000, 500000, 800000, 1000000];
export const DATA_BITRATES = [1000000, 2000000, 4000000, 5000000, 8000000];

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
  /** Period for cyclic sending, ms. */
  periodMs: number;
}

export const TX_DEFAULTS: TxItem[] = [
  { key: "obd-rpm", name: "OBD-II engine speed request", id: "7DF", fd: false, brs: false, rtr: false, data: "02 01 0C 00 00 00 00 00", periodMs: 200 },
  { key: "sdo-vendor", name: "CANopen SDO read 1018sub1 (node 11)", id: "611", fd: false, brs: false, rtr: false, data: "40 18 10 01 00 00 00 00", periodMs: 1000 },
  { key: "nmt-start", name: "CANopen NMT start all nodes", id: "000", fd: false, brs: false, rtr: false, data: "01 00", periodMs: 1000 },
];

export function frameInOf(t: Pick<TxItem, "id" | "fd" | "brs" | "rtr" | "data">): FrameIn | string {
  const id = parseId(t.id);
  if (typeof id === "string") return id;
  const f: FrameIn = { id: id.id, ext: id.ext, rtr: t.rtr, fd: t.fd, brs: t.brs, data: t.rtr ? "" : t.data, rtrLen: 0 };
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
        data: typeof x.data === "string" ? x.data : "", periodMs: typeof x.periodMs === "number" && x.periodMs >= 1 ? x.periodMs : 100,
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
