export interface S7Config {
  host: string;
  port: number;
  rack: number;
  slot: number;
}

export interface TagConfig {
  id: string;
  name: string;
  address: string;
  data_type: string;
}

/** A configured PLC connection. Tags are scoped per device. */
export interface Device {
  id: string;
  label: string;
  host: string;
  port: number;
  rack: number;
  slot: number;
  tags: TagConfig[];
}

export type MainTab = "tags" | "diagnostics" | "blocks";
export type MobileTab = "connect" | "data" | "diagnostics" | "blocks" | "log" | "trend";

export const DATA_TYPES = ["Bool", "Byte", "Word", "DWord", "Real", "String"];
