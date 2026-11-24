import { useEffect, useState, type ReactElement } from "react";
import { useStore } from "./store";
import { invoke, listen } from "./api/bridge";
import {
  ArrowDownToLine,
  Blocks,
  BookOpen,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  Ellipsis,
  Eye,
  FileCode,
  FileInput,
  FilePlus,
  FileText,
  FlaskConical,
  Folder,
  FolderOpen,
  Hammer,
  Info,
  KeyRound,
  List,
  Milestone,
  Monitor,
  Moon,
  Palette,
  Network,
  Play,
  RefreshCw,
  Redo2,
  ScrollText,
  Save,
  Search,
  Sigma,
  SkipForward,
  SlidersHorizontal,
  Sparkles,
  Square,
  StepForward,
  Sun,
  Terminal,
  TextSearch,
  TriangleAlert,
  Undo2,
  Workflow,
  X,
} from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  IconButton,
  Select,
  StatusDot,
  Tabs,
  Toolbar,
} from "@mactools/ui";
import type { SampleTemplate } from "./types";
import EditorRouter from "./components/Editor/EditorRouter";
import WatchTable from "./components/Sidebar/WatchTable";
import RuntimeControls from "./components/Sidebar/RuntimeControls";
import IoPanel from "./components/Sidebar/IoPanel";
import IoMappingPanel from "./components/Sidebar/IoMappingPanel";
import TargetPanel from "./components/Sidebar/TargetPanel";
import OnlinePanel from "./components/Sidebar/OnlinePanel";
import AiChatPanel from "./components/Sidebar/AiChatPanel";
import LicensePanel from "./components/Sidebar/LicensePanel";
import AccountBadge from "./components/Cloud/AccountBadge";
import AiKeysPanel from "./components/Cloud/AiKeysPanel";
import CloudProjectPanel from "./components/Cloud/CloudProjectPanel";
import ProvisionPanel from "./components/Cloud/ProvisionPanel";
import AuthDialog from "./components/Cloud/AuthDialog";
import AuthScreen from "./components/Cloud/AuthScreen";
import UpgradeScreen from "./components/Cloud/UpgradeScreen";
import CycleGraph from "./components/Bottom/CycleGraph";
import Histogram from "./components/Bottom/Histogram";
import SimulationPanel from "./components/Bottom/SimulationPanel";
import TestsPanel from "./components/Tests/TestsPanel";
import ResizeHandle from "./components/ResizeHandle";
import ImportDialog from "./components/ImportDialog";
import { useMediaQuery } from "./lib/useMediaQuery";
import { resolveTheme, type ThemeChoice } from "./lib/theme";
import { FBD_SAMPLES } from "./constants/fbd-samples";
import { LD_SAMPLES } from "./constants/ld-samples";
import { SFC_SAMPLES } from "./constants/sfc-samples";
import { checkForUpdates, installUpdate } from "./updater";
import appIcon from "../src-tauri/icons/128x128.png";

// Local mirrors of the store's tab unions (AppState is not exported).
type BottomTab = "diagnostics" | "log" | "output" | "timing" | "histogram" | "search" | "simulation";
type RightTab = "watch" | "io" | "ai" | "target" | "online" | "refs";

const RUNTIME_TONES = {
  running: "success",
  paused: "info",
  error: "danger",
  stopped: "neutral",
} as const;


const THEME_ICONS: Record<ThemeChoice, ReactElement> = {
  system: <Monitor size={14} strokeWidth={1.75} aria-hidden="true" />,
  dark: <Moon size={14} strokeWidth={1.75} aria-hidden="true" />,
  light: <Sun size={14} strokeWidth={1.75} aria-hidden="true" />,
  petrol: <Palette size={14} strokeWidth={1.75} aria-hidden="true" />,
};

const THEME_OPTIONS: { value: ThemeChoice; label: string; hint?: string; icon: ReactElement }[] = [
  { value: "system", label: "System", hint: "Follow the OS setting", icon: THEME_ICONS.system },
  { value: "dark", label: "Dark", icon: THEME_ICONS.dark },
  { value: "light", label: "Light", icon: THEME_ICONS.light },
  { value: "petrol", label: "Petrol", hint: "Grey panels, petrol-teal accent", icon: THEME_ICONS.petrol },
];

