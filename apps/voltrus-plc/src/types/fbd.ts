import type { Node, Edge } from "@xyflow/react";

export type BlockCategory =
  | "logic"
  | "arithmetic"
  | "comparison"
  | "timer"
  | "counter"
  | "move"
  | "selection"
  | "bistable"
  | "edge"
  | "input"
  | "output";

export type IECDataType =
  | "BOOL"
  | "INT"
  | "DINT"
  | "REAL"
  | "TIME"
  | "STRING";

export type RegisterType = "X" | "Y" | "M" | "D" | "T" | "C" | "SM" | "SD";

export const REGISTER_TYPES: Array<{ id: RegisterType; label: string; description: string; iecType: IECDataType }> = [
  { id: "X", label: "X", description: "Input", iecType: "BOOL" },
  { id: "Y", label: "Y", description: "Output", iecType: "BOOL" },
  { id: "M", label: "M", description: "Internal Relay", iecType: "BOOL" },
  { id: "D", label: "D", description: "Data Register", iecType: "INT" },
  { id: "T", label: "T", description: "Timer Value", iecType: "INT" },
  { id: "C", label: "C", description: "Counter Value", iecType: "INT" },
  { id: "SM", label: "SM", description: "Special Relay", iecType: "BOOL" },
  { id: "SD", label: "SD", description: "Special Register", iecType: "INT" },
];

export function formatRegisterAddress(regType: RegisterType, number: number): string {
  return `${regType}${number}`;
}

export function parseRegisterAddress(name: string): { regType: RegisterType; number: number } | null {
  const m = name.match(/^(X|Y|M|D|T|C|SM|SD)(\d+)$/);
  if (!m) return null;
  return { regType: m[1] as RegisterType, number: parseInt(m[2]) };
}

export function isRegisterAddress(name: string): boolean {
  return parseRegisterAddress(name) !== null;
}

export interface PinDescriptor {
  name: string;
  dataType: IECDataType;
  direction: "input" | "output";
}

export interface BlockDefinition {
  type: string;
  label: string;
  category: BlockCategory;
  pins: PinDescriptor[];
  stTemplate: string;
  isFunctionBlock?: boolean;
}

