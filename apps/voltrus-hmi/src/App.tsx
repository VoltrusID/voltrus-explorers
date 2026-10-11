import { useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useStore, CANVAS_W, CANVAS_H } from "./store";
import { checkForUpdates, installUpdate } from "./updater";
import { TagBrowser } from "./components/TagBrowser";
import { AlarmConfig } from "./components/AlarmConfig";
import { PinSetup } from "./components/PinSetup";
import { LanguageManager } from "./components/LanguageManager";
import { CanvasWidget } from "./components/CanvasWidget";
import { PropertiesPanel } from "./components/PropertiesPanel";
import type { HmiWidget, WidgetType } from "./types";

const WIDGET_TYPES: { type: WidgetType; icon: string; label: string }[] = [
  { type: "gauge", icon: "○", label: "Gauge" },
  { type: "button", icon: "□", label: "Button" },
  { type: "indicator", icon: "●", label: "Indicator" },
  { type: "text", icon: "T", label: "Text" },
  { type: "slider", icon: "═", label: "Slider" },
  { type: "chart", icon: "█", label: "Chart" },
];

const SCREEN_TEMPLATES: { name: string; desc: string; widgets: Omit<HmiWidget, "id">[] }[] = [
  {
    name: "Motor Control",
    desc: "Start/stop with status",
    widgets: [
      { type: "button", x: 20, y: 20, width: 100, height: 50, label: "Start", variableName: "motor_start", min: 0, max: 1, color: "#22c55e", fontSize: 14 },
      { type: "button", x: 140, y: 20, width: 100, height: 50, label: "Stop", variableName: "motor_stop", min: 0, max: 1, color: "#ef4444", fontSize: 14 },
      { type: "indicator", x: 260, y: 20, width: 60, height: 60, label: "Running", variableName: "motor_running", min: 0, max: 1, color: "#22c55e", fontSize: 12 },
      { type: "gauge", x: 20, y: 120, width: 120, height: 120, label: "Speed", variableName: "motor_speed", min: 0, max: 3000, color: "#3b82f6", fontSize: 14 },
      { type: "gauge", x: 160, y: 120, width: 120, height: 120, label: "Current", variableName: "motor_current", min: 0, max: 50, color: "#f59e0b", fontSize: 14 },
    ],
  },
  {
    name: "Tank Level",
    desc: "Level monitoring with pump",
    widgets: [
      { type: "gauge", x: 20, y: 20, width: 140, height: 140, label: "Level", variableName: "tank_level", min: 0, max: 100, color: "#3b82f6", fontSize: 16 },
      { type: "indicator", x: 180, y: 20, width: 60, height: 60, label: "High", variableName: "level_high", min: 0, max: 1, color: "#ef4444", fontSize: 12 },
      { type: "indicator", x: 260, y: 20, width: 60, height: 60, label: "Low", variableName: "level_low", min: 0, max: 1, color: "#f59e0b", fontSize: 12 },
      { type: "button", x: 180, y: 100, width: 140, height: 50, label: "Pump On/Off", variableName: "pump_cmd", min: 0, max: 1, color: "#22c55e", fontSize: 14 },
      { type: "slider", x: 20, y: 180, width: 300, height: 60, label: "Setpoint", variableName: "level_setpoint", min: 0, max: 100, color: "#8b5cf6", fontSize: 12 },
    ],
  },
  {
    name: "Conveyor",
    desc: "Speed + jam + zones",
    widgets: [
      { type: "gauge", x: 20, y: 20, width: 120, height: 120, label: "Speed", variableName: "conveyor_speed", min: 0, max: 100, color: "#3b82f6", fontSize: 14 },
      { type: "indicator", x: 160, y: 20, width: 60, height: 60, label: "Jammed", variableName: "conveyor_jammed", min: 0, max: 1, color: "#ef4444", fontSize: 12 },
      { type: "indicator", x: 20, y: 160, width: 60, height: 60, label: "Zone 1", variableName: "zone_1_active", min: 0, max: 1, color: "#22c55e", fontSize: 11 },
      { type: "indicator", x: 100, y: 160, width: 60, height: 60, label: "Zone 2", variableName: "zone_2_active", min: 0, max: 1, color: "#22c55e", fontSize: 11 },
      { type: "indicator", x: 180, y: 160, width: 60, height: 60, label: "Zone 3", variableName: "zone_3_active", min: 0, max: 1, color: "#22c55e", fontSize: 11 },
      { type: "button", x: 160, y: 80, width: 100, height: 45, label: "Start", variableName: "conveyor_start", min: 0, max: 1, color: "#22c55e", fontSize: 14 },
      { type: "button", x: 270, y: 80, width: 100, height: 45, label: "Stop", variableName: "conveyor_stop", min: 0, max: 1, color: "#ef4444", fontSize: 14 },
      { type: "slider", x: 20, y: 240, width: 300, height: 60, label: "Setpoint", variableName: "conveyor_setpoint", min: 0, max: 100, color: "#8b5cf6", fontSize: 12 },
    ],
  },
  {
    name: "Temp Control",
    desc: "PID + heater/cooler",
    widgets: [
      { type: "text", x: 20, y: 20, width: 120, height: 40, label: "Setpoint", variableName: "temp_setpoint", min: 0, max: 200, color: "#f59e0b", fontSize: 16 },
      { type: "gauge", x: 160, y: 10, width: 130, height: 130, label: "Actual", variableName: "temp_actual", min: 0, max: 200, color: "#ef4444", fontSize: 14 },
      { type: "indicator", x: 20, y: 80, width: 60, height: 60, label: "Heater", variableName: "heater_on", min: 0, max: 1, color: "#ef4444", fontSize: 12 },
      { type: "indicator", x: 90, y: 80, width: 60, height: 60, label: "Cooler", variableName: "cooler_on", min: 0, max: 1, color: "#3b82f6", fontSize: 12 },
      { type: "gauge", x: 20, y: 160, width: 120, height: 120, label: "PID Out", variableName: "pid_output", min: 0, max: 100, color: "#8b5cf6", fontSize: 14 },
      { type: "indicator", x: 160, y: 160, width: 70, height: 70, label: "High Alarm", variableName: "high_temp_alarm", min: 0, max: 1, color: "#ef4444", fontSize: 11 },
    ],
  },
];

