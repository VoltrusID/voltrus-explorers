import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useRef } from "react";

export function useProtocol<TConfig, TTag, TValue>(protocolModule: string) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [values, setValues] = useState<TValue[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pollCount, setPollCount] = useState(0);
  const [latency, setLatency] = useState(0);
  const pollInterval = useRef<ReturnType<typeof setInterval> | null>(null);
  const aborted = useRef(false);

  const connect = useCallback(
    async (config: TConfig): Promise<boolean> => {
      setLoading(true);
      setError(null);
      aborted.current = false;
      try {
        await invoke("connect", { config });
        if (!aborted.current) {
          setConnected(true);
          return true;
        }
        return false;
      } catch (e) {
        if (!aborted.current) {
          setError(String(e));
          setConnected(false);
        }
        return false;
      } finally {
        if (!aborted.current) {
          setLoading(false);
        }
      }
    },
    []
  );

  const cancel = useCallback(() => {
    aborted.current = true;
    setLoading(false);
    setConnected(false);
    setError(null);
  }, []);

  const disconnect = useCallback(async () => {
    try {
      if (pollInterval.current) {
        clearInterval(pollInterval.current);
        pollInterval.current = null;
      }
      await invoke("disconnect");
    } catch {
      // ignore — may not be connected
    } finally {
      setConnected(false);
      setValues([]);
      setPollCount(0);
      setLoading(false);
    }
  }, []);

  const readTags = useCallback(async (tags: TTag[]) => {
    const start = performance.now();
    try {
      const result = await invoke<TValue[]>("read_tags", { tags });
      const elapsed = performance.now() - start;
      setValues(result);
      setLatency(Math.round(elapsed));
      setPollCount((c) => c + 1);
      setError(null);
      return result;
    } catch (e) {
      setError(String(e));
      return [];
    }
  }, []);

  const writeTag = useCallback(
    async (tag: TTag, value: TValue) => {
      try {
        await invoke("write_tag", { tag, value });
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    },
    []
  );

  const startPolling = useCallback(
    (tags: TTag[], intervalMs: number) => {
      if (pollInterval.current) clearInterval(pollInterval.current);
      pollInterval.current = setInterval(() => readTags(tags), intervalMs);
      readTags(tags);
    },
    [readTags]
  );

  const stopPolling = useCallback(() => {
    if (pollInterval.current) {
      clearInterval(pollInterval.current);
      pollInterval.current = null;
    }
  }, []);

  return {
    connected,
    setConnected,
    loading,
    values,
    error,
    pollCount,
    latency,
    connect,
    cancel,
    disconnect,
    readTags,
    writeTag,
    startPolling,
    stopPolling,
  };
}
