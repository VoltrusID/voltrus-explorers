import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface CpuInfo {
  module_type: string;
  serial_number: string;
  as_name: string;
  module_name: string;
  firmware_version: string;
}

export interface PlcMode {
  mode: string;
}

export interface DiagEntry {
  event_number: number;
  timestamp: string;
  event_text: string;
}

export interface DiagBuffer {
  entry_count: number;
  entries: DiagEntry[];
}

export type LogDir = "info" | "rx" | "tx" | "error";

/** CPU information, operating mode and diagnostic buffer of one connected PLC, loaded when connected and on `refreshAll`. */
export function useDiagnostics(deviceId: string, connected: boolean, deviceLabel: string, onLog: (dir: LogDir, msg: string) => void) {
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

  const modeTone: "success" | "danger" | "neutral" = plcMode?.mode === "RUN" ? "success" : plcMode?.mode === "STOP" ? "danger" : "neutral";

  return { cpuInfo, plcMode, diagBuffer, loading, error, refreshAll, modeTone };
}
