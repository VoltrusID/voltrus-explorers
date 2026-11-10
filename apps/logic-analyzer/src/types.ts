// Mirrors of the Rust types (crates/mt-logic and src-tauri/src/core.rs).

export type Level = "data" | "control" | "info" | "warning" | "error";

export interface ChannelInfo {
  name: string;
  color?: string | null;
}

export interface Annotation {
  row: number;
  start: number;
  end: number;
  text: string;
  short: string;
  level: Level;
}

export interface TableRow {
  start: number;
  end: number;
  level: Level;
  cells: string[];
}

export interface Table {
  columns: string[];
  rows: TableRow[];
}

export type Parity = "none" | "even" | "odd" | "mark" | "space";

export interface UartConfig {
  kind: "uart";
  channel: number;
  baud: number;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  inverted: boolean;
  msbFirst: boolean;
}

export interface ModbusConfig {
  kind: "modbusRtu";
  requestChannel: number | null;
  responseChannel: number | null;
  baud: number;
  parity: Parity;
  stopBits: number;
  inverted: boolean;
  timeoutMs: number;
}

export interface SpiConfig {
  kind: "spi";
  clk: number;
  mosi: number | null;
  miso: number | null;
  cs: number | null;
  mode: number;
  wordBits: number;
  lsbFirst: boolean;
  csActiveHigh: boolean;
  device: "none" | "mcp2515";
}

export interface I2cConfig {
  kind: "i2c";
  scl: number;
  sda: number;
  minPulse: number;
}

export interface CanConfig {
  kind: "can";
  channel: number;
  bitrate: number;
  samplePoint: number;
  inverted: boolean;
}

export type DecoderConfig = UartConfig | ModbusConfig | SpiConfig | I2cConfig | CanConfig;
export type DecoderKind = DecoderConfig["kind"];

export interface DecoderView {
  id: number;
  rows: string[];
  summary: string;
  table: Table | null;
  count: number;
}

export type ScenarioId = "modbusRtu" | "canBus" | "spiMcp2515" | "i2cSensors" | "uartConsole";

export interface ScenarioInfo {
  id: ScenarioId;
  name: string;
  description: string;
  defaultRate: number;
  minRate: number;
  duration: number;
  channels: ChannelInfo[];
  decoders: DecoderConfig[];
}

export interface Board {
  vid: number;
  pid: number;
  name: string;
  firmware: string;
}

export interface UsbDeviceInfo {
  id: string;
  vid: number;
  pid: number;
  manufacturer: string | null;
  product: string | null;
  serial: string | null;
  location: string;
}

export interface Detected {
  info: UsbDeviceInfo;
  board: Board;
  firmwareLoaded: boolean;
}

export interface Devices {
  scenarios: ScenarioInfo[];
  boards: Detected[];
  usbError: string | null;
  sampleRates: number[];
}

export interface FoundFirmware {
  name: string;
  path: string;
  verified: "official" | "unknown";
  source: string;
}

export interface FirmwareStatus {
  files: [string, FoundFirmware | null][];
  appDir: string | null;
  userFile: string | null;
  release: string;
  licence: string;
  sourceUrl: string;
  downloadUrl: string;
}

export type Trigger =
  | { kind: "none" }
  | { kind: "edge"; channel: number; edge: "rising" | "falling" | "any" }
  | { kind: "pattern"; mask: number; value: number };

export interface CaptureSettings {
  sampleRate: number;
  samples: number;
  trigger: Trigger;
  preTrigger: number;
}

export type Source = { kind: "simulator"; scenario: ScenarioId } | { kind: "fx2"; id: string };

export interface CaptureInfo {
  sampleRate: number;
  samples: number;
  trigger: number | null;
  channels: ChannelInfo[];
  source: string;
}

export interface Measurement {
  frequency: number;
  period: number;
  duty: number;
  periods: number;
}

export interface Opened {
  info: CaptureInfo;
  session: Session | null;
  warnings: string[];
}

/** What a .vla file keeps besides the samples. */
export interface Session {
  version: 1;
  decoders: { id: number; config: DecoderConfig; hidden?: boolean }[];
  cursors: [number | null, number | null];
  view?: { start: number; spp: number };
}

export interface Progress {
  phase: "waiting" | "recording";
  samples: number;
  total: number;
}
