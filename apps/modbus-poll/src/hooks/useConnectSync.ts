import { useMemo } from "react";
import { useConnectApp, type ConnectStorage } from "@mactools/ui";
import { saveConnProfiles } from "../helpers";
import { fromData, toLocal, upsertProfile } from "../connectProfiles";
import { useStore } from "../store";

/** Hands the saved connection profiles to Voltrus Connect. `loadProfile` is the sidebar's "load profile". */
export function useConnectSync(loadProfile: (name: string) => void) {
  const storage = useMemo<ConnectStorage>(
    () => ({
      list: () => useStore.getState().connProfiles.map(toLocal),
      upsert: ({ key, name, data }) => {
        const s = useStore.getState();
        const next = upsertProfile(s.connProfiles, key, fromData(name, data));
        saveConnProfiles(next);
        s.setConnProfiles(next);
        if (key !== undefined && key !== name && s.activeConnProfile === key) s.setActiveConnProfile(name);
        return name;
      },
      remove: (key) => {
        const s = useStore.getState();
        const next = s.connProfiles.filter((p) => p.name !== key);
        saveConnProfiles(next);
        s.setConnProfiles(next);
        if (s.activeConnProfile === key) {
          s.setActiveConnProfile(null);
          s.setProfileDirty(false);
        }
      },
    }),
    [],
  );

  useConnectApp({
    storage,
    subscribe: (notify) => useStore.subscribe((s, prev) => s.connProfiles !== prev.connProfiles && notify()),
    open: (p) => loadProfile(p.key),
  });
}
