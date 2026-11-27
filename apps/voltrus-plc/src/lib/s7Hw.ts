// Device configuration (`.s7hw`): the station's rack with the CPU and the modules beside it,
// their I/O addresses, and the tags their channels give. The catalog is generic: module kinds and
// channel counts, not a vendor product database; article numbers are typed in or read from the CPU.
import type { TagDoc } from "./s7Docs";

export const HW_EXT = ".s7hw";
export const isHwDoc = (path: string) => path.endsWith(HW_EXT);

export type Family = "S7-1200" | "S7-1500" | "S7-300";
export const FAMILIES: Family[] = ["S7-1200", "S7-1500", "S7-300"];

export type ModuleKind = "cpu" | "ps" | "im" | "di" | "dq" | "dio" | "ai" | "aq" | "cm";

export interface CatalogEntry {
  id: string;
  kind: ModuleKind;
  label: string;
  /** Digital inputs and outputs, analog inputs and outputs. */
  di?: number;
  dq?: number;
  ai?: number;
  aq?: number;
  /** Width on the rail, in units of a narrow module. */
  width: number;
}

export interface HwModule {
  slot: number;
  catalogId: string;
  name: string;
  articleNo?: string;
  firmware?: string;
  /** First byte of the module's inputs and outputs. Analog channels are 2 bytes each. */
  iStart?: number;
  qStart?: number;
  comment?: string;
}

export interface HwDoc {
  format: "voltrus-s7-hw";
  version: 1;
  family: Family;
  modules: HwModule[];
}

// ── Catalog ──

const io = (id: string, kind: ModuleKind, label: string, c: Partial<CatalogEntry>, width = 1): CatalogEntry => ({ id, kind, label, width, ...c });

const MODULES: CatalogEntry[] = [
  io("di8", "di", "DI 8 × 24 V DC", { di: 8 }),
  io("di16", "di", "DI 16 × 24 V DC", { di: 16 }),
  io("di32", "di", "DI 32 × 24 V DC", { di: 32 }, 2),
  io("dq8", "dq", "DQ 8 × 24 V DC", { dq: 8 }),
  io("dq16", "dq", "DQ 16 × 24 V DC", { dq: 16 }),
  io("dq32", "dq", "DQ 32 × 24 V DC", { dq: 32 }, 2),
  io("dq8r", "dq", "DQ 8 × relay", { dq: 8 }),
  io("dio8", "dio", "DI 8 / DQ 8 × 24 V DC", { di: 8, dq: 8 }),
  io("dio16", "dio", "DI 16 / DQ 16 × 24 V DC", { di: 16, dq: 16 }, 2),
  io("ai2", "ai", "AI 2 × U/I", { ai: 2 }),
  io("ai4", "ai", "AI 4 × U/I", { ai: 4 }),
  io("ai8", "ai", "AI 8 × U/I", { ai: 8 }, 2),
  io("ai4rtd", "ai", "AI 4 × RTD/TC", { ai: 4 }),
  io("aq2", "aq", "AQ 2 × U/I", { aq: 2 }),
  io("aq4", "aq", "AQ 4 × U/I", { aq: 4 }),
  io("aio", "ai", "AI 4 / AQ 2 × U/I", { ai: 4, aq: 2 }),
  io("cm-ptp", "cm", "Point-to-point RS-232/485", {}),
  io("cm-dp", "cm", "PROFIBUS DP", {}),
  io("cm-pn", "cm", "PROFINET", {}),
];

const CATALOG: Record<Family, CatalogEntry[]> = {
  "S7-1200": [
    io("cpu1211c", "cpu", "CPU 1211C", { di: 6, dq: 4, ai: 2 }, 2),
    io("cpu1212c", "cpu", "CPU 1212C", { di: 8, dq: 6, ai: 2 }, 2),
    io("cpu1214c", "cpu", "CPU 1214C", { di: 14, dq: 10, ai: 2 }, 3),
    io("cpu1215c", "cpu", "CPU 1215C", { di: 14, dq: 10, ai: 2, aq: 2 }, 3),
    io("cpu1217c", "cpu", "CPU 1217C", { di: 14, dq: 10, ai: 2, aq: 2 }, 3),
    ...MODULES,
  ],
  "S7-1500": [
    io("ps1500", "ps", "Power supply", {}, 2),
    io("cpu1511", "cpu", "CPU 1511-1 PN", {}, 2),
    io("cpu1513", "cpu", "CPU 1513-1 PN", {}, 2),
    io("cpu1515", "cpu", "CPU 1515-2 PN", {}, 2),
    io("cpu1516", "cpu", "CPU 1516-3 PN/DP", {}, 2),
    io("cpu1517", "cpu", "CPU 1517-3 PN/DP", {}, 3),
    ...MODULES,
  ],
  "S7-300": [
    io("ps300", "ps", "Power supply", {}, 2),
    io("cpu312", "cpu", "CPU 312", {}, 2),
    io("cpu314", "cpu", "CPU 314", {}, 2),
    io("cpu315dp", "cpu", "CPU 315-2 DP", {}, 2),
    io("cpu315pn", "cpu", "CPU 315-2 PN/DP", {}, 2),
    io("cpu317pn", "cpu", "CPU 317-2 PN/DP", {}, 3),
    io("cpu319", "cpu", "CPU 319-3 PN/DP", {}, 3),
    io("im365", "im", "Interface module", {}),
    ...MODULES,
  ],
};

