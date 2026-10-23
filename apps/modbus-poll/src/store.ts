import { create } from "zustand";
import type { LogEntry } from "@mactools/ui";
import type { DeviceProfile, RegisterDef } from "./templates";
import type { ReadBlock, DisplayFormat, ConnProfile } from "./types";
import type { TrendSeries } from "./TrendPanel";
import { loadCustomTemplates } from "./templates";
import { defaultBlocks } from "./constants";
import { loadConnProfiles } from "./helpers";
import { newSessionFields, pickSessionFields, type Session, type SessionFields } from "./sessions";

export type AppMode = "master" | "slave";
export type MobileTab = "connect" | "data" | "blocks" | "template" | "log" | "traffic" | "trend";

/** Blocks exist only in master mode, templates only in slave mode. */
const tabExistsIn = (mode: AppMode, tab: MobileTab) =>
  !(tab === "blocks" && mode === "slave") && !(tab === "template" && mode === "master");

export interface AppState {
  // Mode & theme
  mode: AppMode;
  setMode: (m: AppMode) => void;
  theme: "dark" | "light";
  setTheme: (t: "dark" | "light") => void;

  // Layout
  leftWidth: number;
  setLeftWidth: (w: number) => void;
  rightWidth: number;
  setRightWidth: (w: number) => void;
  tableHeight: number;
  setTableHeight: (h: number) => void;
  dragSide: "left" | "right" | "table" | null;
  setDragSide: (s: "left" | "right" | "table" | null) => void;
  mobileTab: MobileTab;
  setMobileTab: (t: MobileTab) => void;
  platform: string;
  setPlatform: (p: string) => void;

  // Master blocks
  blocks: ReadBlock[];
  setBlocks: (b: ReadBlock[] | ((prev: ReadBlock[]) => ReadBlock[])) => void;
  activeBlockTab: number;
  setActiveBlockTab: (t: number) => void;

  // Log
  log: LogEntry[];
  addLog: (direction: LogEntry["direction"], message: string) => void;
  setLog: (l: LogEntry[]) => void;

  // Write dialog
  editingTag: { name: string; address: number; type: string; area: string; currentValue: string } | null;
  setEditingTag: (t: { name: string; address: number; type: string; area: string; currentValue: string } | null) => void;
  showInfo: boolean;
  setShowInfo: (s: boolean) => void;

  // Master transport
  transport: "tcp" | "rtu";
  setTransport: (t: "tcp" | "rtu") => void;
  tcpFormValues: Record<string, string | number>;
  setTcpFormValues: (v: Record<string, string | number>) => void;
  formKey: number;
  setFormKey: (k: number) => void;
  serialPorts: string[];
  setSerialPorts: (p: string[]) => void;
  rtuPort: string;
  setRtuPort: (p: string) => void;
  rtuPortManual: boolean;
  setRtuPortManual: (m: boolean) => void;
  rtuBaud: number;
  setRtuBaud: (b: number) => void;
  rtuDataBits: number;
  setRtuDataBits: (b: number) => void;
  rtuStopBits: number;
  setRtuStopBits: (b: number) => void;
  rtuParity: string;
  setRtuParity: (p: string) => void;
  rtuUnitId: number;
  setRtuUnitId: (u: number) => void;
  rtuTimeout: number;
  setRtuTimeout: (t: number) => void;
  rtuPollInterval: number;
  setRtuPollInterval: (p: number) => void;

  // Bus scanner
  busScanning: boolean;
  setBusScanning: (s: boolean) => void;
  busScanResults: { unit_id: number; value: number }[];
  setBusScanResults: (r: { unit_id: number; value: number }[]) => void;
  showBusScanner: boolean;
  setShowBusScanner: (s: boolean) => void;
  busScanAddr: number;
  setBusScanAddr: (a: number) => void;
  busScanArea: string;
  setBusScanArea: (a: string) => void;
  busScanStart: number;
  setBusScanStart: (s: number) => void;
  busScanEnd: number;
  setBusScanEnd: (e: number) => void;

