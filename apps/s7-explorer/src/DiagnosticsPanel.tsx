import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Activity, RefreshCw } from "lucide-react";
import { Button, Callout, EmptyState, Panel, StatusDot } from "@mactools/ui";

interface CpuInfo {
  module_type: string;
  serial_number: string;
  as_name: string;
  module_name: string;
  firmware_version: string;
}

interface PlcMode {
  mode: string;
}

interface DiagEntry {
  event_number: number;
  timestamp: string;
  event_text: string;
}

interface DiagBuffer {
  entry_count: number;
  entries: DiagEntry[];
}

type LogDir = "info" | "rx" | "tx" | "error";

interface Props {
  deviceId: string;
  connected: boolean;
  deviceLabel: string;
  onLog: (dir: LogDir, msg: string) => void;
}

export default function DiagnosticsPanel({ deviceId, connected, deviceLabel, onLog }: Props) {
  const [cpuInfo, setCpuInfo] = useState<CpuInfo | null>(null);
  const [plcMode, setPlcMode] = useState<PlcMode | null>(null);
  const [diagBuffer, setDiagBuffer] = useState<DiagBuffer | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const refreshAll = useCallback(async () => {
    if (!connected) return;
    setLoading(true);
    setError("");
    try {
      const [cpu, mode, diag] = await Promise.allSettled([
        invoke<CpuInfo>("get_cpu_info", { deviceId }),
        invoke<PlcMode>("get_plc_mode", { deviceId }),
        invoke<DiagBuffer>("get_diag_buffer", { deviceId }),
      ]);
      if (cpu.status === "fulfilled") setCpuInfo(cpu.value);
      else setCpuInfo(null);
      if (mode.status === "fulfilled") setPlcMode(mode.value);
      else setPlcMode(null);
      if (diag.status === "fulfilled") setDiagBuffer(diag.value);
      else setDiagBuffer(null);

      const errs = [cpu, mode, diag]
        .filter((r): r is PromiseRejectedResult => r.status === "rejected")
        .map((r) => String(r.reason));
      if (errs.length > 0) {
        setError(errs.join("; "));
        onLog("error", `[${deviceLabel}] Diagnostics: ${errs.join("; ")}`);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [connected, deviceId, deviceLabel, onLog]);

  useEffect(() => {
    if (connected) refreshAll();
    else {
      setCpuInfo(null);
      setPlcMode(null);
      setDiagBuffer(null);
    }
  }, [connected, refreshAll]);

  const modeTone = plcMode?.mode === "RUN" ? "success" : plcMode?.mode === "STOP" ? "danger" : "neutral";

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
