import { useEffect, useMemo, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { Button, IconButton } from "@mactools/ui";
import { useStore } from "../store";
import type { AlarmRule } from "../types";

export function AlarmConfig() {
  const scadaUrl = useStore((s) => s.scadaUrl);
  const alarms = useStore((s) => s.alarms);
  const plcVariables = useStore((s) => s.plcVariables);
  const setScadaUrl = useStore((s) => s.setScadaUrl);
  const addAlarm = useStore((s) => s.addAlarm);
  const removeAlarm = useStore((s) => s.removeAlarm);
  const updateAlarm = useStore((s) => s.updateAlarm);

  // Enabled == a URL is configured (the export gates SCADA polling on it).
  // Remember the last URL so toggling off/on does not lose it.
  const scadaEnabled = scadaUrl.trim().length > 0;
  const lastScadaUrl = useRef(scadaUrl);
  useEffect(() => {
    if (scadaUrl.trim()) lastScadaUrl.current = scadaUrl;
  }, [scadaUrl]);
  const toggleScada = () => {
    if (scadaEnabled) setScadaUrl("");
    else setScadaUrl(lastScadaUrl.current || "http://localhost:8080");
  };

  const [showForm, setShowForm] = useState(false);
  const [formVar, setFormVar] = useState("");
  const [formSeverity, setFormSeverity] = useState<"critical" | "warning">("warning");
  const [formMessage, setFormMessage] = useState("");

  const alarmVars = useMemo(
    () => plcVariables.filter((v) => v.type === "BOOL" && v.name.endsWith("_alarm")),
    [plcVariables]
  );

  const handleAdd = () => {
    const varName = formVar.trim();
    if (!varName) return;
    addAlarm({
      source: "plc",
      variableName: varName,
      severity: formSeverity,
      message: formMessage.trim() || `${varName} triggered`,
    });
    setFormVar("");
    setFormSeverity("warning");
    setFormMessage("");
    setShowForm(false);
  };

  return (
    <div className="acc-body">
      {/* SCADA alarms */}
      <div className="sub-block">
        <div className="sub-title">SCADA alarms</div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <button
            type="button"
            className="vt-switch"
            aria-pressed={scadaEnabled}
            aria-label={scadaEnabled ? "Disable SCADA alarms" : "Enable SCADA alarms"}
            onClick={toggleScada}
          />
          <span className="status-line" style={{ gap: 0 }}>
            {scadaEnabled ? "Enabled" : "Disabled"}
          </span>
        </div>
        {scadaEnabled && (
          <input
            type="text"
            value={scadaUrl}
            onChange={(e) => setScadaUrl(e.target.value)}
            placeholder="https://scada.example.com/api"
            aria-label="SCADA API URL"
            className="sub-input vt-input vt-input--mono vt-input--sm"
          />
        )}
      </div>

      {/* PLC variable alarms */}
      <div className="sub-block" style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        <div className="sub-title">
          <span>PLC alarms</span>
          <span className="row-meta">{alarms.length}</span>
        </div>

        {/* Add alarm form */}
        {showForm && (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 6,
              padding: "0 0 8px",
              borderBottom: "1px solid var(--border)",
            }}
          >
            <select
              value={formVar}
              onChange={(e) => setFormVar(e.target.value)}
              aria-label="Alarm variable from PLC"
              className="sub-input vt-select vt-input--sm"
            >
              <option value="">Select variable</option>
              {alarmVars.map((v) => (
                <option key={v.name} value={v.name}>
                  {v.name}
                </option>
              ))}
            </select>
            <input
              type="text"
              value={formVar}
              onChange={(e) => setFormVar(e.target.value)}
              placeholder="Or type a variable name"
              aria-label="Alarm variable name"
              className="sub-input vt-input vt-input--sm"
            />
            <select
              value={formSeverity}
              onChange={(e) =>
                setFormSeverity(e.target.value as "critical" | "warning")
              }
              aria-label="Alarm severity"
              className="sub-input vt-select vt-input--sm"
            >
              <option value="warning">Warning</option>
              <option value="critical">Critical</option>
            </select>
            <input
              type="text"
              value={formMessage}
              onChange={(e) => setFormMessage(e.target.value)}
              placeholder="Custom message (optional)"
              aria-label="Custom alarm message"
              className="sub-input vt-input vt-input--sm"
            />
            <div className="sub-actions">
              <Button size="sm" variant="primary" onClick={handleAdd} disabled={!formVar.trim()}>
                Add alarm
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setShowForm(false);
                  setFormVar("");
                  setFormMessage("");
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}

        {/* Alarm list */}
        {!showForm && alarms.length === 0 ? (
          <div className="sub-empty">
            No alarm rules. Click Add to watch a *_alarm variable.
          </div>
        ) : (
          <div className="sub-list">
            {alarms.map((alarm) => (
              <div key={alarm.id} className="list-row is-card" style={{ cursor: "default" }}>
                <span className={`severity-chip severity-chip--${alarm.severity}`}>
                  {alarm.severity}
                </span>
                <span
                  className="row-main"
                  style={{ fontWeight: 500 }}
                  title={alarm.variableName}
                >
                  {alarm.variableName}
                </span>
                <span className="row-meta" style={{ maxWidth: 60, overflow: "hidden", textOverflow: "ellipsis" }} title={alarm.message}>
                  {alarm.message}
                </span>
                <IconButton label="Delete alarm" size="sm" danger onClick={() => removeAlarm(alarm.id)}>
                  <X size={12} strokeWidth={1.75} />
                </IconButton>
              </div>
            ))}
          </div>
        )}

        {!showForm && (
          <div style={{ padding: "0 10px 2px" }}>
            <Button
              size="sm"
              variant="ghost"
              icon={<Plus size={14} strokeWidth={1.75} />}
              onClick={() => setShowForm(true)}
            >
              Add
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
