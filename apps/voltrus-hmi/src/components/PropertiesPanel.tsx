import { useState } from "react";
import { useStore } from "../store";

const PLC_DATA_TYPES = ["BOOL", "BYTE", "INT", "DINT", "REAL", "TIME", "STRING", "WORD", "DWORD"] as const;

export function PropertiesPanel() {
  // Field-level subscriptions: avoid re-rendering the panel on every PLC frame.
  const screens = useStore((s) => s.screens);
  const activeScreenIndex = useStore((s) => s.activeScreenIndex);
  const selectedWidgetId = useStore((s) => s.selectedWidgetId);
  const kioskMode = useStore((s) => s.kioskMode);
  const rpi5Ip = useStore((s) => s.rpi5Ip);
  const rpi5Port = useStore((s) => s.rpi5Port);
  const [esp32Ip, setEsp32Ip] = useState("");

  const screen = screens[activeScreenIndex];
  const widget = screen?.widgets.find((w) => w.id === selectedWidgetId);

  if (!screen) return null;

  if (!widget) {
    return (
      <>
        <div className="sidebar-header">Screen properties</div>
        <div className="prop-group">
          <div className="prop-group-title">General</div>
          <div className="prop-row">
            <span className="prop-label">Name</span>
            <input
              className="prop-input vt-input"
              value={screen.name}
              onChange={(e) => useStore.getState().updateScreen(screen.id, { name: e.target.value })}
            />
          </div>
          <div className="prop-row">
            <span className="prop-label">Background</span>
            <input
              type="color"
              className="prop-color"
              value={screen.backgroundColor}
              onChange={(e) => useStore.getState().updateScreen(screen.id, { backgroundColor: e.target.value })}
            />
            <input
              className="prop-input vt-input"
              value={screen.backgroundColor}
              onChange={(e) => useStore.getState().updateScreen(screen.id, { backgroundColor: e.target.value })}
            />
          </div>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Deploy</div>
          <div className="prop-row">
            <span className="prop-label">RPi5 IP</span>
            <input
              className="prop-input vt-input"
              value={rpi5Ip}
              onChange={(e) => useStore.getState().setRpi5Ip(e.target.value)}
              placeholder="192.168.1.50"
            />
          </div>
          <div className="prop-row">
            <span className="prop-label">Port</span>
            <input
              className="prop-input-sm vt-input vt-input--sm"
              type="number"
              value={rpi5Port}
              onChange={(e) => {
                const v = Number(e.target.value);
                useStore.getState().setRpi5Port(Number.isFinite(v) && v > 0 ? Math.floor(v) : 8080);
              }}
            />
          </div>
          <button
            className="vt-btn vt-btn--sm vt-btn--block"
            disabled={!rpi5Ip}
            onClick={() => useStore.getState().deployToRpi5().catch(() => { /* toast shown in store */ })}
          >
            Deploy to RPi5
          </button>
          <div className="prop-row" style={{ marginTop: 6 }}>
            <span className="prop-label">ESP32 IP</span>
            <input
              className="prop-input vt-input"
              value={esp32Ip}
              onChange={(e) => setEsp32Ip(e.target.value)}
              placeholder="192.168.1.120"
            />
          </div>
          <button
            className="vt-btn vt-btn--sm vt-btn--block"
            disabled={!esp32Ip}
            onClick={() => useStore.getState().deployToEsp32(esp32Ip).catch(() => { /* toast shown in store */ })}
          >
            Deploy to ESP32
          </button>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Export</div>
          <label className="vt-check">
            <input type="checkbox" checked={kioskMode} onChange={(e) => useStore.getState().setKioskMode(e.target.checked)} />
            Kiosk Mode
          </label>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Statistics</div>
          <div className="prop-stat">
            Widgets: {screen.widgets.length}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <div className="sidebar-header">Widget properties</div>
      <div className="prop-group">
        <div className="prop-group-title">Identity</div>
        <div className="prop-row">
          <span className="prop-label">Type</span>
          <span className="prop-value">{widget.type}</span>
        </div>
        <div className="prop-row">
          <span className="prop-label">Label</span>
          <input
            className="prop-input vt-input"
            value={widget.label}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { label: e.target.value })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Position &amp; size</div>
        <div className="prop-row">
          <span className="prop-label">X</span>
          <input
            className="prop-input-sm vt-input vt-input--sm"
            type="number"
            value={Math.round(widget.x)}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { x: Number(e.target.value) })}
          />
          <span className="prop-label inline">Y</span>
          <input
            className="prop-input-sm vt-input vt-input--sm"
            type="number"
            value={Math.round(widget.y)}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { y: Number(e.target.value) })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Width</span>
          <input
            className="prop-input-sm vt-input vt-input--sm"
            type="number"
            value={Math.round(widget.width)}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { width: Number(e.target.value) })}
          />
          <span className="prop-label inline">Height</span>
          <input
            className="prop-input-sm vt-input vt-input--sm"
            type="number"
            value={Math.round(widget.height)}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { height: Number(e.target.value) })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Data binding</div>
        <div className="prop-row">
          <span className="prop-label">Variable</span>
          <input
            className="prop-input vt-input"
            placeholder="e.g. motor_speed"
            value={widget.variableName}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { variableName: e.target.value })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Data Type</span>
          <select
            className="prop-input vt-input"
            value={widget.dataType ?? ""}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { dataType: e.target.value || undefined })}
            title="PLC type used for writes in the exported runtime"
          >
            <option value="">Auto (DINT)</option>
            {PLC_DATA_TYPES.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </div>
        <div className="prop-row">
          <span className="prop-label">Role</span>
          <select
            className="prop-input vt-input"
            value={widget.requiredRole || "none"}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { requiredRole: e.target.value as "none" | "operator" | "supervisor" })}
          >
            <option value="none">None</option>
            <option value="operator">Operator+</option>
            <option value="supervisor">Supervisor</option>
          </select>
        </div>
        {(widget.type === "gauge" || widget.type === "slider" || widget.type === "chart") && (
          <div className="prop-row">
            <span className="prop-label">Min</span>
            <input
              className="prop-input-sm vt-input vt-input--sm"
              type="number"
              value={widget.min}
              onChange={(e) => useStore.getState().updateWidget(widget.id, { min: Number(e.target.value) })}
            />
            <span className="prop-label inline">Max</span>
            <input
              className="prop-input-sm vt-input vt-input--sm"
              type="number"
              value={widget.max}
              onChange={(e) => useStore.getState().updateWidget(widget.id, { max: Number(e.target.value) })}
            />
          </div>
        )}
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Style</div>
        <div className="prop-row">
          <span className="prop-label">Color</span>
          <input
            type="color"
            className="prop-color"
            value={widget.color}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { color: e.target.value })}
          />
          <input
            className="prop-input vt-input"
            value={widget.color}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { color: e.target.value })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Font Size</span>
          <input
            className="prop-input-sm vt-input vt-input--sm"
            type="number"
            value={widget.fontSize}
            onChange={(e) => useStore.getState().updateWidget(widget.id, { fontSize: Number(e.target.value) })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Layer</div>
        <div style={{ display: "flex", gap: 6 }}>
          <button className="vt-btn vt-btn--sm" style={{ flex: 1 }} onClick={() => useStore.getState().bringToFront(widget.id)}>
            Bring to Front
          </button>
          <button className="vt-btn vt-btn--sm" style={{ flex: 1 }} onClick={() => useStore.getState().sendToBack(widget.id)}>
            Send to Back
          </button>
        </div>
      </div>
      <div className="prop-group">
        <button className="vt-btn vt-btn--sm vt-btn--danger" onClick={() => useStore.getState().deleteWidget(widget.id)}>
          Delete Widget
        </button>
      </div>
    </>
  );
}
