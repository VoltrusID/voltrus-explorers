import { useStore } from "../../store";

const TARGET_PRESETS = [
  { label: "192.168.1.100", ip: "192.168.1.100" },
  { label: "192.168.0.100", ip: "192.168.0.100" },
  { label: "10.0.0.1", ip: "10.0.0.1" },
];

export default function TargetPanel() {
  const targetIp = useStore((s) => s.targetIp);
  const setTargetIp = useStore((s) => s.setTargetIp);
  const targetConnected = useStore((s) => s.targetConnected);
  const targetDeploying = useStore((s) => s.targetDeploying);
  const scadaUrl = useStore((s) => s.scadaUrl);
  const setScadaUrl = useStore((s) => s.setScadaUrl);
  const executionMode = useStore((s) => s.executionMode);
  const setExecutionMode = useStore((s) => s.setExecutionMode);
  const runtimeState = useStore((s) => s.runtimeState);
  const cycleCount = useStore((s) => s.cycleCount);
  const lastCycleMs = useStore((s) => s.lastCycleMs);
  const scanCycleMs = useStore((s) => s.scanCycleMs);

  const targetConnect = useStore((s) => s.targetConnect);
  const targetDisconnect = useStore((s) => s.targetDisconnect);
  const targetDeploy = useStore((s) => s.targetDeploy);
  const targetRun = useStore((s) => s.targetRun);
  const targetStop = useStore((s) => s.targetStop);
  const targetStep = useStore((s) => s.targetStep);
  const connectScada = useStore((s) => s.connectScada);

  const isRemote = executionMode === "remote";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: "8px 0" }}>
      {/* Execution Mode */}
      <div className="io-section">
        <div className="io-label">Execution Mode</div>
        <div style={{ display: "flex", gap: 4 }}>
          <button
            className={`bottom-tab ${!isRemote ? "active" : ""}`}
            style={{ flex: 1, fontSize: 11 }}
            onClick={() => setExecutionMode("local")}
          >
            Local
          </button>
          <button
            className={`bottom-tab ${isRemote ? "active" : ""}`}
            style={{ flex: 1, fontSize: 11 }}
            onClick={() => setExecutionMode("remote")}
          >
            Remote
          </button>
        </div>
      </div>

      {/* Target Connection */}
      <div className="io-section">
        <div className="io-label">Target Device</div>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <input
            type="text"
            className="io-input"
            style={{ flex: 1, fontSize: 12 }}
            placeholder="192.168.1.100"
            value={targetIp}
            onChange={(e) => setTargetIp(e.target.value)}
            disabled={targetConnected}
          />
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              background: targetConnected ? "#4ade80" : "#666",
              flexShrink: 0,
            }}
          />
        </div>
        <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
          {TARGET_PRESETS.map((p) => (
            <button
              key={p.ip}
              className="io-btn"
              style={{ fontSize: 10, padding: "2px 6px" }}
              onClick={() => setTargetIp(p.ip)}
              disabled={targetConnected}
            >
              {p.label}
            </button>
          ))}
        </div>
        <button
          className="io-btn"
          style={{
            width: "100%",
            marginTop: 6,
            background: targetConnected ? "#991b1b" : "#1d4ed8",
            color: "#fff",
          }}
          onClick={targetConnected ? targetDisconnect : targetConnect}
          disabled={!isRemote || !targetIp.trim()}
        >
          {targetConnected ? "Disconnect" : "Connect"}
        </button>
      </div>

      {/* Deploy */}
      <div className="io-section">
        <div className="io-label">Deploy Program</div>
        <button
          className="io-btn"
          style={{
            width: "100%",
            background: targetConnected ? "#15803d" : "#333",
            color: "#fff",
          }}
          onClick={targetDeploy}
          disabled={!isRemote || !targetConnected || targetDeploying}
        >
          {targetDeploying ? "Deploying..." : "Build & Deploy"}
        </button>
      </div>

      {/* Remote Runtime Controls */}
      {isRemote && targetConnected && (
        <div className="io-section">
          <div className="io-label">Runtime Controls</div>
          <div style={{ display: "flex", gap: 4 }}>
            <button
              className="io-btn"
              style={{
                flex: 1,
                background: runtimeState === "running" ? "#15803d" : "#1d4ed8",
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
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 4 }}>
            State: {runtimeState} | Cycles: {cycleCount} | Last: {lastCycleMs.toFixed(1)}ms
          </div>
          <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
            Cycle: {scanCycleMs}ms
          </div>
        </div>
      )}

      {/* SCADA Connection */}
      <div className="io-section">
        <div className="io-label">Connect to SCADA</div>
        <input
          type="text"
          className="io-input"
          style={{ width: "100%", fontSize: 12 }}
          placeholder="http://localhost:3000"
          value={scadaUrl}
          onChange={(e) => setScadaUrl(e.target.value)}
        />
        <button
          className="io-btn"
          style={{
            width: "100%",
            marginTop: 6,
            background: targetConnected ? "#7c3aed" : "#333",
            color: "#fff",
          }}
          onClick={connectScada}
          disabled={!targetConnected || !scadaUrl.trim()}
        >
          Register in SCADA
        </button>
        {targetConnected && scadaUrl && (
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 4 }}>
            Device: ESP32-{targetIp} | Transport: websocket
          </div>
        )}
      </div>
    </div>
  );
}
