import { useEffect, useState } from "react";
import { bridgeMode, invoke } from "../../api/bridge";
import { useStore } from "../../store";
import { Button, Field, Segmented, StatusDot } from "@mactools/ui";
import { CloudUpload, Radar, Plug, Unplug } from "lucide-react";
import type { DeployResult, DiscoveredTarget, TargetStatus } from "../../types";
import { compareCrc, formatCrcHex, type DriftInfo } from "./compareCrc";

const TARGET_PRESETS = [
  { label: "192.168.1.100", ip: "192.168.1.100" },
  { label: "192.168.0.100", ip: "192.168.0.100" },
  { label: "10.0.0.1", ip: "10.0.0.1" },
];

const DRIFT_TONE: Record<DriftInfo["status"], "success" | "warn" | "neutral"> = {
  sync: "success",
  drift: "warn",
  unknown: "neutral",
};

const DRIFT_LABEL: Record<DriftInfo["status"], string> = {
  sync: "Running program in sync with project",
  drift: "Running program differs from project",
  unknown: "Running program version unknown",
};

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
  const isCloud = bridgeMode === "cloud";
  const [connecting, setConnecting] = useState(false);
  const [cloudDeploying, setCloudDeploying] = useState(false);
  // Relay device identity (cloud mode): the bridge reads this for every
  // target/deploy command; the desktop flow never touches it.
  const [deviceId, setDeviceId] = useState(() => localStorage.getItem("vplc_device") ?? "");

  const [drift, setDrift] = useState<DriftInfo>({ status: "unknown", projectCrc: null, runningCrc: null });

  // Drift detection (PLC-6): poll the target every 10s while connected and
  // compare the project's last-deployed CRC with the target's loaded CRC.
  useEffect(() => {
    if (!isRemote || !targetConnected) {
      setDrift({ status: "unknown", projectCrc: null, runningCrc: null });
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const status = await invoke<TargetStatus>("target_status");
        if (!cancelled) {
          setDrift({
            status: compareCrc(status.deployed_crc32, status.running_crc32),
            projectCrc: status.deployed_crc32 || null,
            runningCrc: status.running_crc32 || null,
          });
        }
      } catch {
        if (!cancelled) setDrift({ status: "unknown", projectCrc: null, runningCrc: null });
      }
    };
    void poll();
    const id = setInterval(poll, 10_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [isRemote, targetConnected]);

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

  // Cloud deploy (PLC-23): compile via the sim host, ship the module over the
  // relay, then read the status back so the drift badge reflects the target.
  // Desktop keeps store.targetDeploy, which uses the native deploy commands.
  const cloudDeploy = async () => {
    setCloudDeploying(true);
    const { addLog } = useStore.getState();
    try {
      await useStore.getState().compile();
      const result = useStore.getState().lastCompileResult;
      if (!result?.success || !result.module) {
        addLog("system", "Deploy failed: compilation errors");
        return;
      }
      await invoke<DeployResult>("deploy_to_target", {
        ip: targetIp.trim(),
        ioMappings: useStore.getState().ioMappings,
        module: result.module,
      });
      const status = await invoke<TargetStatus>("target_status");
      setDrift({
        status: compareCrc(status.deployed_crc32, status.running_crc32),
        projectCrc: status.deployed_crc32 || null,
        runningCrc: status.running_crc32 || null,
      });
    } catch (e) {
      addLog("system", `Deploy failed: ${e}`);
    } finally {
      setCloudDeploying(false);
    }
  };

  const driftTone = DRIFT_TONE[drift.status];
  const driftColor = drift.status === "sync" ? "var(--accent-text)" : drift.status === "drift" ? "var(--warn)" : "var(--text-muted)";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, fontSize: "var(--fs-sm)" }}>
      {/* Execution Mode */}
      <Field label="Execution mode">
        <Segmented
          block
          label="Execution mode"
          value={executionMode}
          onChange={(m) => setExecutionMode(m)}
          options={[
            { value: "local", label: "Local" },
            { value: "remote", label: "Remote" },
          ]}
        />
      </Field>

      {/* Target Connection */}
      <Field label="Target device">
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <Segmented
            block
            label="Target transport"
            value={targetKind}
            onChange={(k) => setTargetKind(k)}
            options={[
              { value: "websocket", label: "ESP32 (WS)" },
              { value: "grpc", label: "Linux (gRPC)" },
            ]}
          />
          {isCloud && (
            <Field label="Relay device ID" htmlFor="target-device-id">
              <input
                id="target-device-id"
                type="text"
                className="vt-input vt-input--mono"
                placeholder="device id from the cloud pairing"
                value={deviceId}
                onChange={(e) => {
                  setDeviceId(e.target.value);
                  if (e.target.value) localStorage.setItem("vplc_device", e.target.value);
                  else localStorage.removeItem("vplc_device");
                }}
                disabled={targetConnected}
              />
            </Field>
          )}
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              type="text"
              className="vt-input vt-input--mono"
              style={{ flex: 1 }}
              placeholder="192.168.1.100"
              value={targetIp}
              onChange={(e) => setTargetIp(e.target.value)}
              disabled={targetConnected}
              aria-label="Target IP address"
            />
            <StatusDot tone={targetConnected ? "success" : "neutral"} label={targetConnected ? "Connected" : "Disconnected"} />
          </div>
          <Button
            size="sm"
            variant="default"
            icon={<Radar size={14} strokeWidth={1.75} />}
            onClick={scanTargets}
            disabled={scanningTargets || targetConnected}
          >
            {scanningTargets ? "Scanning..." : "Scan network"}
          </Button>
          {discoveredTargets.length > 0 && !targetConnected && (
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              {discoveredTargets.map((t, i) => (
                <button
                  key={i}
                  className="vt-btn vt-btn--sm vt-btn--ghost"
                  style={{ justifyContent: "flex-start", fontFamily: "var(--font-mono)" }}
                  onClick={() => setTargetIp(t.ip)}
                >
                  {t.name.split(".")[0]} ({t.ip})
                </button>
              ))}
            </div>
          )}
          {discoveredTargets.length === 0 && !scanningTargets && !targetConnected && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
              {TARGET_PRESETS.map((p) => (
                <Button
                  key={p.ip}
                  size="sm"
                  variant="ghost"
                  style={{ flex: "1 1 auto", fontFamily: "var(--font-mono)" }}
                  onClick={() => setTargetIp(p.ip)}
                  disabled={targetConnected}
                >
                  {p.label}
                </Button>
              ))}
            </div>
          )}
          <Button
            variant={targetConnected ? "danger" : "primary"}
            block
            icon={targetConnected ? <Unplug size={14} strokeWidth={1.75} /> : <Plug size={14} strokeWidth={1.75} />}
            loading={connecting}
            onClick={handleConnect}
            disabled={(isCloud ? !deviceId.trim() : !targetIp.trim()) || connecting}
          >
            {connecting ? "Connecting..." : targetConnected ? "Disconnect" : "Connect"}
          </Button>
        </div>
      </Field>

      {/* Deploy */}
      <Field label="Deploy program">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <Button
            variant="primary"
            block
            icon={<CloudUpload size={14} strokeWidth={1.75} />}
            loading={isCloud ? cloudDeploying : targetDeploying}
            onClick={isCloud ? cloudDeploy : targetDeploy}
            disabled={!isRemote || !targetConnected || (isCloud ? cloudDeploying : targetDeploying)}
          >
            {(isCloud ? cloudDeploying : targetDeploying) ? "Deploying..." : "Build & deploy"}
          </Button>
          {isRemote && targetConnected && (
            <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: "var(--fs-xs)" }}>
              <StatusDot tone={driftTone} />
              <span style={{ color: driftColor }}>
                {DRIFT_LABEL[drift.status]}
                {drift.status === "drift" &&
                  ` (${formatCrcHex(drift.projectCrc ?? 0)} vs ${formatCrcHex(drift.runningCrc ?? 0)})`}
              </span>
            </div>
          )}
        </div>
      </Field>

      {/* SCADA Connection */}
      <Field label="Connect to SCADA">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <input
            type="text"
            className="vt-input vt-input--mono"
            placeholder="http://localhost:3000"
            value={scadaUrl}
            onChange={(e) => setScadaUrl(e.target.value)}
            aria-label="SCADA server URL"
          />
          <Button
            variant="default"
            block
            onClick={connectScada}
            disabled={!targetConnected || !scadaUrl.trim()}
          >
            Register in SCADA
          </Button>
          {targetConnected && scadaUrl && (
            <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
              Device: ESP32-{targetIp} | Transport: websocket
            </div>
          )}
        </div>
      </Field>
    </div>
  );
}
