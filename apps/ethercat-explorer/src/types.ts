// Mirrors of the Rust views (crates/mt-ethercat/src/master.rs, sim/mod.rs, sii.rs, link.rs).

export type AlStateName = "INIT" | "PRE-OP" | "BOOT" | "SAFE-OP" | "OP" | "?" | "no answer";

export type Template = "machine" | "ioStation" | "drives";

export type LinkSpec = { kind: "simulator"; template: Template } | { kind: "interface"; name: string };

export interface NetInterface {
  name: string;
  mac: string | null;
  up: boolean;
  running: boolean;
  label: string | null;
  wireless: boolean;
}

export interface RawAccess {
  ok: boolean;
  detail: string;
  canGrant: boolean;
}

export interface PortView {
  kind: string;
  link: boolean;
  loopClosed: boolean;
  communication: boolean;
}

export interface DeviceInfo {
  position: number;
  address: number;
  alias: number;
  name: string;
  description: string;
  vendorId: number;
  vendorName: string | null;
  productCode: number;
  revision: number;
  serial: number;
  escType: number;
  escName: string;
  escRevision: number;
  escBuild: number;
  fmmus: number;
  syncManagers: number;
  ramKb: number;
  ports: PortView[];
  dc: string;
  propagationDelayNs: number;
  mailbox: string[];
  coe: boolean;
  sdoInfo: boolean;
  inputBytes: number;
  outputBytes: number;
  state: AlStateName;
  alCode: number | null;
  alText: string | null;
}

export interface ScanResult {
  devices: DeviceInfo[];
  initError: string | null;
  link: string;
}

export interface StateView {
  position: number;
  state: AlStateName;
  alCode: number | null;
  alText: string | null;
}

export interface RegisterView {
  addr: number;
  len: number;
  name: string;
  hex: string;
  value: number;
  decoded: string;
}

export interface FmmuView {
  index: number;
  logicalStart: number;
  length: number;
  logicalBits: [number, number];
  physicalStart: number;
  physicalBit: number;
  kind: string;
  active: boolean;
}

export interface SmView {
  index: number;
  start: number;
  length: number;
  control: number;
  mode: string;
  direction: string;
  status: number;
  mailboxFull: boolean;
  enabled: boolean;
}

export interface RegistersView {
  registers: RegisterView[];
  fmmus: FmmuView[];
  syncManagers: SmView[];
  errorCounters: [string, number][];
}

export interface PdoEntryView {
  index: number;
  sub: number;
  name: string;
  dataType: number;
  typeName: string;
  bits: number;
  bitOffset: number;
}

export interface PdoView {
  index: number;
  name: string;
  entries: PdoEntryView[];
}

export interface PdoMapView {
  inputs: PdoView[];
  outputs: PdoView[];
  source: "CoE" | "SII";
}

export interface ObjectView {
  index: number;
  name: string;
  objectCode: string;
  dataType: number;
  typeName: string;
  maxSub: number;
}

export interface EntryView {
  index: number;
  sub: number;
  name: string;
  dataType: number;
  typeName: string;
  bits: number;
  access: string;
  writable: boolean;
  value: string | null;
  raw: string | null;
  error: string | null;
}

export interface CycleStats {
  running: boolean;
  cycleMs: number;
  cycles: number;
  wkc: number;
  expectedWkc: number;
  wkcErrors: number;
  timeouts: number;
  minUs: number;
  avgUs: number;
  maxUs: number;
}

export interface IoImage {
  inputs: number[];
  outputs: number[];
}

export interface IoEvent {
  kind: "io";
  stats: CycleStats;
  devices: IoImage[];
}

export interface FrameView {
  tsUs: number;
  out: boolean;
  datagrams: string[];
  bytes: number[];
}

export interface SiiSyncManager {
  start: number;
  len: number;
  control: number;
  enable: number;
  kind: string;
}

export interface SiiPdo {
  index: number;
  sm: number;
  name: string;
  entries: { index: number; sub: number; name: string; dataType: number; bits: number }[];
}

export interface Sii {
  pdiControl: number;
  alias: number;
  checksumOk: boolean;
  vendorId: number;
  productCode: number;
  revision: number;
  serial: number;
  bootRx: [number, number];
  bootTx: [number, number];
  stdRx: [number, number];
  stdTx: [number, number];
  mailboxProtocols: number;
  size: number;
  version: number;
  strings: string[];
  general: {
    group: string;
    image: string;
    order: string;
    name: string;
    coeDetails: number;
    foeDetails: number;
    eoeDetails: number;
    soeChannels: number;
    ds402Channels: number;
    flags: number;
    ebusCurrent: number;
  } | null;
  fmmus: number[];
  syncManagers: SiiSyncManager[];
  txPdos: SiiPdo[];
  rxPdos: SiiPdo[];
  categories: [number, number][];
}

export interface SimDeviceState {
  position: number;
  name: string;
  description: string;
  reachable: boolean;
  address: number;
  state: string;
  error: string | null;
  summary: [string, string][];
  actions: [string, string][];
}

export interface SimState {
  template: string;
  frames: number;
  breakAfter: number | null;
  devices: SimDeviceState[];
}

export type SimAction =
  | { kind: "breakCable"; after: number | null }
  | { kind: "rxErrors"; device: number; port: number; count: number }
  | { kind: "device"; device: number; action: string };
