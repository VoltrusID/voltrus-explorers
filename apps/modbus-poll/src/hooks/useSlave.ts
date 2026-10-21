import { useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  saveCustomTemplates,
  initialValues,
  numRegisters,
  type DeviceProfile,
} from "../templates";
import type { SlaveLogEvent } from "../types";
import { useStore } from "../store";

export function useSlave() {
  const {
    slaveRunning, setSlaveRunning, slavePort, slaveRegCount, setSlaveRegCount,
    slaveUnitId, slavePollMs, setSlaveRegs,
    slaveTransport, slaveRtuPort, slaveRtuBaud, slaveRtuDataBits,
    slaveRtuStopBits, slaveRtuParity, slaveRtuVirtualPort, setSlaveVirtualPortPath,
    activeProfile, setActiveProfile, setSimValues, simRunning, setSimRunning,
    simSpeed, customTemplates, setCustomTemplates,
    setShowCustomDialog, customName, setCustomName, customRegs,
    setLockedRegs, addLog,
  } = useStore();

  const prevRegsRef = useRef<number[]>([]);

  // Slave log listener + register refresh
  useEffect(() => {
    const mode = useStore.getState().mode;
    if (mode !== "slave" || !slaveRunning) return;

    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<SlaveLogEvent>("slave-log", (event) => {
      if (cancelled) return;
      addLog("rx", event.payload.message);
    }).then((fn) => {
      if (!cancelled) { unlisten = fn; } else { fn(); }
    });

    const interval = setInterval(async () => {
      try {
        const regs = await invoke<number[]>("slave_read_registers", { start: 0, count: slaveRegCount });
        const prev = prevRegsRef.current;
        if (prev.length === regs.length) {
          for (let i = 0; i < regs.length; i++) {
            if (regs[i] !== prev[i]) {
              addLog("rx", `Reg ${i} changed: ${prev[i]} -> ${regs[i]} (0x${regs[i].toString(16).toUpperCase().padStart(4, "0")})`);
            }
          }
        }
        prevRegsRef.current = regs;
        setSlaveRegs(regs);
      } catch { /* slave may have stopped */ }
    }, slavePollMs);

    return () => {
      cancelled = true;
      unlisten?.();
      clearInterval(interval);
    };
  }, [slaveRunning, slaveRegCount, slavePollMs, addLog]);

  // Poll simulation values from Rust
  useEffect(() => {
    if (!activeProfile || !slaveRunning) return;
    const poll = setInterval(async () => {
      try {
        const vals = await invoke<number[]>("sim_read_values");
        if (vals.length > 0) setSimValues(vals);
      } catch { /* no profile loaded yet */ }
    }, 500);
    return () => clearInterval(poll);
  }, [activeProfile, slaveRunning]);

  /** Start the server for the configured transport; resolves with a description for the log. */
  const startServer = useCallback(async (registerCount: number): Promise<string> => {
    if (slaveTransport === "tcp") {
      await invoke("slave_start", { config: { port: slavePort, unit_id: slaveUnitId, register_count: registerCount, bind_addr: "0.0.0.0" } });
      return `Slave TCP server (unit ${slaveUnitId}) started on port ${slavePort} (${registerCount} registers)`;
    }
    if (slaveRtuVirtualPort) {
      const path = await invoke<string>("slave_start_rtu_virtual", { registerCount, unitId: slaveUnitId });
      setSlaveVirtualPortPath(path);
      return `Virtual RTU slave (unit ${slaveUnitId}) started at ${path} (${registerCount} registers)`;
    }
    if (!slaveRtuPort) throw new Error("No serial port selected");
    await invoke("slave_start_rtu", {
      serialConfig: { port_name: slaveRtuPort, baud_rate: slaveRtuBaud, data_bits: slaveRtuDataBits, stop_bits: slaveRtuStopBits, parity: slaveRtuParity },
      registerCount, unitId: slaveUnitId,
    });
    return `Slave RTU server (unit ${slaveUnitId}) started on ${slaveRtuPort} @ ${slaveRtuBaud} baud (${registerCount} registers)`;
  }, [slaveTransport, slavePort, slaveUnitId, slaveRtuVirtualPort, slaveRtuPort, slaveRtuBaud, slaveRtuDataBits, slaveRtuStopBits, slaveRtuParity]);

  const applyProfile = useCallback(async (profile: DeviceProfile) => {
    setActiveProfile(profile);
    setSimRunning(false);
    // A different profile has different registers; locks of the previous one must not carry over.
    setLockedRegs(new Set());
    const needed = profile.baseAddress + numRegisters(profile);
    setSlaveRegCount(needed);
    const vals = initialValues(profile);
    setSimValues(vals);
    if (slaveRunning) {
      if (needed !== slaveRegCount) {
        try {
          await invoke("slave_stop");
          await startServer(needed);
          addLog("info", `Slave restarted with ${needed} registers for profile`);
        } catch (e) {
          addLog("error", `Slave restart failed: ${String(e)}`);
          return;
        }
      }
      try {
        await invoke("sim_load_profile", { profile });
        addLog("info", `Loaded profile: ${profile.manufacturer} ${profile.model} (base ${profile.baseAddress}, ${numRegisters(profile)} u16 regs)`);
      } catch (e) {
        addLog("error", `Load profile failed: ${String(e)}`);
      }
    }
  }, [slaveRunning, slaveRegCount, startServer, addLog]);

  const toggleSimulation = useCallback(async () => {
    if (!activeProfile || !slaveRunning) return;
    if (simRunning) {
      try {
        await invoke("sim_stop");
        setSimRunning(false);
        addLog("info", "Simulation paused");
      } catch (e) {
        addLog("error", `Sim stop failed: ${String(e)}`);
      }
    } else {
      try {
        await invoke("sim_start", { profile: activeProfile, intervalMs: simSpeed });
        setSimRunning(true);
        addLog("info", `Simulation started (${simSpeed}ms tick)`);
      } catch (e) {
        addLog("error", `Sim start failed: ${String(e)}`);
      }
    }
  }, [simRunning, activeProfile, slaveRunning, simSpeed, addLog]);

  const saveCustomTemplate = useCallback(() => {
    if (!customName) return;
    const t: DeviceProfile = {
      id: `custom-${Date.now()}`,
      manufacturer: "Custom",
      model: customName,
      description: `Custom template: ${customName}`,
      baseAddress: 0,
      registers: customRegs,
    };
    const updated = [...customTemplates, t];
    setCustomTemplates(updated);
    saveCustomTemplates(updated);
    setShowCustomDialog(false);
    setCustomName("");
    addLog("info", `Saved custom template: ${customName}`);
  }, [customName, customRegs, customTemplates, addLog]);

  const handleSlaveStart = useCallback(async () => {
    try {
      addLog("info", await startServer(slaveRegCount));
      setSlaveRunning(true);
      const regs = await invoke<number[]>("slave_read_registers", { start: 0, count: slaveRegCount });
      setSlaveRegs(regs);
    } catch (e) {
      addLog("error", `Slave start failed: ${String(e)}`);
    }
  }, [startServer, slaveRegCount, addLog]);

  const handleSlaveStop = useCallback(async () => {
    try {
      // The backend stops the simulation together with the server.
      await invoke("slave_stop");
      setSimRunning(false);
      setLockedRegs(new Set());
      setSlaveRunning(false);
      setSlaveRegs([]);
      setSlaveVirtualPortPath("");
      addLog("info", "Slave server stopped");
    } catch (e) {
      addLog("error", `Slave stop failed: ${String(e)}`);
    }
  }, [addLog]);

  const handleSlaveRegWrite = useCallback(async (address: number, value: number) => {
    try {
      await invoke("slave_write_register", { address, value });
      setSlaveRegs((prev) => {
        const next = [...prev];
        if (address < next.length) next[address] = value;
        return next;
      });
      addLog("tx", `Local write reg ${address} = ${value} (0x${value.toString(16).toUpperCase().padStart(4, "0")})`);
    } catch (e) {
      addLog("error", `Write reg ${address} failed: ${String(e)}`);
    }
  }, [addLog]);

  return {
    applyProfile, toggleSimulation, saveCustomTemplate,
    handleSlaveStart, handleSlaveStop, handleSlaveRegWrite,
  };
}
