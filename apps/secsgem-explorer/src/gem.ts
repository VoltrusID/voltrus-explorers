import type { AckView, GemEvent, HostConnect, HostForm, LocalReport, ValueView } from "./types";

/** `1, 2, 5-7` -> [1, 2, 5, 6, 7]. Throws a message the form can show. */
export function parseIdList(text: string, limit = 5000): number[] {
  const out: number[] = [];
  for (const part of text.split(/[\s,;]+/).filter(Boolean)) {
    const m = /^(\d+)(?:-(\d+))?$/.exec(part);
    if (!m) throw new Error(`"${part}" is not an id or a range`);
    const a = Number(m[1]);
    const b = m[2] === undefined ? a : Number(m[2]);
    if (b < a) throw new Error(`"${part}" is a backwards range`);
    if (out.length + (b - a + 1) > limit) throw new Error(`More than ${limit} ids`);
    for (let i = a; i <= b; i++) out.push(i);
  }
  return out;
}

/** The message to show when `value` is outside the range an equipment constant reports, else null. */
export function rangeProblem(value: number, min: number | null, max: number | null): string | null {
  if (!Number.isFinite(value)) return "Not a number";
  if (min != null && max != null && (value < min || value > max)) return `Out of range: ${min} to ${max}`;
  if (min != null && max == null && value < min) return `Below the minimum ${min}`;
  if (max != null && min == null && value > max) return `Above the maximum ${max}`;
  return null;
}

export const ackText = (a: AckView) => (a.ok ? a.name : `${a.name} (code ${a.code})`);

export function hostConnectOf(f: HostForm): HostConnect {
  return {
    mode: f.mode,
    host: f.host.trim(),
    port: f.port,
    deviceId: f.deviceId,
    t3Ms: Math.round(f.t3 * 1000),
    t5Ms: Math.round(f.t5 * 1000),
    t6Ms: Math.round(f.t6 * 1000),
    t7Ms: Math.round(f.t7 * 1000),
    t8Ms: Math.round(f.t8 * 1000),
    controlSvid: f.controlSvid,
  };
}

/** Form values (strings or numbers from the ConnectionForm) to a typed form. */
export function hostFormOf(v: Record<string, string | number>, d: HostForm): HostForm {
  const n = (k: string, def: number) => {
    const x = Number(v[k]);
    return Number.isFinite(x) && v[k] !== "" ? x : def;
  };
  return {
    mode: v.mode === "passive" ? "passive" : "active",
    host: String(v.host ?? d.host),
    port: n("port", d.port),
    deviceId: n("deviceId", d.deviceId),
    t3: n("t3", d.t3),
    t5: n("t5", d.t5),
    t6: n("t6", d.t6),
    t7: n("t7", d.t7),
    t8: n("t8", d.t8),
    controlSvid: n("controlSvid", d.controlSvid),
  };
}

export const sameHost = (a: HostForm, b: HostForm) => (Object.keys(a) as (keyof HostForm)[]).every((k) => a[k] === b[k]);

export function csvCell(v: unknown): string {
  const s = String(v ?? "");
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(header: string[], rows: unknown[][]): string {
  return [header, ...rows].map((r) => r.map(csvCell).join(",")).join("\n") + "\n";
}

/** One row of the live event table. */
export interface EventRow {
  time: number;
  dataid: number;
  ceid: number;
  /** `RPTID 1: WaferCount=3, ChamberPressure=0.5`, one per report. */
  summary: string;
}

/** Names the values of an S6F11 with the variables of the reports this session defined. */
export function eventRow(
  time: number,
  ev: Extract<GemEvent, { kind: "event" }>,
  reports: LocalReport[],
  names: Map<number, string>,
): EventRow {
  const parts = ev.reports.map((r) => {
    const def = reports.find((d) => d.rptid === r.rptid);
    const vals = r.values.map((v: ValueView, i: number) => {
      const vid = def?.vids[i];
      return vid === undefined ? v.text : `${names.get(vid) ?? `VID ${vid}`}=${v.text}`;
    });
    return `RPTID ${r.rptid}: ${vals.join(", ") || "(empty)"}`;
  });
  return { time, dataid: ev.dataid, ceid: ev.ceid, summary: parts.join("; ") || "(no reports linked)" };
}

/** Keeps the newest `max` entries. */
export function pushCapped<T>(list: T[], item: T, max: number): T[] {
  return list.length >= max ? [...list.slice(list.length - max + 1), item] : [...list, item];
}
