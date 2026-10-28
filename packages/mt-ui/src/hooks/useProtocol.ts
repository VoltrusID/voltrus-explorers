import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useRef } from "react";
import { noteDeviceError, noteDeviceSuccess } from "../review";

export interface WriteResult {
  ok: boolean;
  /** Set when `ok` is false. */
  error?: string;
}

export function useProtocol<TConfig, TTag, TValue>(protocolModule: string) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [values, setValues] = useState<TValue[]>([]);
  const [error, setError] = useState<string | null>(null);
  /** The most recent read failed, so `values` (if any) no longer reflect the device. */
  const [stale, setStale] = useState(false);
  const [pollCount, setPollCount] = useState(0);
  const [latency, setLatency] = useState(0);
  const pollTimer = useRef<number | null>(null);
  /** Bumped whenever polling starts, restarts or stops; results of older reads are dropped. */
  const generation = useRef(0);
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
          noteDeviceError();
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

  const stopPolling = useCallback(() => {
    generation.current += 1;
    if (pollTimer.current) {
      window.clearTimeout(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  const disconnect = useCallback(async () => {
    try {
      stopPolling();
      await invoke("disconnect");
    } catch {
      // ignore — may not be connected
    } finally {
      setConnected(false);
      setValues([]);
      setStale(false);
      setPollCount(0);
      setLoading(false);
    }
  }, [stopPolling]);

  const readTags = useCallback(async (tags: TTag[]) => {
    const start = performance.now();
    const gen = generation.current;
    try {
      const result = await invoke<TValue[]>("read_tags", { tags });
      // Polling was restarted or stopped while this read was in flight: it describes an old tag list.
      if (gen !== generation.current) return result;
      setValues(result);
      setLatency(Math.round(performance.now() - start));
      setPollCount((c) => c + 1);
      setError(null);
      setStale(false);
      if (tags.length > 0) noteDeviceSuccess();
      return result;
    } catch (e) {
      if (gen === generation.current) {
        setError(String(e));
        setStale(true);
        noteDeviceError();
      }
      return [];
    }
  }, []);

  /** Outcome of a write: `ok`, or the failure message (also stored in `error`). */
  const writeTag = useCallback(
    async (tag: TTag, value: TValue): Promise<WriteResult> => {
      try {
        await invoke("write_tag", { tag, value });
        setError(null);
        return { ok: true };
      } catch (e) {
        const message = String(e);
        setError(message);
        return { ok: false, error: message };
      }
    },
    []
  );

  /**
   * Poll `tags` every `intervalMs`, measured from the start of one read to the start of the next.
   * A new read is only scheduled once the previous one finished, so slow devices never get
   * overlapping requests. Calling again replaces the running poll (and its tag list).
   */
  const startPolling = useCallback(
    (tags: TTag[], intervalMs: number) => {
      stopPolling();
      const gen = generation.current;
      const tick = async () => {
        const began = performance.now();
        await readTags(tags);
        if (gen !== generation.current) return;
        pollTimer.current = window.setTimeout(tick, Math.max(0, intervalMs - (performance.now() - began)));
      };
      void tick();
    },
    [readTags, stopPolling]
  );

  return {
    connected,
    setConnected,
    loading,
    values,
    error,
    stale,
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
