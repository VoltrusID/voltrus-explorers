export type Dir = "rx" | "tx";

/** A frame as the Rust side sends it (`can-frames`). */
export interface FrameView {
  /** Microseconds since the Unix epoch. */
  ts: number;
  dir: Dir;
  id: number;
  ext: boolean;
  rtr: boolean;
  fd: boolean;
  brs: boolean;
  /** CAN FD error state indicator; only present when set. */
  esi?: boolean;
  dlc: number;
  data: number[];
  msg?: string;
  /** `[name, value, unit, label]` */
  sigs?: [string, number, string, string | null][];
  label?: string;
}

export interface Stats {
  rx: number;
  tx: number;
  errors: number;
  dropped: number;
  rxPerSec: number;
  txPerSec: number;
  load: number | null;
}

export interface ConnInfo {
  description: string;
  logName: string;
  canSend: boolean;
}

export interface PortInfo {
  name: string;
  description: string;
  likelyCan: boolean;
}

export type InterfaceKind = "slcan" | "gsusb" | "socketcan" | "virtual";

export type InterfaceConfig =
  | { kind: "slcan"; port: string; bitrate: number; dataBitrate: number | null; serialBaud: number; listenOnly: boolean }
  | { kind: "gsusb"; device: string | null; channel: number; bitrate: number; dataBitrate: number | null; listenOnly: boolean }
  | { kind: "socketcan"; ifname: string }
  | { kind: "virtual" };

export interface GsUsbDevice {
  id: string;
  description: string;
  vid: number;
  pid: number;
}

export interface DbcSignalView {
  name: string;
  startBit: number;
  length: number;
  byteOrder: "intel" | "motorola";
  valueType: "unsigned" | "signed" | "float" | "double";
  factor: number;
  offset: number;
  min: number;
  max: number;
  unit: string;
  multiplex: string | null;
  comment: string;
  values: [number, string][];
}

export interface DbcMessageView {
  id: number;
  ext: boolean;
  name: string;
  size: number;
  transmitter: string;
  cycleMs: number | null;
  /** Sent as CAN FD (DBC VFrameFormat, or longer than 8 bytes), with bit rate switch when `brs`. */
  fd: boolean;
  brs: boolean;
  comment: string;
  signals: DbcSignalView[];
}

export interface DbcView {
  name: string;
  version: string;
  nodes: string[];
  messages: DbcMessageView[];
  skipped: number;
}

export type Layer = "none" | "j1939" | "canopen" | "obd2";

export interface SimState {
  running: boolean;
  throttle: number;
  breakerClosed: boolean;
  pumpEnabled: boolean;
  pumpSetpoint: number;
  fault: boolean;
  rate: number;
  sent: number;
  rpm: number;
  coolant: number;
  oilPressure: number;
  loadKw: number;
  frequency: number;
  pumpSpeed: number;
  hours: number;
  fdBattery: boolean;
  soc: number;
  packCurrent: number;
  packVoltage: number;
  packTemp: number;
}

export type SimAction =
  | { action: "setRunning"; running: boolean }
  | { action: "setThrottle"; value: number }
  | { action: "setBreaker"; closed: boolean }
  | { action: "setPump"; enabled: boolean; setpoint: number }
  | { action: "setFault"; active: boolean }
  | { action: "setRate"; rate: number }
  | { action: "setFdBattery"; enabled: boolean };

/** A frame to send, as the Rust side takes it. */
export interface FrameIn {
  id: number;
  ext: boolean;
  rtr: boolean;
  fd: boolean;
  brs: boolean;
  /** Hex bytes. */
  data: string;
  rtrLen: number;
}

export interface ReplayInfo {
  frames: number;
  durationS: number;
}
