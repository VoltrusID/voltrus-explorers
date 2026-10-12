import { useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChartLine,
  ChevronDown,
  ChevronRight,
  CircleCheck,
  FolderOpen,
  Gauge,
  Grid3x3,
  GripVertical,
  Info,
  Magnet,
  Minus,
  Play,
  Plus,
  RotateCcw,
  Save,
  SlidersHorizontal,
  SquareDashedMousePointer,
  SquareMousePointer,
  CircleDot,
  Sun,
  Moon,
  Type,
  X,
  XCircle,
  Download,
  RefreshCw,
} from "lucide-react";
import { Button, EmptyState, IconButton, Toolbar } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";
import { useStore, CANVAS_W, CANVAS_H } from "./store";
import { checkForUpdates, installUpdate } from "./updater";
import { TagBrowser } from "./components/TagBrowser";
import { AlarmConfig } from "./components/AlarmConfig";
import { PinSetup } from "./components/PinSetup";
import { LanguageManager } from "./components/LanguageManager";
import { CanvasWidget } from "./components/CanvasWidget";
import { PropertiesPanel } from "./components/PropertiesPanel";
import type { HmiWidget, WidgetType } from "./types";

const WIDGET_TYPES: { type: WidgetType; icon: React.ReactNode; label: string }[] = [
  { type: "gauge", icon: <Gauge size={14} strokeWidth={1.75} />, label: "Gauge" },
  { type: "button", icon: <SquareMousePointer size={14} strokeWidth={1.75} />, label: "Button" },
  { type: "indicator", icon: <CircleDot size={14} strokeWidth={1.75} />, label: "Indicator" },
  { type: "text", icon: <Type size={14} strokeWidth={1.75} />, label: "Text" },
  { type: "slider", icon: <SlidersHorizontal size={14} strokeWidth={1.75} />, label: "Slider" },
  { type: "chart", icon: <ChartLine size={14} strokeWidth={1.75} />, label: "Chart" },
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

/** Collapsible sidebar block: chevron + sentence-case title, content below. */
function AccordionSection({
  id,
  title,
  meta,
  collapsed,
  onToggle,
  children,
}: {
  id: string;
  title: string;
  meta?: string;
  collapsed: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <section className="acc-section" data-section={id}>
      <button
        type="button"
        className="acc-header"
        aria-expanded={!collapsed}
        aria-controls={`acc-${id}`}
        onClick={onToggle}
      >
        {collapsed ? (
          <ChevronRight size={14} strokeWidth={1.75} className="acc-chevron" />
        ) : (
          <ChevronDown size={14} strokeWidth={1.75} className="acc-chevron" />
        )}
        <span className="acc-title">{title}</span>
        {meta !== undefined && <span className="acc-meta">{meta}</span>}
      </button>
      {!collapsed && (
        <div id={`acc-${id}`} className="acc-body">
          {children}
        </div>
      )}
    </section>
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
  const alarmCount = useStore((s) => s.alarms.length);
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

  return (
    <div className="app-layout">
      {/* Toolbar */}
      <Toolbar
        title="Voltrus HMI"
        icon={appIcon}
        connected={false}
        context={
          isDirty ? (
            <span className="toolbar-dirty" title="Unsaved changes">
              <span className="dirty-dot" />
              Unsaved changes
            </span>
          ) : undefined
        }
        actions={
          <>
            <IconButton
              label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
              size="sm"
              onClick={() => useStore.getState().setTheme(theme === "dark" ? "light" : "dark")}
            >
              {theme === "dark" ? <Sun size={16} strokeWidth={1.75} /> : <Moon size={16} strokeWidth={1.75} />}
            </IconButton>
            <span className="toolbar-sep" aria-hidden="true" />
            <Button size="sm" variant="ghost" icon={<FolderOpen size={14} strokeWidth={1.75} />} onClick={() => useStore.getState().loadProject()}>
              Open
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Save size={14} strokeWidth={1.75} />}
              onClick={() => useStore.getState().saveProject()}
            >
              Save
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Plus size={14} strokeWidth={1.75} />}
              onClick={() => useStore.getState().addScreen()}
            >
              Add screen
            </Button>
            <Button
              size="sm"
              variant="primary"
              icon={<Download size={14} strokeWidth={1.75} />}
              onClick={() => useStore.getState().exportHtml()}
            >
              Export HTML
            </Button>
            <Button size="sm" variant="ghost" icon={<Play size={14} strokeWidth={1.75} />} onClick={handleLivePreview}>
              Preview
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<RefreshCw size={14} strokeWidth={1.75} />}
              onClick={handleCheckForUpdates}
              loading={checkingUpdate}
            >
              {checkingUpdate ? "Checking…" : "Check for updates"}
            </Button>
          </>
        }
      />

      {/* Screen tabs */}
      <div className="vt-tabs" role="tablist" aria-label="Screens">
        {screens.map((screen, i) => (
          <div
            key={screen.id}
            role="tab"
            aria-selected={i === activeScreenIndex}
            tabIndex={i === activeScreenIndex ? 0 : -1}
            className="vt-tab screen-tab"
            onClick={() => useStore.getState().setActiveScreen(i)}
          >
            {screen.name}
            {screens.length > 1 && (
              <button
                type="button"
                className="screen-tab-close"
                title={`Delete ${screen.name}`}
                aria-label={`Delete ${screen.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  useStore.getState().deleteScreen(screen.id);
                }}
              >
                <X size={12} strokeWidth={1.75} />
              </button>
            )}
          </div>
        ))}
      </div>

      {/* Main content */}
      <div className="main-content">
        {/* Left sidebar: Widget palette, tags, templates, alarms, access */}
        <aside className="vt-sidebar sidebar-left">
          <AccordionSection
            id="widgets"
            title="Widgets"
            collapsed={collapsedSections.widgets}
            onToggle={() => toggleSection("widgets")}
          >
            <div className="widget-palette">
              {WIDGET_TYPES.map((w) => (
                <div
                  key={w.type}
                  className="widget-palette-item"
                  draggable
                  onDragStart={(e) => handleDragStart(e, w.type)}
                >
                  <span className="widget-palette-icon">{w.icon}</span>
                  <span className="widget-palette-label">{w.label}</span>
                  <GripVertical
                    size={12}
                    strokeWidth={1.75}
                    className="widget-palette-grip"
                    aria-hidden="true"
                  />
                </div>
              ))}
            </div>
          </AccordionSection>

          <AccordionSection
            id="tags"
            title="PLC & tags"
            collapsed={collapsedSections.tags}
            onToggle={() => toggleSection("tags")}
          >
            <TagBrowser />
          </AccordionSection>

          <AccordionSection
            id="templates"
            title="Templates"
            collapsed={collapsedSections.templates}
            onToggle={() => toggleSection("templates")}
          >
            <div className="template-list">
              {SCREEN_TEMPLATES.map((t) => (
                <button
                  key={t.name}
                  type="button"
                  className="template-item"
                  onClick={() => useStore.getState().addTemplateWidgets(t.widgets)}
                >
                  <span className="template-name">{t.name}</span>
                  <span className="template-desc">{t.desc}</span>
                </button>
              ))}
            </div>
          </AccordionSection>

          <AccordionSection
            id="alarms"
            title="Alarms"
            meta={alarmCount > 0 ? String(alarmCount) : undefined}
            collapsed={collapsedSections.alarms}
            onToggle={() => toggleSection("alarms")}
          >
            <AlarmConfig />
          </AccordionSection>

          <AccordionSection
            id="access"
            title="Access control"
            collapsed={collapsedSections.access}
            onToggle={() => toggleSection("access")}
          >
            <PinSetup />
          </AccordionSection>

          <AccordionSection
            id="languages"
            title="Languages"
            collapsed={collapsedSections.languages}
            onToggle={() => toggleSection("languages")}
          >
            <LanguageManager />
          </AccordionSection>
        </aside>

        {/* Center: Canvas area */}
        <div className="canvas-area">
          {/* Canvas toolbar */}
          <div className="canvas-toolbar">
            <div className="canvas-toolbar-group" role="group" aria-label="Zoom">
              <IconButton
                label="Zoom out"
                size="sm"
                onClick={() => useStore.getState().setZoom(zoom - 0.1)}
              >
                <Minus size={14} strokeWidth={1.75} />
              </IconButton>
              <span className="canvas-toolbar-label">{Math.round(zoom * 100)}%</span>
              <IconButton
                label="Zoom in"
                size="sm"
                onClick={() => useStore.getState().setZoom(zoom + 0.1)}
              >
                <Plus size={14} strokeWidth={1.75} />
              </IconButton>
              <IconButton
                label="Reset zoom"
                size="sm"
                onClick={() => useStore.getState().setZoom(1)}
              >
                <RotateCcw size={14} strokeWidth={1.75} />
              </IconButton>
            </div>

            <span className="toolbar-sep" aria-hidden="true" />

            <Button
              size="sm"
              aria-pressed={showGrid}
              icon={<Grid3x3 size={14} strokeWidth={1.75} />}
              onClick={() => setShowGrid(!showGrid)}
              title="Toggle grid"
            >
              Grid
            </Button>
            <Button
              size="sm"
              aria-pressed={snapEnabled}
              icon={<Magnet size={14} strokeWidth={1.75} />}
              onClick={() => setSnapEnabled(!snapEnabled)}
              title="Toggle snap to grid"
            >
              Snap
            </Button>
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
                  backgroundColor: activeScreen?.backgroundColor || "#0b0b0a",
                }}
                onDragOver={handleCanvasDragOver}
                onDragLeave={handleCanvasDragLeave}
                onDrop={handleCanvasDrop}
                onClick={handleCanvasClick}
              >
                {activeScreen?.widgets.length === 0 && (
                  <div className="empty-canvas">
                    <EmptyState
                      compact
                      icon={<SquareDashedMousePointer size={20} strokeWidth={1.75} />}
                      title="No widgets yet"
                      description="Drag widgets from the palette onto the canvas."
                    />
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
        <aside className="vt-sidebar sidebar-right">
          <PropertiesPanel />
        </aside>
      </div>

      {/* Live preview modal */}
      {previewSrc !== null && (
        <div className="preview-modal" onClick={() => setPreviewSrc(null)}>
          <div className="preview-frame" onClick={(e) => e.stopPropagation()}>
            <div className="preview-toolbar">
              <span>Live preview</span>
              <IconButton label="Close preview (Esc)" size="sm" onClick={() => setPreviewSrc(null)}>
                <X size={14} strokeWidth={1.75} />
              </IconButton>
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
          {toast.kind === "error" ? (
            <XCircle size={15} strokeWidth={1.75} aria-hidden="true" />
          ) : toast.kind === "success" ? (
            <CircleCheck size={15} strokeWidth={1.75} aria-hidden="true" />
          ) : (
            <Info size={15} strokeWidth={1.75} aria-hidden="true" />
          )}
          <span>{toast.message}</span>
        </div>
      )}
    </div>
  );
}
