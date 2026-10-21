// Minimal EDS (Electronic Data Sheet) parser for the object browser.
// Parses INI-style [Param], [Assembly] and [Enum] sections so the UI can show
// friendly names ("Motor Speed") instead of raw class/instance/attribute.

export interface EdsParam {
  number: number;
  name: string;
  dataType?: string;
  dataSize?: number;
  class?: number;
  instance?: number;
  attribute?: number;
  help?: string;
  units?: string;
}

export interface EdsAssembly {
  number: number;
  name: string;
  /** Member parameter numbers (or instance refs) listed in this assembly. */
  members: number[];
}

export interface EdsEnum {
  number: number;
  name: string;
  /** value/ bit → label */
  pairs: { value: number; label: string }[];
}

export interface Eds {
  fileName: string;
  params: EdsParam[];
  assemblies: EdsAssembly[];
  enums: EdsEnum[];
}

// ── INI parsing ──

function parseIni(text: string): Map<string, Map<string, string>> {
  const sections = new Map<string, Map<string, string>>();
  let current = "DEFAULT";
  sections.set(current, new Map());
  for (const raw of text.split(/\r?\n/)) {
    let line = raw.trim();
    if (!line) continue;
    if (line.startsWith(";") || line.startsWith("#") || line.startsWith("[")) {
      if (line.startsWith("[")) {
        const m = line.match(/^\[(.+)\]$/);
        if (m) {
          current = m[1].trim();
          if (!sections.has(current)) sections.set(current, new Map());
        }
      }
      continue;
    }
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    // strip inline comment
    const valRaw = line.slice(eq + 1);
    const cmt = valRaw.search(/[;#]/);
    const val = (cmt >= 0 ? valRaw.slice(0, cmt) : valRaw).trim().replace(/^"|"$/g, "");
    sections.get(current)!.set(key.toLowerCase(), val);
  }
  return sections;
}

const num = (s: string | undefined): number | undefined => {
  if (s === undefined) return undefined;
  const t = s.trim();
  if (!t) return undefined;
  if (/^0x/i.test(t)) return parseInt(t, 16);
  const n = Number(t);
  return Number.isFinite(n) ? n : undefined;
};

// ── EDS extraction ──

export function parseEds(text: string, fileName = "device.eds"): Eds {
  const ini = parseIni(text);
  const params: EdsParam[] = [];
  const assemblies: EdsAssembly[] = [];
  const enums: EdsEnum[] = [];

  for (const [section, kv] of ini) {
    const pm = section.match(/^Param(\d+)$/i);
    if (pm) {
      params.push({
        number: Number(pm[1]),
        name: kv.get("name") ?? "",
        dataType: kv.get("datatype"),
        dataSize: num(kv.get("datasize")),
        class: num(kv.get("class")),
        instance: num(kv.get("instance")),
        attribute: num(kv.get("attribute")),
        help: kv.get("help"),
        units: kv.get("units"),
      });
      continue;
    }
    const am = section.match(/^Assembly(\d+)$/i);
    if (am) {
      const memberStr = kv.get("member") ?? "";
      const members = memberStr
        .split(",")
        .map((s) => num(s.trim()))
        .filter((n): n is number => n !== undefined);
      assemblies.push({ number: Number(am[1]), name: kv.get("name") ?? "", members });
      continue;
    }
    const em = section.match(/^Enum(\d+)$/i);
    if (em) {
      const pairs: { value: number; label: string }[] = [];
      for (const [k, v] of kv) {
        const val = num(k);
        if (val !== undefined) pairs.push({ value: val, label: v });
      }
      enums.push({ number: Number(em[1]), name: kv.get("name") ?? "", pairs });
    }
  }

  return { fileName, params, assemblies, enums };
}

// ── lookup map ──

export interface EdsMap {
  /** `${class}:${instance}:${attribute}` → param */
  attr: Map<string, EdsParam>;
  /** `${class}:${instance}` → assembly */
  assembly: Map<string, EdsAssembly>;
  params: EdsParam[];
}

export function buildEdsMap(eds: Eds | null): EdsMap {
  const attr = new Map<string, EdsParam>();
  const assembly = new Map<string, EdsAssembly>();
  if (eds) {
    for (const p of eds.params) {
      if (p.class !== undefined && p.instance !== undefined && p.attribute !== undefined) {
        attr.set(`${p.class}:${p.instance}:${p.attribute}`, p);
      }
    }
    for (const a of eds.assemblies) {
      // Assembly instance numbers live in the connection path; key by class 0x04 + instance.
      assembly.set(`4:${a.number}`, a);
    }
  }
  return { attr, assembly, params: eds?.params ?? [] };
}

export function edsAttrName(map: EdsMap, cls: number, inst: number, attr: number): string | undefined {
  return map.attr.get(`${cls}:${inst}:${attr}`)?.name || undefined;
}

// ── persistence ──

const KEY = "enip-explorer.eds";
export function saveEds(eds: Eds) {
  try {
    localStorage.setItem(KEY, JSON.stringify(eds));
  } catch {
    /* ignore quota */
  }
}
export function loadEds(): Eds | null {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Eds) : null;
  } catch {
    return null;
  }
}
export function clearEds() {
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
}
