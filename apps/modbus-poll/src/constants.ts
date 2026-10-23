import type { ReadBlock } from "./types";

export const AREAS = ["holding", "input", "coil", "discrete"];
export const TYPES = ["uint16", "int16", "int32", "float32", "float64"];
/**
 * What a fresh install polls: ten holding registers from address 0. It is deliberately generic: nearly every device answers
 * it (the old default was a PM5560 map at 2999..3034, which any other device rejects with "Illegal Data Address"). The
 * PM5560 map lives on as a slave simulator template (`templates.ts`) the user picks.
 */
export function defaultBlocks(): ReadBlock[] {
  return [{ name: "Registers", address: 0, quantity: 10, type: "uint16", area: "holding" }];
}

/** What each register area is, for the picker sheets on a phone. */
export const AREA_HINTS: Record<string, string> = {
  holding: "Registers you can read and write (function 03)",
  input: "Read-only registers (function 04)",
  coil: "Bits you can read and write (function 01)",
  discrete: "Read-only bits (function 02)",
};
export const TYPE_HINTS: Record<string, string> = {
  uint16: "1 register, 0 to 65535",
  int16: "1 register, signed",
  int32: "2 registers, signed",
  float32: "2 registers, IEEE 754 single",
  float64: "4 registers, IEEE 754 double",
};
/** Select value for the in-app virtual RTU slave (a PTY, not a path on disk). */
export const VIRTUAL_PORT = "__virtual__";
export const ENDIAN_OPTS: { value: string; label: string; hint: string }[] = [
  { value: "big", label: "AB CD", hint: "High word first (the Modbus standard)" },
  { value: "little", label: "CD AB", hint: "Low word first (word swap)" },
  { value: "swapped", label: "BA DC", hint: "Bytes swapped inside each word" },
];

export const sidebarStyle: React.CSSProperties = {
  width: 264,
  flexShrink: 0,
  borderRight: "1px solid var(--border)",
  background: "var(--card)",
  padding: 16,
  overflowY: "auto",
  display: "flex",
  flexDirection: "column",
  gap: 16,
};

export const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "6px 12px",
  borderBottom: "1px solid var(--border)",
  color: "var(--text-muted)",
  fontSize: "var(--fs-xs)",
  fontWeight: 600,
  position: "sticky" as const,
  top: 0,
  background: "var(--card)",
  whiteSpace: "nowrap" as const,
};


/** Free tier: polled values across all blocks (one float is 2 registers but 1 value). */
export const FREE_VALUES = 20;
/** Free tier: saved connection profiles. */
export const FREE_PROFILE_LIMIT = 1;
/** Free tier: connections open at once (one tab). */
export const FREE_CONNECTIONS = 1;
