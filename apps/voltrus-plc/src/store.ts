import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
  SampleTemplate,
  ProjectFile,
  TargetStatus,
  DeployResult,
  ScadaRegistration,
} from "./types";

export interface CodeValidation {
  code: string;
  result: CompileResult | null;
  validating: boolean;
  validated: boolean;
}

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
  saveFile: (index: number) => Promise<void>;
  openProjectDir: () => Promise<void>;
  refreshProjectFiles: () => Promise<void>;
  loadSampleTemplate: (template: SampleTemplate) => void;

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

  // Bottom panel
  bottomTab: "diagnostics" | "log" | "output" | "timing" | "histogram";
  setBottomTab: (tab: "diagnostics" | "log" | "output" | "timing" | "histogram") => void;
  log: LogEntry[];
  addLog: (direction: LogEntry["direction"], message: string) => void;

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
  rightPanelTab: "watch" | "io" | "ai" | "target";
  setRightPanelTab: (t: "watch" | "io" | "ai" | "target") => void;

  // Target (remote ESP32)
  targetIp: string;
  targetConnected: boolean;
  targetDeploying: boolean;
  scadaUrl: string;
  executionMode: "local" | "remote";
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

  // AI Features
  aiCodeValidations: Map<string, CodeValidation>;
  validateAiCode: (code: string) => Promise<CompileResult | null>;
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
    if (newFiles[index]) {
      newFiles[index] = { ...newFiles[index], content, modified: true };
      set({ openFiles: newFiles });
    }
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

  saveFile: async (index) => {
    const { openFiles } = get();
    const file = openFiles[index];
    if (!file) return;
    try {
      await invoke("save_file", { path: file.path, content: file.content });
      const newFiles = [...openFiles];
      newFiles[index] = { ...newFiles[index], modified: false };
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

  // Compilation
  compiling: false,
  lastCompileResult: null,
  diagnostics: [],

  compile: async () => {
    const { openFiles, activeFileIndex } = get();
    const file = openFiles[activeFileIndex];
    if (!file) return;

    set({ compiling: true });
    try {
      const extraSources: [string, string][] = openFiles
        .filter((_, i) => i !== activeFileIndex)
        .map((f) => [f.name, f.content]);
      const result = await invoke<CompileResult>("compile_source", {
        source: file.content,
        fileName: file.name,
        extraSources: extraSources.length > 0 ? extraSources : undefined,
      });
      set({
        compiling: false,
        lastCompileResult: result,
        diagnostics: result.diagnostics,
        bottomTab: result.success ? "output" : "diagnostics",
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
    try {
      await invoke("plc_stop");
      set({ runtimeState: "stopped" });
    } catch (e) {
      get().addLog("system", `Stop error: ${e}`);
    }
  },

  plcStep: async () => {
    try {
      await invoke("plc_step");
    } catch (e) {
      get().addLog("system", `Step error: ${e}`);
    }
  },

  setScanCycle: (ms) => set({ scanCycleMs: ms }),
  setMaxCycle: (ms) => set({ maxCycleMs: ms }),

  // Watch
  watchedVariables: [],
  addWatch: (name) => {
    const { watchedVariables } = get();
    if (watchedVariables.some((v) => v.name === name)) return;
    set({
      watchedVariables: [
        ...watchedVariables,
        { name, value: null, display_value: "-", type_name: "-", quality: "good" },
      ],
    });
  },
  removeWatch: (index) => {
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

  // Bottom panel
  bottomTab: "diagnostics",
  setBottomTab: (tab) => set({ bottomTab: tab }),
  log: [],
  addLog: (direction, message) => {
    set({
      log: [
        ...get().log,
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
  addAiMessage: (msg) => set({ aiMessages: [...get().aiMessages, msg] }),
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
  setTargetIp: (ip) => set({ targetIp: ip }),
  setScadaUrl: (url) => set({ scadaUrl: url }),
  setExecutionMode: (mode) => set({ executionMode: mode }),

  targetConnect: async () => {
    const ip = get().targetIp.trim();
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
      const deployResult = await invoke<DeployResult>("deploy_to_target", {
        ip,
        ioMappings: get().ioMappings.map((m, i) => ({
          variable_index: i,
          io_type: m.direction === "input" ? 0 : 1,
          protocol: 0,
          address: parseInt(m.address.replace(/[^0-9]/g, "")) || 0,
        })),
      });
      if (deployResult.success) {
        get().addLog("system", `Deployed ${deployResult.byte_count} bytes to ${ip}`);
      } else {
        get().addLog("system", `Deploy failed: ${deployResult.message}`);
      }
    } catch (e) {
      get().addLog("system", `Deploy error: ${e}`);
    }
    set({ targetDeploying: false });
  },

  targetRun: async () => {
    try {
      await invoke("target_start", { cycleMs: get().scanCycleMs });
      get().addLog("system", "Remote runtime started");
    } catch (e) {
      get().addLog("system", `Start failed: ${e}`);
    }
  },

  targetStop: async () => {
    try {
      await invoke("target_stop");
      get().addLog("system", "Remote runtime stopped");
    } catch (e) {
      get().addLog("system", `Stop failed: ${e}`);
    }
  },

  targetStep: async () => {
    try {
      await invoke("target_step");
    } catch (e) {
      get().addLog("system", `Step failed: ${e}`);
    }
  },

  connectScada: async () => {
    const scadaUrl = get().scadaUrl.trim();
    const targetIp = get().targetIp.trim();
    if (!scadaUrl || !targetIp) return;
    try {
      const registration: ScadaRegistration = {
        scada_url: scadaUrl,
        device_name: `ESP32-${targetIp}`,
        device_url: targetIp,
        transport: "websocket",
        interval_ms: get().scanCycleMs,
      };
      const result = await invoke("register_scada_device", { registration });
      get().addLog("system", `Registered in SCADA: ${JSON.stringify(result)}`);
    } catch (e) {
      get().addLog("system", `SCADA registration failed: ${e}`);
    }
  },

  // AI Features
  aiCodeValidations: new Map(),
  validateAiCode: async (code: string) => {
    try {
      const result = await invoke<CompileResult>("compile_source", {
        source: code,
        fileName: "AI_Generated.st",
      });
      const { aiCodeValidations } = get();
      const newMap = new Map(aiCodeValidations);
      newMap.set(code, {
        code,
        result,
        validating: false,
        validated: true,
      });
      set({ aiCodeValidations: newMap });
      return result;
    } catch (e) {
      get().addLog("system", `AI validation failed: ${e}`);
      return null;
    }
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
  addBreakpoint: (line) =>
    set((s) => ({
      breakpoints: s.breakpoints.includes(line)
        ? s.breakpoints
        : [...s.breakpoints, line].sort((a, b) => a - b),
    })),
  removeBreakpoint: (line) =>
    set((s) => ({
      breakpoints: s.breakpoints.filter((l) => l !== line),
    })),
  currentExecutionLine: null,
  debugState: "stopped",
  setDebugState: (s) => set({ debugState: s }),
  setCurrentExecutionLine: (line) => set({ currentExecutionLine: line }),

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
        streaming: s.aiStreaming,
        ollama_endpoint: s.ollamaEndpoint,
      },
      watch_list: s.watchedVariables.map((v) => v.name),
      breakpoints: [[s.openFiles[s.activeFileIndex]?.name || "", s.breakpoints]],
      target_ip: s.targetIp || undefined,
      scada_url: s.scadaUrl || undefined,
      execution_mode: s.executionMode,
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
      scanCycleMs: p.scan_cycle_ms || 10,
      maxCycleMs: p.max_cycle_ms || 1000,
      aiProvider: (p.ai_settings?.provider || "claude") as "claude" | "ollama",
      aiModel: p.ai_settings?.model || "claude-sonnet-4-6",
      aiStreaming: p.ai_settings?.streaming || false,
      ollamaEndpoint: p.ai_settings?.ollama_endpoint || "localhost:11434",
      breakpoints: p.breakpoints?.[0]?.[1] || [],
      targetIp: p.target_ip || "",
      scadaUrl: p.scada_url || "http://localhost:3000",
      executionMode: p.execution_mode || "local",
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
    });
    if (info.state === "error" && info.error_message) {
      useStore.getState().addLog("system", `Runtime error: ${info.error_message}`);
    }
  });
}

export async function teardownRuntimeListeners() {
  unlistenWatch?.();
  unlistenState?.();
  unlistenWatch = null;
  unlistenState = null;
}

function parseForceValue(input: string): unknown {
  const trimmed = input.trim().toUpperCase();
  if (trimmed === "TRUE" || trimmed === "1") return true;
  if (trimmed === "FALSE" || trimmed === "0") return false;
  const num = Number(input.trim());
  if (!isNaN(num)) return num;
  return input.trim();
}
