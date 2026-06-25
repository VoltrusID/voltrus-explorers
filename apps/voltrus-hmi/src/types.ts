export type WidgetType = "gauge" | "button" | "indicator" | "text" | "slider" | "chart";

export interface HmiWidget {
  id: string;
  type: WidgetType;
  x: number;
  y: number;
  width: number;
  height: number;
  label: string;
  variableName: string;
  min: number;
  max: number;
  color: string;
  fontSize: number;
  requiredRole?: "none" | "operator" | "supervisor";
  zIndex?: number;
}

export interface HmiScreen {
  id: string;
  name: string;
  backgroundColor: string;
  widgets: HmiWidget[];
}

export interface PlcVariable {
  name: string;
  type: string; // "BOOL", "INT", "DINT", "REAL", "WORD", etc.
  value: unknown;
}

export interface HmiTag {
  id: string;
  name: string;
  plcVariable: string;
  plcIp: string;
  dataType: string;
  access: "read" | "write" | "readwrite";
}

export interface ScreenTemplate {
  name: string;
  description: string;
  screens: HmiScreen[];
}

export interface AlarmRule {
  id: string;
  source: "scada" | "plc";
  variableName: string;
  severity: "critical" | "warning";
  message: string;
}

export interface AccessConfig {
  operatorPinHash: string;
  supervisorPinHash: string;
  salt: string;
  autoLogoutMinutes: number;
}

export interface TranslationSet {
  [languageCode: string]: { [key: string]: string };
}
