export type Protocol = "mc" | "fins";

export interface TagConfig {
  id: string;
  name: string;
  address: string;
  data_type: string;
}

/** MC protocol (SLMP) settings of a Mitsubishi connection. */
export interface McSettings {
  transport: "tcp" | "udp";
  frame: "3E" | "4E";
  encoding: "binary" | "ascii";
  network: number;
  pc: number;
  io: number;
  station: number;
  /** iQ-F (FX5): X and Y in octal. */
  xyOctal: boolean;
}

/** FINS settings of an Omron connection. 0 for a node means "work it out" (see the Rust FinsConfig). */
export interface FinsSettings {
  transport: "udp" | "tcp";
  destNetwork: number;
  destNode: number;
  destUnit: number;
  srcNetwork: number;
  srcNode: number;
  localPort: number;
}

/** A configured PLC connection. Tags are scoped per device. */
export interface Device {
  id: string;
  label: string;
  protocol: Protocol;
  host: string;
  port: number;
  timeoutMs: number;
  mc: McSettings;
  fins: FinsSettings;
  tags: TagConfig[];
}

export const DATA_TYPES = ["Bool", "Int16", "UInt16", "Int32", "UInt32", "Float32", "BCD16", "Hex16"];

export const DEFAULT_PORT: Record<Protocol, number> = { mc: 5000, fins: 9600 };

export const PROTOCOL_NAME: Record<Protocol, string> = { mc: "Mitsubishi MC", fins: "Omron FINS" };

/** Address examples for the tag editor and the monitor. */
export const ADDRESS_HINT: Record<Protocol, string> = {
  mc: "D100, D100.F, M0, X1F, Y20, W1A, B0, R0, ZR0, SM400, TN0",
  fins: "D100, D100.03, CIO0.05, W10, H0, A448, E0_100, T0, C0",
};