/** True when a keyboard event target is a field the user is typing into. */
function isEditableTarget(target: unknown): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    target.isContentEditable
  );
}

// ---------------------------------------------------------------------------
// Main App
// ---------------------------------------------------------------------------

export default function App() {
  // Field-level subscriptions: a whole-store subscription would re-render the
  // entire editor on every PLC WebSocket frame.
  const screens = useStore((s) => s.screens);
  const activeScreenIndex = useStore((s) => s.activeScreenIndex);
  const selectedWidgetId = useStore((s) => s.selectedWidgetId);
  const zoom = useStore((s) => s.zoom);
  const plcIp = useStore((s) => s.plcIp);
  const theme = useStore((s) => s.theme);
  const isDirty = useStore((s) => s.isDirty);
  const toast = useStore((s) => s.toast);

  const canvasRef = useRef<HTMLDivElement>(null);
  const [dragOver, setDragOver] = useState(false);
  const [showGrid, setShowGrid] = useState(true);
  const [snapEnabled, setSnapEnabled] = useState(true);
  const [dragOverWidgetId, setDragOverWidgetId] = useState<string | null>(null);
  // URL of the Live Preview document (served by the `hmipreview` scheme).
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const [collapsedSections, setCollapsedSections] = useState<Record<string, boolean>>({
    widgets: false,
    templates: true,
    alarms: true,
    access: true,
    languages: true,
    tags: false,
  });

  const toggleSection = (key: string) => {
    setCollapsedSections((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  const activeScreen = screens[activeScreenIndex];

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  useEffect(() => {
    const unlisten = listen<string>("deep-link", (event) => {
      const url = event.payload;
      try {
        const parsed = new URL(url);
        const params = parsed.searchParams;
        const scada = params.get("scada");
        if (scada) useStore.getState().setScadaUrl(scada);
      } catch {
        // ignore malformed URLs
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Warn before closing with unsaved changes.
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (useStore.getState().isDirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, []);

  // Auto-update: check quietly on launch, notify when one is available.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const info = await checkForUpdates();
        if (!cancelled && info.available) {
          useStore
            .getState()
            .showToast("info", `Update ${info.version} available — use Check for Updates to install`);
        }
      } catch {
        // offline / endpoint not deployed yet — silent on launch
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const handleCheckForUpdates = async () => {
    setCheckingUpdate(true);
    const toast = useStore.getState().showToast;
    try {
      const info = await checkForUpdates();
      if (!info.available) {
        toast("info", "You are up to date");
        return;
      }
      const install = window.confirm(
        `Update ${info.version} available.\n\n${info.notes ?? ""}\n\nDownload and install now? The app will restart.`
      );
      if (!install) return;
      toast("info", "Downloading update…");
      const ok = await installUpdate();
      if (!ok) toast("error", "Update installation failed");
    } catch (e) {
      toast("error", `Update check failed: ${String(e)}`);
    } finally {
      setCheckingUpdate(false);
    }
  };

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // The live preview is modal: only Escape (close) applies — editing
      // shortcuts must not delete/undo widgets hidden behind it.
      if (previewSrc !== null) {
        if (e.key === "Escape") setPreviewSrc(null);
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        useStore.getState().saveProject();
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (selectedWidgetId && !isEditableTarget(e.target)) {
          useStore.getState().deleteWidget(selectedWidgetId);
        }
      }
      if (e.key === "Escape") {
        useStore.getState().selectWidget(null);
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        useStore.getState().undo();
      }
      if ((e.metaKey || e.ctrlKey) && (e.key === "y" || (e.key === "z" && e.shiftKey))) {
        e.preventDefault();
        useStore.getState().redo();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "c") {
        if (selectedWidgetId && !isEditableTarget(e.target)) {
          e.preventDefault();
          useStore.getState().copyWidget(selectedWidgetId);
        }
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "v") {
        if (useStore.getState().clipboard && !isEditableTarget(e.target)) {
          e.preventDefault();
          useStore.getState().pasteWidget();
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectedWidgetId, previewSrc]);

  // Drag and drop from palette
  const handleDragStart = (e: React.DragEvent, type: WidgetType) => {
    e.dataTransfer.setData("widget-type", type);
    e.dataTransfer.effectAllowed = "copy";
  };

  const handleCanvasDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setDragOver(true);
  };

  const handleCanvasDragLeave = () => {
    setDragOver(false);
  };

  const handleCanvasDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const type = e.dataTransfer.getData("widget-type") as WidgetType;
    if (!type) return;

    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const x = (e.clientX - rect.left) / zoom;
    const y = (e.clientY - rect.top) / zoom;
    const snappedX = snapEnabled ? Math.round(x / 10) * 10 : x;
    const snappedY = snapEnabled ? Math.round(y / 10) * 10 : y;
    useStore.getState().addWidget(type, snappedX, snappedY);
  };

  const handleCanvasClick = (e: React.MouseEvent) => {
    if (e.target === canvasRef.current || e.target === e.currentTarget) {
      useStore.getState().selectWidget(null);
    }
  };

  // Tag drop handlers for individual widgets
  const makeWidgetDragHandlers = (widgetId: string) => ({
    onDragOver: (e: React.DragEvent) => {
      if (e.dataTransfer.types.includes("application/tag") || e.dataTransfer.types.includes("application/variable")) {
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "link";
        setDragOverWidgetId(widgetId);
      }
    },
    onDragLeave: (e: React.DragEvent) => {
      e.stopPropagation();
      setDragOverWidgetId(null);
    },
    onDrop: (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDragOverWidgetId(null);

      const bind = useStore.getState().bindVariable;
      const tagData = e.dataTransfer.getData("application/tag");
      if (tagData) {
        try {
          const tag = JSON.parse(tagData);
          bind(widgetId, tag.plcVariable || tag.name || tagData, tag.dataType);
        } catch {
          bind(widgetId, tagData);
        }
        return;
      }

      const varData = e.dataTransfer.getData("application/variable");
      if (varData) {
        try {
          const v = JSON.parse(varData);
          bind(widgetId, v.plcVariable || v.name || varData, v.type);
        } catch {
          bind(widgetId, varData);
        }
      }
    },
  });

  // Live preview: the exported runtime in an in-app sandboxed iframe. It is
  // served from the `hmipreview` scheme with its own CSP — a srcdoc iframe
  // would inherit the app CSP, which blocks the runtime's inline scripts.
  const handleLivePreview = async () => {
    const s = useStore.getState();
    try {
      await invoke("prepare_preview", {
        request: {
          screens: s.screens,
          plcIp,
          scadaUrl: s.scadaUrl || undefined,
          alarms: s.alarms,
          accessConfig: s.accessConfig || undefined,
          translations: Object.keys(s.translations).length > 0 ? s.translations : undefined,
          defaultLanguage: s.defaultLanguage || undefined,
          kioskMode: s.kioskMode,
        },
      });
      // Unique query per open so the webview never reuses a stale document.
      setPreviewSrc(`${convertFileSrc("index.html", "hmipreview")}?r=${Date.now()}`);
    } catch (e) {
      s.showToast("error", `Preview failed: ${String(e)}`);
    }
  };

  // Zoom helpers
  const zoomIn = () => useStore.getState().setZoom(zoom + 0.1);
  const zoomOut = () => useStore.getState().setZoom(zoom - 0.1);
  const zoomReset = () => useStore.getState().setZoom(1);

  return (
    <div className="app-layout">
      {/* Toolbar */}
      <div className="toolbar">
        <span className="toolbar-title">
          Voltrus HMI{isDirty ? <span className="dirty-dot" title="Unsaved changes" /> : null}
        </span>
        <button
          className="toolbar-btn icon"
          onClick={() => useStore.getState().setTheme(theme === "dark" ? "light" : "dark")}
          title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        >
          {theme === "dark" ? "☀" : "☽"}
        </button>
        <span className="toolbar-divider" />
        <button className="toolbar-btn" onClick={() => useStore.getState().loadProject()} title="Open Project">
          Open
        </button>
        <button
          className="toolbar-btn"
          onClick={() => useStore.getState().saveProject()}
          title="Save Project (Cmd+S)"
        >
          Save{isDirty ? " •" : ""}
        </button>
        <span className="toolbar-divider" />
        <button className="toolbar-btn" onClick={() => useStore.getState().addScreen()} title="Add Screen">
          + Screen
        </button>
        <span className="toolbar-spacer" />
        <button
          className="toolbar-btn primary"
          onClick={() => useStore.getState().exportHtml()}
          title="Export a standalone HTML file"
        >
          Export HTML
        </button>
        <button className="toolbar-btn" onClick={handleLivePreview} title="Live Preview">
          Preview
        </button>
        <button
          className="toolbar-btn"
          onClick={handleCheckForUpdates}
          disabled={checkingUpdate}
          title="Check for Updates"
        >
          {checkingUpdate ? "Checking…" : "Check for Updates"}
        </button>
      </div>

      {/* Screen tabs */}
      <div className="screen-tabs">
        {screens.map((screen, i) => (
          <div
            key={screen.id}
            className={`screen-tab ${i === activeScreenIndex ? "active" : ""}`}
            onClick={() => useStore.getState().setActiveScreen(i)}
          >
            {screen.name}
            {screens.length > 1 && (
              <span
                className="screen-tab-close"
                title={`Delete ${screen.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  useStore.getState().deleteScreen(screen.id);
                }}
              >
                ×
              </span>
            )}
          </div>
        ))}
      </div>

      {/* Main content */}
      <div className="main-content">
        {/* Left sidebar: Widget Palette */}
        <div className="sidebar-left">
          {/* Widgets */}
          <div>
            <div
              className="sidebar-header"
              style={{ cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("widgets")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.widgets ? "▸" : "▾"}</span>
              Widgets
            </div>
            {!collapsedSections.widgets && (
              <>
                {WIDGET_TYPES.map((w) => (
                  <div
                    key={w.type}
                    className="widget-palette-item"
                    draggable
                    onDragStart={(e) => handleDragStart(e, w.type)}
                  >
                    <div className="widget-palette-icon">{w.icon}</div>
                    <span>{w.label}</span>
                  </div>
                ))}
              </>
            )}
          </div>

          {/* PLC & Tags */}
          <div>
            <div
              className="sidebar-header"
              style={{ marginTop: 4, cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("tags")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.tags ? "▸" : "▾"}</span>
              PLC & Tags
            </div>
            {!collapsedSections.tags && <TagBrowser />}
          </div>

          {/* Templates */}
          <div>
            <div
              className="sidebar-header"
              style={{ marginTop: 4, cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("templates")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.templates ? "▸" : "▾"}</span>
              Templates
            </div>
            {!collapsedSections.templates && (
              <>
                {SCREEN_TEMPLATES.map((t) => (
                  <div
                    key={t.name}
                    className="template-item"
                    onClick={() => useStore.getState().addTemplateWidgets(t.widgets)}
                  >
                    <span className="template-name">{t.name}</span>
                    <span className="template-desc">{t.desc}</span>
                  </div>
                ))}
              </>
            )}
          </div>

          {/* Alarms */}
          <div>
            <div
              className="sidebar-header"
              style={{ marginTop: 4, cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("alarms")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.alarms ? "▸" : "▾"}</span>
              Alarms
            </div>
            {!collapsedSections.alarms && <AlarmConfig />}
          </div>

          {/* Access Control */}
          <div>
            <div
              className="sidebar-header"
              style={{ marginTop: 4, cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("access")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.access ? "▸" : "▾"}</span>
              Access Control
            </div>
            {!collapsedSections.access && <PinSetup />}
          </div>

          {/* Languages */}
          <div>
            <div
              className="sidebar-header"
              style={{ marginTop: 4, cursor: "pointer", userSelect: "none" }}
              onClick={() => toggleSection("languages")}
            >
              <span style={{ marginRight: 4 }}>{collapsedSections.languages ? "▸" : "▾"}</span>
              Languages
            </div>
            {!collapsedSections.languages && <LanguageManager />}
          </div>

        </div>

        {/* Center: Canvas area */}
        <div className="canvas-area">
          {/* Canvas toolbar */}
          <div className="canvas-toolbar">
            <button className="canvas-toolbar-btn" onClick={zoomOut} title="Zoom Out">
              −
            </button>
            <span className="canvas-toolbar-label">{Math.round(zoom * 100)}%</span>
            <button className="canvas-toolbar-btn" onClick={zoomIn} title="Zoom In">
              +
            </button>
            <button className="canvas-toolbar-btn" onClick={zoomReset} title="Reset Zoom">
              Reset
            </button>

            <span className="canvas-toolbar-sep" />

            <button
              className={`canvas-toolbar-btn ${showGrid ? "active" : ""}`}
              onClick={() => setShowGrid(!showGrid)}
              title="Toggle Grid"
            >
              Grid
            </button>
            <button
              className={`canvas-toolbar-btn ${snapEnabled ? "active" : ""}`}
              onClick={() => setSnapEnabled(!snapEnabled)}
              title="Toggle Snap to Grid"
            >
              Snap
            </button>
          </div>

          {/* Scrollable canvas container.
              The wrapper carries the zoom-scaled layout size so the scroll
              extent matches what scale() actually renders. */}
          <div className="canvas-scroll-container">
            <div
              className="canvas-zoom-wrapper"
              style={{ width: CANVAS_W * zoom + 48, height: CANVAS_H * zoom + 48 }}
            >
              <div
                ref={canvasRef}
                className={`canvas-drop-zone ${dragOver ? "drag-over" : ""} ${showGrid ? "show-grid" : ""}`}
                style={{
                  width: CANVAS_W,
                  height: CANVAS_H,
                  transform: `scale(${zoom})`,
                  transformOrigin: "top left",
                  backgroundColor: activeScreen?.backgroundColor || "#1e1e2e",
                }}
                onDragOver={handleCanvasDragOver}
                onDragLeave={handleCanvasDragLeave}
                onDrop={handleCanvasDrop}
                onClick={handleCanvasClick}
              >
                {activeScreen?.widgets.length === 0 && (
                  <div className="empty-canvas">
                    <div className="empty-canvas-icon">+</div>
                    <div className="empty-canvas-text">Drag widgets here</div>
                  </div>
                )}
                {activeScreen?.widgets.map((widget) => {
                  const tagHandlers = makeWidgetDragHandlers(widget.id);
                  return (
                    <CanvasWidget
                      key={widget.id}
                      widget={widget}
                      selected={widget.id === selectedWidgetId}
                      onSelect={() => useStore.getState().selectWidget(widget.id)}
                      onDragStart={() => useStore.getState().beginWidgetDrag()}
                      onMove={(x, y) => useStore.getState().moveWidget(widget.id, x, y)}
                      onResize={(w, h) => useStore.getState().resizeWidget(widget.id, w, h)}
                      zoom={zoom}
                      snapEnabled={snapEnabled}
                      dragOverTag={dragOverWidgetId === widget.id}
                      onDragOverTag={tagHandlers.onDragOver}
                      onDragLeaveTag={tagHandlers.onDragLeave}
                      onDropTag={tagHandlers.onDrop}
                    />
                  );
                })}
              </div>
            </div>
          </div>
        </div>

        {/* Right sidebar: Properties */}
        <div className="sidebar-right">
          <PropertiesPanel />
        </div>
      </div>

      {/* Live preview modal */}
      {previewSrc !== null && (
        <div className="preview-modal" onClick={() => setPreviewSrc(null)}>
          <div className="preview-frame" onClick={(e) => e.stopPropagation()}>
            <div className="preview-toolbar">
              <span>Live Preview</span>
              <button className="preview-close" onClick={() => setPreviewSrc(null)} title="Close (Esc)">
                ×
              </button>
            </div>
            <iframe
              title="HMI Live Preview"
              src={previewSrc}
              // allow-scripts ONLY — never add allow-same-origin. Tauri serves
              // custom schemes (hmipreview) as a local origin; with
              // same-origin the exported runtime could reach the app and its
              // IPC. Pinned by src/previewSandbox.test.ts.
              sandbox="allow-scripts"
            />
          </div>
        </div>
      )}

      {/* Toast */}
      {toast && (
        <div className={`toast toast-${toast.kind}`} onClick={() => useStore.getState().dismissToast()}>
          {toast.kind === "error" ? "✕ " : toast.kind === "success" ? "✓ " : "ℹ "}
          {toast.message}
        </div>
      )}
    </div>
  );
}
