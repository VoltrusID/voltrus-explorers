import { useStore } from "../../store";
import { Badge, Button, Field } from "@mactools/ui";
import { Play, Square } from "lucide-react";

export default function RuntimeControls() {
  const {
    runtimeState,
    cycleCount,
    lastCycleMs,
    avgCycleMs,
    scanCycleMs,
    maxCycleMs,
    setScanCycle,
    setMaxCycle,
    executionMode,
    targetRun,
    targetStop,
    targetStep,
    targetConnected,
  } = useStore();

  const isRemote = executionMode === "remote";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--fs-sm)" }}>
      {/* Remote runtime controls */}
      {isRemote && targetConnected && (
        <div style={{ display: "flex", gap: 4 }}>
          <Button
            size="sm"
            variant={runtimeState === "running" ? "danger" : "primary"}
            icon={runtimeState === "running" ? <Square size={14} strokeWidth={1.75} /> : <Play size={14} strokeWidth={1.75} />}
            style={{ flex: 1 }}
            onClick={runtimeState === "running" ? targetStop : targetRun}
          >
            {runtimeState === "running" ? "Stop" : "Run"}
          </Button>
          <Button size="sm" variant="default" style={{ flex: 1 }} onClick={targetStep}>
            Step
          </Button>
        </div>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <div className="rt-row">
          <span className="vt-label">State</span>
          <Badge tone={runtimeState === "running" ? "success" : runtimeState === "error" ? "danger" : "neutral"}>
            <span className={`vt-dot ${runtimeState === "running" ? "vt-dot--success" : runtimeState === "error" ? "vt-dot--danger" : ""}`} />
            {runtimeState.charAt(0).toUpperCase() + runtimeState.slice(1)}
          </Badge>
        </div>
        <div className="rt-row">
          <span className="vt-label">Cycles</span>
          <span className="rt-value" style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)" }}>{cycleCount.toLocaleString()}</span>
        </div>
        {cycleCount > 0 && (
          <div className="rt-row">
            <span className="vt-label">Last / avg cycle</span>
            <span className="rt-value" style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>
              {lastCycleMs.toFixed(2)} / {avgCycleMs.toFixed(2)} ms
            </span>
          </div>
        )}
      </div>

      <CycleChart />

      <Field label="Scan interval">
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <input
            className="vt-input vt-input--sm"
            type="number"
            min={1}
            max={60000}
            value={scanCycleMs}
            onChange={(e) => setScanCycle(Number(e.target.value))}
            disabled={runtimeState === "running"}
            aria-label="Scan interval in milliseconds"
          />
          <span className="unit">ms</span>
        </div>
      </Field>

      <Field label="Watchdog">
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <input
            className="vt-input vt-input--sm"
            type="number"
            min={10}
            max={60000}
            value={maxCycleMs}
            onChange={(e) => setMaxCycle(Number(e.target.value))}
            disabled={runtimeState === "running"}
            aria-label="Watchdog in milliseconds"
          />
          <span className="unit">ms</span>
        </div>
      </Field>
    </div>
  );
}

function CycleChart() {
  const history = useStore((s) => s.cycleHistory);
  const maxCycleMs = useStore((s) => s.maxCycleMs);

  if (history.length === 0) return null;

  const MAX_BARS = 40;
  const display = history.slice(-MAX_BARS);
  const maxMs = Math.max(maxCycleMs * 1.1, ...display);
  const barWidth = Math.max(3, Math.floor(160 / MAX_BARS));

  return (
    <div>
      <div className="vt-label" style={{ marginBottom: 4 }}>
        Cycle time (ms)
      </div>
      <div className="cycle-spark">
        {display.map((ms, i) => {
          const height = Math.max(2, (ms / maxMs) * 34);
          const overBudget = ms > maxCycleMs;
          return (
            <div
              key={i}
              title={`${ms.toFixed(2)}ms`}
              style={{
                width: barWidth,
                height,
                background: overBudget
                  ? "var(--danger)"
                  : i === display.length - 1
                    ? "var(--accent)"
                    : "var(--border-light)",
                borderRadius: 1,
                transition: "height 0.05s",
              }}
            />
          );
        })}
      </div>
    </div>
  );
}
