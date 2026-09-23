import { create } from "zustand";
import { bridgeMode, invoke, listen, type UnlistenFn } from "./api/bridge";
import type {
  ChatMessage,
  CompileResult,
  Diagnostic,
  RuntimeStateInfo,
  OpenFile,
  FileEntry,
  WatchedVar,
  LogEntry,
  IoDriverConfig,
  IoMapping,
  CompiledVariable,
  SampleTemplate,
  ProjectFile,
  TargetStatus,
  DeployResult,
  ScadaRegistration,
  DiscoveredTarget,
  TargetKind,
} from "./types";
import { transpileFbdToSt } from "./components/Editor/FbdEditor/transpiler/fbd-to-st";
import { transpileLdToSt } from "./components/Editor/LdEditor/transpiler/ld-to-st";
import { transpileSfcToSt } from "./components/Editor/SfcEditor/transpiler/sfc-to-st";
import type { FbdProgram } from "./types/fbd";
import type { SfcProgram } from "./types/sfc";
import type { LdProgram } from "./types/ld";
import {
  canonicalizeFileContent,
  serializeFbd,
  serializeLd,
  serializeSfc,
  DIAGRAM_FORMAT_VERSION,
  LD_FORMAT_VERSION,
} from "./types/canonical";

interface AppState {
  // Theme
  theme: "dark" | "light";
  setTheme: (t: "dark" | "light") => void;

  // File management
  openFiles: OpenFile[];
  activeFileIndex: number;
  projectDir: string | null;
  projectFiles: FileEntry[];
  openFile: (path: string) => Promise<void>;
  closeFile: (index: number) => void;
  setActiveFile: (index: number) => void;
  updateFileContent: (index: number, content: string) => void;
  newFile: (name: string, pouType: string) => Promise<void>;
  newFbdFile: (name: string) => void;
  saveFile: (index: number) => Promise<void>;
  openProjectDir: () => Promise<void>;
  refreshProjectFiles: () => Promise<void>;
  loadSampleTemplate: (template: SampleTemplate) => void;
  loadFbdSample: (sample: { name: string; program: unknown }) => void;
  newLdFile: (name: string) => void;
  loadLdSample: (sample: { name: string; program: unknown }) => void;
  newSfcFile: (name: string) => void;
  loadSfcSample: (sample: { name: string; program: unknown }) => void;

  // Compilation
  compiling: boolean;
  lastCompileResult: CompileResult | null;
  diagnostics: Diagnostic[];
  compile: () => Promise<void>;

  // Runtime
  runtimeState: "stopped" | "running" | "paused" | "error";
  scanCycleMs: number;
  maxCycleMs: number;
  cycleCount: number;
  lastCycleMs: number;
  avgCycleMs: number;
  cycleHistory: number[];
  plcRun: () => Promise<void>;
  plcStop: () => Promise<void>;
  plcStep: () => Promise<void>;
  setScanCycle: (ms: number) => void;
  setMaxCycle: (ms: number) => void;

  // Watch table
  watchedVariables: WatchedVar[];
  addWatch: (name: string) => void;
  removeWatch: (index: number) => void;
  forceVariable: (name: string, value: string) => Promise<void>;
  unforceVariable: (name: string) => Promise<void>;

  // I/O
  ioDriver: IoDriverConfig | null;
  ioMappings: IoMapping[];
  compiledVariables: CompiledVariable[];
  addIoMapping: (mapping: IoMapping) => void;
  updateIoMapping: (index: number, mapping: IoMapping) => void;
  removeIoMapping: (index: number) => void;
  clearIoMappings: () => void;
  autoMapGpio: () => void;

  // Bottom panel
  bottomTab: "diagnostics" | "log" | "output" | "timing" | "histogram" | "search" | "simulation";
  setBottomTab: (tab: "diagnostics" | "log" | "output" | "timing" | "histogram" | "search" | "simulation") => void;
  log: LogEntry[];
  addLog: (direction: LogEntry["direction"], message: string) => void;

  // Simulation
  simulationInputs: Map<string, boolean>;
  toggleSimulationInput: (name: string) => void;
  resetSimulationInputs: () => void;

  // Search
  searchQuery: string;
  searchUseRegex: boolean;
  searchResults: Array<{ file: string; line: number; col: number; text: string }>;
  setSearchQuery: (q: string) => void;
  setSearchUseRegex: (r: boolean) => void;
  searchAllFiles: () => void;

  // AI Chat
  aiMessages: ChatMessage[];
  addAiMessage: (msg: ChatMessage) => void;
  clearAiMessages: () => void;
  aiProvider: "claude" | "ollama";
  setAiProvider: (p: "claude" | "ollama") => void;
  aiModel: string;
  setAiModel: (m: string) => void;
  aiStreaming: boolean;
  setAiStreaming: (s: boolean) => void;
  ollamaEndpoint: string;
  setOllamaEndpoint: (e: string) => void;
  aiApiKey: string;
  setAiApiKey: (k: string) => void;
  rightPanelTab: "watch" | "io" | "ai" | "target" | "refs";
  setRightPanelTab: (t: "watch" | "io" | "ai" | "target" | "refs") => void;

  // Target (remote ESP32)
  targetIp: string;
  targetConnected: boolean;
  targetDeploying: boolean;
  scadaUrl: string;
  executionMode: "local" | "remote";
  discoveredTargets: DiscoveredTarget[];
  scanningTargets: boolean;
  targetKind: TargetKind;
  setTargetKind: (kind: TargetKind) => void;
  setTargetIp: (ip: string) => void;
  setScadaUrl: (url: string) => void;
  setExecutionMode: (mode: "local" | "remote") => void;
  targetConnect: () => Promise<void>;
  targetDisconnect: () => Promise<void>;
  targetDeploy: () => Promise<void>;
  targetRun: () => Promise<void>;
  targetStop: () => Promise<void>;
  targetStep: () => Promise<void>;
  connectScada: () => Promise<void>;
  scanTargets: () => Promise<void>;