export function catalogFor(family: Family): CatalogEntry[] {
  return CATALOG[family];
}

export function entryOf(family: Family, id: string): CatalogEntry | undefined {
  return CATALOG[family].find((e) => e.id === id);
}

// ── Slots ──

export interface SlotInfo {
  slot: number;
  /** What may go here. */
  accepts: ModuleKind[];
  label: string;
}

/** The slots of a rack, left to right. */
export function slotsOf(family: Family): SlotInfo[] {
  const io: ModuleKind[] = ["di", "dq", "dio", "ai", "aq", "cm"];
  switch (family) {
    case "S7-1200":
      return [
        ...[103, 102, 101].map((slot) => ({ slot, accepts: ["cm"] as ModuleKind[], label: String(slot) })),
        { slot: 1, accepts: ["cpu"], label: "1" },
        ...range(2, 9).map((slot) => ({ slot, accepts: io.filter((k) => k !== "cm"), label: String(slot) })),
      ];
    case "S7-1500":
      return [
        { slot: 0, accepts: ["ps"], label: "0" },
        { slot: 1, accepts: ["cpu"], label: "1" },
        ...range(2, 15).map((slot) => ({ slot, accepts: io, label: String(slot) })),
      ];
    case "S7-300":
      return [
        { slot: 1, accepts: ["ps"], label: "1" },
        { slot: 2, accepts: ["cpu"], label: "2" },
        { slot: 3, accepts: ["im"], label: "3" },
        ...range(4, 11).map((slot) => ({ slot, accepts: io, label: String(slot) })),
      ];
  }
}

function range(a: number, b: number) {
  return Array.from({ length: b - a + 1 }, (_, i) => a + i);
}

// ── Addresses ──

export interface IoSize {
  iBytes: number;
  qBytes: number;
  analog: boolean;
}

export function ioSize(e: CatalogEntry): IoSize {
  const di = Math.ceil((e.di ?? 0) / 8);
  const dq = Math.ceil((e.dq ?? 0) / 8);
  return { iBytes: di + (e.ai ?? 0) * 2, qBytes: dq + (e.aq ?? 0) * 2, analog: !!(e.ai || e.aq) };
}

/** Byte ranges of a CPU with on-board I/O: digital from 0, analog from 64 (S7-1200). */
function cpuRanges(e: CatalogEntry, m: HwModule): { i: [number, number][]; q: [number, number][] } {
  const i: [number, number][] = [];
  const q: [number, number][] = [];
  const di = Math.ceil((e.di ?? 0) / 8);
  const dq = Math.ceil((e.dq ?? 0) / 8);
  const base = m.iStart ?? 0;
  if (di) i.push([base, base + di]);
  if (dq) q.push([m.qStart ?? 0, (m.qStart ?? 0) + dq]);
  if (e.ai) i.push([64, 64 + e.ai * 2]);
  if (e.aq) q.push([64, 64 + e.aq * 2]);
  return { i, q };
}

export interface ModuleRanges {
  i: [number, number][];
  q: [number, number][];
}

/** The byte ranges `[from, to)` a module occupies in the input and output areas. */
export function rangesOf(family: Family, m: HwModule): ModuleRanges {
  const e = entryOf(family, m.catalogId);
  if (!e) return { i: [], q: [] };
  if (e.kind === "cpu") return cpuRanges(e, m);
  const s = ioSize(e);
  return {
    i: s.iBytes && m.iStart !== undefined ? [[m.iStart, m.iStart + s.iBytes]] : [],
    q: s.qBytes && m.qStart !== undefined ? [[m.qStart, m.qStart + s.qBytes]] : [],
  };
}