  // Slave
  slaveRunning: boolean;
  setSlaveRunning: (r: boolean) => void;
  slavePort: number;
  setSlavePort: (p: number) => void;
  slaveRegCount: number;
  setSlaveRegCount: (c: number) => void;
  slaveUnitId: number;
  setSlaveUnitId: (u: number) => void;
  slavePollMs: number;
  setSlavePollMs: (ms: number) => void;
  slaveRegs: number[];
  setSlaveRegs: (r: number[] | ((prev: number[]) => number[])) => void;
  slaveTransport: "tcp" | "rtu";
  setSlaveTransport: (t: "tcp" | "rtu") => void;
  slaveRtuPort: string;
  setSlaveRtuPort: (p: string) => void;
  slaveRtuBaud: number;
  setSlaveRtuBaud: (b: number) => void;
  slaveRtuDataBits: number;
  setSlaveRtuDataBits: (b: number) => void;
  slaveRtuStopBits: number;
  setSlaveRtuStopBits: (b: number) => void;
  slaveRtuParity: string;
  setSlaveRtuParity: (p: string) => void;
  slaveRtuVirtualPort: boolean;
  setSlaveRtuVirtualPort: (v: boolean) => void;
  slaveVirtualPortPath: string;
  setSlaveVirtualPortPath: (p: string) => void;

  // Simulation
  activeProfile: DeviceProfile | null;
  setActiveProfile: (p: DeviceProfile | null) => void;
  simValues: number[];
  setSimValues: (v: number[]) => void;
  simRunning: boolean;
  setSimRunning: (r: boolean) => void;
  simSpeed: number;
  setSimSpeed: (s: number) => void;
  customTemplates: DeviceProfile[];
  setCustomTemplates: (t: DeviceProfile[]) => void;
  showCustomDialog: boolean;
  setShowCustomDialog: (s: boolean) => void;
  customName: string;
  setCustomName: (n: string) => void;
  customRegs: RegisterDef[];
  setCustomRegs: (r: RegisterDef[]) => void;
  lockedRegs: Set<number>;
  setLockedRegs: (s: Set<number>) => void;

  // Connection profiles
  connProfiles: ConnProfile[];
  setConnProfiles: (p: ConnProfile[] | ((prev: ConnProfile[]) => ConnProfile[])) => void;
  activeConnProfile: string | null;
  setActiveConnProfile: (n: string | null) => void;
  profileDirty: boolean;
  setProfileDirty: (d: boolean) => void;

  // New block form
  newName: string;
  setNewName: (n: string) => void;
  newAddr: string;
  setNewAddr: (a: string) => void;
  newQty: string;
  setNewQty: (q: string) => void;
  newType: string;
  setNewType: (t: string) => void;
  newArea: string;
  setNewArea: (a: string) => void;
  editingBlock: number | null;
  setEditingBlock: (i: number | null) => void;

  // Display options
  useOneBased: boolean;
  setUseOneBased: (b: boolean) => void;
  displayFormat: DisplayFormat;
  setDisplayFormat: (f: DisplayFormat) => void;
  endianness: string;
  setEndianness: (e: string) => void;

  // Network scan
  scanSubnet: string;
  setScanSubnet: (s: string) => void;
  scanResults: { ip: string; port_open: boolean; hostname: string | null }[];
  setScanResults: (r: { ip: string; port_open: boolean; hostname: string | null }[]) => void;
  scanning: boolean;
  setScanning: (s: boolean) => void;
  showScan: boolean;
  setShowScan: (s: boolean) => void;

  // Bottom panel
  bottomTab: "log" | "traffic" | "trend";
  setBottomTab: (t: "log" | "traffic" | "trend") => void;

  // Trending
  trendSeries: Map<string, TrendSeries>;
  setTrendSeries: (s: Map<string, TrendSeries> | ((prev: Map<string, TrendSeries>) => Map<string, TrendSeries>)) => void;
  trendWindow: number;
  setTrendWindow: (w: number) => void;

  // Slave table
  slavePage: number;
  setSlavePage: (p: number) => void;

