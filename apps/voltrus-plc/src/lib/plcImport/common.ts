// Shared pieces of the engineering-export importers: names, data types, addresses.

/**
 * Turn a symbol name into an IEC 61131-3 identifier: letters, digits and single underscores,
 * not starting with a digit. `Motor 1 "Speed"` becomes `Motor_1_Speed`.
 */
export function toIdentifier(name: string): string {
  let id = name
    .trim()
    .replace(/[^A-Za-z0-9_]/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "");
  if (!id) id = "Unnamed";
  if (/^[0-9]/.test(id)) id = `T_${id}`;
  return id;
}

/** Make every identifier in a list unique by appending _2, _3, ... */
export function uniqueIdentifiers(names: string[]): string[] {
  const seen = new Map<string, number>();
  return names.map((n) => {
    const base = toIdentifier(n);
    const key = base.toUpperCase();
    const count = (seen.get(key) ?? 0) + 1;
    seen.set(key, count);
    return count === 1 ? base : `${base}_${count}`;
  });
}

const ELEMENTARY: Record<string, string> = {
  BOOL: "BOOL",
  BYTE: "BYTE",
  WORD: "WORD",
  DWORD: "DWORD",
  LWORD: "LWORD",
  SINT: "SINT",
  USINT: "USINT",
  INT: "INT",
  UINT: "UINT",
  DINT: "DINT",
  UDINT: "UDINT",
  LINT: "LINT",
  ULINT: "ULINT",
  REAL: "REAL",
  LREAL: "LREAL",
  TIME: "TIME",
  LTIME: "LTIME",
  S5TIME: "TIME",
  DATE: "DATE",
  TIME_OF_DAY: "TIME_OF_DAY",
  TOD: "TIME_OF_DAY",
  DATE_AND_TIME: "DT",
  DT: "DT",
  CHAR: "CHAR",
  WCHAR: "WCHAR",
  STRING: "STRING",
  WSTRING: "WSTRING",
  TON_TIME: "TON",
  TOF_TIME: "TOF",
  TP_TIME: "TP",
  IEC_TIMER: "TON",
  CTU_INT: "CTU",
  CTD_INT: "CTD",
  CTUD_INT: "CTUD",
  IEC_COUNTER: "CTU",
  R_TRIG: "R_TRIG",
  F_TRIG: "F_TRIG",
};

/** Types with no ST equivalent here: pointers, any-types, hardware identifiers. */
const UNSUPPORTED = /^(VARIANT|ANY|POINTER|DB_ANY|BLOCK_\w+|HW_\w+|CONN_\w+|EVENT_\w+|OB_\w+|PORT|AOM_IDENT|REMOTE|DTL)$/i;

export interface TypeMapping {
  st: string;
  warning?: string;
}

/**
 * Map an elementary type name from an export (`Int`, `String[20]`, `"MyUdt"`) to ST.
 * Arrays and structs are handled by the callers that parse them.
 */
export function mapElementaryType(raw: string): TypeMapping {
  const t = raw.trim();
  const quoted = t.match(/^"(.+)"$/);
  if (quoted) return { st: toIdentifier(quoted[1]) };
  const sized = t.match(/^(W?STRING)\s*\[\s*(\d+)\s*\]$/i);
  if (sized) return { st: `${sized[1].toUpperCase()}[${sized[2]}]` };
  const upper = t.toUpperCase();
  if (ELEMENTARY[upper]) {
    const st = ELEMENTARY[upper];
    if (upper === "S5TIME") return { st, warning: "S5TIME became TIME" };
    if (upper === "IEC_TIMER") return { st, warning: "IEC_TIMER became TON; use TOF/TP if the instance is called as one" };
    if (upper === "IEC_COUNTER") return { st, warning: "IEC_COUNTER became CTU" };
    return { st };
  }
  if (UNSUPPORTED.test(t)) return { st: t, warning: `${t} has no equivalent here` };
  return { st: toIdentifier(t) };
}

/** The display type the Online watch table uses for an elementary type, or "" if it has none. */
export function watchTypeFor(raw: string): string {
  const t = raw.trim().toUpperCase().replace(/\s+/g, "");
  if (/^STRING(\[\d+\])?$/.test(t)) return "STRING";
  const map: Record<string, string> = {
    BOOL: "BOOL", BYTE: "BYTE", WORD: "WORD", DWORD: "DWORD", LWORD: "LWORD",
    SINT: "SINT", USINT: "USINT", INT: "INT", UINT: "UINT", DINT: "DINT", UDINT: "UDINT",
    LINT: "LINT", ULINT: "ULINT", REAL: "REAL", LREAL: "LREAL", TIME: "TIME", CHAR: "CHAR",
    S5TIME: "WORD", DATE: "UINT", TIME_OF_DAY: "UDINT", TOD: "UDINT", TIMER: "S5TIME", COUNTER: "COUNTER",
  };
  return map[t] ?? "";
}

/**
 * Absolute S7 address to IEC notation: `%I0.0` to `%IX0.0`, `%MW10` stays, `%DB1.DBX0.0` has no
 * IEC form (null).
 */
export function s7ToIecAddress(address: string): string | null {
  const a = address.trim().replace(/^%/, "").toUpperCase().replace(/^E/, "I").replace(/^A(?=[BWD]?\d)/, "Q");
  const bit = a.match(/^([IQM])(\d+)\.([0-7])$/);
  if (bit) return `%${bit[1]}X${bit[2]}.${bit[3]}`;
  const word = a.match(/^([IQM])([BWDL])(\d+)$/);
  if (word) return `%${word[1]}${word[2]}${word[3]}`;
  const peripheral = a.match(/^P([IQ])([BWD])(\d+)(:P)?$/);
  if (peripheral) return `%${peripheral[1]}${peripheral[2]}${peripheral[3]}`;
  return null;
}

/** Absolute address in the form the Online watch table reads (`I0.0`, `MW10`, `DB1.DBD4`), or null. */
export function s7WatchAddress(address: string): string | null {
  const a = address.trim().replace(/^%/, "").toUpperCase().replace(/:P$/, "");
  if (/^[IQM]\d+\.[0-7]$/.test(a) || /^[IQM][BWD]\d+$/.test(a)) return a;
  if (/^DB\d+\.DB[XBWD]\d+(\.[0-7])?$/.test(a)) return a;
  if (/^[TC]\d+$/.test(a)) return a;
  return null;
}
