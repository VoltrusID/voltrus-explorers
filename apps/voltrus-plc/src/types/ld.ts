import type {
  IECDataType,
  RegisterType,
  PinDescriptor,
  BlockDefinition,
} from "./fbd";
import {
  REGISTER_TYPES,
  formatRegisterAddress,
  parseRegisterAddress,
  getBlockDef,
} from "./fbd";

export type {
  IECDataType,
  RegisterType,
  PinDescriptor,
  BlockDefinition,
} from "./fbd";
export { REGISTER_TYPES, formatRegisterAddress, parseRegisterAddress, getBlockDef };

// --- LD Element Types ---

export type LdElementType =
  | "contact_no"
  | "contact_nc"
  | "contact_re"
  | "contact_fe"
  | "coil"
  | "coil_set"
  | "coil_reset"
  | "coil_toggle"
  | "fb_block";

export type LdElementCategory = "contact" | "coil" | "timer" | "counter" | "function";

export interface LdElementDef {
  type: LdElementType;
  label: string;
  symbol: string;
  category: LdElementCategory;
  blockType?: string;
}

export const LD_ELEMENT_DEFS: LdElementDef[] = [
  { type: "contact_no", label: "NO Contact", symbol: "| |", category: "contact" },
  { type: "contact_nc", label: "NC Contact", symbol: "|/|", category: "contact" },
  { type: "contact_re", label: "Rising Edge", symbol: "|P|", category: "contact" },
  { type: "contact_fe", label: "Falling Edge", symbol: "|N|", category: "contact" },
  { type: "coil", label: "Coil", symbol: "( )", category: "coil" },
  { type: "coil_set", label: "Set Coil", symbol: "(S)", category: "coil" },
  { type: "coil_reset", label: "Reset Coil", symbol: "(R)", category: "coil" },
  { type: "coil_toggle", label: "Toggle Coil", symbol: "(T)", category: "coil" },
  { type: "fb_block", label: "TON", symbol: "TON", category: "timer", blockType: "TON" },
  { type: "fb_block", label: "TOF", symbol: "TOF", category: "timer", blockType: "TOF" },
  { type: "fb_block", label: "TP", symbol: "TP", category: "timer", blockType: "TP" },
  { type: "fb_block", label: "CTU", symbol: "CTU", category: "counter", blockType: "CTU" },
  { type: "fb_block", label: "CTD", symbol: "CTD", category: "counter", blockType: "CTD" },
  { type: "fb_block", label: "MOVE", symbol: "MOV", category: "function", blockType: "MOVE" },
  { type: "fb_block", label: "ADD", symbol: "ADD", category: "function", blockType: "ADD" },
  { type: "fb_block", label: "GT", symbol: "CMP", category: "function", blockType: "GT" },
];

export function getLdElementDef(type: string, blockType?: string): LdElementDef | undefined {
  return LD_ELEMENT_DEFS.find((d) => d.type === type && (!d.blockType || d.blockType === blockType));
}

// --- Grid Data Model ---

export const isContactType = (t: LdElementType) =>
  t === "contact_no" || t === "contact_nc" || t === "contact_re" || t === "contact_fe";

export const isCoilType = (t: LdElementType) =>
  t === "coil" || t === "coil_set" || t === "coil_reset" || t === "coil_toggle";

export interface LdElement {
  id: string;
  type: LdElementType;
  variable?: string;
  variableType?: IECDataType;
  registerType?: RegisterType;
  registerNumber?: number;
  blockType?: string;
  instanceName?: string;
  pinValues?: Record<string, string>;
}

export interface LdPath {
  id: string;
  elements: LdElement[];
}

export interface LdRung {
  id: string;
  label?: string;
  paths: LdPath[];
}

export interface LdProgram {
  version: string;
  programName: string;
  rungs: LdRung[];
}

export interface LdCursor {
  rungIndex: number;
  pathIndex: number;
  colIndex: number;
}

export const MIN_CONTACT_COLS = 4;

export const isOutputType = (t: LdElementType) => isCoilType(t) || t === "fb_block";

let _nextId = 0;
export function nextLdId(prefix = "e"): string {
  return `${prefix}_${++_nextId}`;
}

export function makeLdElement(type: LdElementType, overrides?: Partial<LdElement>): LdElement {
  return {
    id: nextLdId("e"),
    type,
    ...overrides,
  };
}

export function makeLdPath(elements: LdElement[]): LdPath {
  return { id: nextLdId("p"), elements };
}

export function makeLdRung(paths: LdPath[], label?: string): LdRung {
  return { id: nextLdId("r"), paths, label };
}