export default function App() {
  const {
    openFiles,
    activeFileIndex,
    setActiveFile,
    closeFile,
    updateFileContent,
    compiling,
    compile,
    diagnostics,
    runtimeState,
    plcRun,
    plcStop,
    plcStep,
    debugState,
    setDebugState,
    currentExecutionLine,
    setCurrentExecutionLine,
    bottomTab,
    setBottomTab,
    log,
    lastCompileResult,
    openProjectDir,
    saveFile,
    loadSampleTemplate,
    theme,
    setTheme,
    setRightPanelTab,
    projectPath,
    saveProject,
    saveProjectAs,
    loadProject,
    sendAiErrorFix,
    sendAiTestGeneration,
    undo,
    redo,
  } = useStore();

  const [templates, setTemplates] = useState<SampleTemplate[]>([]);
  const [showTemplates, setShowTemplates] = useState(false);
  const [showNewFile, setShowNewFile] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [showLicense, setShowLicense] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [leftWidth, setLeftWidth] = useState(264);
  const [rightWidth, setRightWidth] = useState(280);
  const [bottomHeight, setBottomHeight] = useState(200);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [targetConnecting, setTargetConnecting] = useState(false);
  // The Tests overlay lives outside the store's fixed bottomTab union.
  const [testsActive, setTestsActive] = useState(false);
  const [cloudOpen, setCloudOpen] = useState(false);
  const isMobile = useMediaQuery("(max-width: 768px)");
  // Below this width the secondary toolbar actions collapse into the
  // overflow menu so the row never wraps or overflows, even with the
  // paused badge and the update button visible.
  const isCompact = useMediaQuery("(max-width: 1183px)");
  const [leftOpen, setLeftOpen] = useState(false);
  const [rightOpen, setRightOpen] = useState(false);

  const activeFile = openFiles[activeFileIndex];

  useEffect(() => {
    invoke<SampleTemplate[]>("get_sample_templates").then(setTemplates);
  }, []);

  useEffect(() => {
    const unlisten = listen<string>("deep-link", (event) => {
      const url = event.payload;
      try {
        const parsed = new URL(url);
        const params = parsed.searchParams;
        const scada = params.get("scada");
        const device = params.get("device");
        const ip = params.get("ip");
        if (scada) useStore.getState().setScadaUrl(scada);
        if (ip) useStore.getState().setTargetIp(ip);
        if (scada && ip) {
          useStore.getState().addLog("system", `Deep link: connecting to ${scada}, device ${device || ip}`);
        }
      } catch {
        // ignore malformed URLs
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  const systemPrefersLight = useMediaQuery("(prefers-color-scheme: light)");
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", resolveTheme(theme, systemPrefersLight));
  }, [theme, systemPrefersLight]);

  // Close the toolbar overflow menu on Escape or click-away.
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menuOpen]);

  const handleInstallUpdate = async () => {
    if (updating) return;
    setUpdating(true);
    try {
      await installUpdate();
    } finally {
      setUpdating(false);
    }
  };

  const handleCheckUpdates = async () => {
    const result = await checkForUpdates();
    if (result.available && result.version) {
      setUpdateVersion(result.version);
    }
  };

  const runContinue = async () => {
    try {
      const result = await invoke<{ state: string; current_line: number | null }>("continue_execution");
      if (result.current_line) setCurrentExecutionLine(result.current_line);
      if (result.state === "paused") setDebugState("paused");
      else setDebugState("stopped");
    } catch (e) { console.error("continue:", e); }
  };

  const runStepInto = async () => {
    try {
      const result = await invoke<{ state: string; current_line: number | null }>("step_instruction");
      if (result.current_line) setCurrentExecutionLine(result.current_line);
      setDebugState("paused");
    } catch (e) { console.error("step:", e); }
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Skip F-key shortcuts when a graphical editor (.fbd/.ld) is active — those keys are used by the editor
      const activeFile = openFiles[activeFileIndex];
      const isGraphicalEditor = activeFile?.name.endsWith(".fbd") || activeFile?.name.endsWith(".ld") || activeFile?.name.endsWith(".sfc");
      if (isGraphicalEditor && e.key.startsWith("F")) return;

      if (e.key === "Escape") {
        if (showNewFile) { setShowNewFile(false); return; }
        if (showTemplates) { setShowTemplates(false); return; }
        if (showLicense) { setShowLicense(false); return; }
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        saveFile(activeFileIndex);
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "b") {
        e.preventDefault();
        compile();
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "a") {
        e.preventDefault();
        setRightPanelTab("ai");
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "s") {
        e.preventDefault();
        saveProject();
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "F") {
        e.preventDefault();
        setTestsActive(false);
        setBottomTab("search");
      }
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key === "z") {
        e.preventDefault();
        undo();
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "z") {
        e.preventDefault();
        redo();
      }
      if (e.key === "F5") {
        e.preventDefault();
        invoke<{ state: string; current_line: number | null }>("continue_execution").then((r) => {
          if (r.current_line) setCurrentExecutionLine(r.current_line);
          setDebugState(r.state === "paused" ? "paused" : "stopped");
        }).catch(console.error);
      }
      if (e.key === "F11") {
        e.preventDefault();
        invoke<{ state: string; current_line: number | null }>("step_instruction").then((r) => {
          if (r.current_line) setCurrentExecutionLine(r.current_line);
          setDebugState("paused");
        }).catch(console.error);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeFileIndex, saveFile, compile, setRightPanelTab, setCurrentExecutionLine, setDebugState, saveProject, showNewFile, showTemplates, showLicense]);

  // Listen for breakpoint-hit events from Tauri
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ state: string; current_line: number | null }>("breakpoint-hit", (event) => {
      if (event.payload.current_line) {
        setCurrentExecutionLine(event.payload.current_line);
      }
      setDebugState("paused");
      // Also emit editor-goto to scroll editor to the line
      if (event.payload.current_line) {
        window.dispatchEvent(new CustomEvent("editor-goto", {
          detail: { line: event.payload.current_line },
        }));
      }
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [setCurrentExecutionLine, setDebugState]);

  const toolbarActions = (
    <>
      {isMobile && (
        <IconButton
          label="Project"
          title="Project"
          size="sm"
          onClick={() => { setLeftOpen(true); setRightOpen(false); }}
        >
          <List size={16} strokeWidth={1.75} />
        </IconButton>
      )}
      <Select<ThemeChoice>
        value={theme}
        onChange={setTheme}
        options={THEME_OPTIONS}
        sheetTitle="Theme"
        size="sm"
        fullWidth={false}
        aria-label="Theme"
        title="Theme"
        display={THEME_ICONS[theme]}
      />
      <span className="vt-divider--v" />
      <IconButton label="Open folder" title="Open project folder" size="sm" onClick={() => openProjectDir()}>
        <FolderOpen size={16} strokeWidth={1.75} />
      </IconButton>
      <IconButton label="Import" title="Import tag table, DB or SCL source" size="sm" onClick={() => setShowImport(true)}>
        <FileInput size={16} strokeWidth={1.75} />
      </IconButton>
      <IconButton label="New file" title="New file" size="sm" onClick={() => setShowNewFile(true)}>
        <FilePlus size={16} strokeWidth={1.75} />
      </IconButton>
      {!isCompact && (
        <IconButton label="Examples" title="Example programs" size="sm" onClick={() => setShowTemplates(!showTemplates)}>
          <BookOpen size={16} strokeWidth={1.75} />
        </IconButton>
      )}
      <IconButton label="Save project" title="Save project (Cmd+Shift+S)" size="sm" onClick={() => saveProject()}>
        <Save size={16} strokeWidth={1.75} />
      </IconButton>
      {!isCompact && (
        <IconButton label="Open project file" title="Open project file" size="sm" onClick={() => loadProject()}>
          <Folder size={16} strokeWidth={1.75} />
        </IconButton>
      )}
      <span className="vt-divider--v" />
      <IconButton label="Undo" title="Undo (Cmd+Z)" size="sm" onClick={undo}>
        <Undo2 size={16} strokeWidth={1.75} />
      </IconButton>
      <IconButton label="Redo" title="Redo (Cmd+Shift+Z)" size="sm" onClick={redo}>
        <Redo2 size={16} strokeWidth={1.75} />
      </IconButton>
      <span className="vt-divider--v" />
      {runtimeState === "running" ? (
        <Button size="sm" variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => plcStop()}>
          Stop
        </Button>
      ) : (
        <Button
          size="sm"
          variant="primary"
          icon={<Play size={14} strokeWidth={1.75} />}
          onClick={() => plcRun()}
          disabled={!lastCompileResult?.success}
        >
          Run
        </Button>
      )}
      <Button
        size="sm"
        variant="ghost"
        icon={<Hammer size={14} strokeWidth={1.75} />}
        onClick={() => compile()}
        disabled={compiling}
        title="Build (Cmd+B)"
      >
        {compiling ? "Compiling..." : "Build"}
      </Button>
      {!isCompact && (
        <>
          <Button
            size="sm"
            variant="ghost"
            icon={<FlaskConical size={14} strokeWidth={1.75} />}
            onClick={() => sendAiTestGeneration()}
            title="AI: generate test scenarios for current POU"
          >
            Test gen
          </Button>
          <IconButton label="Single scan" title="Run a single scan cycle" size="sm" onClick={() => plcStep()}>
            <StepForward size={16} strokeWidth={1.75} />
          </IconButton>
          <IconButton label="Continue" title="Continue (F5)" size="sm" onClick={runContinue}>
            <SkipForward size={16} strokeWidth={1.75} />
          </IconButton>
          <IconButton label="Step into" title="Step into (F11)" size="sm" onClick={runStepInto}>
            <ArrowDownToLine size={16} strokeWidth={1.75} />
          </IconButton>
        </>
      )}
      {debugState === "paused" && currentExecutionLine && (
        <Badge tone="warn">Paused at line {currentExecutionLine}</Badge>
      )}
      <span className="vt-divider--v" />
      {updateVersion && (
        <Button
          size="sm"
          variant="primary"
          loading={updating}
          disabled={updating}
          onClick={handleInstallUpdate}
          title={`Install update v${updateVersion}`}
        >
          {updating ? "Installing..." : `Update to ${updateVersion}`}
        </Button>
      )}
      <div className="tool-menu">
        {menuOpen && <div className="tool-menu__backdrop" onClick={() => setMenuOpen(false)} />}
        <IconButton
          label="More options"
          title="More options"
          size="sm"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          pressed={menuOpen}
          onClick={() => setMenuOpen(!menuOpen)}
        >
          <Ellipsis size={16} strokeWidth={1.75} />
        </IconButton>
        {menuOpen && (
          <div className="tool-menu__pop" role="menu">
            {isCompact && (
              <>
                <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); setShowTemplates(!showTemplates); }}>
                  <BookOpen size={14} strokeWidth={1.75} />
                  Examples
                </button>
                <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); loadProject(); }}>
                  <Folder size={14} strokeWidth={1.75} />
                  Open project file
                </button>
                <button className="vt-menu-item" role="menuitem" onClick={() => sendAiTestGeneration()}>
                  <FlaskConical size={14} strokeWidth={1.75} />
                  Test gen
                </button>
                <button className="vt-menu-item" role="menuitem" onClick={() => plcStep()}>
                  <StepForward size={14} strokeWidth={1.75} />
                  Single scan
                </button>
                <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); void runContinue(); }}>
                  <SkipForward size={14} strokeWidth={1.75} />
                  Continue
                </button>
                <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); void runStepInto(); }}>
                  <ArrowDownToLine size={14} strokeWidth={1.75} />
                  Step into
                </button>
                <hr className="vt-divider" style={{ margin: "4px 0" }} />
              </>
            )}
            <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); void handleCheckUpdates(); }}>
              <RefreshCw size={14} strokeWidth={1.75} />
              Check for updates
            </button>
            <button className="vt-menu-item" role="menuitem" onClick={() => { setMenuOpen(false); setShowLicense(true); }}>
              <KeyRound size={14} strokeWidth={1.75} />
              License
            </button>
          </div>
        )}
      </div>
      {isMobile && (
        <IconButton
          label="Watch and tools"
          title="Watch / I/O / AI"
          size="sm"
          onClick={() => { setRightOpen(true); setLeftOpen(false); }}
        >
          <SlidersHorizontal size={16} strokeWidth={1.75} />
        </IconButton>
      )}
    </>
  );

  return (
    <div className="app-layout">
      <Toolbar
        title="Voltrus PLC"
        icon={appIcon}
        connected={false}
        onInfo={() => setShowLicense(true)}
        context={
          <Badge tone={RUNTIME_TONES[runtimeState]}>
            <StatusDot tone={RUNTIME_TONES[runtimeState]} />
            {runtimeState.charAt(0).toUpperCase() + runtimeState.slice(1)}
          </Badge>
        }
        actions={toolbarActions}
      />

      {/* New File Modal */}
      <ImportDialog open={showImport} onClose={() => setShowImport(false)} />
      {showNewFile && (
        <div className="modal-overlay" onClick={() => setShowNewFile(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="new-file-title" onClick={(e) => e.stopPropagation()}>
            <h3 id="new-file-title">New file</h3>
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewProgram", "program");
                setShowNewFile(false);
              }}>
                <FileCode size={14} strokeWidth={1.75} /> Program
              </button>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewFunction", "function");
                setShowNewFile(false);
              }}>
                <Sigma size={14} strokeWidth={1.75} /> Function
              </button>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewFB", "function_block");
                setShowNewFile(false);
              }}>
                <Blocks size={14} strokeWidth={1.75} /> Function block
              </button>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newFbdFile("NewFBD");
                setShowNewFile(false);
              }}>
                <Workflow size={14} strokeWidth={1.75} /> FBD diagram
              </button>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newLdFile("NewLD");
                setShowNewFile(false);
              }}>
                <Network size={14} strokeWidth={1.75} /> Ladder diagram
              </button>
              <button className="vt-menu-item" onClick={() => {
                const store = useStore.getState();
                store.newSfcFile("NewSFC");
                setShowNewFile(false);
              }}>
                <Milestone size={14} strokeWidth={1.75} /> SFC diagram
              </button>
            </div>
            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 12 }}>
              <Button size="sm" variant="ghost" onClick={() => setShowNewFile(false)}>Cancel</Button>
            </div>
          </div>
        </div>
      )}

      {/* Templates Panel */}
      {showTemplates && (
        <div className="modal-overlay" onClick={() => setShowTemplates(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="templates-title" onClick={(e) => e.stopPropagation()}>
            <h3 id="templates-title">Example programs</h3>
            {templates.length > 0 && (
              <>
                {templates.map((t) => (
                  <button
                    key={t.id}
                    className="sample-item"
                    onClick={() => {
                      loadSampleTemplate(t);
                      setShowTemplates(false);
                    }}
                  >
                    <strong>{t.name}</strong>
                    <span>{t.category} · {t.description}</span>
                  </button>
                ))}
                <div className="menu-divider" />
              </>
            )}
            <div className="menu-caption">FBD diagrams</div>
            {FBD_SAMPLES.map((s) => (
              <button
                key={s.id}
                className="sample-item"
                onClick={() => {
                  useStore.getState().loadFbdSample(s);
                  setShowTemplates(false);
                }}
              >
                <strong>{s.name}</strong>
                <span>{s.category} · {s.description}</span>
              </button>
            ))}
            <div className="menu-caption">Ladder diagrams</div>
            {LD_SAMPLES.map((s) => (
              <button
                key={s.id}
                className="sample-item"
                onClick={() => {
                  useStore.getState().loadLdSample(s);
                  setShowTemplates(false);
                }}
              >
                <strong>{s.name}</strong>
                <span>{s.category} · {s.description}</span>
              </button>
            ))}
            <div className="menu-caption">SFC diagrams</div>
            {SFC_SAMPLES.map((s) => (
              <button
                key={s.id}
                className="sample-item"
                onClick={() => {
                  useStore.getState().loadSfcSample(s);
                  setShowTemplates(false);
                }}
              >
                <strong>{s.name}</strong>
                <span>{s.category} · {s.description}</span>
              </button>
            ))}
            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 12 }}>
              <Button size="sm" variant="ghost" onClick={() => setShowTemplates(false)}>Cancel</Button>
            </div>
          </div>
        </div>
      )}

      {/* License Modal */}
      {showLicense && (
        <div className="modal-overlay" onClick={() => setShowLicense(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="license-title" onClick={(e) => e.stopPropagation()} style={{ minWidth: 360, maxWidth: 420 }}>
            <h3 id="license-title">License</h3>
            <LicensePanel />
            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 12 }}>
              <Button size="sm" variant="ghost" onClick={() => setShowLicense(false)}>Close</Button>
            </div>
          </div>
        </div>
      )}

      {/* Main content: 3 columns */}
      <div className="main-content">
        {/* Left sidebar: File explorer */}
        {isMobile ? null : (
          <div className="sidebar-left" style={{ width: leftWidth }}>
            <ProjectSidebar cloudOpen={cloudOpen} setCloudOpen={setCloudOpen} />
          </div>
        )}

        {isMobile ? null : (
          <ResizeHandle
            axis="x"
            onDrag={(d) => setLeftWidth((w) => Math.min(500, Math.max(140, w + d)))}
          />
        )}

        {/* Center: Editor */}
        <div className="editor-area">
          {openFiles.length > 0 && (
            <div className="vt-tabs editor-tabs">
              {openFiles.map((file, i) => (
                <button
                  key={file.path}
                  className="vt-tab editor-tab"
                  aria-selected={i === activeFileIndex}
                  onClick={() => setActiveFile(i)}
                >
                  {file.modified && <span className="tab-dirty" />}
                  {file.name}
                  <span
                    className="tab-close"
                    role="button"
                    aria-label={`Close ${file.name}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      closeFile(i);
                    }}
                  >
                    <X size={12} strokeWidth={1.75} />
                  </span>
                </button>
              ))}
            </div>
          )}
          <div className="editor-container">
            {activeFile ? (
              <EditorRouter
                key={activeFile.path}
                filePath={activeFile.path}
                content={activeFile.content}
                onChange={(value) => updateFileContent(activeFileIndex, value)}
              />
            ) : (
              <EmptyState
                icon={<FileCode size={20} strokeWidth={1.75} />}
                title="No file open"
                description="Open a project folder or create a new program to start coding."
              />
            )}
          </div>
        </div>

        {isMobile ? null : (
          <ResizeHandle
            axis="x"
            onDrag={(d) => setRightWidth((w) => Math.min(500, Math.max(180, w - d)))}
          />
        )}

        {isMobile ? null : (
          <div className="sidebar-right" style={{ width: rightWidth }}>
            <RightSidebarBody />
          </div>
        )}
      </div>

      {/* Mobile drawers: fixed-position overlays driven by the toolbar toggles */}
      {isMobile && (
        <>
          <div
            className={`drawer-backdrop ${leftOpen || rightOpen ? "visible" : ""}`}
            onClick={() => { setLeftOpen(false); setRightOpen(false); }}
          />
          <aside className={`drawer drawer-left ${leftOpen ? "open" : ""}`} aria-hidden={!leftOpen}>
            <ProjectSidebar cloudOpen={cloudOpen} setCloudOpen={setCloudOpen} />
          </aside>
          <aside className={`drawer drawer-right ${rightOpen ? "open" : ""}`} aria-hidden={!rightOpen}>
            <RightSidebarBody />
          </aside>
        </>
      )}

      {isMobile ? null : (
        <ResizeHandle
          axis="y"
          onDrag={(d) => setBottomHeight((h) => Math.min(500, Math.max(100, h - d)))}
        />
      )}

      {/* Bottom panel */}
      <div className="bottom-panel" style={{ height: bottomHeight }}>
        <Tabs
          label="Bottom panel"
          value={testsActive ? "tests" : bottomTab}
          onChange={(v) => {
            if (v === "tests") {
              setTestsActive(true);
            } else {
              setTestsActive(false);
              setBottomTab(v as BottomTab);
            }
          }}
          items={[
            { value: "diagnostics", label: "Diagnostics", count: diagnostics.length || undefined },
            { value: "tests", label: "Tests" },
            { value: "log", label: "Runtime log" },
            { value: "output", label: "Output" },
            { value: "timing", label: "Timing" },
            { value: "histogram", label: "Histogram" },
            { value: "search", label: "Search" },
            { value: "simulation", label: "Simulation" },
          ]}
        />
        <div className="bottom-content">
          {testsActive && <TestsPanel />}
          {!testsActive && bottomTab === "diagnostics" && (
            diagnostics.length === 0 ? (
              <EmptyState
                compact
                icon={<CircleCheck size={18} strokeWidth={1.75} />}
                title="No diagnostics"
                description="Build the program to check it for errors."
              />
            ) : (
              diagnostics.map((d, i) => (
                <div key={i} className="diagnostic-item">
                  <span
                    className={`severity ${d.severity}`}
                    style={d.line > 0 ? { cursor: "pointer" } : undefined}
                    onClick={() => {
                      if (d.line > 0) {
                        window.dispatchEvent(new CustomEvent("editor-goto", { detail: { line: d.line } }));
                      }
                    }}
                  >
                    {d.severity === "error" ? (
                      <CircleAlert size={12} strokeWidth={1.75} />
                    ) : d.severity === "warning" ? (
                      <TriangleAlert size={12} strokeWidth={1.75} />
                    ) : (
                      <Info size={12} strokeWidth={1.75} />
                    )}
                  </span>
                  <span className="location">{d.line}:{d.col}</span>
                  <span style={{ flex: 1, minWidth: 0 }}>{d.message}</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<Sparkles size={12} strokeWidth={1.75} />}
                    onClick={() => sendAiErrorFix(d)}
                    title="Ask AI to fix this error"
                  >
                    Fix
                  </Button>
                </div>
              ))
            )
          )}
          {!testsActive && bottomTab === "log" && (
            <div style={{ display: "flex", flexDirection: "column" }}>
              {log.map((entry, i) => (
                <div key={i} className="log-line">
                  <span className="log-line__dir">[{entry.direction}]</span>
                  <span className="log-line__msg">{entry.message}</span>
                </div>
              ))}
              {log.length === 0 && (
                <EmptyState
                  compact
                  icon={<ScrollText size={18} strokeWidth={1.75} />}
                  title="No log entries"
                  description="Runtime messages appear here once the program runs."
                />
              )}
            </div>
          )}
          {bottomTab === "output" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {lastCompileResult ? (
                <div className={`output-line ${lastCompileResult.success ? "output-line--ok" : "output-line--err"}`}>
                  {lastCompileResult.success
                    ? `Build successful: ${lastCompileResult.pou_count} POU, ${lastCompileResult.variable_count} variables`
                    : `Build failed: ${lastCompileResult.diagnostics.filter(d => d.severity === "error").length} errors`}
                </div>
              ) : (
                <EmptyState
                  compact
                  icon={<Hammer size={18} strokeWidth={1.75} />}
                  title="No build yet"
                  description="Build the program to see compiler output here."
                />
              )}
              {runtimeState === "error" && (
                <div className="output-line output-line--err">Runtime error. Check the Runtime log tab for details.</div>
              )}
              {runtimeState === "running" && (
                <div className="output-line output-line--ok">Runtime is running. Watch variables update in the watch table.</div>
              )}
              {runtimeState === "stopped" && lastCompileResult?.success && (
                <div className="output-line output-line--muted">Ready. Press Run to start the runtime.</div>
              )}
            </div>
          )}
          {bottomTab === "timing" && <CycleGraph />}
          {bottomTab === "histogram" && <Histogram />}
          {bottomTab === "search" && <SearchPanel />}
          {bottomTab === "simulation" && <SimulationPanel />}
        </div>
      </div>

      {/* Cloud auth modal — auto-shows on any 401 from a cloud action */}
      <AuthDialog />
      {/* Web build: full-page sign-in/register until authenticated or skipped */}
      <AuthScreen />
      {/* Paywall: raised by 402 plan_required or from the plan chip */}
      <UpgradeScreen />
    </div>
  );
}

// File-type icon for tree rows.
function FileIcon({ name }: { name: string }) {
  const ext = name.split(".").pop()?.toLowerCase();
  const size = 14;
  const common = { size, strokeWidth: 1.75 };
  if (ext === "fbd") return <Workflow {...common} />;
  if (ext === "ld") return <Network {...common} />;
  if (ext === "sfc") return <Milestone {...common} />;
  if (ext === "txt") return <FileText {...common} />;
  return <FileCode {...common} />;
}

// Left rail: project files + collapsible Cloud section. Rendered inside the
// desktop docked sidebar and the mobile drawer — identical content, host
// decides placement.
function ProjectSidebar({ cloudOpen, setCloudOpen }: { cloudOpen: boolean; setCloudOpen: (v: boolean) => void }) {
  const projectDir = useStore((s) => s.projectDir);
  const projectFiles = useStore((s) => s.projectFiles);
  const openFiles = useStore((s) => s.openFiles);
  const activeFileIndex = useStore((s) => s.activeFileIndex);
  const openFile = useStore((s) => s.openFile);
  const setActiveFile = useStore((s) => s.setActiveFile);
  const refreshProjectFiles = useStore((s) => s.refreshProjectFiles);

  const stFiles = projectFiles.filter(
    (f) => !f.is_dir && (f.name.endsWith(".st") || f.name.endsWith(".txt") || f.name.endsWith(".fbd") || f.name.endsWith(".ld") || f.name.endsWith(".sfc"))
  );

  return (
    <>
      <section className="vt-section">
        <div className="vt-section__header">
          <span className="vt-section__title">Project</span>
          {projectDir && (
            <IconButton label="Refresh project files" title="Refresh" size="sm" onClick={() => refreshProjectFiles()}>
              <RefreshCw size={14} strokeWidth={1.75} />
            </IconButton>
          )}
        </div>
        {projectDir ? (
          stFiles.length === 0 ? (
            <EmptyState
              compact
              icon={<FolderOpen size={18} strokeWidth={1.75} />}
              title="No program files"
              description="No .st, .fbd, .ld or .sfc files in this folder yet."
            />
          ) : (
            <ul className="file-tree">
              {stFiles.map((f) => (
                <li
                  key={f.path}
                  className={`file-row ${openFiles[activeFileIndex]?.path === f.path ? "is-active" : ""}`}
                  onClick={() => openFile(f.path)}
                >
                  <FileIcon name={f.name} />
                  <span className="file-row__name">{f.name}</span>
                </li>
              ))}
            </ul>
          )
        ) : openFiles.length === 0 ? (
          <EmptyState
            compact
            icon={<FolderOpen size={18} strokeWidth={1.75} />}
            title="No project open"
            description="Open a project folder to browse its program files."
          />
        ) : (
          <ul className="file-tree">
            {openFiles.map((file, i) => (
              <li
                key={file.path}
                className={`file-row ${i === activeFileIndex ? "is-active" : ""}`}
                onClick={() => setActiveFile(i)}
              >
                <FileIcon name={file.name} />
                <span className="file-row__name">{file.name}</span>
                {file.modified && <span className="tab-dirty" title="Unsaved changes" />}
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* Cloud section (PLC-20): collapsible; local project flow above is untouched */}
      <section className="vt-section sidebar-cloud">
        <button
          className="cloud-toggle"
          aria-expanded={cloudOpen}
          onClick={() => setCloudOpen(!cloudOpen)}
        >
          <span className="vt-section__title">Cloud</span>
          {cloudOpen ? <ChevronDown size={14} strokeWidth={1.75} /> : <ChevronRight size={14} strokeWidth={1.75} />}
        </button>
        {cloudOpen && (
          <div style={{ overflow: "auto", flexShrink: 0, maxHeight: "60%", display: "flex", flexDirection: "column", gap: 8, paddingTop: 10 }}>
            <AccountBadge />
            <CloudProjectPanel />
            <AiKeysPanel />
            <ProvisionPanel />
          </div>
        )}
      </section>
    </>
  );
}

// Right rail: Watch / I/O / Target / AI / Refs tab strip and panels.
function RightSidebarBody() {
  const rightPanelTab = useStore((s) => s.rightPanelTab);
  const setRightPanelTab = useStore((s) => s.setRightPanelTab);

  return (
    <>
      <Tabs
        label="Inspector panels"
        value={rightPanelTab}
        onChange={(t) => setRightPanelTab(t as RightTab)}
        items={[
          { value: "watch", label: "Watch" },
          { value: "io", label: "I/O" },
          { value: "target", label: "Target" },
          { value: "online", label: "Online" },
          { value: "ai", label: "AI" },
          { value: "refs", label: "Refs" },
        ]}
      />
      {rightPanelTab === "watch" && (
        <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">Runtime</span>
            </div>
            <RuntimeControls />
          </section>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">Watch table</span>
            </div>
            <WatchTable />
          </section>
        </div>
      )}
      {rightPanelTab === "io" && (
        <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">I/O driver</span>
            </div>
            <IoPanel />
          </section>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">I/O mappings</span>
            </div>
            <IoMappingPanel />
          </section>
        </div>
      )}
      {rightPanelTab === "target" && (
        <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">Target device</span>
            </div>
            <TargetPanel />
          </section>
        </div>
      )}
      {rightPanelTab === "online" && (
        <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
          <OnlinePanel />
        </div>
      )}
      {rightPanelTab === "ai" && (
        <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden", minHeight: 0 }}>
          <AiChatPanel />
        </div>
      )}
      {rightPanelTab === "refs" && (
        <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">Cross-reference</span>
            </div>
            <XrefPanel />
          </section>
        </div>
      )}
    </>
  );
}

function SearchPanel() {
  const searchQuery = useStore((s) => s.searchQuery);
  const setSearchQuery = useStore((s) => s.setSearchQuery);
  const searchUseRegex = useStore((s) => s.searchUseRegex);
  const setSearchUseRegex = useStore((s) => s.setSearchUseRegex);
  const searchResults = useStore((s) => s.searchResults);
  const searchAllFiles = useStore((s) => s.searchAllFiles);
  const setActiveFile = useStore((s) => s.setActiveFile);
  const openFiles = useStore((s) => s.openFiles);

  const handleSearch = () => {
    searchAllFiles();
  };

  const gotoResult = (file: string, line: number) => {
    const idx = openFiles.findIndex((f) => f.name === file);
    if (idx >= 0) {
      setActiveFile(idx);
      setTimeout(() => {
        window.dispatchEvent(new CustomEvent("editor-goto", { detail: { line } }));
      }, 50);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", gap: 8 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <input
          className="vt-input"
          style={{ flex: 1 }}
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") handleSearch(); }}
          placeholder="Search in project"
          aria-label="Search in project"
        />
        <label className="vt-check">
          <input
            type="checkbox"
            checked={searchUseRegex}
            onChange={(e) => setSearchUseRegex(e.target.checked)}
          />
          Regex
        </label>
        <Button size="sm" variant="default" icon={<Search size={14} strokeWidth={1.75} />} onClick={handleSearch}>
          Search
        </Button>
      </div>
      {searchResults.length === 0 ? (
        <EmptyState
          compact
          icon={<Search size={18} strokeWidth={1.75} />}
          title={searchQuery ? "No results" : "Search the project"}
          description={searchQuery ? "No matches for this query." : "Type a query and press Enter to search every open project file."}
        />
      ) : (
        <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
          {searchResults.map((r, i) => (
            <div
              key={i}
              className="diagnostic-item"
              onClick={() => gotoResult(r.file, r.line)}
            >
              <span className="location">{r.file}:{r.line}:{r.col}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontSize: "var(--fs-xs)" }}>
                {r.text}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function XrefPanel() {
  const xrefVariable = useStore((s) => s.xrefVariable);
  const setXrefVariable = useStore((s) => s.setXrefVariable);
  const xrefResults = useStore((s) => s.xrefResults);
  const findReferences = useStore((s) => s.findReferences);
  const compiledVariables = useStore((s) => s.compiledVariables);
  const setActiveFile = useStore((s) => s.setActiveFile);
  const openFiles = useStore((s) => s.openFiles);

  const gotoRef = (file: string, line: number) => {
    const idx = openFiles.findIndex((f) => f.name === file);
    if (idx >= 0) {
      setActiveFile(idx);
      setTimeout(() => {
        window.dispatchEvent(new CustomEvent("editor-goto", { detail: { line } }));
      }, 50);
    }
  };

  const kindTone: Record<string, "accent" | "info" | "neutral"> = {
    declare: "accent",
    write: "info",
    read: "neutral",
  };
  const kindLabel: Record<string, string> = {
    declare: "DEC",
    write: "W",
    read: "R",
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", gap: 6 }}>
        <select
          className="vt-select"
          value={xrefVariable}
          onChange={(e) => setXrefVariable(e.target.value)}
          aria-label="Variable"
        >
          <option value="">Select variable</option>
          {compiledVariables.map((v) => (
            <option key={v.name} value={v.name}>{v.name}</option>
          ))}
        </select>
        <Button size="sm" variant="default" icon={<TextSearch size={14} strokeWidth={1.75} />} onClick={findReferences}>
          Find
        </Button>
      </div>
      {xrefResults.length === 0 ? (
        <EmptyState
          compact
          icon={<TextSearch size={18} strokeWidth={1.75} />}
          title={xrefVariable ? "No references found" : "No variable selected"}
          description={xrefVariable ? "This variable is not referenced in any program." : "Select a variable to find every place it is declared, read or written."}
        />
      ) : (
        <div>
          {xrefResults.map((r, i) => (
            <div
              key={i}
              className="diagnostic-item"
              onClick={() => gotoRef(r.file, r.line)}
            >
              <Badge tone={kindTone[r.kind] ?? "neutral"}>{kindLabel[r.kind] ?? "?"}</Badge>
              <span className="location">{r.file}:{r.line}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                {r.context}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