export function rangeText(r: [number, number][]): string {
  return r.map(([a, b]) => (b - a === 1 ? `${a}` : `${a}…${b - 1}`)).join(", ");
}

/** Default addresses for a module put in a slot: slot-fixed on S7-300, next free elsewhere. */
export function suggestAddresses(doc: HwDoc, entry: CatalogEntry, slot: number): { iStart?: number; qStart?: number } {
  const s = ioSize(entry);
  if (entry.kind === "cpu") return { iStart: entry.di ? 0 : undefined, qStart: entry.dq ? 0 : undefined };
  if (doc.family === "S7-300") {
    const base = s.analog ? 256 + 16 * (slot - 4) : 4 * (slot - 4);
    return { iStart: s.iBytes ? base : undefined, qStart: s.qBytes ? base : undefined };
  }
  const from = doc.family === "S7-1200" ? (s.analog ? 96 : 8) : 0;
  return {
    iStart: s.iBytes ? nextFree(doc, "i", s.iBytes, from) : undefined,
    qStart: s.qBytes ? nextFree(doc, "q", s.qBytes, from) : undefined,
  };
}

function nextFree(doc: HwDoc, area: "i" | "q", size: number, from: number): number {
  const used = doc.modules.flatMap((m) => rangesOf(doc.family, m)[area]);
  let start = from;
  for (;;) {
    const clash = used.find(([a, b]) => start < b && a < start + size);
    if (!clash) return start;
    start = clash[1];
  }
}

/** Modules whose address ranges overlap, as messages. */
export function addressConflicts(doc: HwDoc): string[] {
  const out: string[] = [];
  for (const area of ["i", "q"] as const) {
    const ranges = doc.modules.flatMap((m) => rangesOf(doc.family, m)[area].map((r) => ({ m, r })));
    for (let a = 0; a < ranges.length; a++) {
      for (let b = a + 1; b < ranges.length; b++) {
        const [x, y] = [ranges[a], ranges[b]];
        if (x.m === y.m) continue;
        if (x.r[0] < y.r[1] && y.r[0] < x.r[1]) {
          out.push(`${area === "i" ? "Input" : "Output"} bytes of ${x.m.name} (slot ${x.m.slot}) and ${y.m.name} (slot ${y.m.slot}) overlap`);
        }
      }
    }
  }
  return out;
}

// ── Channels ──

export interface Channel {
  /** `DI 3`, `AI 0`. */
  label: string;
  kind: "di" | "dq" | "ai" | "aq";
  address: string;
  dataType: "Bool" | "Int";
}

export function channelsOf(family: Family, m: HwModule): Channel[] {
  const e = entryOf(family, m.catalogId);
  if (!e) return [];
  const out: Channel[] = [];
  const bits = (n: number, start: number, kind: "di" | "dq", area: "I" | "Q") => {
    for (let c = 0; c < n; c++) out.push({ label: `${kind.toUpperCase()} ${c}`, kind, address: `${area}${start + (c >> 3)}.${c & 7}`, dataType: "Bool" });
  };
  const words = (n: number, start: number, kind: "ai" | "aq", area: "I" | "Q") => {
    for (let c = 0; c < n; c++) out.push({ label: `${kind.toUpperCase()} ${c}`, kind, address: `${area}W${start + 2 * c}`, dataType: "Int" });
  };
  if (e.kind === "cpu") {
    if (e.di) bits(e.di, m.iStart ?? 0, "di", "I");
    if (e.dq) bits(e.dq, m.qStart ?? 0, "dq", "Q");
    if (e.ai) words(e.ai, 64, "ai", "I");
    if (e.aq) words(e.aq, 64, "aq", "Q");
    return out;
  }
  const di = Math.ceil((e.di ?? 0) / 8);
  const dq = Math.ceil((e.dq ?? 0) / 8);
  if (e.di && m.iStart !== undefined) bits(e.di, m.iStart, "di", "I");
  if (e.dq && m.qStart !== undefined) bits(e.dq, m.qStart, "dq", "Q");
  if (e.ai && m.iStart !== undefined) words(e.ai, m.iStart + di, "ai", "I");
  if (e.aq && m.qStart !== undefined) words(e.aq, m.qStart + dq, "aq", "Q");
  return out;
}

