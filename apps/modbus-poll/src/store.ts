import { create } from "zustand";
import type { LogEntry } from "@mactools/ui";
import type { DeviceProfile, RegisterDef } from "./templates";
import type { ReadBlock, DisplayFormat, ConnProfile } from "./types";
import type { TrendSeries } from "./TrendPanel";
import { loadCustomTemplates } from "./templates";
import { loadConnProfiles } from "./helpers";

interface AppState {
  // Mode & theme
  mode: "master" | "slave";
  setMode: (m: "master" | "slave") => void;
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
  mobileTab: "connect" | "data" | "blocks" | "template" | "log";
  setMobileTab: (t: "connect" | "data" | "blocks" | "template" | "log") => void;
  isMobile: boolean;
  setIsMobile: (m: boolean) => void;
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

  // Auto-detect
  autoDetecting: boolean;
  setAutoDetecting: (a: boolean) => void;

  // Trending
  trendSeries: Map<string, TrendSeries>;
  setTrendSeries: (s: Map<string, TrendSeries> | ((prev: Map<string, TrendSeries>) => Map<string, TrendSeries>)) => void;
  trendWindow: number;
  setTrendWindow: (w: number) => void;

  // Slave table
  slavePage: number;
  setSlavePage: (p: number) => void;
}

export const useStore = create<AppState>((set) => ({
  // Mode & theme
  mode: "master",
  setMode: (mode) => set({ mode }),
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
  isMobile: typeof window !== "undefined" && window.innerWidth < 768,
  setIsMobile: (isMobile) => set({ isMobile }),
  platform: "",
  setPlatform: (platform) => set({ platform }),

  // Master blocks
  blocks: [
    { name: "Voltage", address: 2999, quantity: 3, type: "float32", area: "holding" },
    { name: "Current", address: 3005, quantity: 3, type: "float32", area: "holding" },
    { name: "Power", address: 3011, quantity: 3, type: "float32", area: "holding" },
    { name: "Power Quality", address: 3017, quantity: 4, type: "float32", area: "holding" },
    { name: "Temperature", address: 3025, quantity: 1, type: "float32", area: "holding" },
    { name: "Energy", address: 3027, quantity: 4, type: "float32", area: "holding" },
  ],
  setBlocks: (blocks) => set((s) => ({ blocks: typeof blocks === "function" ? blocks(s.blocks) : blocks })),
  activeBlockTab: 0,
  setActiveBlockTab: (activeBlockTab) => set({ activeBlockTab }),

  // Log
  log: [],
  addLog: (direction, message) =>
    set((s) => {
      const entry = { timestamp: Date.now(), direction, message };
      const log = [...s.log, entry];
      return { log: log.length > 5000 ? log.slice(-5000) : log };
    }),
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

  // Auto-detect
  autoDetecting: false,
  setAutoDetecting: (autoDetecting) => set({ autoDetecting }),

  // Trending
  trendSeries: new Map<string, TrendSeries>(),
  setTrendSeries: (trendSeries) => set((s) => ({ trendSeries: typeof trendSeries === "function" ? trendSeries(s.trendSeries) : trendSeries })),
  trendWindow: 60,
  setTrendWindow: (trendWindow) => set({ trendWindow }),

  // Slave table
  slavePage: 0,
  setSlavePage: (slavePage) => set({ slavePage }),
}));
