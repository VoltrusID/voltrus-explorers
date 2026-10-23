import { Activity, RefreshCw } from "lucide-react";
import { Button, Callout, EmptyState, Panel, StatusDot } from "@mactools/ui";
import { useDiagnostics, type LogDir } from "./useDiagnostics";

interface Props {
  deviceId: string;
  connected: boolean;
  deviceLabel: string;
  onLog: (dir: LogDir, msg: string) => void;
}

export default function DiagnosticsPanel({ deviceId, connected, deviceLabel, onLog }: Props) {
  const { cpuInfo, plcMode, diagBuffer, loading, error, refreshAll, modeTone } = useDiagnostics(deviceId, connected, deviceLabel, onLog);

  return (
    <div style={{ padding: 12, overflow: "auto", height: "100%", display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
        <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>{deviceLabel} diagnostics</span>
        <Button
          size="sm"
          variant="ghost"
          icon={<RefreshCw size={14} strokeWidth={1.75} style={loading ? { animation: "vt-spin 700ms linear infinite" } : undefined} />}
          disabled={!connected || loading}
          onClick={refreshAll}
        >
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>

      {error && <Callout tone="danger">{error}</Callout>}

      {!connected && (
        <EmptyState
          icon={<Activity size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a PLC to view diagnostics."
        />
      )}

      {connected && (
        <>
          <Panel title="Operating mode" style={{ flexShrink: 0 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: 14 }}>
              <div style={{ fontSize: "var(--fs-xl)", fontWeight: 700, color: modeTone === "success" ? "var(--success)" : modeTone === "danger" ? "var(--danger)" : "var(--text-muted)" }}>
                {plcMode?.mode ?? "—"}
              </div>
              <StatusDot tone={modeTone} label={`PLC mode ${plcMode?.mode ?? "unknown"}`} />
            </div>
          </Panel>

          <Panel title="CPU information" style={{ flexShrink: 0 }}>
            {cpuInfo ? (
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10, padding: 14 }}>
                <div><div className="vt-label">Module</div><div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{cpuInfo.module_type || "—"}</div></div>
                <div><div className="vt-label">Firmware</div><div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{cpuInfo.firmware_version || "—"}</div></div>
                <div><div className="vt-label">Serial</div><div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{cpuInfo.serial_number || "—"}</div></div>
                <div><div className="vt-label">AS name</div><div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{cpuInfo.as_name || "—"}</div></div>
                <div style={{ gridColumn: "1 / -1" }}><div className="vt-label">Module name</div><div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{cpuInfo.module_name || "—"}</div></div>
              </div>
            ) : (
              <div style={{ padding: 14, color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>No CPU info available</div>
            )}
          </Panel>

          <Panel title={`Diagnostic buffer (${diagBuffer?.entry_count ?? 0} entries)`} style={{ flexShrink: 0 }}>
            {diagBuffer && diagBuffer.entries.length > 0 ? (
              <div style={{ padding: 14, overflowX: "auto", flexShrink: 0 }}>
                <table className="vt-table">
                  <thead>
                    <tr>
                      <th style={{ width: 64 }}>Event</th>
                      <th style={{ width: 160 }}>Timestamp</th>
                      <th>Description</th>
                    </tr>
                  </thead>
                  <tbody>
                    {diagBuffer.entries.map((entry, i) => (
                      <tr key={i}>
                        <td className="mono">{`0x${entry.event_number.toString(16).padStart(4, "0").toUpperCase()}`}</td>
                        <td style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>{entry.timestamp}</td>
                        <td style={{ color: "var(--text)" }}>{entry.event_text}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <div style={{ padding: 14, color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>No diagnostic entries</div>
            )}
          </Panel>
        </>
      )}
    </div>
  );
}
