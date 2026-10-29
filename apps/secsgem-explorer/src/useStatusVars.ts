import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { noteDeviceError, noteDeviceSuccess } from "@mactools/ui";
import type { SvRow, SvValue } from "./types";

/** Status variable names, the selection, the last values and the live poll of the selection. */
export function useStatusVars(connected: boolean, onError: (m: string) => void) {
  const [names, setNames] = useState<SvRow[]>([]);
  const [values, setValues] = useState<Map<number, SvValue>>(new Map());
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [live, setLive] = useState(false);
  const [intervalMs, setIntervalMs] = useState(1000);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [polls, setPolls] = useState(0);
  const [latency, setLatency] = useState(0);

  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const intervalRef = useRef(intervalMs);
  intervalRef.current = intervalMs;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    if (!connected) {
      setNames([]);
      setValues(new Map());
      setSelected(new Set());
      setLive(false);
      setError(null);
      setPolls(0);
    }
  }, [connected]);

  const read = useCallback(async (ids: number[]) => {
    const started = performance.now();
    const got = await invoke<SvValue[]>("gem_sv_read", { ids });
    setLatency(Math.round(performance.now() - started));
    setValues((prev) => {
      const next = new Map(prev);
      for (const v of got) next.set(v.id, v);
      return next;
    });
    setPolls((c) => c + 1);
    setError(null);
    noteDeviceSuccess();
  }, []);

  const loadNames = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setNames(await invoke<SvRow[]>("gem_sv_names"));
    } catch (e) {
      setError(String(e));
      noteDeviceError();
    } finally {
      setBusy(false);
    }
  }, []);

  const readSelected = useCallback(
    async (ids?: number[]) => {
      const list = ids ?? Array.from(selectedRef.current);
      if (list.length === 0) return;
      setBusy(true);
      try {
        await read(list);
      } catch (e) {
        setError(String(e));
        noteDeviceError();
      } finally {
        setBusy(false);
      }
    },
    [read],
  );

  useEffect(() => {
    if (!connected || !live) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const ids = Array.from(selectedRef.current);
      if (ids.length > 0) {
        try {
          await read(ids);
        } catch (e) {
          if (!alive) return;
          setError(String(e));
          onErrorRef.current(String(e));
          noteDeviceError();
        }
      }
      if (alive) timer = setTimeout(tick, Math.max(250, intervalRef.current));
    };
    void tick();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [connected, live, read]);

  const toggle = useCallback((id: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  return { names, values, selected, setSelected, toggle, live, setLive, intervalMs, setIntervalMs, error, busy, polls, latency, loadNames, readSelected };
}
