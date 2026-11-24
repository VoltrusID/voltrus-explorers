// SCL sources (`.scl`, generated from blocks or written by hand) to IEC 61131-3 ST.
//
// SCL is ST with vendor extensions, so the conversion is mostly lexical: `#local` and
// `"global"` names become plain identifiers, block attributes and header lines go, REGIONs
// become comments, organization blocks become PROGRAMs and vendor timer types become the IEC
// ones. What has no ST equivalent is left in place and reported, so the compiler points at it.
import { mapElementaryType, s7ToIecAddress, toIdentifier } from "./common";

export interface SclResult {
  st: string;
  warnings: string[];
  /** Names of the POUs found, for the file name. */
  pous: string[];
}

const HEADER_LINE = /^\s*(VERSION\s*:|TITLE\s*=|AUTHOR\s*:|FAMILY\s*:|NAME\s*:|KNOW_HOW_PROTECT\b|NON_RETAIN\s*$|S7_\w+\s*:=)/i;

const TYPE_WORDS = /\b(TON_TIME|TOF_TIME|TP_TIME|IEC_TIMER|IEC_COUNTER|CTU_INT|CTD_INT|CTUD_INT|S5TIME)\b/gi;

const UNSUPPORTED_CALLS = /\b(PEEK|POKE|PEEK_BOOL|POKE_BOOL|POKE_BLK|SCATTER|GATHER|SERIALIZE|DESERIALIZE|TypeOf|TypeOfElements|VariantGet|VariantPut|CountOfElements|RD_SYS_T|WR_SYS_T|RUNTIME|GET_ERR_ID|GET_ERROR|TSEND_C|TRCV_C|TCON|TDISCON|MB_CLIENT|MB_SERVER|ATTACH|DETACH)\s*\(/gi;

/** Convert the code part of a line (no strings or comments in it). */
function convertCode(code: string, warn: (msg: string) => void): string {
  let out = code;
  // #local -> local
  out = out.replace(/#([A-Za-z_][A-Za-z0-9_]*)/g, (m, id, offset, whole: string) => {
    // Keep typed literals such as T#5s, 16#FF, INT#3 (a word or digit right before #).
    const before = whole[offset - 1];
    return before && /[A-Za-z0-9_]/.test(before) ? m : id;
  });
  out = out.replace(/\bS5T#/gi, "T#");
  // Absolute addresses
  out = out.replace(/%DB\d+\.DB[XBWD]\d+(\.\d)?/gi, (m) => {
    warn(`${m}: absolute DB addresses have no ST form; use a symbolic name`);
    return m;
  });
  out = out.replace(/%[IQMEA][BWDX]?\d+(\.\d)?(:P)?/gi, (m) => {
    const iec = s7ToIecAddress(m);
    if (!iec) {
      warn(`${m}: address has no IEC form`);
      return m;
    }
    return iec;
  });
  out = out.replace(TYPE_WORDS, (m) => {
    const mapped = mapElementaryType(m);
    if (mapped.warning) warn(mapped.warning);
    return mapped.st;
  });
  out = out.replace(/\bVoid\b/g, "VOID");
  out = out.replace(/\bARRAY\s*\[/gi, "ARRAY[").replace(/\]\s+OF\s+/gi, "] OF ");
  // Single-instance calls through the instance DB: "IEC_Timer_0_DB".TON(...) -> IEC_Timer_0_DB(...)
  out = out.replace(/\b([A-Za-z_][A-Za-z0-9_]*)(\u0002?)\.(TON|TOF|TP|CTU|CTD|CTUD)\s*\(/g, (_m, inst, mark, fb) => {
    warn(`${inst}: called as a single instance of ${fb}; declare "${inst} : ${fb};" in the POU`);
    return `${inst}${mark}(`;
  });
  if (/\bGOTO\b/i.test(out)) warn("GOTO has no ST equivalent");
  for (const m of out.matchAll(UNSUPPORTED_CALLS)) warn(`${m[1]}() is a system function with no equivalent here`);
  if (/\b(Variant|DB_ANY|Any|Pointer)\b/.test(out)) warn("Variant/ANY/POINTER parameters have no equivalent here");
  if (/\bENO\b/.test(out)) warn("ENO has no equivalent here");
  return out;
}

export function sclToSt(src: string): SclResult {
  const warnings = new Set<string>();
  const warn = (msg: string) => warnings.add(msg);
  const pous: string[] = [];
  // Quoted names used inside a block body: global tags or DBs the POU does not declare.
  const globals = new Set<string>();

  // Pass 1: walk the text, converting code and keeping strings and comments verbatim.
  let out = "";
  let code = "";
  const flush = () => {
    out += convertCode(code, warn);
    code = "";
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const two = src.slice(i, i + 2);
    if (two === "//") {
      flush();
      const end = src.indexOf("\n", i);
      out += src.slice(i, end < 0 ? undefined : end);
      i = end < 0 ? src.length : end;
    } else if (two === "(*" || two === "/*") {
      flush();
      const close = two === "(*" ? "*)" : "*/";
      const end = src.indexOf(close, i + 2);
      const body = src.slice(i + 2, end < 0 ? undefined : end);
      out += `(*${body.replace(/\*\)/g, "* )")}*)`;
      i = end < 0 ? src.length : end + 2;
    } else if (c === "'") {
      flush();
      let j = i + 1;
      while (j < src.length && src[j] !== "'") j += src[j] === "$" ? 2 : 1;
      out += src.slice(i, j + 1);
      i = j + 1;
    } else if (c === '"') {
      const end = src.indexOf('"', i + 1);
      // Marked so pass 2 can tell names used in a body (globals) from declarations.
      code += `\u0001${toIdentifier(src.slice(i + 1, end < 0 ? undefined : end))}\u0002`;
      i = end < 0 ? src.length : end + 1;
    } else if (c === "{") {
      // Block and member attributes: { S7_Optimized_Access := 'TRUE' }, { ExternalAccessible := 'False' }
      const end = src.indexOf("}", i);
      i = end < 0 ? src.length : end + 1;
      // `name {attr} : Type` -> `name : Type`, not `name  : Type`.
      if (code.endsWith(" ")) while (src[i] === " " || src[i] === "\t") i++;
    } else {
      code += c;
      i++;
    }
  }
  flush();

  // Pass 2: line-level rewrites.
  const lines: string[] = [];
  let inPou = false;
  let declDone = false;
  let functionName: string | null = null;
  for (const marked of out.split("\n")) {
    if (inPou && declDone) for (const m of marked.matchAll(/\u0001([^\u0002]*)\u0002/g)) globals.add(m[1]);
    const raw = marked.replace(/[\u0001\u0002]/g, "");
    const line = raw.replace(/\s+$/, "");
    const trimmed = line.trim();
    const upper = trimmed.toUpperCase();
    const pou = trimmed.match(/^(FUNCTION_BLOCK|FUNCTION|ORGANIZATION_BLOCK|PROGRAM)\s+([A-Za-z_][A-Za-z0-9_]*)(.*)$/i);
    if (pou) {
      const kind = pou[1].toUpperCase() === "ORGANIZATION_BLOCK" ? "PROGRAM" : pou[1].toUpperCase();
      pous.push(pou[2]);
      functionName = kind === "FUNCTION" && !/:\s*VOID\b/i.test(pou[3]) ? pou[2] : null;
      inPou = true;
      declDone = false;
      lines.push(`${kind} ${pou[2]}${pou[3]}`);
      continue;
    }
    if (/^END_ORGANIZATION_BLOCK\b/i.test(trimmed)) {
      lines.push(line.replace(/END_ORGANIZATION_BLOCK/i, "END_PROGRAM"));
      inPou = false;
      continue;
    }
    if (/^END_(FUNCTION_BLOCK|FUNCTION|PROGRAM)\b/i.test(trimmed)) {
      lines.push(line);
      inPou = false;
      continue;
    }
    if (/^DATA_BLOCK\b/i.test(trimmed)) warn("Data blocks in an SCL file are skipped; import the .db source instead");
    if (inPou && !declDone && HEADER_LINE.test(trimmed)) continue;
    if (inPou && upper === "BEGIN") {
      declDone = true;
      continue;
    }
    if (/^VAR_STAT\b/i.test(trimmed)) {
      lines.push(line.replace(/VAR_STAT/i, "VAR"));
      continue;
    }
    const region = trimmed.match(/^REGION\b\s*(.*)$/i);
    if (region) {
      lines.push(line.replace(trimmed, `(* REGION ${region[1]} *)`));
      continue;
    }
    if (/^END_REGION\b/i.test(trimmed)) {
      lines.push(line.replace(trimmed, "(* END_REGION *)"));
      continue;
    }
    if (functionName && new RegExp(`^${functionName}\\s*:=`, "i").test(trimmed)) {
      warn(`${functionName}: the return value is assigned by name, which the compiler does not support yet`);
    }
    lines.push(line);
  }

  const used = [...globals].filter((g) => !pous.includes(g) && !/^(TON|TOF|TP|CTU|CTD|CTUD)$/i.test(g));
  if (used.length > 0) {
    warn(`Uses global tags or DBs: ${used.join(", ")}. Declare them in the POU (a tag table or DB import gives you the declarations).`);
  }
  if (pous.length === 0) warn("No FUNCTION, FUNCTION_BLOCK or ORGANIZATION_BLOCK found");
  return { st: lines.join("\n").replace(/\n{3,}/g, "\n\n"), warnings: [...warnings], pous };
}