  // Cross-reference
  xrefVariable: string;
  xrefResults: Array<{ file: string; line: number; context: string; kind: "read" | "write" | "declare" }>;
  setXrefVariable: (v: string) => void;
  findReferences: () => void;

  // AI Features
  sendAiErrorFix: (diagnostic: Diagnostic) => void;
  sendAiTestGeneration: () => void;

  // Debug
  breakpoints: number[];
  addBreakpoint: (line: number) => void;
  removeBreakpoint: (line: number) => void;
  currentExecutionLine: number | null;
  debugState: "running" | "paused" | "stopped";
  setDebugState: (s: "running" | "paused" | "stopped") => void;
  setCurrentExecutionLine: (line: number | null) => void;

  // Undo/Redo
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;

  // Project serialization
  projectPath: string | null;
  serializeProject: () => ProjectFile;
  deserializeProject: (p: ProjectFile) => void;
  saveProject: () => Promise<void>;
  saveProjectAs: () => Promise<void>;
  loadProject: () => Promise<void>;
}

const DEFAULT_PROGRAM = `PROGRAM Main
VAR
    counter : INT := 0;
    enable : BOOL := TRUE;
END_VAR

    IF enable THEN
        counter := counter + 1;
    END_IF;

    IF counter > 100 THEN
        counter := 0;
    END_IF;
END_PROGRAM`;

// Waveshare ESP32-S3-Relay-6ch auto-map: variable name → GPIO address index
const GPIO_AUTO_MAP: Record<string, number> = {
  relay0: 0, relay1: 1, relay2: 2, relay3: 3, relay4: 4, relay5: 5,
  buzzer: 6, red: 7, green: 8, blue: 9,
  led: 0, onboard_led: 0, coil0: 0,
};

// Undo/Redo infrastructure
type UndoableState = Pick<AppState, "ioMappings" | "watchedVariables" | "scanCycleMs" | "maxCycleMs" | "breakpoints">;
const MAX_UNDO = 50;
const undoStack: UndoableState[] = [];
const redoStack: UndoableState[] = [];

function snapshotUndoable(state: AppState): UndoableState {
  return {
    ioMappings: JSON.parse(JSON.stringify(state.ioMappings)),
    watchedVariables: JSON.parse(JSON.stringify(state.watchedVariables)),
    scanCycleMs: state.scanCycleMs,
    maxCycleMs: state.maxCycleMs,
    breakpoints: [...state.breakpoints],
  };
}

function pushUndo(state: AppState) {
  undoStack.push(snapshotUndoable(state));
  if (undoStack.length > MAX_UNDO) undoStack.shift();
  redoStack.length = 0;
  useStore.setState({ canUndo: true, canRedo: false });
}

