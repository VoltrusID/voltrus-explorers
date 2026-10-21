/** One recorded value of a monitored node. */
export interface ValueSample {
  /** Epoch milliseconds. */
  t: number;
  /** Session label at the time of the sample. */
  session: string;
  nodeId: string;
  name: string;
  value: unknown;
}

/** One row of the monitored-values snapshot. */
export interface ValueRow {
  session: string;
  nodeId: string;
  name: string;
  dataType: string;
  value: unknown;
  /** Epoch milliseconds, 0 when the server gave none. */
  timestamp: number;
}

const NUMERIC = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;

function text(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

/**
 * Quote a CSV field. Values come from servers the user does not control, so text that a spreadsheet would run as a
 * formula (leading `=`, `+`, `-`, `@`, tab or CR) gets a leading apostrophe; plain numbers are left alone.
 */
export function csvCell(v: unknown): string {
  let s = text(v);
  if (typeof v !== "number" && /^[=+\-@\t\r]/.test(s) && !NUMERIC.test(s)) s = `'${s}`;
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function iso(ms: number): string {
  return ms > 0 ? new Date(ms).toISOString() : "";
}

function csv(header: string[], rows: unknown[][]): string {
  return [header.join(","), ...rows.map((r) => r.map(csvCell).join(","))].join("\r\n") + "\r\n";
}

/** CSV of the current value of every row. */
export function valuesToCsv(rows: ValueRow[]): string {
  return csv(
    ["Session", "Node ID", "Browse Name", "Data Type", "Value", "Timestamp"],
    rows.map((r) => [r.session, r.nodeId, r.name, r.dataType, r.value, iso(r.timestamp)]),
  );
}

/** CSV of recorded value changes, oldest first. */
export function samplesToCsv(samples: ValueSample[]): string {
  return csv(
    ["Timestamp", "Session", "Node ID", "Browse Name", "Value"],
    samples.map((s) => [iso(s.t), s.session, s.nodeId, s.name, s.value]),
  );
}

/**
 * In-memory record of value changes per node. A node only gets a new sample when its value differs from the last
 * one recorded, so repeated reads or polls of an unchanged node add nothing. Keeps the newest `cap` samples.
 */
export class ValueLog {
  private samples: ValueSample[] = [];
  private last = new Map<string, string>();

  constructor(private readonly cap = 20000) {}

  get size(): number {
    return this.samples.length;
  }

  /** `scope` identifies the session, `session` is its display label. */
  record(scope: string, session: string, nodeId: string, name: string, value: unknown, t: number): void {
    const key = `${scope}|${nodeId}`;
    const sig = text(value);
    if (this.last.get(key) === sig) return;
    this.last.set(key, sig);
    this.samples.push({ t, session, nodeId, name, value });
    if (this.samples.length > this.cap) this.samples.splice(0, this.samples.length - this.cap);
  }

  /** Forget what was last seen for a session (on disconnect), keeping its samples. */
  forget(scope: string): void {
    const prefix = `${scope}|`;
    for (const key of this.last.keys()) if (key.startsWith(prefix)) this.last.delete(key);
  }

  clear(): void {
    this.samples = [];
    this.last.clear();
  }

  all(): ValueSample[] {
    return this.samples;
  }
}
