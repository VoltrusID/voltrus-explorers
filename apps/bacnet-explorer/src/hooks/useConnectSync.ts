import { useMemo, useRef } from "react";
import { useConnectApp, type ConnectStorage } from "@mactools/ui";
import { loadProfiles, normalizeData, onProfilesChanged, saveProfiles, upsertLocal, type BacnetProfileData } from "../profiles";
import { trendKey, type TrendEntry, type TrendSource } from "./useBacnetTrend";

interface Args {
  device: { ip: string; port: number; device_id: number } | null;
  series: TrendEntry[];
  /** Connects like the connection form does; resolves to whether it worked. */
  connect: (values: { ip: string; port: number; deviceId: number }) => Promise<boolean>;
  addSeries: (obj: { type: string; instance: number }, name: string) => void;
  setSource: (key: string, source: TrendSource) => void;
}

/** Hands the saved device profiles (a device plus its watch list) to Voltrus Connect. */
export function useConnectSync(args: Args) {
  const latest = useRef(args);
  latest.current = args;

  const storage = useMemo<ConnectStorage>(
    () => ({
      list: loadProfiles,
      upsert: ({ key, name, data }) => {
        saveProfiles(upsertLocal(loadProfiles(), key, name, data));
        return name;
      },
      remove: (key) => saveProfiles(loadProfiles().filter((p) => p.key !== key)),
    }),
    [],
  );

  useConnectApp({
    storage,
    subscribe: onProfilesChanged,
    captureCurrent: () => {
      const { device, series } = latest.current;
      if (!device) return null;
      const data: BacnetProfileData = {
        device,
        watch: series.map((s) => ({ type: s.obj.type, instance: s.obj.instance, name: s.name, source: s.source })),
      };
      return { name: `${device.ip} (device ${device.device_id})`, data: { ...data } };
    },
    open: async (profile) => {
      const data = normalizeData(profile.data);
      const ok = await latest.current.connect({ ip: data.device.ip, port: data.device.port, deviceId: data.device.device_id });
      if (!ok) return;
      for (const w of data.watch) {
        const key = trendKey(w.type, w.instance);
        if (!latest.current.series.some((s) => s.key === key)) latest.current.addSeries({ type: w.type, instance: w.instance }, w.name);
        if (w.source === "cov") latest.current.setSource(key, "cov");
      }
    },
  });
}