export const useStore = create<AppState>((set, get) => ({
  // Theme
  theme: "dark",
  setTheme: (t) => set({ theme: t }),

  // Files
  openFiles: [
    {
      name: "Main.st",
      path: "Main.st",
      content: DEFAULT_PROGRAM,
      modified: false,
    },
  ],
  activeFileIndex: 0,
  projectDir: null,
  projectFiles: [],

  openFile: async (path: string) => {
    try {
      const content = await invoke<string>("open_file", { path });
      const name = path.split("/").pop() || path;
      const { openFiles } = get();
      const existingIndex = openFiles.findIndex((f) => f.path === path);
      if (existingIndex >= 0) {
        set({ activeFileIndex: existingIndex });
        return;
      }
      set({
        openFiles: [...openFiles, { name, path, content, modified: false }],
        activeFileIndex: openFiles.length,
      });
    } catch (e) {
      get().addLog("system", `Failed to open ${path}: ${e}`);
    }
  },

  closeFile: (index) => {
    const { openFiles, activeFileIndex } = get();
    const newFiles = openFiles.filter((_, i) => i !== index);
    let newIndex = activeFileIndex;
    if (activeFileIndex >= newFiles.length) {
      newIndex = Math.max(0, newFiles.length - 1);
    } else if (activeFileIndex > index) {
      newIndex = activeFileIndex - 1;
    }
    set({ openFiles: newFiles, activeFileIndex: newIndex });
  },

  setActiveFile: (index) => set({ activeFileIndex: index }),

  updateFileContent: (index, content) => {
    const { openFiles } = get();
    const newFiles = [...openFiles];
    const file = newFiles[index];
    // Editors re-serialize on mount; identical content must not flip the dirty flag.
    if (!file || file.content === content) return;
    newFiles[index] = { ...file, content, modified: true };
    set({ openFiles: newFiles });
  },

  newFile: async (name, pouType) => {
    try {
      const content = await invoke<string>("new_file", { name, pouType });
      const { openFiles } = get();
      const fileName = name.endsWith(".st") ? name : `${name}.st`;
      set({
        openFiles: [
          ...openFiles,
          { name: fileName, path: fileName, content, modified: false },
        ],
        activeFileIndex: openFiles.length,
      });
    } catch (e) {
      get().addLog("system", `Failed to create file: ${e}`);
    }
  },

  newFbdFile: (name: string) => {
    const { openFiles } = get();
    const fileName = name.endsWith(".fbd") ? name : `${name}.fbd`;
    const content = serializeFbd({
      version: DIAGRAM_FORMAT_VERSION,
      programName: name.replace(/\.fbd$/, ""),
      declarations: [],
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    set({
      openFiles: [...openFiles, { name: fileName, path: fileName, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  },

  saveFile: async (index) => {
    const { openFiles } = get();
    const file = openFiles[index];
    if (!file) return;
    // PLC-5: every diagram save is canonicalized before it reaches disk.
    const content = canonicalizeFileContent(file.name, file.content);
    try {
      await invoke("save_file", { path: file.path, content });
      const newFiles = [...openFiles];
      newFiles[index] = { ...newFiles[index], content, modified: false };
      set({ openFiles: newFiles });
      get().addLog("system", `Saved ${file.name}`);
    } catch (e) {
      get().addLog("system", `Failed to save ${file.name}: ${e}`);
    }
  },

  openProjectDir: async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "Open Project Folder" });
      if (selected) {
        set({ projectDir: selected });
        get().addLog("system", `Opened project: ${selected}`);
        await get().refreshProjectFiles();
      }
    } catch (e) {
      get().addLog("system", `Failed to open folder: ${e}`);
    }
  },

  refreshProjectFiles: async () => {
    const { projectDir } = get();
    if (!projectDir) return;
    try {
      const files = await invoke<FileEntry[]>("list_project_files", { dir: projectDir });
      set({ projectFiles: files });
    } catch (e) {
      get().addLog("system", `Failed to list files: ${e}`);
    }
  },

  loadSampleTemplate: (template: SampleTemplate) => {
    const { openFiles } = get();
    const fileName = `${template.name.replace(/\s+/g, "")}.st`;
    set({
      openFiles: [
        ...openFiles,
        { name: fileName, path: fileName, content: template.source, modified: false },
      ],
      activeFileIndex: openFiles.length,
    });
  },

  loadFbdSample: (sample: { name: string; program: unknown }) => {
    const { openFiles } = get();
    const fileName = `${sample.name.replace(/\s+/g, "")}.fbd`;
    const content = serializeFbd(sample.program as FbdProgram);
    set({
      openFiles: [
        ...openFiles,
        { name: fileName, path: fileName, content, modified: false },
      ],
      activeFileIndex: openFiles.length,
    });
  },

  newLdFile: (name: string) => {
    const { openFiles } = get();
    const fileName = name.endsWith(".ld") ? name : `${name}.ld`;
    const content = serializeLd({
      version: LD_FORMAT_VERSION,
      programName: name.replace(/\.ld$/, ""),
      rungs: [],
    });
    set({
      openFiles: [...openFiles, { name: fileName, path: fileName, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  },

  loadLdSample: (sample: { name: string; program: unknown }) => {
    const { openFiles } = get();
    const fileName = `${sample.name.replace(/\s+/g, "")}.ld`;
    const content = serializeLd(sample.program as LdProgram);
    set({
      openFiles: [...openFiles, { name: fileName, path: fileName, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  },

  newSfcFile: (name: string) => {
    const { openFiles } = get();
    const fileName = name.endsWith(".sfc") ? name : `${name}.sfc`;
    const content = serializeSfc({
      version: DIAGRAM_FORMAT_VERSION,
      programName: name.replace(/\.sfc$/, ""),
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    set({
      openFiles: [...openFiles, { name: fileName, path: fileName, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  },

  loadSfcSample: (sample: { name: string; program: unknown }) => {
    const { openFiles } = get();
    const fileName = `${sample.name.replace(/\s+/g, "")}.sfc`;
    const content = serializeSfc(sample.program as SfcProgram);
    set({
      openFiles: [...openFiles, { name: fileName, path: fileName, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  },
  compiling: false,
  lastCompileResult: null,
  diagnostics: [],

  compile: async () => {
    const { openFiles, activeFileIndex } = get();
    const file = openFiles[activeFileIndex];
    if (!file) return;

    // Transpile a single file to ST if it's a graphical language
    async function transpileFile(f: typeof file): Promise<{
      source: string;
      fileName: string;
      diagnostics: Array<{ severity: string; message: string; source: string }>;
    } | null> {
      const ext = f.name.split(".").pop()?.toLowerCase();
      if (ext === "st" || ext === "txt" || !ext) {
        return { source: f.content, fileName: f.name, diagnostics: [] };
      }

      const transpilerMap: Record<string, { transpile: (program: any) => { source: string; diagnostics: any[] }; label: string }> = {
        fbd: { transpile: transpileFbdToSt, label: "FBD" },
        ld:  { transpile: transpileLdToSt,  label: "LD" },
        sfc: { transpile: transpileSfcToSt, label: "SFC" },
      };

      const entry = transpilerMap[ext ?? ""];
      if (!entry) return { source: f.content, fileName: f.name, diagnostics: [] };

      try {
        const program = JSON.parse(f.content);
        if (!program || typeof program !== "object" || typeof program.programName !== "string") {
          return {
            source: "",
            fileName: f.name,
            diagnostics: [{ severity: "error", message: `Invalid ${entry.label} diagram: missing required fields`, source: `${ext}-transpiler` }],
          };
        }
        const result = entry.transpile(program);
        return {
          source: result.source,
          fileName: f.name.replace(new RegExp(`\\.${ext}$`), ".st"),
          diagnostics: result.diagnostics.map((d: any) => ({
            severity: d.severity,
            message: d.message,
            source: `${ext}-transpiler`,
          })),
        };
      } catch (e) {
        return {
          source: "",
          fileName: f.name,
          diagnostics: [{ severity: "error", message: `${entry.label} parse error: ${e}`, source: `${ext}-transpiler` }],
        };
      }
    }

    // Transpile ALL files in the project
    const allDiagnostics: Array<{ severity: string; message: string; source: string }> = [];
    let mainSource = file.content;
    let mainFileName = file.name;

    // Transpile active file
    const mainResult = await transpileFile(file);
    if (mainResult) {
      mainSource = mainResult.source;
      mainFileName = mainResult.fileName;
      allDiagnostics.push(...mainResult.diagnostics);
    }

    // Transpile extra files (mixed-language support)
    const extraSources: [string, string][] = [];
    for (let i = 0; i < openFiles.length; i++) {
      if (i === activeFileIndex) continue;
      const extraResult = await transpileFile(openFiles[i]);
      if (extraResult) {
        extraSources.push([extraResult.fileName, extraResult.source]);
        allDiagnostics.push(...extraResult.diagnostics);
      }
    }

    // Check for transpiler errors across all files
    const transpileErrors = allDiagnostics.filter((d) => d.severity === "error");
    if (transpileErrors.length > 0) {
      set({
        compiling: false,
        diagnostics: transpileErrors.map((d) => ({
          severity: d.severity as "error",
          message: d.message,
          line: 0, col: 0, end_line: 0, end_col: 0, source: d.source,
        })),
      });
      get().addLog("system", `Transpile failed: ${transpileErrors.length} errors`);
      return;
    }

    const transpileWarnings = allDiagnostics.filter((d) => d.severity === "warning");

    set({ compiling: true });
    try {
      const result = await invoke<CompileResult>("compile_source", {
        source: mainSource,
        fileName: mainFileName,
        extraSources: extraSources.length > 0 ? extraSources : undefined,
      });
      set({
        compiling: false,
        lastCompileResult: result,
        diagnostics: [
          ...transpileWarnings.map((d) => ({
            severity: d.severity as "warning",
            message: d.message,
            line: 0, col: 0, end_line: 0, end_col: 0, source: d.source,
          })),
          ...result.diagnostics,
        ],
        compiledVariables: result.success && result.module
          ? result.module.variable_slots
              .map(([name, _val], i) => ({ name: String(name), type_name: "", index: i }))
              .filter((v) => !v.name.includes(".") && !v.name.includes("["))
          : [],
      });
      get().addLog(
        "system",
        result.success
          ? `Compiled successfully (${result.pou_count} POU, ${result.variable_count} vars)`
          : `Compilation failed (${result.diagnostics.length} errors)`
      );
    } catch (e) {
      set({ compiling: false });
      get().addLog("system", `Compile error: ${e}`);
    }
  },

  // Runtime
  runtimeState: "stopped",
  scanCycleMs: 10,
  maxCycleMs: 1000,
  cycleCount: 0,
  lastCycleMs: 0,
  avgCycleMs: 0,
  cycleHistory: [],

  plcRun: async () => {
    if (get().executionMode === "remote" && get().targetConnected) {
      return get().targetRun();
    }
    try {
      await invoke("plc_run", {
        config: { scan_cycle_ms: get().scanCycleMs, max_cycle_ms: get().maxCycleMs },
      });
      set({ runtimeState: "running" });
    } catch (e) {
      get().addLog("system", `Runtime error: ${e}`);
      set({ runtimeState: "error" });
    }
  },

  plcStop: async () => {
    if (get().executionMode === "remote" && get().targetConnected) {
      return get().targetStop();
    }
    try {
      await invoke("plc_stop");
      set({ runtimeState: "stopped", simulationInputs: new Map() });
    } catch (e) {
      get().addLog("system", `Stop error: ${e}`);
    }
  },

  plcStep: async () => {
    if (get().executionMode === "remote" && get().targetConnected) {
      return get().targetStep();
    }
    try {
      await invoke("plc_step");
    } catch (e) {
      get().addLog("system", `Step error: ${e}`);
    }
  },

  setScanCycle: (ms) => { pushUndo(get()); set({ scanCycleMs: ms }); },
  setMaxCycle: (ms) => { pushUndo(get()); set({ maxCycleMs: ms }); },

  // Watch
  watchedVariables: [],
  addWatch: (name) => {
    const { watchedVariables } = get();
    if (watchedVariables.some((v) => v.name === name)) return;
    pushUndo(get());
    set({
      watchedVariables: [
        ...watchedVariables,
        { name, value: null, display_value: "-", type_name: "-", quality: "good" },
      ],
    });
  },
  removeWatch: (index) => {
    pushUndo(get());
    const { watchedVariables } = get();
    set({ watchedVariables: watchedVariables.filter((_, i) => i !== index) });
  },
  forceVariable: async (name, value) => {
    try {
      const parsed = parseForceValue(value);
      await invoke("force_variable", { name, value: parsed });
      get().addLog("system", `Forced ${name} := ${value}`);
    } catch (e) {
      get().addLog("system", `Force failed: ${e}`);
    }
  },
  unforceVariable: async (name) => {
    try {
      await invoke("unforce_variable", { name });
      get().addLog("system", `Unforced ${name}`);
    } catch (e) {
      get().addLog("system", `Unforce failed: ${e}`);
    }
  },

  // I/O
  ioDriver: null,
  ioMappings: [],
  compiledVariables: [],

  addIoMapping: (mapping) => {
    const { ioMappings } = get();
    if (ioMappings.some((m) => m.variable === mapping.variable)) return;
    pushUndo(get());
    set({ ioMappings: [...ioMappings, mapping] });
  },

  updateIoMapping: (index, mapping) => {
    const { ioMappings } = get();
    pushUndo(get());
    set({ ioMappings: ioMappings.map((m, i) => (i === index ? mapping : m)) });
  },

  removeIoMapping: (index) => {
    pushUndo(get());
    set({ ioMappings: get().ioMappings.filter((_, i) => i !== index) });
  },

  clearIoMappings: () => { pushUndo(get()); set({ ioMappings: [] }); },

  autoMapGpio: () => {
    const { compiledVariables, ioMappings } = get();
    const mapped = new Set(ioMappings.map((m) => m.variable));
    const newMappings: IoMapping[] = [];
    for (const v of compiledVariables) {
      if (mapped.has(v.name)) continue;
      const match = GPIO_AUTO_MAP[v.name];
      if (match) {
        newMappings.push({
          variable: v.name,
          direction: "output",
          address: String(match),
          driver_id: "gpio",
          protocol_address: `GPIO${match}`,
        });
        mapped.add(v.name);
      }
    }
    if (newMappings.length > 0) {
      pushUndo(get());
      set({ ioMappings: [...ioMappings, ...newMappings] });
    }
  },

  // Bottom panel
  bottomTab: "diagnostics",
  setBottomTab: (tab) => set({ bottomTab: tab }),
  log: [],

  // Simulation
  simulationInputs: new Map(),
  toggleSimulationInput: (name) => {
    const { simulationInputs, forceVariable, unforceVariable } = get();
    const next = new Map(simulationInputs);
    if (next.has(name)) {
      next.delete(name);
      unforceVariable(name).catch(() => {
        const reverted = new Map(get().simulationInputs);
        reverted.set(name, true);
        set({ simulationInputs: reverted });
      });
    } else {
      next.set(name, true);
      forceVariable(name, "TRUE").catch(() => {
        const reverted = new Map(get().simulationInputs);
        reverted.delete(name);
        set({ simulationInputs: reverted });
      });
    }
    set({ simulationInputs: next });
  },
  resetSimulationInputs: () => {
    const { simulationInputs, unforceVariable } = get();
    for (const name of simulationInputs.keys()) {
      unforceVariable(name);
    }
    set({ simulationInputs: new Map() });
  },

  searchQuery: "",
  searchUseRegex: false,
  searchResults: [],
  setSearchQuery: (q) => set({ searchQuery: q }),
  setSearchUseRegex: (r) => set({ searchUseRegex: r }),
  searchAllFiles: () => {
    const { openFiles, searchQuery, searchUseRegex } = get();
    if (!searchQuery.trim()) {
      set({ searchResults: [] });
      return;
    }
    const escaped = searchQuery.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const pattern = searchUseRegex ? searchQuery : escaped;
    try {
      const re = new RegExp(pattern, "gi");
      const results: Array<{ file: string; line: number; col: number; text: string }> = [];
      for (const file of openFiles) {
        const lines = file.content.split("\n");
        for (let i = 0; i < lines.length; i++) {
          re.lastIndex = 0;
          let match;
          while ((match = re.exec(lines[i])) !== null) {
            results.push({ file: file.name, line: i + 1, col: match.index + 1, text: lines[i].trim() });
            if (results.length > 200) break;
          }
          if (results.length > 200) break;
        }
        if (results.length > 200) break;
      }
      set({ searchResults: results });
    } catch {
      set({ searchResults: [] });
    }
  },
  addLog: (direction, message) => {
    set({
      log: [
        ...get().log.slice(-999),
        {
          direction,
          message,
          timestamp: Date.now(),
        },
      ],
    });
  },

  // AI Chat
  aiMessages: [],
  addAiMessage: (msg) => set({ aiMessages: [...get().aiMessages.slice(-100), msg] }),
  clearAiMessages: () => set({ aiMessages: [] }),
  aiProvider: "claude",
  setAiProvider: (p) => set({ aiProvider: p }),
  aiModel: "claude-sonnet-4-6",
  setAiModel: (m) => set({ aiModel: m }),
  aiStreaming: false,
  setAiStreaming: (s) => set({ aiStreaming: s }),
  ollamaEndpoint: "localhost:11434",
  setOllamaEndpoint: (e) => set({ ollamaEndpoint: e }),
  aiApiKey: "",
  setAiApiKey: (k) => set({ aiApiKey: k }),
  rightPanelTab: "watch",
  setRightPanelTab: (t) => set({ rightPanelTab: t }),

  // Target (remote ESP32)
  targetIp: "",
  targetConnected: false,
  targetDeploying: false,
  scadaUrl: "http://localhost:3000",
  executionMode: "local",
  discoveredTargets: [],
  scanningTargets: false,
  targetKind: "websocket" as TargetKind,
  setTargetKind: (kind) => {
    set({ targetKind: kind });
    invoke("set_target_kind", { kind: kind === "grpc" ? "Grpc" : "WebSocket" }).catch(() => {});
  },
  setTargetIp: (ip) => set({ targetIp: ip }),
  setScadaUrl: (url) => set({ scadaUrl: url }),
  setExecutionMode: (mode) => {
    const { runtimeState, targetConnected } = get();
    if (mode === "remote" && runtimeState === "running") {
      invoke("plc_stop").catch(() => {});
      set({ runtimeState: "stopped", simulationInputs: new Map() });
    }
    if (mode === "local" && targetConnected) {
      get().targetStop();
    }
    set({ executionMode: mode });
  },

  targetConnect: async () => {
    // Cloud mode connects through the device relay keyed by the relay device
    // id (vplc_device), not a LAN IP — the bridge ignores `ip` there.
    const ip =
      bridgeMode === "cloud" ? (localStorage.getItem("vplc_device") ?? "") : get().targetIp.trim();
    if (!ip) return;
    try {
      await invoke("target_connect", { ip });
      set({ targetConnected: true, executionMode: "remote" });
      get().addLog("system", `Connected to target ${ip}`);
    } catch (e) {
      get().addLog("system", `Connection failed: ${e}`);
    }
  },

  targetDisconnect: async () => {
    try {
      await invoke("target_disconnect");
      set({ targetConnected: false, executionMode: "local" });
      get().addLog("system", "Disconnected from target");
    } catch (e) {
      get().addLog("system", `Disconnect failed: ${e}`);
    }
  },

  targetDeploy: async () => {
    const ip = get().targetIp.trim();
    if (!ip) return;
    set({ targetDeploying: true });
    try {
      // Compile first
      await get().compile();
      const result = get().lastCompileResult;
      if (!result?.success) {
        get().addLog("system", "Deploy failed: compilation errors");
        set({ targetDeploying: false });
        return;
      }

      // Deploy with retry (3 attempts)
      let lastError: string | null = null;
      const isGrpc = get().targetKind === "grpc";
      for (let attempt = 1; attempt <= 3; attempt++) {
        try {
          const deployResult = isGrpc
            ? await invoke<DeployResult>("deploy_via_grpc", { addr: `http://${ip}:50051`, ioMappings: get().ioMappings })
            : await invoke<DeployResult>("deploy_to_target", { ip, ioMappings: get().ioMappings });
          if (deployResult.success) {
            get().addLog("system", `Deployed ${deployResult.byte_count} bytes to ${ip}`);
            set({ targetDeploying: false });
            return;
          }
          lastError = deployResult.message;
        } catch (e) {
          lastError = String(e);
        }
        if (attempt < 3) {
          get().addLog("system", `Deploy attempt ${attempt} failed, retrying...`);
          await new Promise((r) => setTimeout(r, 500 * attempt));
        }
      }
      get().addLog("system", `Deploy failed after 3 attempts: ${lastError}`);
    } catch (e) {
      get().addLog("system", `Deploy error: ${e}`);
    }
    set({ targetDeploying: false });
  },

  targetRun: async () => {
    if (get().targetKind === "grpc") {
      try {
        const addr = `http://${get().targetIp.trim()}:50051`;
        await invoke("grpc_start", { addr, cycleMs: get().scanCycleMs });
        get().addLog("system", "Remote runtime started (gRPC)");
      } catch (e) { get().addLog("system", `gRPC start failed: ${e}`); }
      return;
    }
    try {
      await invoke("target_start", { cycleMs: get().scanCycleMs });
      get().addLog("system", "Remote runtime started");
    } catch (e) {
      get().addLog("system", `Start failed: ${e}`);
    }
  },

  targetStop: async () => {
    if (get().targetKind === "grpc") {
      try {
        const addr = `http://${get().targetIp.trim()}:50051`;
        await invoke("grpc_stop", { addr });
        get().addLog("system", "Remote runtime stopped (gRPC)");
      } catch (e) { get().addLog("system", `gRPC stop failed: ${e}`); }
      return;
    }
    try {
      await invoke("target_stop");
      get().addLog("system", "Remote runtime stopped");
    } catch (e) {
      get().addLog("system", `Stop failed: ${e}`);
    }
  },

  targetStep: async () => {
    if (get().targetKind === "grpc") {
      try {
        const addr = `http://${get().targetIp.trim()}:50051`;
        await invoke("grpc_step", { addr });
      } catch (e) { get().addLog("system", `gRPC step failed: ${e}`); }
      return;
    }
    try {
      await invoke("target_step");
    } catch (e) {
      get().addLog("system", `Step failed: ${e}`);
    }
  },

  connectScada: async () => {
    const scadaUrl = get().scadaUrl.trim();
    // In cloud mode the "target" identity is the relay device id, not an IP.
    const targetIdentity =
      bridgeMode === "cloud" ? (localStorage.getItem("vplc_device") ?? "") : get().targetIp.trim();
    if (!scadaUrl || !targetIdentity || !get().targetConnected) return;

    const registration: ScadaRegistration = {
      scada_url: scadaUrl,
      device_name: bridgeMode === "cloud" ? `Voltrus-Relay-${targetIdentity}` : `ESP32-${targetIdentity}`,
      device_url:
        bridgeMode === "cloud" ? `relay://${targetIdentity}` : `ws://${targetIdentity}/ws/plc`,
      transport: "websocket",
      interval_ms: 1000,
    };

    for (let attempt = 1; attempt <= 3; attempt++) {
      try {
        const result = await invoke("register_scada_device", { registration });
        get().addLog("system", `Registered in SCADA: ${JSON.stringify(result)}`);
        return;
      } catch (e) {
        if (attempt < 3) {
          get().addLog("system", `SCADA registration attempt ${attempt} failed, retrying...`);
          await new Promise((r) => setTimeout(r, 1000 * attempt));
        } else {
          get().addLog("system", `SCADA registration failed after 3 attempts: ${e}`);
        }
      }
    }
  },

  scanTargets: async () => {
    set({ scanningTargets: true });
    try {
      const targets = await invoke<DiscoveredTarget[]>("scan_mdns_targets");
      set({ discoveredTargets: targets });
      if (targets.length > 0) {
        get().addLog("system", `Found ${targets.length} device(s) on network`);
      } else {
        get().addLog("system", "No devices found on network");
      }
    } catch (e) {
      get().addLog("system", `mDNS scan failed: ${e}`);
      set({ discoveredTargets: [] });
    }
    set({ scanningTargets: false });
  },

  // AI Features
  xrefVariable: "",
  xrefResults: [],
  setXrefVariable: (v) => set({ xrefVariable: v }),
  findReferences: () => {
    const { openFiles, xrefVariable } = get();
    if (!xrefVariable.trim()) {
      set({ xrefResults: [] });
      return;
    }
    const varName = xrefVariable.trim();
    const escaped = varName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const results: Array<{ file: string; line: number; context: string; kind: "read" | "write" | "declare" }> = [];
    const anyRe = new RegExp(`\\b${escaped}\\b`, "g");
    const declareRe = new RegExp(`\\b${escaped}\\b\\s*:(?!=)`); // colon not followed by = (not :=)
    const lhsWriteRe = new RegExp(`(?:^|[,;])\\s*\\b${escaped}\\b\\s*:=`); // var on LHS of :=
    for (const file of openFiles) {
      const lines = file.content.split("\n");
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (!anyRe.test(line)) continue;
        anyRe.lastIndex = 0;
        if (declareRe.test(line)) {
          results.push({ file: file.name, line: i + 1, context: line.trim(), kind: "declare" });
          continue; // declaration lines aren't also reads/writes
        }
        if (lhsWriteRe.test(line)) {
          results.push({ file: file.name, line: i + 1, context: line.trim(), kind: "write" });
        }
        // Check for reads: count all occurrences minus the LHS write occurrence
        const allMatches = line.match(anyRe);
        const writeCount = lhsWriteRe.test(line) ? 1 : 0;
        if (allMatches && allMatches.length > writeCount) {
          results.push({ file: file.name, line: i + 1, context: line.trim(), kind: "read" });
        }
        if (results.length > 100) break;
      }
      if (results.length > 100) break;
    }
    set({ xrefResults: results });
  },

  sendAiErrorFix: (diagnostic: Diagnostic) => {
    const { openFiles, activeFileIndex } = get();
    const source = openFiles[activeFileIndex]?.content ?? "";
    const prompt = `Fix this compile error in my IEC 61131-3 Structured Text program:

Error: ${diagnostic.severity} at line ${diagnostic.line}:${diagnostic.col} — ${diagnostic.message}

Current code (line ${diagnostic.line} highlighted):
\`\`\`st
${source.split("\n").map((line, i) => `${i + 1}${i + 1 === diagnostic.line ? " >>>" : "   "}: ${line}`).join("\n")}
\`\`\`

Provide the corrected code in a single \`\`\`st code block. Explain what was wrong briefly.`;

    const userMsg: ChatMessage = { role: "user", content: prompt };
    get().addAiMessage(userMsg);
    get().setRightPanelTab("ai");
  },
  sendAiTestGeneration: () => {
    const { openFiles, activeFileIndex } = get();
    const source = openFiles[activeFileIndex]?.content ?? "";
    const fileName = openFiles[activeFileIndex]?.name ?? "Unknown";
    const prompt = `Generate test scenarios for this IEC 61131-3 Structured Text program "${fileName}":

\`\`\`st
${source}
\`\`\`

Create test cases that cover:
1. Normal operation paths
2. Edge cases (boundary values, overflow)
3. Error conditions
4. Timer/counter behavior if applicable

For each test case provide:
- Description
- Input variable values (initial state)
- Expected output after one scan cycle
- Expected output after N scan cycles (if timer/counter related)

Format as a structured test plan.`;

    const userMsg: ChatMessage = { role: "user", content: prompt };
    get().addAiMessage(userMsg);
    get().setRightPanelTab("ai");
  },

  // Debug
  breakpoints: [],
  addBreakpoint: (line) => {
    const s = get();
    if (s.breakpoints.includes(line)) return;
    pushUndo(s);
    if (s.executionMode === "remote" && s.targetConnected) {
      invoke("set_remote_breakpoint", { line }).catch((e) =>
        get().addLog("system", `Remote breakpoint set failed: ${e}`)
      );
    }
    set({ breakpoints: [...s.breakpoints, line].sort((a, b) => a - b) });
  },
  removeBreakpoint: (line) => {
    pushUndo(get());
    const s = get();
    if (s.executionMode === "remote" && s.targetConnected) {
      invoke("clear_remote_breakpoint", { line }).catch((e) =>
        get().addLog("system", `Remote breakpoint clear failed: ${e}`)
      );
    }
    set((s) => ({ breakpoints: s.breakpoints.filter((l) => l !== line) }));
  },
  currentExecutionLine: null,
  debugState: "stopped",
  setDebugState: (s) => set({ debugState: s }),
  setCurrentExecutionLine: (line) => set({ currentExecutionLine: line }),

  // Undo/Redo
  canUndo: false,
  canRedo: false,
  undo: () => {
    if (undoStack.length === 0) return;
    redoStack.push(snapshotUndoable(get()));
    const snapshot = undoStack.pop()!;
    set({
      ...snapshot,
      canUndo: undoStack.length > 0,
      canRedo: true,
    });
  },
  redo: () => {
    if (redoStack.length === 0) return;
    undoStack.push(snapshotUndoable(get()));
    const snapshot = redoStack.pop()!;
    set({
      ...snapshot,
      canUndo: true,
      canRedo: redoStack.length > 0,
    });
  },

  // Project serialization
  projectPath: null,

  serializeProject: () => {
    const s = get();
    return {
      version: "0.4",
      files: s.openFiles.map((f) => ({
        name: f.name,
        path: f.path,
        content: f.content,
        is_active: s.openFiles[s.activeFileIndex]?.path === f.path,
      })),
      io_config: s.ioDriver,
      io_mappings: s.ioMappings,
      scan_cycle_ms: s.scanCycleMs,
      max_cycle_ms: s.maxCycleMs,
      ai_settings: {
        provider: s.aiProvider,
        model: s.aiModel,
        ollama_endpoint: s.ollamaEndpoint,
      },
      watch_list: s.watchedVariables.map((v) => v.name),
      breakpoints: [[s.openFiles[s.activeFileIndex]?.name || "", s.breakpoints]],
      target_ip: s.targetIp || undefined,
      scada_url: s.scadaUrl || undefined,
      execution_mode: s.executionMode,
      target_kind: s.targetKind,
      created_at: new Date().toISOString(),
    };
  },

  deserializeProject: (p) => {
    set({
      openFiles: p.files.map((f) => ({
        name: f.name,
        path: f.path,
        content: f.content,
        modified: false,
      })),
      activeFileIndex: Math.max(
        0,
        p.files.findIndex((f) => f.is_active)
      ),
      ioDriver: p.io_config,
      ioMappings: p.io_mappings || [],
      compiledVariables: [],
      scanCycleMs: p.scan_cycle_ms || 10,
      maxCycleMs: p.max_cycle_ms || 1000,
      aiProvider: (p.ai_settings?.provider || "claude") as "claude" | "ollama",
      aiModel: p.ai_settings?.model || "claude-sonnet-4-6",
      aiStreaming: false,
      ollamaEndpoint: p.ai_settings?.ollama_endpoint || "localhost:11434",
      breakpoints: p.breakpoints?.[0]?.[1] || [],
      targetIp: p.target_ip || "",
      scadaUrl: p.scada_url || "http://localhost:3000",
      executionMode: "local",
      targetKind: (p as any).target_kind === "grpc" ? "grpc" : "websocket",
      watchedVariables: [],
      diagnostics: [],
      log: [],
    });
    get().addLog("system", `Loaded project (${p.files.length} files, v${p.version})`);
  },

  saveProject: async () => {
    const { projectPath, serializeProject } = get();
    const project = serializeProject();
    if (projectPath) {
      try {
        await invoke("save_project", { path: projectPath, project });
        get().addLog("system", `Project saved to ${projectPath}`);
      } catch (e) {
        get().addLog("system", `Failed to save project: ${e}`);
      }
    } else {
      await get().saveProjectAs();
    }
  },

  saveProjectAs: async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({
        title: "Save Project As",
        filters: [{ name: "Voltrus Project", extensions: ["vltrus"] }],
      });
      if (path) {
        const project = get().serializeProject();
        await invoke("save_project", { path, project });
        set({ projectPath: path });
        get().addLog("system", `Project saved to ${path}`);
      }
    } catch (e) {
      get().addLog("system", `Failed to save project: ${e}`);
    }
  },

  loadProject: async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({
        title: "Open Project",
        filters: [{ name: "Voltrus Project", extensions: ["vltrus"] }],
      });
      if (path) {
        const project = await invoke<ProjectFile>("load_project", { path });
        get().deserializeProject(project);
        set({ projectPath: path });
      }
    } catch (e) {
      get().addLog("system", `Failed to load project: ${e}`);
    }
  },
}));

// Listen for runtime events from Tauri backend
let unlistenWatch: UnlistenFn | null = null;
let unlistenState: UnlistenFn | null = null;

let unlistenDisconnect: UnlistenFn | null = null;

export function clearUndoHistory() {
  undoStack.length = 0;
  redoStack.length = 0;
  useStore.setState({ canUndo: false, canRedo: false });
}

export async function setupRuntimeListeners() {
  if (unlistenWatch) return; // Already set up

  unlistenWatch = await listen<WatchedVar[]>("watch-update", (event) => {
    useStore.setState({ watchedVariables: event.payload });
  });

  unlistenState = await listen<RuntimeStateInfo>("runtime-state", (event) => {
    const info = event.payload;
    useStore.setState({
      runtimeState: info.state as "stopped" | "running" | "paused" | "error",
      cycleCount: info.cycle_count,
      lastCycleMs: info.last_cycle_ms,
      avgCycleMs: info.avg_cycle_ms,
      maxCycleMs: info.max_cycle_ms,
      cycleHistory: info.cycle_history || [],
      ...(info.state === "stopped" ? { simulationInputs: new Map() } : {}),
    });
    if (info.state === "error" && info.error_message) {
      useStore.getState().addLog("system", `Runtime error: ${info.error_message}`);
    }
  });

  unlistenDisconnect = await listen<{ ip: string }>("target-disconnected", async (event) => {
    const wasConnected = useStore.getState().targetConnected;
    useStore.setState({
      targetConnected: false,
      runtimeState: "stopped",
    });
    useStore.getState().addLog("system", "Target device disconnected");

    // Auto-reconnect with exponential backoff (3 attempts)
    if (wasConnected) {
      const ip = useStore.getState().targetIp.trim();
      if (ip) {
        useStore.getState().addLog("system", `Attempting to reconnect to ${ip}...`);
        for (let attempt = 1; attempt <= 3; attempt++) {
          await new Promise((r) => setTimeout(r, 1000 * attempt));
          try {
            await invoke("target_connect", { ip });
            useStore.setState({ targetConnected: true });
            useStore.getState().addLog("system", `Reconnected to ${ip} (attempt ${attempt})`);
            return;
          } catch {
            useStore.getState().addLog("system", `Reconnect attempt ${attempt} failed`);
          }
        }
        useStore.getState().addLog("system", `Auto-reconnect to ${ip} failed after 3 attempts`);
        useStore.setState({ executionMode: "local" });
      }
    }
  });
}

export async function teardownRuntimeListeners() {
  unlistenWatch?.();
  unlistenState?.();
  unlistenDisconnect?.();
  unlistenWatch = null;
  unlistenState = null;
  unlistenDisconnect = null;
}

function parseForceValue(input: string): unknown {
  const trimmed = input.trim().toUpperCase();
  if (trimmed === "TRUE" || trimmed === "1") return true;
  if (trimmed === "FALSE" || trimmed === "0") return false;
  const num = Number(input.trim());
  if (!isNaN(num)) return num;
  return input.trim();
}
