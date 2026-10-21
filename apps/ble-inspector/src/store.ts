import { isFormatId, type FormatId } from "./decode";

/* ── Samples (notify streams, RSSI) ─────────────────────────────────────── */

export interface Sample<T> {
  t: number;
  v: T;
}

/** Bounded in-memory history per key. Not React state: panels poll it on a timer. */
export class SampleStore<T> {
  private data = new Map<string, Sample<T>[]>();
  constructor(private readonly cap: number) {}

  push(key: string, t: number, v: T): void {
    let list = this.data.get(key);
    if (!list) {
      list = [];
      this.data.set(key, list);
    }
    list.push({ t, v });
    if (list.length > this.cap) list.splice(0, list.length - this.cap);
  }

  get(key: string): readonly Sample<T>[] {
    return this.data.get(key) ?? [];
  }

  keys(): string[] {
    return [...this.data.keys()];
  }

  clear(key?: string): void {
    if (key === undefined) this.data.clear();
    else this.data.delete(key);
  }
}

/** Raw notification payloads as hex, keyed `deviceId|charUuid`. Kept raw so the decode format can change later. */
export const notifyStore = new SampleStore<string>(4000);
/** RSSI in dBm per device id. */
export const rssiStore = new SampleStore<number>(1200);

/* ── Persisted preferences ──────────────────────────────────────────────── */

export interface DecodePref {
  format: FormatId;
  offset: number;
  scale: number;
}

export interface Prefs {
  gattNames: boolean;
  decode: boolean;
  /** Decode settings per characteristic UUID. */
  formats: Record<string, DecodePref>;
}

export type WriteMode = "hex" | "text";

export interface WritePreset {
  id: string;
  name: string;
  /** Characteristic UUID this preset writes to. */
  charUuid: string;
  mode: WriteMode;
  value: string;
}

export interface MacroStep {
  presetId: string;
  /** Pause after this step, in milliseconds. */
  delayMs: number;
}

export interface Macro {
  id: string;
  name: string;
  repeat: number;
  steps: MacroStep[];
}

interface Stored {
  prefs: Prefs;
  presets: WritePreset[];
  macros: Macro[];
}

const KEY = "ble-inspector:v1";

export const MAX_DELAY_MS = 600_000;
export const MAX_REPEAT = 1000;

export const DEFAULT_PREFS: Prefs = { gattNames: true, decode: true, formats: {} };

const isObj = (x: unknown): x is Record<string, unknown> => typeof x === "object" && x !== null && !Array.isArray(x);
const num = (x: unknown, fallback: number): number => (typeof x === "number" && Number.isFinite(x) ? x : fallback);

export function parseStored(raw: unknown): Stored {
  const out: Stored = { prefs: { ...DEFAULT_PREFS, formats: {} }, presets: [], macros: [] };
  if (!isObj(raw)) return out;

  if (isObj(raw.prefs)) {
    const p = raw.prefs;
    if (typeof p.gattNames === "boolean") out.prefs.gattNames = p.gattNames;
    if (typeof p.decode === "boolean") out.prefs.decode = p.decode;
    if (isObj(p.formats)) {
      for (const [uuid, f] of Object.entries(p.formats)) {
        if (!isObj(f) || !isFormatId(f.format)) continue;
        out.prefs.formats[uuid] = {
          format: f.format,
          offset: Math.max(0, Math.floor(num(f.offset, 0))),
          scale: num(f.scale, 1),
        };
      }
    }
  }

  if (Array.isArray(raw.presets)) {
    for (const p of raw.presets) {
      if (!isObj(p) || typeof p.id !== "string" || typeof p.name !== "string" || typeof p.charUuid !== "string") continue;
      if (typeof p.value !== "string" || (p.mode !== "hex" && p.mode !== "text")) continue;
      out.presets.push({ id: p.id, name: p.name, charUuid: p.charUuid, mode: p.mode, value: p.value });
    }
  }

  if (Array.isArray(raw.macros)) {
    for (const m of raw.macros) {
      if (!isObj(m) || typeof m.id !== "string" || typeof m.name !== "string" || !Array.isArray(m.steps)) continue;
      const steps: MacroStep[] = [];
      for (const s of m.steps) {
        if (!isObj(s) || typeof s.presetId !== "string") continue;
        steps.push({ presetId: s.presetId, delayMs: Math.min(MAX_DELAY_MS, Math.max(0, Math.floor(num(s.delayMs, 0)))) });
      }
      out.macros.push({
        id: m.id,
        name: m.name,
        repeat: Math.min(MAX_REPEAT, Math.max(1, Math.floor(num(m.repeat, 1)))),
        steps,
      });
    }
  }
  return out;
}

export function loadStored(): Stored {
  try {
    const raw = localStorage.getItem(KEY);
    return parseStored(raw ? JSON.parse(raw) : null);
  } catch {
    return parseStored(null);
  }
}

export function saveStored(s: Stored): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // Quota or private-mode errors: presets are a convenience, never fatal.
  }
}

export function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Drop macro steps whose preset no longer exists. */
export function pruneMacros(macros: Macro[], presets: WritePreset[]): Macro[] {
  const ids = new Set(presets.map((p) => p.id));
  return macros.map((m) => ({ ...m, steps: m.steps.filter((s) => ids.has(s.presetId)) }));
}
