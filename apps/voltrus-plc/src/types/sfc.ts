import type { Node, Edge } from "@xyflow/react";
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

// --- SFC Element Types ---

export type SfcElementType =
  | "step"
  | "initial_step"
  | "transition"
  | "or_diverge"
  | "or_converge"
  | "and_diverge"
  | "and_converge"
  | "jump";

export type SfcElementCategory = "step" | "transition" | "branch" | "jump";

// --- Action Qualifiers ---

export type ActionQualifier = "N" | "S" | "R" | "P1" | "P0";

export interface SfcAction {
  qualifier: ActionQualifier;
  variableName: string;
  expression?: string;
  description?: string;
}

// --- Element Definitions ---

export interface SfcElementDef {
  type: SfcElementType;
  label: string;
  symbol: string;
  category: SfcElementCategory;
}

export const SFC_ELEMENT_DEFS: SfcElementDef[] = [
  { type: "step", label: "Step", symbol: "[ ]", category: "step" },
  { type: "initial_step", label: "Initial Step", symbol: "[[ ]]", category: "step" },
  { type: "transition", label: "Transition", symbol: "---", category: "transition" },
  { type: "or_diverge", label: "OR Diverge", symbol: "<>", category: "branch" },
  { type: "or_converge", label: "OR Converge", symbol: "><", category: "branch" },
  { type: "jump", label: "Jump", symbol: ">>", category: "jump" },
];

export function getSfcElementDef(type: string): SfcElementDef | undefined {
  return SFC_ELEMENT_DEFS.find((d) => d.type === type);
}

// --- Node Data ---

export interface SfcNodeData {
  [key: string]: unknown;
  elementType: SfcElementType;
  stepName?: string;
  actions?: SfcAction[];
  condition?: string;
  targetStepName?: string;
  label?: string;
  pinValues: Record<string, string>;
}

export type SfcNode = Node<SfcNodeData>;
export type SfcEdge = Edge;

// --- Program ---

export interface SfcProgram {
  version: string;
  programName: string;
  nodes: SfcNode[];
  edges: SfcEdge[];
  viewport: { x: number; y: number; zoom: number };
}
