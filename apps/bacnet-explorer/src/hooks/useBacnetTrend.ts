import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BacnetObjectDetail, CovNotificationEvent } from "../types";

export type TrendSource = "poll" | "cov";

export interface TrendEntry {
  key: string;
  name: string;
  color: string;
  source: TrendSource;
  obj: { type: string; instance: number };
  points: { t: number; v: number }[];
}

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

export function trendKey(type: string, instance: number) {
  return `${type}:${instance}`;
}

export function useBacnetTrend(connected: boolean) {
  const [series, setSeries] = useState<Map<string, TrendEntry>>(new Map());
  const [pollMs, setPollMs] = useState(1000);
  const [windowSec, setWindowSec] = useState(60);

  const seriesRef = useRef(series);
  seriesRef.current = series;
  const windowRef = useRef(windowSec);
  windowRef.current = windowSec;

  const appendPoint = useCallback((key: string, v: number) => {
    if (isNaN(v)) return;
    const ts = Date.now();
    const cutoff = ts - windowRef.current * 1000 - 5000;
    setSeries((prev) => {
      const entry = prev.get(key);
      if (!entry) return prev;
      const next = new Map(prev);
      next.set(key, {
        ...entry,
        points: [...entry.points, { t: ts, v }].filter((p) => p.t > cutoff).slice(-500),
      });
      return next;
    });
  }, []);

  const toggle = useCallback((obj: { type: string; instance: number }, name: string) => {
    const key = trendKey(obj.type, obj.instance);
    setSeries((prev) => {
      const next = new Map(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.set(key, {
          key,
          name,
          color: COLORS[next.size % COLORS.length],
          source: "poll",
          obj,
          points: [],
        });
      }
      return next;
    });
  }, []);

  const remove = useCallback((key: string) => {
    setSeries((prev) => {
      const next = new Map(prev);
      next.delete(key);
      return next;
    });
  }, []);

  const setSource = useCallback((key: string, source: TrendSource) => {
    setSeries((prev) => {
      const entry = prev.get(key);
      if (!entry || entry.source === source) return prev;
      const next = new Map(prev);
      next.set(key, { ...entry, source, points: [] });
      return next;
    });
  }, []);

  // ── Poll: read presentValue of each poll-source object (proven read_object_properties path) ──
  useEffect(() => {
    if (!connected) return;
    let cancelled = false;

    const tick = async () => {
      const pollEntries = Array.from(seriesRef.current.values()).filter((e) => e.source === "poll");
      if (pollEntries.length === 0) return;
      await Promise.all(
        pollEntries.map(async (e) => {
          try {
            const detail = await invoke<BacnetObjectDetail | null>("read_object_properties", {
              objectType: e.obj.type,
              objectInstance: e.obj.instance,
            });
            if (cancelled) return;
            const pvEntry = detail?.properties.find((p) => p.property_identifier === 85);
            const pv = detail?.present_value ?? pvEntry?.raw_value ?? null;
            if (pv != null) appendPoint(e.key, parseFloat(pv));
          } catch {
            /* ignore */
          }
        }),
      );
    };

    tick();
    const id = setInterval(tick, Math.max(100, pollMs));
    return () => { cancelled = true; clearInterval(id); };
  }, [connected, pollMs, appendPoint]);

  // ── COV: listen for notifications ──
  useEffect(() => {
    if (!connected) return;
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    invoke("start_cov_listener").catch(() => {});
    listen<CovNotificationEvent>("cov-notification", (event) => {
      const p = event.payload;
      const key = trendKey(p.monitored_object_type, p.monitored_object_instance);
      const entry = seriesRef.current.get(key);
      if (!entry || entry.source !== "cov") return;
      const raw = p.values?.[0]?.value;
      appendPoint(key, parseFloat(raw));
    }).then((u) => { if (cancelled) u(); else unlisten = u; });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [connected, appendPoint]);

  // ── COV: subscribe/unsubscribe diff for cov-source objects ──
  const subscribedRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (!connected) {
      subscribedRef.current = new Set();
      return;
    }
    const want = new Set<string>();
    for (const e of series.values()) if (e.source === "cov") want.add(e.key);

    // Subscribe new cov entries.
    for (const e of series.values()) {
      if (e.source === "cov" && !subscribedRef.current.has(e.key)) {
        invoke("subscribe_cov_cmd", {
          objectType: e.obj.type,
          objectInstance: e.obj.instance,
          lifetime: 3600,
          confirmed: false,
        }).catch(() => {});
        subscribedRef.current.add(e.key);
      }
    }
    // Unsubscribe entries no longer cov (removed or switched to poll).
    for (const key of Array.from(subscribedRef.current)) {
      if (!want.has(key)) {
        const e = seriesRef.current.get(key);
        if (e) {
          invoke("unsubscribe_cov_cmd", {
            objectType: e.obj.type,
            objectInstance: e.obj.instance,
          }).catch(() => {});
        }
        subscribedRef.current.delete(key);
      }
    }
  }, [connected, series]);

  // Clear on disconnect.
  useEffect(() => {
    if (!connected) {
      setSeries(new Map());
      subscribedRef.current = new Set();
    }
  }, [connected]);

  return {
    series: Array.from(series.values()),
    hasSeries: series.size > 0,
    pollMs,
    setPollMs,
    windowSec,
    setWindowSec,
    toggle,
    remove,
    setSource,
  };
}
