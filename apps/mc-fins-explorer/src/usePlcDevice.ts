import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Device, TagConfig } from "./types";
import { connectConfig } from "./helpers";

/** A value as the backend returns it: a number, a bool, or text (Hex16, an invalid BCD word). */
export type PlcValue = boolean | number | string;

export interface TagSpec {
  address: string;
  data_type: string;
}

export function toSpecs(tags: TagConfig[]): TagSpec[] {
  return tags.map((t) => ({ address: t.address, data_type: t.data_type }));
}

/** One live connection, scoped to a device id so several PLCs can be open at once. */
export function usePlcDevice(deviceId: string) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [values, setValues] = useState<PlcValue[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pollCount, setPollCount] = useState(0);
  const [latency, setLatency] = useState(0);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const busy = useRef(false);

  const connect = useCallback(
    async (device: Device): Promise<string | null> => {
      setLoading(true);
      setError(null);
      try {
        const summary = await invoke<string>("connect", { deviceId, config: connectConfig(device) });
        setConnected(true);
        return summary;
      } catch (e) {
        setError(String(e));
        setConnected(false);
        return null;
      } finally {
        setLoading(false);
      }
    },
    [deviceId],
  );

  const readTags = useCallback(
    async (tags: TagSpec[]): Promise<PlcValue[]> => {
      // A slow PLC must not pile up requests behind each other.
      if (busy.current) return [];
      busy.current = true;
      const start = performance.now();
      try {
        const result = await invoke<PlcValue[]>("read_tags", { deviceId, tags });
        setValues(result);
        setLatency(Math.round(performance.now() - start));
        setPollCount((c) => c + 1);
        setError(null);
        return result;
      } catch (e) {
        setValues([]);
        setError(String(e));
        return [];
      } finally {
        busy.current = false;
      }
    },
    [deviceId],
  );

  const writeTag = useCallback(
    async (address: string, dataType: string, value: string | number | boolean): Promise<string | null> => {
      try {
        await invoke("write_tag", { deviceId, address, dataType, value });
        setError(null);
        return null;
      } catch (e) {
        return String(e);
      }
    },
    [deviceId],
  );

  const startPolling = useCallback(
    (tags: TagSpec[], intervalMs: number) => {
      if (pollRef.current) clearInterval(pollRef.current);
      pollRef.current = setInterval(() => void readTags(tags), intervalMs);
      void readTags(tags);
    },
    [readTags],
  );

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const disconnect = useCallback(async () => {
    stopPolling();
    try {
      await invoke("disconnect", { deviceId });
    } catch {
      /* already gone */
    }
    setConnected(false);
    setValues([]);
    setPollCount(0);
  }, [deviceId, stopPolling]);

  useEffect(() => () => { if (pollRef.current) clearInterval(pollRef.current); }, []);

  return { connected, loading, values, error, pollCount, latency, connect, readTags, writeTag, startPolling, stopPolling, disconnect, setError };
}
