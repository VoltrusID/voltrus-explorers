import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ChevronLeft } from "lucide-react";
import { Badge, Button, EmptyState, StatusDot } from "@mactools/ui";

interface SimSnapPoint {
  key: string;
  label: string;
  object_type: string;
  object_instance: number;
  unit: string | null;
  value: number;
  display: string;
  writable: boolean;
  kind: string;
  decimals: number;
  min: number | null;
  max: number | null;
}

interface SimSnapshot {
  device_id: number;
  port: number;
  running: boolean;
  points: SimSnapPoint[];
}

const HISTORY = 60;

function Sparkline({ data, color }: { data: number[]; color: string }) {
  if (data.length < 2) {
    return <svg width="100%" height={32} />;
  }
  const w = 200;
  const h = 32;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const range = max - min || 1;
  const pts = data
    .map((v, i) => `${(i / (data.length - 1)) * w},${h - ((v - min) / range) * (h - 4) - 2}`)
    .join(" ");
  return (
    <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" style={{ display: "block" }}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

const cardStyle: React.CSSProperties = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
  padding: 12,
};

const sectionLabel: React.CSSProperties = {
  fontSize: "var(--fs-sm)",
  fontWeight: 600,
  color: "var(--text)",
  margin: "14px 0 8px",
};

/**
 * Simulator control panel. Desktop opens it in its own window; where there is only one window (iOS) the app
 * renders it inline and passes `onClose`, which returns to the app (the simulator keeps running).
 */
export function DevicePanel({ onClose }: { onClose?: () => void } = {}) {
  const [snap, setSnap] = useState<SimSnapshot | null>(null);
  const [history, setHistory] = useState<Record<string, number[]>>({});
  const [flash, setFlash] = useState<string | null>(null);

  useEffect(() => {
    let unlistenTick: (() => void) | null = null;
    let unlistenWrite: (() => void) | null = null;

    listen<SimSnapshot>("sim-tick", (e) => {
      setSnap(e.payload);
      setHistory((prev) => {
        const next = { ...prev };
        for (const p of e.payload.points) {
          const arr = (next[p.key] ?? []).slice(-(HISTORY - 1));
          arr.push(p.value);
          next[p.key] = arr;
        }
        return next;
      });
    }).then((f) => { unlistenTick = f; });

    listen<{ key: string; value: number }>("sim-written", (e) => {
      setFlash(e.payload.key);
      setTimeout(() => setFlash((cur) => (cur === e.payload.key ? null : cur)), 700);
    }).then((f) => { unlistenWrite = f; });

    return () => {
      unlistenTick?.();
      unlistenWrite?.();
    };
  }, []);

  const sensors = useMemo(() => (snap?.points ?? []).filter((p) => !p.writable), [snap]);
  const setpoints = useMemo(
    () => (snap?.points ?? []).filter((p) => p.writable && p.kind === "analog"),
    [snap],
  );
  const outputs = useMemo(
    () => (snap?.points ?? []).filter((p) => p.writable && p.kind === "bool"),
    [snap],
  );

  const setAnalog = async (key: string, value: number) => {
    try {
      await invoke("sim_set_value", { key, value });
    } catch {}
  };
  const toggleBool = async (key: string, active: boolean) => {
    try {
      await invoke("sim_set_bool", { key, active });
    } catch {}
  };

  const stop = async () => {
    try {
      await invoke("sim_stop");
    } catch {}
    if (onClose) onClose();
    else await getCurrentWindow().close();
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: onClose ? "100%" : "100vh", background: "var(--bg)", color: "var(--text)" }}>
      {/* Header */}
      <header style={{ padding: onClose ? "calc(12px + env(safe-area-inset-top)) 14px 12px 8px" : "12px 14px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", gap: 10 }}>
        {onClose && (
          <Button variant="ghost" icon={<ChevronLeft size={16} strokeWidth={1.75} />} onClick={onClose} aria-label="Back to BACnet Explorer">
            Back
          </Button>
        )}
        <StatusDot tone={snap?.running ? "success" : "neutral"} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: "var(--fs-lg)", fontWeight: 600 }}>Sim device {snap?.device_id ?? "…"}</div>
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            BACnet/IP · listening <span className="mono">:{snap?.port ?? "…"}</span>
          </div>
        </div>
        <Button variant="danger" size="sm" onClick={stop}>Stop</Button>
      </header>

      <div style={{ flex: 1, overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 10 }}>
        {!snap && (
          <EmptyState
            title="Waiting for simulator"
            description="Start a simulation from BACnet Explorer to populate this panel."
          />
        )}

        {snap && sensors.length > 0 && (
          <>
            <h2 style={sectionLabel}>Sensors</h2>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              {sensors.map((p) => {
                return (
                  <div key={p.key} style={{ ...cardStyle, position: "relative", overflow: "hidden" }}>
                    <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginBottom: 4 }}>{p.label}</div>
                    <div style={{ fontSize: "var(--fs-xl)", fontWeight: 700, color: "var(--text)", fontVariantNumeric: "tabular-nums" }}>
                      {primaryDisplay(p)}
                    </div>
                    {(p.kind === "analog") && history[p.key] && history[p.key].length > 1 && (
                      <div style={{ marginTop: 6, opacity: 0.8 }}>
                        <Sparkline data={history[p.key]} color="var(--accent)" />
                      </div>
                    )}
                    {p.kind === "bool" && (
                      <div style={{ marginTop: 6, fontSize: "var(--fs-xs)", fontWeight: 600, color: p.value >= 0.5 ? "var(--success)" : "var(--text-muted)" }}>
                        {p.value >= 0.5 ? "Active" : "Inactive"}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </>
        )}

        {snap && setpoints.length > 0 && (
          <>
            <h2 style={sectionLabel}>Setpoints and commands</h2>
            {setpoints.map((p) => (
              <div key={p.key} style={{ ...cardStyle, outline: flash === p.key ? "1px solid var(--accent)" : "none" }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 8 }}>
                  <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>{p.label}</span>
                  <span className="mono" style={{ fontSize: "var(--fs-md)", fontWeight: 700, fontVariantNumeric: "tabular-nums" }}>
                    {p.display}
                  </span>
                </div>
                <input
                  type="range"
                  min={p.min ?? 0}
                  max={p.max ?? 100}
                  step={p.decimals ? Math.pow(10, -p.decimals) : 1}
                  value={p.value}
                  onChange={(e) => setAnalog(p.key, Number(e.target.value))}
                  style={{ width: "100%", accentColor: "var(--accent)" }}
                />
              </div>
            ))}
          </>
        )}

        {snap && outputs.length > 0 && (
          <>
            <h2 style={sectionLabel}>Digital outputs</h2>
            {outputs.map((p) => {
              const active = p.value >= 0.5;
              return (
                <button
                  key={p.key}
                  onClick={() => toggleBool(p.key, !active)}
                  style={{
                    ...cardStyle,
                    cursor: "pointer",
                    textAlign: "left",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    outline: flash === p.key ? "1px solid var(--accent)" : "none",
                    borderColor: active ? "var(--accent)" : "var(--border)",
                  }}
                >
                  <span style={{ fontSize: "var(--fs-sm)" }}>{p.label}</span>
                  <Badge tone={active ? "success" : "neutral"}>{active ? "On" : "Off"}</Badge>
                </button>
              );
            })}
          </>
        )}
      </div>

      <footer style={{ padding: "8px 14px calc(8px + env(safe-area-inset-bottom))", borderTop: "1px solid var(--border)", fontSize: "var(--fs-xs)", color: "var(--text-muted)", display: "flex", justifyContent: "space-between" }}>
        <span>Connect from YABE{snap ? ` on UDP :${snap.port}` : ""}</span>
        <span>{snap ? `${snap.points.length} points` : ""}</span>
      </footer>
    </div>
  );
}

function primaryDisplay(p: SimSnapPoint): string {
  // Show just the magnitude for the big gauge; unit is in the sub-line.
  if (p.kind === "bool") return p.value >= 0.5 ? "Active" : "Inactive";
  return p.display;
}