  // Master connections (tabs). The active one lives in the fields above; see sessions.ts.
  sessions: Session[];
  activeSessionId: string;
  nextSessionSeq: number;
  /** Open a new empty connection and make it active. Returns its id. */
  addSession: () => string;
  switchSession: (id: string) => void;
  /** Forget a connection's state (the last one can not be closed). Disconnect it first. */
  closeSession: (id: string) => void;
  /** Change connection `id`'s state, whether or not it is the active one. */
  patchSession: (id: string, change: (fields: SessionFields) => Partial<SessionFields>) => void;
  /** `addLog` for a specific connection: poll results and background events do not belong to the tab in front. */
  addLogFor: (id: string, direction: LogEntry["direction"], message: string) => void;
}

const MAX_LOG = 5000;
const appendLog = (log: LogEntry[], direction: LogEntry["direction"], message: string): LogEntry[] => {
  const next = [...log, { timestamp: Date.now(), direction, message }];
  return next.length > MAX_LOG ? next.slice(-MAX_LOG) : next;
};

export const useStore = create<AppState>((set, get) => ({
  // Mode & theme
  mode: "master",
  // A tab the new mode does not have would leave the content area empty.
  setMode: (mode) => set((s) => ({ mode, mobileTab: tabExistsIn(mode, s.mobileTab) ? s.mobileTab : "connect" })),
  theme: (() => {
    const stored = typeof localStorage !== "undefined" ? localStorage.getItem("modbus-poll-theme") : null;
    if (stored === "light" || stored === "dark") return stored;
    return typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  })(),
  setTheme: (theme) => set({ theme }),

  // Layout
  leftWidth: 264,
  setLeftWidth: (leftWidth) => set({ leftWidth }),
  rightWidth: 280,
  setRightWidth: (rightWidth) => set({ rightWidth }),
  tableHeight: 300,
  setTableHeight: (tableHeight) => set({ tableHeight }),
  dragSide: null,
  setDragSide: (dragSide) => set({ dragSide }),
  mobileTab: "connect",
  setMobileTab: (mobileTab) => set({ mobileTab }),
  platform: "",
  setPlatform: (platform) => set({ platform }),

  // Master blocks
  blocks: defaultBlocks(),
  setBlocks: (blocks) => set((s) => ({ blocks: typeof blocks === "function" ? blocks(s.blocks) : blocks })),
  activeBlockTab: 0,
  setActiveBlockTab: (activeBlockTab) => set({ activeBlockTab }),

  // Log
  log: [],
  addLog: (direction, message) =>
    set((s) => ({ log: appendLog(s.log, direction, message) })),
  setLog: (log) => set({ log }),

  // Write dialog
  editingTag: null,
  setEditingTag: (editingTag) => set({ editingTag }),
  showInfo: false,
  setShowInfo: (showInfo) => set({ showInfo }),

  // Master transport
  transport: "tcp",
  setTransport: (transport) => set({ transport }),
  tcpFormValues: { host: "127.0.0.1", port: 502, unitId: 1, pollInterval: 1000 },
  setTcpFormValues: (tcpFormValues) => set({ tcpFormValues }),
  formKey: 0,
  setFormKey: (formKey) => set({ formKey }),
  serialPorts: [],
  setSerialPorts: (serialPorts) => set({ serialPorts }),
  rtuPort: "",
  setRtuPort: (rtuPort) => set({ rtuPort }),
  rtuPortManual: false,
  setRtuPortManual: (rtuPortManual) => set({ rtuPortManual }),
  rtuBaud: 9600,
  setRtuBaud: (rtuBaud) => set({ rtuBaud }),
  rtuDataBits: 8,
  setRtuDataBits: (rtuDataBits) => set({ rtuDataBits }),
  rtuStopBits: 1,
  setRtuStopBits: (rtuStopBits) => set({ rtuStopBits }),
  rtuParity: "none",
  setRtuParity: (rtuParity) => set({ rtuParity }),
  rtuUnitId: 1,
  setRtuUnitId: (rtuUnitId) => set({ rtuUnitId }),
  rtuTimeout: 5000,
  setRtuTimeout: (rtuTimeout) => set({ rtuTimeout }),
  rtuPollInterval: 1000,
  setRtuPollInterval: (rtuPollInterval) => set({ rtuPollInterval }),

  // Bus scanner
  busScanning: false,
  setBusScanning: (busScanning) => set({ busScanning }),
  busScanResults: [],
  setBusScanResults: (busScanResults) => set({ busScanResults }),
  showBusScanner: false,
  setShowBusScanner: (showBusScanner) => set({ showBusScanner }),
  busScanAddr: 0,
  setBusScanAddr: (busScanAddr) => set({ busScanAddr }),
  busScanArea: "holding",
  setBusScanArea: (busScanArea) => set({ busScanArea }),
  busScanStart: 1,
  setBusScanStart: (busScanStart) => set({ busScanStart }),
  busScanEnd: 247,
  setBusScanEnd: (busScanEnd) => set({ busScanEnd }),

  // Slave
  slaveRunning: false,
  setSlaveRunning: (slaveRunning) => set({ slaveRunning }),
  slavePort: 5020,
  setSlavePort: (slavePort) => set({ slavePort }),
  slaveRegCount: 100,
  setSlaveRegCount: (slaveRegCount) => set({ slaveRegCount }),
  slaveUnitId: 1,
  setSlaveUnitId: (slaveUnitId) => set({ slaveUnitId }),
  slavePollMs: 500,
  setSlavePollMs: (slavePollMs) => set({ slavePollMs }),
  slaveRegs: [],
  setSlaveRegs: (slaveRegs) => set((s) => ({ slaveRegs: typeof slaveRegs === "function" ? slaveRegs(s.slaveRegs) : slaveRegs })),
  slaveTransport: "tcp",
  setSlaveTransport: (slaveTransport) => set({ slaveTransport }),
  slaveRtuPort: "",
  setSlaveRtuPort: (slaveRtuPort) => set({ slaveRtuPort }),
  slaveRtuBaud: 9600,
  setSlaveRtuBaud: (slaveRtuBaud) => set({ slaveRtuBaud }),
  slaveRtuDataBits: 8,
  setSlaveRtuDataBits: (slaveRtuDataBits) => set({ slaveRtuDataBits }),
  slaveRtuStopBits: 1,
  setSlaveRtuStopBits: (slaveRtuStopBits) => set({ slaveRtuStopBits }),
  slaveRtuParity: "none",
  setSlaveRtuParity: (slaveRtuParity) => set({ slaveRtuParity }),
  slaveRtuVirtualPort: false,
  setSlaveRtuVirtualPort: (slaveRtuVirtualPort) => set({ slaveRtuVirtualPort }),
  slaveVirtualPortPath: "",
  setSlaveVirtualPortPath: (slaveVirtualPortPath) => set({ slaveVirtualPortPath }),

  // Simulation
  activeProfile: null,
  setActiveProfile: (activeProfile) => set({ activeProfile }),
  simValues: [],
  setSimValues: (simValues) => set({ simValues }),
  simRunning: false,
  setSimRunning: (simRunning) => set({ simRunning }),
  simSpeed: 1000,
  setSimSpeed: (simSpeed) => set({ simSpeed }),
  customTemplates: loadCustomTemplates(),
  setCustomTemplates: (customTemplates) => set({ customTemplates }),
  showCustomDialog: false,
  setShowCustomDialog: (showCustomDialog) => set({ showCustomDialog }),
  customName: "",
  setCustomName: (customName) => set({ customName }),
  customRegs: [
    { name: "Value", unit: "", description: "", initial: 0, min: 0, max: 65535, drift: 1, isCounter: false, counterRate: 0 },
  ],
  setCustomRegs: (customRegs) => set({ customRegs }),
  lockedRegs: new Set<number>(),
  setLockedRegs: (lockedRegs) => set({ lockedRegs }),

  // Connection profiles
  connProfiles: loadConnProfiles(),
  setConnProfiles: (connProfiles) => set((s) => ({ connProfiles: typeof connProfiles === "function" ? connProfiles(s.connProfiles) : connProfiles })),
  activeConnProfile: null,
  setActiveConnProfile: (activeConnProfile) => set({ activeConnProfile }),
  profileDirty: false,
  setProfileDirty: (profileDirty) => set({ profileDirty }),

  // New block form
  newName: "",
  setNewName: (newName) => set({ newName }),
  newAddr: "",
  setNewAddr: (newAddr) => set({ newAddr }),
  newQty: "",
  setNewQty: (newQty) => set({ newQty }),
  newType: "uint16",
  setNewType: (newType) => set({ newType }),
  newArea: "holding",
  setNewArea: (newArea) => set({ newArea }),
  editingBlock: null,
  setEditingBlock: (editingBlock) => set({ editingBlock }),

  // Display options
  useOneBased: false,
  setUseOneBased: (useOneBased) => set({ useOneBased }),
  displayFormat: "dec",
  setDisplayFormat: (displayFormat) => set({ displayFormat }),
  endianness: "big",
  setEndianness: (endianness) => set({ endianness }),

  // Network scan
  scanSubnet: "",
  setScanSubnet: (scanSubnet) => set({ scanSubnet }),
  scanResults: [],
  setScanResults: (scanResults) => set({ scanResults }),
  scanning: false,
  setScanning: (scanning) => set({ scanning }),
  showScan: false,
  setShowScan: (showScan) => set({ showScan }),

  // Bottom panel
  bottomTab: "log",
  setBottomTab: (bottomTab) => set({ bottomTab }),

  // Trending
  trendSeries: new Map<string, TrendSeries>(),
  setTrendSeries: (trendSeries) => set((s) => ({ trendSeries: typeof trendSeries === "function" ? trendSeries(s.trendSeries) : trendSeries })),
  trendWindow: 60,
  setTrendWindow: (trendWindow) => set({ trendWindow }),

  // Slave table
  slavePage: 0,
  setSlavePage: (slavePage) => set({ slavePage }),

  // Master connections
  sessions: [{ id: "c1", fields: null }],
  activeSessionId: "c1",
  nextSessionSeq: 2,
  addSession: () => {
    const id = `c${get().nextSessionSeq}`;
    set((s) => ({
      sessions: [...s.sessions.map((x) => (x.id === s.activeSessionId ? { ...x, fields: pickSessionFields(s) } : x)), { id, fields: null }],
      activeSessionId: id,
      nextSessionSeq: s.nextSessionSeq + 1,
      formKey: s.formKey + 1,
      ...newSessionFields(),
    }));
    return id;
  },
  switchSession: (id) =>
    set((s) => {
      const target = s.sessions.find((x) => x.id === id);
      if (id === s.activeSessionId || !target?.fields) return s;
      return {
        sessions: s.sessions.map((x) => (x.id === s.activeSessionId ? { ...x, fields: pickSessionFields(s) } : x.id === id ? { ...x, fields: null } : x)),
        activeSessionId: id,
        formKey: s.formKey + 1,
        ...target.fields,
      };
    }),
  closeSession: (id) =>
    set((s) => {
      if (s.sessions.length <= 1) return s;
      const index = s.sessions.findIndex((x) => x.id === id);
      if (index < 0) return s;
      const remaining = s.sessions.filter((x) => x.id !== id);
      if (id !== s.activeSessionId) return { sessions: remaining };
      // The neighbour to the right takes over, else the one to the left.
      const next = remaining[Math.min(index, remaining.length - 1)];
      return {
        sessions: remaining.map((x) => (x.id === next.id ? { ...x, fields: null } : x)),
        activeSessionId: next.id,
        formKey: s.formKey + 1,
        ...next.fields!,
      };
    }),
  patchSession: (id, change) =>
    set((s) => {
      if (id === s.activeSessionId) return change(pickSessionFields(s));
      const target = s.sessions.find((x) => x.id === id);
      if (!target?.fields) return s;
      const patch = change(target.fields);
      return { sessions: s.sessions.map((x) => (x.id === id ? { ...x, fields: { ...x.fields!, ...patch } } : x)) };
    }),
  addLogFor: (id, direction, message) => get().patchSession(id, (f) => ({ log: appendLog(f.log, direction, message) })),
}));