/** Tag rows for every channel of the modules: `DI16_1_DI_3` at `I8.3`, ... */
export function channelTags(doc: HwDoc, modules: HwModule[]): TagDoc["rows"] {
  return modules.flatMap((m) =>
    channelsOf(doc.family, m).map((c) => ({
      name: `${m.name.replace(/[^A-Za-z0-9_]+/g, "_")}_${c.label.replace(" ", "_")}`,
      dataType: c.dataType,
      address: c.address,
      comment: `${m.name}, slot ${m.slot}, ${c.label}`,
    })),
  );
}

// ── Documents ──

export function emptyHwDoc(family: Family = "S7-1200"): HwDoc {
  return { format: "voltrus-s7-hw", version: 1, family, modules: [] };
}

export function parseHwDoc(text: string): HwDoc {
  if (!text.trim()) return emptyHwDoc();
  const raw = JSON.parse(text);
  const family: Family = FAMILIES.includes(raw?.family) ? raw.family : "S7-1200";
  const modules: HwModule[] = (Array.isArray(raw?.modules) ? raw.modules : [])
    .filter((m: unknown): m is Record<string, unknown> => !!m && typeof m === "object")
    .map((m: Record<string, unknown>) => ({
      slot: Number(m.slot),
      catalogId: String(m.catalogId ?? ""),
      name: String(m.name ?? ""),
      articleNo: typeof m.articleNo === "string" && m.articleNo ? m.articleNo : undefined,
      firmware: typeof m.firmware === "string" && m.firmware ? m.firmware : undefined,
      iStart: typeof m.iStart === "number" ? m.iStart : undefined,
      qStart: typeof m.qStart === "number" ? m.qStart : undefined,
      comment: typeof m.comment === "string" && m.comment ? m.comment : undefined,
    }))
    .filter((m: HwModule) => Number.isFinite(m.slot));
  return { format: "voltrus-s7-hw", version: 1, family, modules };
}

export function serializeHwDoc(doc: HwDoc): string {
  const modules = [...doc.modules].sort((a, b) => a.slot - b.slot);
  return `${JSON.stringify({ ...doc, modules }, (_k, v) => (v === undefined ? undefined : v), 2)}\n`;
}

/** Put a catalog module in a slot (replacing what is there), with suggested addresses. */
export function placeModule(doc: HwDoc, entry: CatalogEntry, slot: number): HwDoc {
  const rest = doc.modules.filter((m) => m.slot !== slot);
  const base = { ...doc, modules: rest };
  const sameKind = rest.filter((m) => entryOf(doc.family, m.catalogId)?.kind === entry.kind).length;
  const name = entry.kind === "cpu" ? "PLC_1" : `${entry.label.split(" ×")[0].split(" /")[0].replace(/\s+/g, "")}_${sameKind + 1}`;
  const module: HwModule = { slot, catalogId: entry.id, name, ...suggestAddresses(base, entry, slot) };
  return { ...doc, modules: [...rest, module] };
}

/** The first empty slot that takes this kind of module. */
export function freeSlotFor(doc: HwDoc, entry: CatalogEntry): number | undefined {
  const taken = new Set(doc.modules.map((m) => m.slot));
  return slotsOf(doc.family).find((s) => s.accepts.includes(entry.kind) && !taken.has(s.slot))?.slot;
}

/** Best catalog CPU for an order number read from the CPU (`6ES7 214-...` is a 1214C). */
export function cpuFromOrderCode(family: Family, order: string, moduleType: string): CatalogEntry | undefined {
  const cpus = catalogFor(family).filter((e) => e.kind === "cpu");
  const text = `${moduleType} ${order}`.toUpperCase();
  const byName = cpus.find((e) => text.includes(e.label.replace("CPU ", "").split(" ")[0]));
  if (byName) return byName;
  const m = order.replace(/\s+/g, "").match(/^6ES7(\d{3})/);
  if (!m) return undefined;
  const code = m[1];
  const table: Record<string, string> = {
    "211": "cpu1211c", "212": "cpu1212c", "214": "cpu1214c", "215": "cpu1215c", "217": "cpu1217c",
    "511": "cpu1511", "513": "cpu1513", "515": "cpu1515", "516": "cpu1516", "517": "cpu1517",
    "312": "cpu312", "314": "cpu314", "315": "cpu315dp", "317": "cpu317pn", "318": "cpu319",
  };
  return cpus.find((e) => e.id === table[code]);
}

/** The family an order number belongs to, if it tells. */
export function familyFromOrderCode(order: string): Family | undefined {
  const m = order.replace(/\s+/g, "").match(/^6ES7(\d)/);
  if (!m) return undefined;
  return m[1] === "2" ? "S7-1200" : m[1] === "5" ? "S7-1500" : m[1] === "3" ? "S7-300" : undefined;
}
