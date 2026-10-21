import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button, StatusDot } from "@mactools/ui";

interface SimSnapPoint {
  key: string;
  name: string;
  data_type: string;
  unit: string;
  value: number;
  display: string;
  writable: boolean;
  is_bool: boolean;
  decimals: number;
  min: number | null;
  max: number | null;
}

interface SimSnapshot {
  port: number;
  running: boolean;
  points: SimSnapPoint[];
}

const HISTORY = 60;

function Sparkline({ data, color }: { data: number[]; color: string }) {
  if (data.length < 2) return <svg width="100%" height={32} />;
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
  borderRadius: 8,
  padding: 12,
};

const sectionLabel: React.CSSProperties = {
  fontSize: 12,
  fontWeight: 600,
  color: "var(--text)",
  margin: "14px 0 8px",
};

export function EnipDevicePanel() {
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

  const sensors = useMemo(() => (snap?.points ?? []).filter((p) => !p.writable && !p.is_bool), [snap]);
  const boolSensors = useMemo(() => (snap?.points ?? []).filter((p) => !p.writable && p.is_bool), [snap]);
  const setpoints = useMemo(() => (snap?.points ?? []).filter((p) => p.writable && !p.is_bool), [snap]);
  const outputs = useMemo(() => (snap?.points ?? []).filter((p) => p.writable && p.is_bool), [snap]);

  const primary = sensors[0] ?? null;
  const modePoint = useMemo(
    () => (snap?.points ?? []).find((p) => p.writable && !p.is_bool && /^mode$/i.test(p.key)) ?? null,
    [snap]
  );

  const setAnalog = async (key: string, value: number) => {
    try { await invoke("sim_set_value", { key, value }); } catch {}
  };
  const toggleBool = async (key: string, active: boolean) => {
    try { await invoke("sim_set_bool", { key, active }); } catch {}
  };
  const stop = async () => {
    try { await invoke("sim_stop"); } catch {}
    await getCurrentWindow().close();
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <header style={{ padding: "12px 14px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", gap: 10 }}>
        <StatusDot tone={snap?.running ? "success" : "neutral"} label={snap?.running ? "Simulator running" : "Simulator idle"} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 14, fontWeight: 600 }}>EtherNet/IP Sim Device</div>
          <div className="mono" style={{ fontSize: 11, color: "var(--text-muted)" }}>
            CIP tags · listening :{snap?.port ?? "…"}
          </div>
        </div>
        {primary && (
          <div style={{ textAlign: "right" }}>
            <div className="mono" style={{ fontSize: 20, fontWeight: 700, color: "var(--accent-text)", fontVariantNumeric: "tabular-nums", lineHeight: 1 }}>
              {primary.display}
            </div>
          </div>
        )}
        <Button variant="danger" size="sm" onClick={stop}>Stop</Button>
      </header>

      {snap && modePoint && (
        <div style={{ display: "flex", gap: 6, padding: "12px 12px 0" }}>
          <SegmentedMode
            value={String(Math.round(modePoint.value))}
            onChange={(v) => setAnalog(modePoint.key, Number(v))}
          />
        </div>
      )}

      <div style={{ flex: 1, overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 10 }}>
        {!snap && (
          <div style={{ color: "var(--text-muted)", fontSize: 12, textAlign: "center", marginTop: 24 }}>
            Waiting for simulator data
          </div>
        )}

        {snap && sensors.length > 0 && (
          <>
            <div style={sectionLabel}>Sensors</div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              {sensors.map((p) => (
                <div key={p.key} style={{ ...cardStyle, position: "relative", overflow: "hidden" }}>
                  <div className="mono" style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>{p.key}</div>
                  <div className="mono" style={{ fontSize: 20, fontWeight: 700, color: "var(--accent-text)", fontVariantNumeric: "tabular-nums" }}>
                    {p.display}
                  </div>
                  {history[p.key] && history[p.key].length > 1 && (
                    <div style={{ marginTop: 6, opacity: 0.8 }}>
                      <Sparkline data={history[p.key]} color="var(--accent)" />
                    </div>
                  )}
                </div>
              ))}
            </div>
          </>
        )}

        {snap && boolSensors.length > 0 && (
          <>
            <div style={sectionLabel}>Status</div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              {boolSensors.map((p) => (
                <div key={p.key} style={cardStyle}>
                  <div className="mono" style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>{p.key}</div>
                  <div style={{ fontSize: 12, fontWeight: 700, color: p.value >= 0.5 ? "var(--accent-text)" : "var(--text-muted)" }}>
                    {p.value >= 0.5 ? "Active" : "Inactive"}
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {snap && setpoints.length > 0 && (
          <>
            <div style={sectionLabel}>Setpoints and commands</div>
            {setpoints.map((p) => (
              <div key={p.key} style={{ ...cardStyle, outline: flash === p.key ? "1px solid var(--accent)" : "none" }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 8 }}>
                  <span className="mono" style={{ fontSize: 12, color: "var(--text-dim)" }}>{p.key}</span>
                  <span className="mono" style={{ fontSize: 14, fontWeight: 700, fontVariantNumeric: "tabular-nums" }}>{p.display}</span>
                </div>
                <input
                  type="range"
                  min={p.min ?? 0}
                  max={p.max ?? 100}
                  step={p.decimals ? Math.pow(10, -p.decimals) : 1}
                  value={p.value}
                  onChange={(e) => setAnalog(p.key, Number(e.target.value))}
                  aria-label={p.key}
                  style={{ width: "100%", accentColor: "var(--accent)" }}
                />
              </div>
            ))}
          </>
        )}

        {snap && outputs.length > 0 && (
          <>
            <div style={sectionLabel}>Digital outputs</div>
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
                    width: "100%",
                    outline: flash === p.key ? "1px solid var(--accent)" : "none",
                    borderColor: active ? "var(--accent)" : "var(--border)",
                  }}
                >
                  <span className="mono" style={{ fontSize: 12 }}>{p.key}</span>
                  <span
                    style={{
                      fontSize: 11,
                      fontWeight: 700,
                      padding: "3px 10px",
                      borderRadius: 999,
                      background: active ? "var(--accent)" : "var(--border)",
                      color: active ? "var(--accent-fg)" : "var(--text-muted)",
                    }}
                  >
                    {active ? "ON" : "OFF"}
                  </span>
                </button>
              );
            })}
          </>
        )}
      </div>

      <footer style={{ padding: "8px 14px", borderTop: "1px solid var(--border)", fontSize: 11, color: "var(--text-muted)", display: "flex", justifyContent: "space-between" }}>
        <span>Point a client at this host, port {snap?.port ?? "44818"}</span>
        <span>{snap ? `${snap.points.length} tags` : ""}</span>
      </footer>
    </div>
  );
}

const MODES: Array<[string, string]> = [
  ["Off", "0"],
  ["Hand", "1"],
  ["Auto", "2"],
];

function SegmentedMode({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div role="radiogroup" aria-label="Mode" style={{ display: "flex", gap: 6, width: "100%" }}>
      {MODES.map(([label, val]) => {
        const active = value === val;
        return (
          <button
            key={val}
            role="radio"
            aria-checked={active}
            onClick={() => onChange(val)}
            style={{
              flex: 1,
              padding: "8px 0",
              borderRadius: 6,
              cursor: "pointer",
              fontSize: 12,
              fontWeight: 600,
              border: active ? "1px solid var(--accent)" : "1px solid var(--border)",
              background: active ? "var(--accent-soft)" : "transparent",
              color: active ? "var(--accent-text)" : "var(--text-muted)",
            }}
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}
