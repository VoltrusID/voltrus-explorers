import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Cpu, Play, RefreshCw, Square } from "lucide-react";
import { Button, Dialog, EmptyState, ProBadge, useProStatus } from "@mactools/ui";
import type { Device } from "./types";

interface PlcInfo {
  model: string;
  model_code?: string;
  version?: string;
  status?: string;
  mode?: string;
  error?: string;
  route: string;
}

interface Props {
  device: Device;
  connected: boolean;
  onLog: (dir: "info" | "rx" | "tx" | "error", msg: string) => void;
}

/** CPU model and status, and remote RUN / STOP. */
export default function PlcPanel({ device, connected, onLog }: Props) {
  const { requirePro } = useProStatus();
  const [info, setInfo] = useState<PlcInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirm, setConfirm] = useState<"run" | "stop" | null>(null);

  const refresh = async () => {
    setLoading(true);
    try {
      const i = await invoke<PlcInfo>("plc_info", { deviceId: device.id });
      setInfo(i);
      setError(null);
    } catch (e) {
      setInfo(null);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (connected) void refresh();
    else { setInfo(null); setError(null); }
  }, [connected, device.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const doRemote = async (what: "run" | "stop") => {
    setConfirm(null);
    try {
      await invoke(what === "run" ? "plc_run" : "plc_stop", { deviceId: device.id });
      onLog("tx", `[${device.label}] Remote ${what.toUpperCase()} accepted`);
      await refresh();
    } catch (e) {
      onLog("error", `[${device.label}] Remote ${what.toUpperCase()} failed: ${String(e)}`);
      setError(String(e));
    }
  };

  if (!connected) {
    return <EmptyState compact icon={<Cpu size={18} strokeWidth={1.75} />} title="Not connected" description="Connect to read the CPU model and status." />;
  }

  const rows: [string, string | undefined][] = [
    ["Model", info?.model],
    ["Model code", info?.model_code],
    ["Version", info?.version],
    ["Status", info?.status],
    ["Mode", info?.mode],
    ["Errors", info?.error],
    [device.protocol === "mc" ? "Route" : "Nodes", info?.route],
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 14, padding: 14, maxWidth: 560 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} loading={loading} onClick={() => void refresh()}>Read again</Button>
        <Button size="sm" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => requirePro("Remote RUN and STOP") && setConfirm("run")}>
          {device.protocol === "mc" ? "Remote RUN" : "RUN (MONITOR mode)"} <ProBadge />
        </Button>
        <Button size="sm" variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => requirePro("Remote RUN and STOP") && setConfirm("stop")}>
          {device.protocol === "mc" ? "Remote STOP" : "STOP (PROGRAM mode)"} <ProBadge />
        </Button>
      </div>
      {error && <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-sm)", lineHeight: 1.5 }}>{error}</div>}
      <table className="vt-table">
        <tbody>
          {rows.filter(([, v]) => v !== undefined && v !== "").map(([k, v]) => (
            <tr key={k}>
              <td style={{ width: 140, color: "var(--text-muted)" }}>{k}</td>
              <td className="mono">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {device.protocol === "mc" && (
        <p style={{ margin: 0, fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
          Writes while the CPU runs and remote RUN/STOP need online change allowed in the PLC's Ethernet settings ("Enable online change" in GX Works2). Remote RUN also needs the CPU's RUN/STOP switch in RUN.
        </p>
      )}
      <Dialog
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title={confirm === "stop" ? `Stop ${device.label}?` : `Run ${device.label}?`}
        width={400}
        footer={
          <>
            <Button onClick={() => setConfirm(null)}>Cancel</Button>
            <Button variant={confirm === "stop" ? "danger" : "primary"} onClick={() => confirm && void doRemote(confirm)}>
              {confirm === "stop" ? "Stop the PLC" : "Run the PLC"}
            </Button>
          </>
        }
      >
        <p style={{ margin: 0, fontSize: "var(--fs-md)", color: "var(--text-dim)", lineHeight: 1.5 }}>
          {confirm === "stop"
            ? "The program stops executing and outputs turn off. Make sure the machine is in a safe state."
            : "The program starts executing and drives the outputs. Make sure the machine is safe to start."}
        </p>
      </Dialog>
    </div>
  );
}
