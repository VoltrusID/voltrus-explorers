import { useCallback, useEffect } from "react";
import type { ConnProfile } from "../types";
import { saveConnProfiles, validateBlock } from "../helpers";
import { useStore } from "../store";

export function useConnectionProfiles(masterFormRef: React.RefObject<Record<string, string | number> | null>) {
  const {
    transport, tcpFormValues, setTcpFormValues,
    rtuPort, rtuBaud, rtuDataBits, rtuStopBits, rtuParity, rtuUnitId,
    blocks, connProfiles, setConnProfiles,
    activeConnProfile, setActiveConnProfile,
    profileDirty, setProfileDirty, formKey, setFormKey,
  } = useStore();

  const buildCurrentProfile = useCallback((): ConnProfile => ({
    name: "",
    transport,
    tcp: { host: String(tcpFormValues.host ?? "127.0.0.1"), port: Number(tcpFormValues.port ?? 502), unitId: Number(tcpFormValues.unitId ?? 1), pollInterval: Number(tcpFormValues.pollInterval ?? 1000) },
    rtu: { port: rtuPort, baudRate: rtuBaud, dataBits: rtuDataBits, stopBits: rtuStopBits, parity: rtuParity, unitId: rtuUnitId },
    blocks,
  }), [transport, tcpFormValues, rtuPort, rtuBaud, rtuDataBits, rtuStopBits, rtuParity, rtuUnitId, blocks]);

  const profileMatches = useCallback((a: ConnProfile, b: ConnProfile) =>
    a.transport === b.transport
    && JSON.stringify(a.tcp) === JSON.stringify(b.tcp)
    && JSON.stringify(a.rtu) === JSON.stringify(b.rtu)
    && JSON.stringify(a.blocks) === JSON.stringify(b.blocks)
  , []);

  const saveProfileAs = useCallback((name: string) => {
    if (!name.trim()) return;
    const profile = { ...buildCurrentProfile(), name: name.trim() };
    setConnProfiles((prev) => {
      const next = [...prev.filter((p) => p.name !== profile.name), profile];
      saveConnProfiles(next);
      return next;
    });
    setActiveConnProfile(profile.name);
    setProfileDirty(false);
  }, [buildCurrentProfile]);

  const saveProfileChanges = useCallback(() => {
    if (!activeConnProfile) return;
    saveProfileAs(activeConnProfile);
  }, [activeConnProfile, saveProfileAs]);

  const loadProfile = useCallback((name: string) => {
    const p = connProfiles.find((c) => c.name === name);
    if (!p) return;
    useStore.getState().setTransport(p.transport);
    const tcpVals = { host: p.tcp.host, port: p.tcp.port, unitId: p.tcp.unitId, pollInterval: p.tcp.pollInterval };
    if (masterFormRef.current) masterFormRef.current = tcpVals;
    setTcpFormValues(tcpVals);
    useStore.getState().setRtuPort(p.rtu.port);
    useStore.getState().setRtuBaud(p.rtu.baudRate);
    useStore.getState().setRtuDataBits(p.rtu.dataBits);
    useStore.getState().setRtuStopBits(p.rtu.stopBits);
    useStore.getState().setRtuParity(p.rtu.parity);
    useStore.getState().setRtuUnitId(p.rtu.unitId);
    // Profiles saved by older versions may hold blocks that can no longer be polled (e.g. type "string").
    const usable = p.blocks.filter((b) => validateBlock(b) === null);
    if (usable.length < p.blocks.length) {
      useStore.getState().addLog("error", `Profile '${p.name}': skipped ${p.blocks.length - usable.length} block(s) that can not be polled`);
    }
    useStore.getState().setBlocks(usable);
    setActiveConnProfile(p.name);
    setProfileDirty(false);
    setFormKey(formKey + 1);
  }, [connProfiles]);

  const deleteProfile = useCallback((name: string) => {
    setConnProfiles((prev) => {
      const next = prev.filter((p) => p.name !== name);
      saveConnProfiles(next);
      return next;
    });
    if (activeConnProfile === name) {
      setActiveConnProfile(null);
      setProfileDirty(false);
    }
  }, [activeConnProfile]);

  // Detect dirty state when config changes
  useEffect(() => {
    if (!activeConnProfile) { setProfileDirty(false); return; }
    const saved = connProfiles.find((p) => p.name === activeConnProfile);
    if (!saved) { setProfileDirty(false); return; }
    const current = buildCurrentProfile();
    setProfileDirty(!profileMatches(saved, current));
  }, [activeConnProfile, connProfiles, buildCurrentProfile, profileMatches, transport, tcpFormValues, rtuPort, rtuBaud, rtuDataBits, rtuStopBits, rtuParity, rtuUnitId, blocks]);

  return {
    buildCurrentProfile, profileMatches,
    saveProfileAs, saveProfileChanges, loadProfile, deleteProfile,
  };
}
