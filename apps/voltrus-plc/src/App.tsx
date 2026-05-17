import { useState, useEffect } from "react";
import { useStore } from "./store";
import { invoke } from "@tauri-apps/api/core";
import StEditor from "./components/Editor/StEditor";
import WatchTable from "./components/Sidebar/WatchTable";
import RuntimeControls from "./components/Sidebar/RuntimeControls";
import IoPanel from "./components/Sidebar/IoPanel";
import TargetPanel from "./components/Sidebar/TargetPanel";
import AiChatPanel from "./components/Sidebar/AiChatPanel";
import AiSettings from "./components/Sidebar/AiSettings";
import LicensePanel from "./components/Sidebar/LicensePanel";
import CycleGraph from "./components/Bottom/CycleGraph";
import Histogram from "./components/Bottom/Histogram";
import ResizeHandle from "./components/ResizeHandle";
import type { SampleTemplate } from "./types";
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
    projectDir,
    projectFiles,
    openFile,
    openProjectDir,
    refreshProjectFiles,
    saveFile,
    loadSampleTemplate,
    theme,
    setTheme,
    rightPanelTab,
    setRightPanelTab,
    projectPath,
    saveProject,
    saveProjectAs,
    loadProject,
    sendAiErrorFix,
    sendAiTestGeneration,
    validateAiCode,
  } = useStore();

  const [templates, setTemplates] = useState<SampleTemplate[]>([]);
  const [showTemplates, setShowTemplates] = useState(false);
  const [showNewFile, setShowNewFile] = useState(false);
  const [leftWidth, setLeftWidth] = useState(220);
  const [rightWidth, setRightWidth] = useState(280);
  const [bottomHeight, setBottomHeight] = useState(200);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);

  const activeFile = openFiles[activeFileIndex];

  useEffect(() => {
    invoke<SampleTemplate[]>("get_sample_templates").then(setTemplates);
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
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
  }, [activeFileIndex, saveFile, compile, setRightPanelTab, setCurrentExecutionLine, setDebugState, saveProject]);

  // Listen for breakpoint-hit events from Tauri
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => {
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
    });
    return () => { unlisten?.(); };
  }, [setCurrentExecutionLine, setDebugState]);

  const stFiles = projectFiles.filter(
    (f) => !f.is_dir && (f.name.endsWith(".st") || f.name.endsWith(".txt"))
  );

  return (
    <div className="app-layout">
      {/* Toolbar */}
      <div className="toolbar">
        <span className="toolbar-title">Voltrus PLC</span>
        <button
          className="toolbar-btn theme-toggle"
          onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          title="Toggle theme"
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
          Step
        </button>
        <span className="toolbar-divider" />
        <button
          className="toolbar-btn"
          onClick={async () => {
            try {
              const { invoke } = await import("@tauri-apps/api/core");
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
              const { invoke } = await import("@tauri-apps/api/core");
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
        <div className="runtime-indicator">
          <span className={`runtime-led ${runtimeState}`} />
          <span>{runtimeState.toUpperCase()}</span>
        </div>
      </div>

      {/* New File Modal */}
      {showNewFile && (
        <div className="modal-overlay" onClick={() => setShowNewFile(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>New File</h3>
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
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Example Programs</h3>
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
            </div>
            <button className="toolbar-btn" style={{ marginTop: 12 }} onClick={() => setShowTemplates(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Main content: 3 columns */}
      <div className="main-content">
        {/* Left sidebar: File explorer */}
        <div className="sidebar-left" style={{ width: leftWidth }}>
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
                  No .st files found
                </li>
              ) : (
                stFiles.map((f) => (
                  <li
                    key={f.path}
                    className={`file-item ${openFiles[activeFileIndex]?.path === f.path ? "active" : ""}`}
                    onClick={() => openFile(f.path)}
                  >
                    <span style={{ fontSize: 10 }}>ST</span>
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
                    {file.modified ? "\u25CF" : "\u{1F4C4}"}
                  </span>
                  <span>{file.name}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <ResizeHandle
          axis="x"
          onDrag={(d) => setLeftWidth((w) => Math.min(500, Math.max(140, w + d)))}
        />

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
              <StEditor
                key={activeFile.path}
                value={activeFile.content}
                onChange={(value) => updateFileContent(activeFileIndex, value)}
              />
            ) : (
              <div className="placeholder-editor">
                Open or create a file to start coding
              </div>
            )}
          </div>
        </div>

        <ResizeHandle
          axis="x"
          onDrag={(d) => setRightWidth((w) => Math.min(500, Math.max(180, w - d)))}
        />

        {/* Right sidebar: tabs for Watch, I/O, Target, AI */}
        <div className="sidebar-right" style={{ width: rightWidth }}>
          <div style={{ display: "flex", borderBottom: "1px solid var(--border)" }}>
            {(["watch", "io", "target", "ai"] as const).map((tab) => (
              <button
                key={tab}
                className={`bottom-tab ${rightPanelTab === tab ? "active" : ""}`}
                style={{ flex: 1, justifyContent: "center" }}
                onClick={() => setRightPanelTab(tab)}
              >
                {tab === "watch" ? "Watch" : tab === "io" ? "I/O" : tab === "target" ? "Target" : "AI"}
              </button>
            ))}
          </div>
          {rightPanelTab === "watch" && (
            <>
              <div className="sidebar-header">Runtime</div>
              <RuntimeControls />
              <div className="sidebar-header">Watch Table</div>
              <WatchTable />
            </>
          )}
          {rightPanelTab === "io" && (
            <>
              <div className="sidebar-header">I/O Driver</div>
              <IoPanel />
            </>
          )}
          {rightPanelTab === "target" && (
            <>
              <div className="sidebar-header">Target Device</div>
              <TargetPanel />
            </>
          )}
          {rightPanelTab === "ai" && (
            <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
              <div className="sidebar-header">AI Settings</div>
              <AiSettings />
              <div className="sidebar-header">License</div>
              <LicensePanel />
              <div className="sidebar-header">AI Chat</div>
              <div style={{ flex: 1, overflow: "auto", padding: "0 8px" }}>
                <AiChatPanel />
              </div>
            </div>
          )}
        </div>
      </div>

      <ResizeHandle
        axis="y"
        onDrag={(d) => setBottomHeight((h) => Math.min(500, Math.max(100, h - d)))}
      />

      {/* Bottom panel */}
      <div className="bottom-panel" style={{ height: bottomHeight }}>
        <div className="bottom-tabs">
          <button
            className={`bottom-tab ${bottomTab === "diagnostics" ? "active" : ""}`}
            onClick={() => setBottomTab("diagnostics")}
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
            className={`bottom-tab ${bottomTab === "log" ? "active" : ""}`}
            onClick={() => setBottomTab("log")}
          >
            Runtime Log
          </button>
          <button
            className={`bottom-tab ${bottomTab === "output" ? "active" : ""}`}
            onClick={() => setBottomTab("output")}
          >
            Output
          </button>
          <button
            className={`bottom-tab ${bottomTab === "timing" ? "active" : ""}`}
            onClick={() => setBottomTab("timing")}
          >
            Timing
          </button>
          <button
            className={`bottom-tab ${bottomTab === "histogram" ? "active" : ""}`}
            onClick={() => setBottomTab("histogram")}
          >
            Histogram
          </button>
        </div>
        <div className="bottom-content">
          {bottomTab === "diagnostics" && (
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
        </div>
      </div>
    </div>
  );
}
