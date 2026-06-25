import { useStore } from "../store";

export function PropertiesPanel() {
  const {
    screens, activeScreenIndex, selectedWidgetId, updateWidget, deleteWidget,
    updateScreen, bringToFront, sendToBack, kioskMode, setKioskMode,
    rpi5Ip, rpi5Port, setRpi5Ip, setRpi5Port, deployToRpi5,
  } = useStore();
  const screen = screens[activeScreenIndex];
  const widget = screen?.widgets.find((w) => w.id === selectedWidgetId);

  if (!screen) return null;

  if (!widget) {
    return (
      <>
        <div className="sidebar-header">Screen Properties</div>
        <div className="prop-group">
          <div className="prop-group-title">General</div>
          <div className="prop-row">
            <span className="prop-label">Name</span>
            <input
              className="prop-input"
              value={screen.name}
              onChange={(e) => updateScreen(screen.id, { name: e.target.value })}
            />
          </div>
          <div className="prop-row">
            <span className="prop-label">Background</span>
            <input
              type="color"
              className="prop-color"
              value={screen.backgroundColor}
              onChange={(e) => updateScreen(screen.id, { backgroundColor: e.target.value })}
            />
            <input
              className="prop-input"
              value={screen.backgroundColor}
              onChange={(e) => updateScreen(screen.id, { backgroundColor: e.target.value })}
            />
          </div>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Deploy</div>
          <div className="prop-row">
            <span className="prop-label">RPi5 IP</span>
            <input
              className="prop-input"
              value={rpi5Ip}
              onChange={(e) => setRpi5Ip(e.target.value)}
              placeholder="192.168.1.50"
            />
          </div>
          <div className="prop-row">
            <span className="prop-label">Port</span>
            <input
              className="prop-input-sm"
              type="number"
              value={rpi5Port}
              onChange={(e) => setRpi5Port(Number(e.target.value))}
            />
          </div>
          <button
            className="prop-btn"
            style={{ width: "100%", marginTop: 4 }}
            disabled={!rpi5Ip}
            onClick={async () => { try { await deployToRpi5(); } catch { /* logged in store */ } }}
          >
            Deploy to RPi5
          </button>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Export</div>
          <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, color: "var(--text-muted)", cursor: "pointer" }}>
            <input type="checkbox" checked={kioskMode} onChange={(e) => setKioskMode(e.target.checked)} />
            Kiosk Mode
          </label>
        </div>
        <div className="prop-group">
          <div className="prop-group-title">Statistics</div>
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
            Widgets: {screen.widgets.length}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <div className="sidebar-header">Widget Properties</div>
      <div className="prop-group">
        <div className="prop-group-title">Identity</div>
        <div className="prop-row">
          <span className="prop-label">Type</span>
          <span style={{ fontSize: 12, textTransform: "capitalize" }}>{widget.type}</span>
        </div>
        <div className="prop-row">
          <span className="prop-label">Label</span>
          <input
            className="prop-input"
            value={widget.label}
            onChange={(e) => updateWidget(widget.id, { label: e.target.value })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Position & Size</div>
        <div className="prop-row">
          <span className="prop-label">X</span>
          <input
            className="prop-input-sm"
            type="number"
            value={Math.round(widget.x)}
            onChange={(e) => updateWidget(widget.id, { x: Number(e.target.value) })}
          />
          <span className="prop-label">Y</span>
          <input
            className="prop-input-sm"
            type="number"
            value={Math.round(widget.y)}
            onChange={(e) => updateWidget(widget.id, { y: Number(e.target.value) })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Width</span>
          <input
            className="prop-input-sm"
            type="number"
            value={Math.round(widget.width)}
            onChange={(e) => updateWidget(widget.id, { width: Number(e.target.value) })}
          />
          <span className="prop-label">Height</span>
          <input
            className="prop-input-sm"
            type="number"
            value={Math.round(widget.height)}
            onChange={(e) => updateWidget(widget.id, { height: Number(e.target.value) })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Data Binding</div>
        <div className="prop-row">
          <span className="prop-label">Variable</span>
          <input
            className="prop-input"
            placeholder="e.g. motor_speed"
            value={widget.variableName}
            onChange={(e) => updateWidget(widget.id, { variableName: e.target.value })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Role</span>
          <select
            className="prop-input"
            value={widget.requiredRole || "none"}
            onChange={(e) => updateWidget(widget.id, { requiredRole: e.target.value as "none" | "operator" | "supervisor" })}
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
              className="prop-input-sm"
              type="number"
              value={widget.min}
              onChange={(e) => updateWidget(widget.id, { min: Number(e.target.value) })}
            />
            <span className="prop-label">Max</span>
            <input
              className="prop-input-sm"
              type="number"
              value={widget.max}
              onChange={(e) => updateWidget(widget.id, { max: Number(e.target.value) })}
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
            onChange={(e) => updateWidget(widget.id, { color: e.target.value })}
          />
          <input
            className="prop-input"
            value={widget.color}
            onChange={(e) => updateWidget(widget.id, { color: e.target.value })}
          />
        </div>
        <div className="prop-row">
          <span className="prop-label">Font Size</span>
          <input
            className="prop-input-sm"
            type="number"
            value={widget.fontSize}
            onChange={(e) => updateWidget(widget.id, { fontSize: Number(e.target.value) })}
          />
        </div>
      </div>
      <div className="prop-group">
        <div className="prop-group-title">Layer</div>
        <div style={{ display: "flex", gap: 6 }}>
          <button className="prop-btn" style={{ flex: 1 }} onClick={() => bringToFront(widget.id)}>
            Bring to Front
          </button>
          <button className="prop-btn" style={{ flex: 1 }} onClick={() => sendToBack(widget.id)}>
            Send to Back
          </button>
        </div>
      </div>
      <div className="prop-group">
        <button className="prop-btn danger" onClick={() => deleteWidget(widget.id)}>
          Delete Widget
        </button>
      </div>
    </>
  );
}