export const BLOCK_LIBRARY: BlockDefinition[] = [
  // Logic
  { type: "AND", label: "AND", category: "logic", pins: [
    { name: "IN1", dataType: "BOOL", direction: "input" },
    { name: "IN2", dataType: "BOOL", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} AND {{IN2}};" },
  { type: "OR", label: "OR", category: "logic", pins: [
    { name: "IN1", dataType: "BOOL", direction: "input" },
    { name: "IN2", dataType: "BOOL", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} OR {{IN2}};" },
  { type: "NOT", label: "NOT", category: "logic", pins: [
    { name: "IN", dataType: "BOOL", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := NOT {{IN}};" },
  { type: "XOR", label: "XOR", category: "logic", pins: [
    { name: "IN1", dataType: "BOOL", direction: "input" },
    { name: "IN2", dataType: "BOOL", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} XOR {{IN2}};" },
  // Arithmetic
  { type: "ADD", label: "ADD", category: "arithmetic", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} + {{IN2}};" },
  { type: "SUB", label: "SUB", category: "arithmetic", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} - {{IN2}};" },
  { type: "MUL", label: "MUL", category: "arithmetic", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} * {{IN2}};" },
  { type: "DIV", label: "DIV", category: "arithmetic", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} / {{IN2}};" },
  { type: "MOD", label: "MOD", category: "arithmetic", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} MOD {{IN2}};" },
  // Comparison
  { type: "GT", label: "GT", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} > {{IN2}};" },
  { type: "LT", label: "LT", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} < {{IN2}};" },
  { type: "GE", label: "GE", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} >= {{IN2}};" },
  { type: "LE", label: "LE", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} <= {{IN2}};" },
  { type: "EQ", label: "EQ", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} = {{IN2}};" },
  { type: "NE", label: "NE", category: "comparison", pins: [
    { name: "IN1", dataType: "INT", direction: "input" },
    { name: "IN2", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN1}} <> {{IN2}};" },
  // Timers
  { type: "TON", label: "TON", category: "timer", isFunctionBlock: true, pins: [
    { name: "IN", dataType: "BOOL", direction: "input" },
    { name: "PT", dataType: "TIME", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
    { name: "ET", dataType: "TIME", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(IN := {{IN}}, PT := {{PT}});" },
  { type: "TOF", label: "TOF", category: "timer", isFunctionBlock: true, pins: [
    { name: "IN", dataType: "BOOL", direction: "input" },
    { name: "PT", dataType: "TIME", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
    { name: "ET", dataType: "TIME", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(IN := {{IN}}, PT := {{PT}});" },
  { type: "TP", label: "TP", category: "timer", isFunctionBlock: true, pins: [
    { name: "IN", dataType: "BOOL", direction: "input" },
    { name: "PT", dataType: "TIME", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
    { name: "ET", dataType: "TIME", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(IN := {{IN}}, PT := {{PT}});" },
  // Counters
  { type: "CTU", label: "CTU", category: "counter", isFunctionBlock: true, pins: [
    { name: "CU", dataType: "BOOL", direction: "input" },
    { name: "R", dataType: "BOOL", direction: "input" },
    { name: "PV", dataType: "INT", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
    { name: "CV", dataType: "INT", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(CU := {{CU}}, R := {{R}}, PV := {{PV}});" },
  { type: "CTD", label: "CTD", category: "counter", isFunctionBlock: true, pins: [
    { name: "CD", dataType: "BOOL", direction: "input" },
    { name: "LD", dataType: "BOOL", direction: "input" },
    { name: "PV", dataType: "INT", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
    { name: "CV", dataType: "INT", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(CD := {{CD}}, LD := {{LD}}, PV := {{PV}});" },
  // Move
  { type: "MOVE", label: "MOVE", category: "move", pins: [
    { name: "IN", dataType: "INT", direction: "input" },
    { name: "OUT", dataType: "INT", direction: "output" },
  ], stTemplate: "{{OUT}} := {{IN}};" },
  // Bistable
  { type: "SR", label: "SR", category: "bistable", isFunctionBlock: true, pins: [
    { name: "SET", dataType: "BOOL", direction: "input" },
    { name: "RESET1", dataType: "BOOL", direction: "input" },
    { name: "Q1", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(SET := {{SET}}, RESET1 := {{RESET1}});" },
  { type: "RS", label: "RS", category: "bistable", isFunctionBlock: true, pins: [
    { name: "SET1", dataType: "BOOL", direction: "input" },
    { name: "RESET", dataType: "BOOL", direction: "input" },
    { name: "Q1", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(SET1 := {{SET1}}, RESET := {{RESET}});" },
  // Edge detection
  { type: "R_TRIG", label: "R_TRIG", category: "edge", isFunctionBlock: true, pins: [
    { name: "CLK", dataType: "BOOL", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(CLK := {{CLK}});" },
  { type: "F_TRIG", label: "F_TRIG", category: "edge", isFunctionBlock: true, pins: [
    { name: "CLK", dataType: "BOOL", direction: "input" },
    { name: "Q", dataType: "BOOL", direction: "output" },
  ], stTemplate: "{{INSTANCE}}(CLK := {{CLK}});" },
  // I/O pins
  { type: "INPUT_PIN", label: "Input", category: "input", pins: [
    { name: "OUT", dataType: "BOOL", direction: "output" },
  ], stTemplate: "" },
  { type: "OUTPUT_PIN", label: "Output", category: "output", pins: [
    { name: "IN", dataType: "BOOL", direction: "input" },
  ], stTemplate: "" },
];

export function getBlockDef(type: string): BlockDefinition | undefined {
  return BLOCK_LIBRARY.find((b) => b.type === type);
}

export interface BlockNodeData {
  [key: string]: unknown;
  blockType: string;
  label: string;
  variableName?: string;
  variableType?: IECDataType;
  registerType?: RegisterType;
  registerNumber?: number;
  instanceName?: string;
  pinValues: Record<string, string>;
}

export type FbdNode = Node<BlockNodeData>;

export interface WireData {
  [key: string]: unknown;
  sourcePin: string;
  targetPin: string;
}

export type FbdEdge = Edge<WireData>;

export interface FbdDeclaration {
  name: string;
  dataType: string;
  direction: "var" | "var_input" | "var_output" | "var_temp";
  initialValue?: string;
}

export interface FbdProgram {
  version: string;
  programName: string;
  declarations: FbdDeclaration[];
  nodes: FbdNode[];
  edges: FbdEdge[];
  viewport: { x: number; y: number; zoom: number };
}
