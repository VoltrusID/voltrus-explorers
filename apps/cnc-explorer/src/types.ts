// JSON shapes of the Rust commands (crates/mt-mtconnect, crates/focas-proto and src-tauri/src/core.rs).

export type ConnType = "mtconnect" | "fanuc";

// ── MTConnect ──

export type Category = "SAMPLE" | "EVENT" | "CONDITION";
export type ConditionLevel = "Normal" | "Warning" | "Fault" | "Unavailable";

export interface DataItem {
  id: string;
  name: string | null;
  category: Category;
  type: string;
  subType: string | null;
  units: string | null;
  representation: string | null;
}

export interface Component {
  element: string;
  id: string;
  name: string | null;
  dataItems: DataItem[];
  components: Component[];
}

export interface Device {
  isAgent: boolean;
  id: string;
  name: string;
  uuid: string | null;
  description: { manufacturer: string | null; model: string | null; serialNumber: string | null } | null;
  dataItems: DataItem[];
  components: Component[];
}

export interface Header {
  instanceId: number;
  version: string | null;
  bufferSize: number | null;
  nextSequence: number | null;
}

export interface Devices {
  header: Header;
  devices: Device[];
}

export type ObsValue =
  | { kind: "Unavailable" }
  | { kind: "Number"; value: number }
  | { kind: "Text"; value: string }
  | { kind: "TimeSeries"; value: number[] }
  | { kind: "DataSet"; value: { key: string; value: string; removed: boolean }[] };

export interface Condition {
  level: ConditionLevel;
  nativeCode: string | null;
  nativeSeverity: string | null;
  qualifier: string | null;
  message: string | null;
}

export interface Observation {
  dataItemId: string;
  name: string | null;
  element: string;
  category: Category;
  dataItemType: string | null;
  subType: string | null;
  timestamp: string;
  sequence: number;
  value: ObsValue;
  condition: Condition | null;
}

export interface Streams {
  header: Header;
  devices: { name: string | null; uuid: string | null; components: { component: string; observations: Observation[] }[] }[];
}

export interface MtcStreamEvent {
  event: "snapshot" | "observations" | "resync" | "error";
  observations?: Observation[];
  nextSequence?: number | null;
  message?: string;
}

export interface Asset {
  assetType: string;
  assetId: string;
  deviceUuid: string | null;
  timestamp: string | null;
  removed: boolean;
  rawXml: string;
}

export interface AssetList {
  assets: Asset[];
}

export interface SimItemState {
  id: string;
  name: string | null;
  category: Category;
  dataType: string;
  units: string | null;
  value: ObsValue;
  condition: Condition | null;
  pinned: boolean;
}

export interface MtcSimSnapshot {
  instanceId: number;
  paused: boolean;
  nextSequence: number;
  items: SimItemState[];
}

export interface MtcSimInfo {
  baseUrl: string;
  httpPort: number;
  shdrPort: number | null;
}

// ── FANUC ──

export interface SysInfo {
  addinfo: number;
  max_axis: number;
  cnc_type: string;
  mt_type: string;
  series: string;
  version: string;
  axes: string;
}

export interface Coded {
  label: string;
  name: string;
}

export interface StatusView {
  mode: Coded;
  run: Coded;
  motion: Coded;
  mstb: Coded;
  emergency: Coded;
  alarm: Coded;
  edit: Coded;
}

export interface AxisRow {
  name: string;
  absolute: number | null;
  machine: number | null;
  relative: number | null;
  distance_to_go: number | null;
  servo_load: number | null;
}

/** A macro variable the live view watches; `null` when the controller flags it empty or refuses it. */
export interface MacroRow {
  number: number;
  value: number | null;
}

export interface FanucPoll {
  status: StatusView | null;
  axes: AxisRow[];
  feed_rate: number | null;
  spindle_speed: number | null;
  program: { running: number | null; main: number | null; name: string | null; sequence: number | null; block: string | null };
  macros: MacroRow[];
  alarm_bits: number | null;
  alarm_types: string[];
  errors: string[];
}

export interface AlarmMessage {
  number: number;
  alarm_type: unknown;
  axis: number;
  text: string | null;
}

export interface ProgramEntry {
  number: number;
  size: number;
  comment: string;
}

export interface NumberedValues {
  number: number;
  axis: number;
  data_type: number;
  values: number[];
}

/** What the operator says the machine is doing while a capture is recorded; the slug goes into the file name and `meta`. */
export type MachineState = "idle" | "running" | "feed-hold" | "alarm" | "estop" | "mdi";

/** A recorded capture: the three files to save and how the recording went (Rust `CaptureBundle`). */
export interface CaptureBundle {
  capture_json: string;
  report_json: string;
  summary: string;
  base_name: string;
  exchanges: number;
  calls: number;
  failed_calls: number;
  aborted: string | null;
  replay_ok: boolean;
  notes: string[];
}

export interface FocasSimView {
  profile: string;
  port: number;
  status: StatusView;
  axes: [string, number][];
  feed_rate: number;
  spindle_speed: number;
  program: number;
  block: string;
  sequence: number;
  part_count: number;
  alarms: { number: number; alarm_type: unknown; axis: number; message: string }[];
  macros: [number, number][];
  sessions: number;
  requests: number;
}
