import { useState } from "react";
import { useStore } from "../../store";
import type { DiscoveredTarget } from "../../types";
import "./ToggleGroup.css";

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

  const targetConnect = useStore((s) => s.targetConnect);
  const targetDisconnect = useStore((s) => s.targetDisconnect);
  const targetDeploy = useStore((s) => s.targetDeploy);
  const connectScada = useStore((s) => s.connectScada);

  const discoveredTargets = useStore((s) => s.discoveredTargets);
  const scanningTargets = useStore((s) => s.scanningTargets);
  const scanTargets = useStore((s) => s.scanTargets);
  const targetKind = useStore((s) => s.targetKind);
  const setTargetKind = useStore((s) => s.setTargetKind);

  const isRemote = executionMode === "remote";
  const [connecting, setConnecting] = useState(false);

  const handleConnect = async () => {
    setConnecting(true);
    try {
      if (targetConnected) {
        await targetDisconnect();
      } else {
        await targetConnect();
      }
    } finally {
      setConnecting(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: "8px 0" }}>
      {/* Execution Mode */}
      <div className="io-section">
        <div className="io-label">Execution Mode</div>
        <div className="toggle-group">
          <button
            className={`toggle-group-btn ${!isRemote ? "active" : ""}`}
            onClick={() => setExecutionMode("local")}
          >
            Local
          </button>
          <button
            className={`toggle-group-btn ${isRemote ? "active" : ""}`}
            onClick={() => setExecutionMode("remote")}
          >
            Remote
          </button>
        </div>
      </div>

      {/* Target Connection */}
      <div className="io-section">
        <div className="io-label">Target Device</div>
        <div className="toggle-group" style={{ marginBottom: 4 }}>
          <button
            className={`toggle-group-btn ${targetKind === "websocket" ? "active" : ""}`}
            onClick={() => setTargetKind("websocket")}
            style={{ fontSize: 10, padding: "2px 4px" }}
          >
            ESP32 (WS)
          </button>
          <button
            className={`toggle-group-btn ${targetKind === "grpc" ? "active" : ""}`}
            onClick={() => setTargetKind("grpc")}
            style={{ fontSize: 10, padding: "2px 4px" }}
          >
            Linux (gRPC)
          </button>
        </div>
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
              background: targetConnected ? "var(--accent)" : "var(--text-muted)",
              flexShrink: 0,
            }}
          />
        </div>
        <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
          <button
            className="io-btn"
            style={{ fontSize: 10, padding: "2px 6px", flex: 1 }}
            onClick={scanTargets}
            disabled={scanningTargets || targetConnected}
          >
            {scanningTargets ? "Scanning..." : "Scan Network"}
          </button>
        </div>
        {discoveredTargets.length > 0 && !targetConnected && (
          <div style={{ display: "flex", flexDirection: "column", gap: 2, marginTop: 4 }}>
            {discoveredTargets.map((t, i) => (
              <button
                key={i}
                className="io-btn"
                style={{ fontSize: 10, padding: "2px 6px", textAlign: "left" }}
                onClick={() => setTargetIp(t.ip)}
              >
                {t.name.split(".")[0]} ({t.ip})
              </button>
            ))}
          </div>
        )}
        {discoveredTargets.length === 0 && !scanningTargets && !targetConnected && (
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
        )}
        <button
          className="io-btn"
          style={{
            width: "100%",
            marginTop: 6,
            background: targetConnected ? "var(--danger)" : "var(--accent)",
            color: "#fff",
          }}
          onClick={handleConnect}
          disabled={!targetIp.trim() || connecting}
        >
          {connecting ? "Connecting..." : targetConnected ? "Disconnect" : "Connect"}
        </button>
      </div>

      {/* Deploy */}
      <div className="io-section">
        <div className="io-label">Deploy Program</div>
        <button
          className="io-btn"
          style={{
            width: "100%",
            background: targetConnected ? "var(--accent)" : "var(--border)",
            color: "#fff",
          }}
          onClick={targetDeploy}
          disabled={!isRemote || !targetConnected || targetDeploying}
        >
          {targetDeploying ? "Deploying..." : "Build & Deploy"}
        </button>
      </div>

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
            background: "#7c3aed",
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
