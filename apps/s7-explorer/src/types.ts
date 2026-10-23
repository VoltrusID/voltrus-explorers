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
/** Phone tabs: the first four are in the tab bar, the rest under More. */
export type PhoneTab = "connect" | "watch" | "blocks" | "trend" | "diagnostics" | "sim" | "log";

export const DATA_TYPES = ["Bool", "Byte", "Word", "DWord", "Real", "String", "Timer", "Counter"];

/** Timers and counters are read-only and their type is fixed by the address (T5, C10). */
export const READ_ONLY_TYPES = ["Timer", "Counter"];
