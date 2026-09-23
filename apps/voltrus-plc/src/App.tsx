import { useState, useEffect } from "react";
import { useStore } from "./store";
import { invoke, listen } from "./api/bridge";
import EditorRouter from "./components/Editor/EditorRouter";
import WatchTable from "./components/Sidebar/WatchTable";
import RuntimeControls from "./components/Sidebar/RuntimeControls";
import IoPanel from "./components/Sidebar/IoPanel";
import IoMappingPanel from "./components/Sidebar/IoMappingPanel";
import TargetPanel from "./components/Sidebar/TargetPanel";
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
import { useMediaQuery } from "./lib/useMediaQuery";
import type { SampleTemplate } from "./types";
import { FBD_SAMPLES } from "./constants/fbd-samples";
import { LD_SAMPLES } from "./constants/ld-samples";
import { SFC_SAMPLES } from "./constants/sfc-samples";
import { checkForUpdates, installUpdate } from "./updater";

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
  const [showLicense, setShowLicense] = useState(false);
  const [leftWidth, setLeftWidth] = useState(220);
  const [rightWidth, setRightWidth] = useState(280);
  const [bottomHeight, setBottomHeight] = useState(200);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [targetConnecting, setTargetConnecting] = useState(false);
  // The Tests overlay lives outside the store's fixed bottomTab union.
  const [testsActive, setTestsActive] = useState(false);
  const [cloudOpen, setCloudOpen] = useState(false);
  const isMobile = useMediaQuery("(max-width: 768px)");
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

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

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
        invoke("continue_execution").then((r: any) => {
          if (r.current_line) setCurrentExecutionLine(r.current_line);
          setDebugState(r.state === "paused" ? "paused" : "stopped");
        }).catch(console.error);
      }
      if (e.key === "F11") {
        e.preventDefault();
        invoke("step_instruction").then((r: any) => {
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

  return (
    <div className="app-layout">
      {/* Toolbar */}
      <div className="toolbar">
        {isMobile && (
          <button
            className="toolbar-btn"
            onClick={() => { setLeftOpen(true); setRightOpen(false); }}
            aria-label="Open project panel"
            title="Project"
          >
            ☰
          </button>
        )}
        <span className="toolbar-title">Voltrus PLC</span>
        <button
          className="toolbar-btn theme-toggle"
          onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          title="Toggle theme"
          aria-label="Toggle theme"
        >
          {theme === "dark" ? "\u2600" : "\u263D"}
        </button>
        <button className="toolbar-btn" onClick={() => openProjectDir()}>
          Open Folder
        </button>
        <button className="toolbar-btn" onClick={() => setShowNewFile(true)}>
          New File
        </button>
        <button className="toolbar-btn" onClick={() => setShowTemplates(!showTemplates)}>
          Examples
        </button>
        <button className="toolbar-btn" onClick={() => saveProject()} title="Save Project (Cmd+Shift+S)">
          Save
        </button>
        <button className="toolbar-btn" onClick={() => loadProject()} title="Open Project">
          Open
        </button>
        <span className="toolbar-divider" />
        <button className="toolbar-btn" onClick={undo} title="Undo (Cmd+Z)">↩</button>
        <button className="toolbar-btn" onClick={redo} title="Redo (Cmd+Shift+Z)">↪</button>
        <span className="toolbar-divider" />
        <button
          className="toolbar-btn primary"
          onClick={() => compile()}
          disabled={compiling}
        >
          {compiling ? "Compiling..." : "Build"}
        </button>
        <button
          className="toolbar-btn"
          onClick={() => sendAiTestGeneration()}
          title="AI: Generate test scenarios for current POU"
        >
          Test Gen
        </button>
        {runtimeState === "running" ? (
          <button className="toolbar-btn danger" onClick={() => plcStop()}>
            Stop
          </button>
        ) : (
          <button
            className="toolbar-btn primary"
            onClick={() => plcRun()}
            disabled={!lastCompileResult?.success}
          >
            Run
          </button>
        )}
        <button className="toolbar-btn" onClick={() => plcStep()}>
          Single Scan
        </button>
        <span className="toolbar-divider" />
        <button
          className="toolbar-btn"
          onClick={async () => {
            try {
              const result = await invoke<{ state: string; current_line: number | null }>("continue_execution");
              if (result.current_line) setCurrentExecutionLine(result.current_line);
              if (result.state === "paused") setDebugState("paused");
              else setDebugState("stopped");
            } catch (e) { console.error("continue:", e); }
          }}
          title="Continue (F5)"
        >
          Continue
        </button>
        <button
          className="toolbar-btn"
          onClick={async () => {
            try {
              const result = await invoke<{ state: string; current_line: number | null }>("step_instruction");
              if (result.current_line) setCurrentExecutionLine(result.current_line);
              setDebugState("paused");
            } catch (e) { console.error("step:", e); }
          }}
          title="Step Into (F11)"
        >
          Step Into
        </button>
        {debugState === "paused" && currentExecutionLine && (
          <span className="debug-info">Paused at line {currentExecutionLine}</span>
        )}
        <span className="toolbar-spacer" />
        {isMobile && (
          <button
            className="toolbar-btn"
            onClick={() => { setRightOpen(true); setLeftOpen(false); }}
            aria-label="Open watch and tools panel"
            title="Watch / I/O / AI"
          >
            ⚙
          </button>
        )}
        {updateVersion && (
          <button
            className="toolbar-btn"
            style={{ fontSize: 10, color: "var(--amber)" }}
            onClick={async () => {
              setUpdating(true);
              await installUpdate();
              setUpdating(false);
            }}
            disabled={updating}
          >
            {updating ? "Installing..." : `Update to ${updateVersion}`}
          </button>
        )}
        <button
          className="toolbar-btn"
          style={{ fontSize: 10 }}
          onClick={async () => {
            const result = await checkForUpdates();
            if (result.available && result.version) {
              setUpdateVersion(result.version);
            }
          }}
          title="Check for updates"
        >
          Check Updates
        </button>
        <button
          className="toolbar-btn"
          style={{ fontSize: 10 }}
          onClick={() => setShowLicense(true)}
          title="License"
        >
          License
        </button>
        <div className="runtime-indicator">
          <span className={`runtime-led ${runtimeState}`} />
          <span>{runtimeState.toUpperCase()}</span>
        </div>
      </div>

      {/* New File Modal */}
      {showNewFile && (
        <div className="modal-overlay" onClick={() => setShowNewFile(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="new-file-title" onClick={(e) => e.stopPropagation()}>
            <h3 id="new-file-title">New File</h3>
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewProgram", "program");
                setShowNewFile(false);
              }}>
                PROGRAM
              </button>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewFunction", "function");
                setShowNewFile(false);
              }}>
                FUNCTION
              </button>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newFile("NewFB", "function_block");
                setShowNewFile(false);
              }}>
                FUNCTION_BLOCK
              </button>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newFbdFile("NewFBD");
                setShowNewFile(false);
              }}>
                FBD DIAGRAM
              </button>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newLdFile("NewLD");
                setShowNewFile(false);
              }}>
                LADDER DIAGRAM
              </button>
              <button className="toolbar-btn" onClick={() => {
                const store = useStore.getState();
                store.newSfcFile("NewSFC");
                setShowNewFile(false);
              }}>
                SFC DIAGRAM
              </button>
            </div>
            <button className="toolbar-btn" style={{ marginTop: 12 }} onClick={() => setShowNewFile(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Templates Panel */}
      {showTemplates && (
        <div className="modal-overlay" onClick={() => setShowTemplates(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="templates-title" onClick={(e) => e.stopPropagation()}>
            <h3 id="templates-title">Example Programs</h3>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {templates.map((t) => (
                <button
                  key={t.id}
                  className="toolbar-btn"
                  style={{ textAlign: "left", display: "block" }}
                  onClick={() => {
                    loadSampleTemplate(t);
                    setShowTemplates(false);
                  }}
                >
                  <strong>{t.name}</strong>
                  <span style={{ color: "var(--text-muted)", marginLeft: 8, fontSize: 11 }}>
                    {t.category} — {t.description}
                  </span>
                </button>
              ))}
              <div style={{ borderTop: "1px solid var(--border, #45475a)", margin: "6px 0" }} />
              <div style={{ fontSize: 9, color: "var(--text-muted, #6c7086)", fontWeight: 600, marginBottom: 2 }}>
                FBD DIAGRAMS
              </div>
              {FBD_SAMPLES.map((s) => (
                <button
                  key={s.id}
                  className="toolbar-btn"
                  style={{ textAlign: "left", display: "block" }}
                  onClick={() => {
                    useStore.getState().loadFbdSample(s);
                    setShowTemplates(false);
                  }}
                >
                  <strong style={{ color: "var(--accent, #7c3aed)" }}>{s.name}</strong>
                  <span style={{ color: "var(--text-muted)", marginLeft: 8, fontSize: 11 }}>
                    {s.category} — {s.description}
                  </span>
                </button>
              ))}
              <div style={{ borderTop: "1px solid var(--border, #45475a)", margin: "6px 0" }} />
              <div style={{ fontSize: 9, color: "var(--text-muted, #6c7086)", fontWeight: 600, marginBottom: 2 }}>
                LADDER DIAGRAMS
              </div>
              {LD_SAMPLES.map((s) => (
                <button
                  key={s.id}
                  className="toolbar-btn"
                  style={{ textAlign: "left", display: "block" }}
                  onClick={() => {
                    useStore.getState().loadLdSample(s);
                    setShowTemplates(false);
                  }}
                >
                  <strong style={{ color: "#10b981" }}>{s.name}</strong>
                  <span style={{ color: "var(--text-muted)", marginLeft: 8, fontSize: 11 }}>
                    {s.category} — {s.description}
                  </span>
                </button>
              ))}
              <div style={{ borderTop: "1px solid var(--border, #45475a)", margin: "6px 0" }} />
              <div style={{ fontSize: 9, color: "var(--text-muted, #6c7086)", fontWeight: 600, marginBottom: 2 }}>
                SFC DIAGRAMS
              </div>
              {SFC_SAMPLES.map((s) => (
                <button
                  key={s.id}
                  className="toolbar-btn"
                  style={{ textAlign: "left", display: "block" }}
                  onClick={() => {
                    useStore.getState().loadSfcSample(s);
                    setShowTemplates(false);
                  }}
                >
                  <strong style={{ color: "#a855f7" }}>{s.name}</strong>
                  <span style={{ color: "var(--text-muted)", marginLeft: 8, fontSize: 11 }}>
                    {s.category} — {s.description}
                  </span>
                </button>
              ))}
            </div>
            <button className="toolbar-btn" style={{ marginTop: 12 }} onClick={() => setShowTemplates(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* License Modal */}
      {showLicense && (
        <div className="modal-overlay" onClick={() => setShowLicense(false)}>
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="license-title" onClick={(e) => e.stopPropagation()} style={{ minWidth: 360, maxWidth: 420 }}>
            <h3 id="license-title">License</h3>
            <LicensePanel />
            <button className="toolbar-btn" style={{ marginTop: 12 }} onClick={() => setShowLicense(false)}>
              Close
            </button>
          </div>
        </div>
      )}

      {/* Main content: 3 columns */}
      <div className="main-content">
        {/* Left sidebar: File explorer */}
        {isMobile ? null : (
          <div className="sidebar-left" style={{ width: leftWidth }}>
            <ProjectSidebar />
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
          <div className="editor-tabs">
            {openFiles.map((file, i) => (
              <button
                key={file.path}
                className={`editor-tab ${i === activeFileIndex ? "active" : ""}`}
                onClick={() => setActiveFile(i)}
              >
                {file.modified && <span className="dot" />}
                {file.name}
                <span
                  className="close-btn"
                  role="button"
                  aria-label={`Close ${file.name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    closeFile(i);
                  }}
                >
                  x
                </span>
              </button>
            ))}
          </div>
          <div className="editor-container">
            {activeFile ? (
              <EditorRouter
                key={activeFile.path}
                filePath={activeFile.path}
                content={activeFile.content}
                onChange={(value) => updateFileContent(activeFileIndex, value)}
              />
            ) : (
              <div className="placeholder-editor">
                Open or create a file to start coding
              </div>
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
            <ProjectSidebar />
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
        <div className="bottom-tabs">
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "diagnostics" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("diagnostics"); }}
          >
            Diagnostics
            {diagnostics.length > 0 && (
              <span
                style={{
                  marginLeft: 4,
                  color: diagnostics.some((d) => d.severity === "error")
                    ? "var(--danger)"
                    : "#f59e0b",
                }}
              >
                ({diagnostics.length})
              </span>
            )}
          </button>
          <button
            className={`bottom-tab ${testsActive ? "active" : ""}`}
            onClick={() => setTestsActive(true)}
          >
            Tests
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "log" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("log"); }}
          >
            Runtime Log
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "output" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("output"); }}
          >
            Output
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "timing" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("timing"); }}
          >
            Timing
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "histogram" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("histogram"); }}
          >
            Histogram
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "search" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("search"); }}
          >
            Search
          </button>
          <button
            className={`bottom-tab ${!testsActive && bottomTab === "simulation" ? "active" : ""}`}
            onClick={() => { setTestsActive(false); setBottomTab("simulation"); }}
          >
            Simulation
          </button>
        </div>
        <div className="bottom-content">
          {testsActive && <TestsPanel />}
          {!testsActive && bottomTab === "diagnostics" && (
            <>
              {diagnostics.length === 0 ? (
                <span style={{ color: "var(--text-muted)" }}>No diagnostics</span>
              ) : (
                diagnostics.map((d, i) => (
                  <div
                    key={i}
                    className="diagnostic-item"
                    style={{ display: "flex", alignItems: "center", gap: 6 }}
                  >
                    <span
                      className={`severity ${d.severity}`}
                      style={d.line > 0 ? { cursor: "pointer" } : undefined}
                      onClick={() => {
                        if (d.line > 0) {
                          window.dispatchEvent(new CustomEvent("editor-goto", { detail: { line: d.line } }));
                        }
                      }}
                    >
                      {d.severity === "error" ? "!" : d.severity === "warning" ? "?" : "i"}
                    </span>
                    <span className="location">{d.line}:{d.col}</span>
                    <span style={{ flex: 1 }}>{d.message}</span>
                    <button
                      className="toolbar-btn"
                      style={{ fontSize: 10, padding: "1px 6px", color: "var(--accent)" }}
                      onClick={() => sendAiErrorFix(d)}
                      title="Ask AI to fix this error"
                    >
                      Fix
                    </button>
                  </div>
                ))
              )}
            </>
          )}
          {bottomTab === "log" && (
            <>
              {log.map((entry, i) => (
                <div key={i} style={{ color: "var(--text-muted)" }}>
                  <span style={{ color: "var(--accent)", marginRight: 8 }}>
                    [{entry.direction}]
                  </span>
                  {entry.message}
                </div>
              ))}
              {log.length === 0 && (
                <span style={{ color: "var(--text-muted)" }}>No log entries</span>
              )}
            </>
          )}
          {bottomTab === "output" && (
            <div>
              {lastCompileResult ? (
                <div style={{ color: lastCompileResult.success ? "var(--accent)" : "var(--danger)", marginBottom: 8 }}>
                  {lastCompileResult.success
                    ? `Build successful: ${lastCompileResult.pou_count} POU, ${lastCompileResult.variable_count} variables`
                    : `Build failed: ${lastCompileResult.diagnostics.filter(d => d.severity === "error").length} errors`}
                </div>
              ) : (
                <div style={{ color: "var(--text-muted)" }}>Build output will appear here</div>
              )}
              {runtimeState === "error" && (
                <div style={{ color: "var(--danger)", marginTop: 8 }}>
                  Runtime error — check the Runtime Log tab for details
                </div>
              )}
              {runtimeState === "running" && (
                <div style={{ color: "var(--accent)", marginTop: 8 }}>
                  Runtime is running. Watch variables update in the Watch Table.
                </div>
              )}
              {runtimeState === "stopped" && lastCompileResult?.success && (
                <div style={{ color: "var(--text-muted)", marginTop: 8 }}>
                  Ready. Press Run to start the softPLC runtime.
                </div>
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

// Left rail: project files + collapsible Cloud section. Rendered inside the
// desktop docked sidebar and the mobile drawer — identical content, host
// decides placement.
function ProjectSidebar() {
  const projectDir = useStore((s) => s.projectDir);
  const projectFiles = useStore((s) => s.projectFiles);
  const openFiles = useStore((s) => s.openFiles);
  const activeFileIndex = useStore((s) => s.activeFileIndex);
  const openFile = useStore((s) => s.openFile);
  const setActiveFile = useStore((s) => s.setActiveFile);
  const refreshProjectFiles = useStore((s) => s.refreshProjectFiles);
  const [cloudOpen, setCloudOpen] = useState(false);

  const stFiles = projectFiles.filter(
    (f) => !f.is_dir && (f.name.endsWith(".st") || f.name.endsWith(".txt") || f.name.endsWith(".fbd") || f.name.endsWith(".ld") || f.name.endsWith(".sfc"))
  );

  return (
    <>
      <div className="sidebar-header">
        Project
        {projectDir && (
          <button
            style={{
              marginLeft: "auto", fontSize: 10, background: "none",
              border: "none", color: "var(--accent)", cursor: "pointer",
            }}
            onClick={() => refreshProjectFiles()}
          >
            Refresh
          </button>
        )}
      </div>
      {projectDir ? (
        <ul className="file-list">
          {stFiles.length === 0 ? (
            <li className="file-item" style={{ color: "var(--text-muted)" }}>
              No program files found
            </li>
          ) : (
            stFiles.map((f) => (
              <li
                key={f.path}
                className={`file-item ${openFiles[activeFileIndex]?.path === f.path ? "active" : ""}`}
                onClick={() => openFile(f.path)}
              >
                <span style={{ fontSize: 10 }}>
                  {(f.name.split(".").pop() || "st").toUpperCase().slice(0, 3)}
                </span>
                <span>{f.name}</span>
              </li>
            ))
          )}
        </ul>
      ) : (
        <ul className="file-list">
          {openFiles.map((file, i) => (
            <li
              key={file.path}
              className={`file-item ${i === activeFileIndex ? "active" : ""}`}
              onClick={() => setActiveFile(i)}
            >
              <span className="icon">
                {file.modified ? <svg width="10" height="10" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="var(--accent)" /></svg> : <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"><path d="M4 1h5l4 4v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1z" /><polyline points="9 1 9 5 13 5" /></svg>}
              </span>
              <span>{file.name}</span>
            </li>
          ))}
        </ul>
      )}

      {/* Cloud section (PLC-20): collapsible; local project flow above is untouched */}
      <div
        className="sidebar-header"
        role="button"
        aria-expanded={cloudOpen}
        style={{ cursor: "pointer", display: "flex", alignItems: "center", marginTop: "auto" }}
        onClick={() => setCloudOpen(!cloudOpen)}
      >
        Cloud
        <span style={{ marginLeft: "auto", fontSize: 9 }}>{cloudOpen ? "\u25BE" : "\u25B8"}</span>
      </div>
      {cloudOpen && (
        <div style={{ overflow: "auto", flexShrink: 0, maxHeight: "60%" }}>
          <AccountBadge />
          <CloudProjectPanel />
          <AiKeysPanel />
          <ProvisionPanel />
        </div>
      )}
    </>
  );
}

// Right rail: Watch / I/O / Target / AI / Refs tab strip and panels.
function RightSidebarBody() {
  const rightPanelTab = useStore((s) => s.rightPanelTab);
  const setRightPanelTab = useStore((s) => s.setRightPanelTab);

  return (
    <>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)" }}>
        {(["watch", "io", "target", "ai", "refs"] as const).map((tab) => (
          <button
            key={tab}
            className={`bottom-tab ${rightPanelTab === tab ? "active" : ""}`}
            style={{ flex: 1, justifyContent: "center" }}
            onClick={() => setRightPanelTab(tab)}
          >
            {tab === "watch" ? "Watch" : tab === "io" ? "I/O" : tab === "target" ? "Target" : tab === "ai" ? "AI" : "Refs"}
          </button>
        ))}
      </div>
      {rightPanelTab === "watch" && (
        <div style={{ flex: 1, overflow: "auto" }}>
          <div className="sidebar-header">Runtime</div>
          <RuntimeControls />
          <div className="sidebar-header">Watch Table</div>
          <WatchTable />
        </div>
      )}
      {rightPanelTab === "io" && (
        <div style={{ flex: 1, overflow: "auto" }}>
          <div className="sidebar-header">I/O Driver</div>
          <IoPanel />
          <div className="sidebar-header">I/O Mappings</div>
          <IoMappingPanel />
        </div>
      )}
      {rightPanelTab === "target" && (
        <div style={{ flex: 1, overflow: "auto" }}>
          <div className="sidebar-header">Target Device</div>
          <TargetPanel />
        </div>
      )}
      {rightPanelTab === "ai" && (
        <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
          <AiChatPanel />
        </div>
      )}
      {rightPanelTab === "refs" && (
        <div style={{ flex: 1, overflow: "auto" }}>
          <div className="sidebar-header">Cross-Reference</div>
          <XrefPanel />
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
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", gap: 6, marginBottom: 8, alignItems: "center" }}>
        <input
          className="scan-cycle-input"
          style={{ flex: 1, textAlign: "left", width: "auto" }}
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") handleSearch(); }}
          placeholder="Search in project..."
        />
        <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11, color: "var(--text-muted)", cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={searchUseRegex}
            onChange={(e) => setSearchUseRegex(e.target.checked)}
          />
          Regex
        </label>
        <button className="toolbar-btn" style={{ fontSize: 10, padding: "2px 8px" }} onClick={handleSearch}>
          Search
        </button>
      </div>
      {searchResults.length === 0 ? (
        <span style={{ color: "var(--text-muted)" }}>
          {searchQuery ? "No results" : "Type a query and press Enter or Search"}
        </span>
      ) : (
        <div style={{ flex: 1, overflow: "auto" }}>
          {searchResults.map((r, i) => (
            <div
              key={i}
              className="diagnostic-item"
              style={{ cursor: "pointer", padding: "2px 0" }}
              onClick={() => gotoResult(r.file, r.line)}
            >
              <span style={{ color: "var(--accent)", whiteSpace: "nowrap", fontSize: 11 }}>{r.file}:{r.line}:{r.col}</span>
              <span style={{ color: "var(--text-muted)", fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
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

  const kindLabel: Record<string, { label: string; color: string }> = {
    declare: { label: "DEC", color: "#a855f7" },
    write: { label: "W", color: "#3b82f6" },
    read: { label: "R", color: "#10b981" },
  };

  return (
    <div style={{ padding: "0 12px", fontSize: 12 }}>
      <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
        <select
          value={xrefVariable}
          onChange={(e) => setXrefVariable(e.target.value)}
          style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
        >
          <option value="">Select variable...</option>
          {compiledVariables.map((v) => (
            <option key={v.name} value={v.name}>{v.name}</option>
          ))}
        </select>
        <button className="toolbar-btn" style={{ fontSize: 10, padding: "2px 8px" }} onClick={findReferences}>
          Find
        </button>
      </div>
      {xrefResults.length === 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 11 }}>
          {xrefVariable ? "No references found" : "Select a variable to find all references"}
        </div>
      ) : (
        <div>
          {xrefResults.map((r, i) => (
            <div
              key={i}
              style={{ display: "flex", gap: 4, padding: "3px 0", cursor: "pointer", borderBottom: "1px solid var(--border)" }}
              onClick={() => gotoRef(r.file, r.line)}
            >
              <span style={{
                fontSize: 9, fontWeight: 700, width: 24, flexShrink: 0,
                color: kindLabel[r.kind]?.color || "var(--text-muted)",
              }}>
                {kindLabel[r.kind]?.label || "?"}
              </span>
              <span style={{ fontSize: 11, color: "var(--accent)", whiteSpace: "nowrap" }}>
                {r.file}:{r.line}
              </span>
              <span style={{
                fontSize: 10, color: "var(--text-muted)", overflow: "hidden",
                textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1,
              }}>
                {r.context}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
