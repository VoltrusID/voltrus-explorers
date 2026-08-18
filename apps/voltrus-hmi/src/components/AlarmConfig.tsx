import { useEffect, useMemo, useRef, useState } from "react";
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

  const severityColor = (sev: AlarmRule["severity"]) =>
    sev === "critical" ? "#ef4444" : "#eab308";

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        fontSize: 12,
      }}
    >
      {/* SCADA Alarms */}
      <div
        style={{
          padding: "10px 12px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div
          style={{
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
            marginBottom: 8,
          }}
        >
          SCADA Alarms
        </div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginBottom: 8,
          }}
        >
          <button
            onClick={toggleScada}
            style={{
              width: 36,
              height: 20,
              borderRadius: 10,
              border: "none",
              background: scadaEnabled ? "#22c55e" : "var(--border)",
              cursor: "pointer",
              position: "relative",
              flexShrink: 0,
              transition: "background 0.15s",
            }}
          >
            <span
              style={{
                position: "absolute",
                top: 2,
                left: scadaEnabled ? 18 : 2,
                width: 16,
                height: 16,
                borderRadius: "50%",
                background: "#fff",
                transition: "left 0.15s",
              }}
            />
          </button>
          <span style={{ fontSize: 11, color: "var(--text)" }}>
            {scadaEnabled ? "Enabled" : "Disabled"}
          </span>
        </div>
        {scadaEnabled && (
          <input
            type="text"
            value={scadaUrl}
            onChange={(e) => setScadaUrl(e.target.value)}
            placeholder="https://scada.example.com/api"
            style={{
              width: "100%",
              padding: "4px 8px",
              border: "1px solid var(--border)",
              borderRadius: 4,
              background: "var(--bg)",
              color: "var(--text)",
              fontSize: 12,
              outline: "none",
              boxSizing: "border-box",
            }}
          />
        )}
      </div>

      {/* PLC Variable Alarms */}
      <div
        style={{
          flex: 1,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div
          style={{
            padding: "10px 12px 6px",
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <span>PLC Alarms ({alarms.length})</span>
          {!showForm && (
            <button
              onClick={() => setShowForm(true)}
              style={{
                padding: "2px 8px",
                border: "1px solid var(--border)",
                borderRadius: 4,
                background: "transparent",
                color: "var(--accent)",
                fontSize: 11,
                cursor: "pointer",
              }}
            >
              + Add
            </button>
          )}
        </div>

        {/* Add alarm form */}
        {showForm && (
          <div
            style={{
              padding: "6px 12px 10px",
              borderBottom: "1px solid var(--border)",
            }}
          >
            <div style={{ marginBottom: 6 }}>
              <select
                value={formVar}
                onChange={(e) => setFormVar(e.target.value)}
                style={{
                  width: "100%",
                  padding: "4px 8px",
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  background: "var(--bg)",
                  color: "var(--text)",
                  fontSize: 12,
                  outline: "none",
                  boxSizing: "border-box",
                }}
              >
                <option value="">-- select variable --</option>
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
                placeholder="Or type variable name..."
                style={{
                  width: "100%",
                  padding: "4px 8px",
                  marginTop: 4,
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  background: "var(--bg)",
                  color: "var(--text)",
                  fontSize: 12,
                  outline: "none",
                  boxSizing: "border-box",
                }}
              />
            </div>
            <div style={{ marginBottom: 6 }}>
              <select
                value={formSeverity}
                onChange={(e) =>
                  setFormSeverity(e.target.value as "critical" | "warning")
                }
                style={{
                  width: "100%",
                  padding: "4px 8px",
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  background: "var(--bg)",
                  color: "var(--text)",
                  fontSize: 12,
                  outline: "none",
                  boxSizing: "border-box",
                }}
              >
                <option value="warning">Warning</option>
                <option value="critical">Critical</option>
              </select>
            </div>
            <div style={{ marginBottom: 6 }}>
              <input
                type="text"
                value={formMessage}
                onChange={(e) => setFormMessage(e.target.value)}
                placeholder="Custom message (optional)"
                style={{
                  width: "100%",
                  padding: "4px 8px",
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  background: "var(--bg)",
                  color: "var(--text)",
                  fontSize: 12,
                  outline: "none",
                  boxSizing: "border-box",
                }}
              />
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <button
                onClick={handleAdd}
                disabled={!formVar.trim()}
                style={{
                  flex: 1,
                  padding: "4px 0",
                  border: "1px solid var(--accent)",
                  borderRadius: 4,
                  background: formVar.trim() ? "var(--accent)" : "var(--surface)",
                  color: formVar.trim() ? "#000" : "var(--text-muted)",
                  fontSize: 11,
                  cursor: formVar.trim() ? "pointer" : "default",
                }}
              >
                Add
              </button>
              <button
                onClick={() => {
                  setShowForm(false);
                  setFormVar("");
                  setFormMessage("");
                }}
                style={{
                  padding: "4px 10px",
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  background: "transparent",
                  color: "var(--text)",
                  fontSize: 11,
                  cursor: "pointer",
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Alarm list */}
        <div style={{ flex: 1, overflowY: "auto", padding: "0 12px 8px" }}>
          {alarms.length === 0 ? (
            <div
              style={{
                color: "var(--text-muted)",
                fontSize: 11,
                textAlign: "center",
                padding: "12px 0",
                opacity: 0.6,
              }}
            >
              No alarm rules configured.
            </div>
          ) : (
            alarms.map((alarm) => (
              <div
                key={alarm.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "5px 6px",
                  borderRadius: 4,
                  marginBottom: 2,
                  background: "var(--surface)",
                  border: "1px solid var(--border)",
                }}
              >
                <span
                  style={{
                    fontSize: 10,
                    fontWeight: 600,
                    padding: "1px 5px",
                    borderRadius: 3,
                    background: severityColor(alarm.severity),
                    color: alarm.severity === "critical" ? "#fff" : "#000",
                    flexShrink: 0,
                    textTransform: "uppercase",
                  }}
                >
                  {alarm.severity}
                </span>
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    color: "var(--text)",
                    fontWeight: 500,
                  }}
                  title={alarm.variableName}
                >
                  {alarm.variableName}
                </span>
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-muted)",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    maxWidth: 60,
                  }}
                  title={alarm.message}
                >
                  {alarm.message}
                </span>
                <button
                  onClick={() => removeAlarm(alarm.id)}
                  title="Delete alarm"
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--text-muted)",
                    cursor: "pointer",
                    padding: "0 2px",
                    fontSize: 14,
                    lineHeight: 1,
                    flexShrink: 0,
                  }}
                >
                  &times;
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
