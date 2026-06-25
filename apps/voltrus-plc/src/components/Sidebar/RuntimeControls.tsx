import { useStore } from "../../store";

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
    <div style={{ padding: 12, fontSize: 12 }}>
      {/* Remote runtime controls */}
      {isRemote && targetConnected && (
        <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
          <button
            className="io-btn"
            style={{
              flex: 1,
              background: runtimeState === "running" ? "var(--danger)" : "var(--accent)",
              color: "#fff",
            }}
            onClick={runtimeState === "running" ? targetStop : targetRun}
          >
            {runtimeState === "running" ? "Stop" : "Run"}
          </button>
          <button className="io-btn" style={{ flex: 1 }} onClick={targetStep}>
            Step
          </button>
        </div>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <span style={{ color: "var(--text-muted)" }}>State:</span>
        <span
          style={{
            color:
              runtimeState === "running"
                ? "var(--accent)"
                : runtimeState === "error"
                  ? "var(--danger)"
                  : "var(--text)",
            fontWeight: 600,
          }}
        >
          {runtimeState.toUpperCase()}
        </span>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <span style={{ color: "var(--text-muted)" }}>Cycles:</span>
        <span style={{ fontFamily: '"SF Mono", monospace' }}>{cycleCount}</span>
      </div>

      {cycleCount > 0 && (
        <div style={{ marginBottom: 8, fontSize: 11, color: "var(--text-muted)" }}>
          Last: {lastCycleMs.toFixed(2)}ms | Avg: {avgCycleMs.toFixed(2)}ms
        </div>
      )}

      <CycleChart />

      <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 8 }}>
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>Scan:</span>
        <input
          className="scan-cycle-input"
          type="number"
          min={1}
          max={60000}
          value={scanCycleMs}
          onChange={(e) => setScanCycle(Number(e.target.value))}
          disabled={runtimeState === "running"}
        />
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>ms</span>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}>
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>Watchdog:</span>
        <input
          className="scan-cycle-input"
          type="number"
          min={10}
          max={60000}
          value={maxCycleMs}
          onChange={(e) => setMaxCycle(Number(e.target.value))}
          disabled={runtimeState === "running"}
        />
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>ms</span>
      </div>
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
    <div style={{ marginTop: 4 }}>
      <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 2 }}>
        Cycle Time (ms)
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          height: 40,
          gap: 1,
          background: "var(--surface)",
          borderRadius: 3,
          padding: "2px 4px",
        }}
      >
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
                    : "var(--border)",
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
