import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { TagConfig } from "./types";

/** S7Value as serialized from the backend (untagged enum). */
export type S7Value =
  | boolean
  | number
  | string;

export interface TagSpec {
  address: string;
  data_type?: string;
  name?: string;
}

/**
 * One live S7 connection, scoped to a device id.
 *
 * Unlike the shared {@link useProtocol} hook, every command carries the device
 * id so multiple PLCs can be open simultaneously.
 */
export function useS7Device(deviceId: string) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [values, setValues] = useState<S7Value[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pollCount, setPollCount] = useState(0);
  const [latency, setLatency] = useState(0);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const connect = useCallback(
    async (config: { host: string; port: number; rack: number; slot: number }) => {
      setLoading(true);
      setError(null);
      try {
        await invoke("connect", { deviceId, config });
        setConnected(true);
        return true;
      } catch (e) {
        setError(String(e));
        setConnected(false);
        return false;
      } finally {
        setLoading(false);
      }
    },
    [deviceId],
  );

  const readTags = useCallback(
    async (tags: TagSpec[]): Promise<S7Value[]> => {
      const start = performance.now();
      try {
        const result = await invoke<S7Value[]>("read_tags", { deviceId, tags });
        setValues(result);
        setLatency(Math.round(performance.now() - start));
        setPollCount((c) => c + 1);
        setError(null);
        return result;
      } catch (e) {
        setError(String(e));
        return [];
      }
    },
    [deviceId],
  );

  const writeTag = useCallback(
    async (address: string, dataType: string, value: string | number | boolean) => {
      try {
        await invoke("write_tag", { deviceId, address, dataType, value });
        setError(null);
        return true;
      } catch (e) {
        setError(String(e));
        return false;
      }
    },
    [deviceId],
  );

  const startPolling = useCallback(
    (tags: TagSpec[], intervalMs: number) => {
      if (pollRef.current) clearInterval(pollRef.current);
      pollRef.current = setInterval(() => {
        void readTags(tags);
      }, intervalMs);
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
      /* ignore */
    }
    setConnected(false);
    setValues([]);
    setPollCount(0);
  }, [deviceId, stopPolling]);

  // Clean up on unmount / device switch.
  useEffect(() => {
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  return {
    connected,
    loading,
    values,
    error,
    pollCount,
    latency,
    connect,
    readTags,
    writeTag,
    startPolling,
    stopPolling,
    disconnect,
    setError,
  };
}

/** Convert a TagConfig list to TagSpec for read calls. */
export function toSpecs(tags: TagConfig[]): TagSpec[] {
  return tags.map((t) => ({ address: t.address, data_type: t.data_type, name: t.name }));
}
