import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { noteDeviceError, noteDeviceSuccess } from "@mactools/ui";
import { bacnetValueToNumber } from "../types";
import type {
  BacnetObjectDetail,
  CovListenerErrorEvent,
  CovNotificationEvent,
  CovSubscribeFailedEvent,
} from "../types";

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

const POLL_ERROR_PREFIX = "Polling failed: ";

/** SubscribeCOV lifetime in seconds; the backend renews each subscription at 80% of it. */
const COV_LIFETIME_SEC = 3600;

export function trendKey(type: string, instance: number) {
  return `${type}:${instance}`;
}

/**
 * Trend series for the active device. `deviceKey` is `null` when no device is open; `openKeys`
 * lists every open device. Series of an open device that is not active are kept aside and come
 * back (without the readings taken meanwhile) when that device is activated again.
 */
export function useBacnetTrend(deviceKey: string | null, openKeys: string[], onError?: (message: string) => void) {
  const connected = deviceKey !== null;
  const [series, setSeries] = useState<Map<string, TrendEntry>>(new Map());
  const [errors, setErrors] = useState<Map<string, string>>(new Map());
  const [pollMs, setPollMs] = useState(1000);
  const [windowSec, setWindowSec] = useState(60);

  /** Record (or, with null, clear) the latest poll failure of a series without touching COV notices. */
  const setPollError = useCallback((key: string, message: string | null) => {
    setErrors((prev) => {
      const current = prev.get(key);
      if (message === null) {
        if (current === undefined || !current.startsWith(POLL_ERROR_PREFIX)) return prev;
        const next = new Map(prev);
        next.delete(key);
        return next;
      }
      if (current === message) return prev;
      return new Map(prev).set(key, message);
    });
  }, []);

  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const seriesRef = useRef(series);
  seriesRef.current = series;
  const windowRef = useRef(windowSec);
  windowRef.current = windowSec;
  const openKeysRef = useRef(openKeys);
  openKeysRef.current = openKeys;
  const errorsRef = useRef(errors);
  errorsRef.current = errors;
  const stashRef = useRef(new Map<string, { series: Map<string, TrendEntry>; errors: Map<string, string> }>());
  const ownerRef = useRef<string | null>(null);

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
    setErrors((prev) => {
      if (!prev.has(key)) return prev;
      const next = new Map(prev);
      next.delete(key);
      return next;
    });
    setSeries((prev) => {
      const entry = prev.get(key);
      if (!entry || entry.source === source) return prev;
      const next = new Map(prev);
      next.set(key, { ...entry, source, points: [] });
      return next;
    });
  }, []);

  /** COV could not be used for `key`: record why and continue with polling. */
  const fallBackToPoll = useCallback((key: string, reason: string) => {
    const message = `COV subscription for ${key} failed (${reason}); using polling instead`;
    setErrors((prev) => new Map(prev).set(key, message));
    setSeries((prev) => {
      const entry = prev.get(key);
      if (!entry || entry.source !== "cov") return prev;
      const next = new Map(prev);
      next.set(key, { ...entry, source: "poll" });
      return next;
    });
    onErrorRef.current?.(message);
  }, []);

  // ── Device switch: park the series of the device being left, restore those of the one entered ──
  // Declared before the effects below so they already see the restored series.
  useEffect(() => {
    const previous = ownerRef.current;
    if (previous === deviceKey) return;
    const open = new Set(openKeysRef.current);
    if (previous !== null && open.has(previous)) {
      stashRef.current.set(previous, { series: seriesRef.current, errors: errorsRef.current });
    }
    for (const k of Array.from(stashRef.current.keys())) if (!open.has(k)) stashRef.current.delete(k);
    ownerRef.current = deviceKey;
    subscribedRef.current = new Map();
    const restored = (deviceKey !== null ? stashRef.current.get(deviceKey) : undefined) ?? {
      series: new Map<string, TrendEntry>(),
      errors: new Map<string, string>(),
    };
    if (deviceKey !== null) stashRef.current.delete(deviceKey);
    seriesRef.current = restored.series;
    setSeries(restored.series);
    setErrors(restored.errors);
  }, [deviceKey]);

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
            if (pv == null) throw new Error("device returned no presentValue");
            appendPoint(e.key, parseFloat(pv));
            setPollError(e.key, null);
            noteDeviceSuccess();
          } catch (err) {
            if (!cancelled) {
              setPollError(e.key, `${POLL_ERROR_PREFIX}${err}`);
              noteDeviceError();
            }
          }
        }),
      );
    };

    tick();
    const id = setInterval(tick, Math.max(100, pollMs));
    return () => { cancelled = true; clearInterval(id); };
  }, [deviceKey, pollMs, appendPoint, setPollError]);

  // ── COV: listen for notifications and for backend failures ──
  useEffect(() => {
    if (!connected) return;
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];
    const track = (p: Promise<UnlistenFn>) =>
      p.then((u) => { if (cancelled) u(); else unlisteners.push(u); });

    invoke("start_cov_listener").catch((e) => {
      onErrorRef.current?.(`COV listener could not start: ${e}`);
    });

    track(listen<CovNotificationEvent>("cov-notification", (event) => {
      const p = event.payload;
      const key = trendKey(p.monitored_object_type, p.monitored_object_instance);
      const entry = seriesRef.current.get(key);
      if (!entry || entry.source !== "cov") return;
      const pv = p.values?.find((v) => v.property_name === "presentValue");
      if (!pv) return;
      // `value` is a serialized BacnetValue ({ Real: 1.5 }); "Null" arrives as a bare string.
      appendPoint(key, typeof pv.value === "object" ? bacnetValueToNumber(pv.value) : parseFloat(pv.raw_value));
    }));

    // A SubscribeCOV the device refused (or never answered): show why and fall back to polling.
    track(listen<CovSubscribeFailedEvent>("cov-subscribe-failed", (event) => {
      const p = event.payload;
      fallBackToPoll(trendKey(p.object_type, p.object_instance), p.message);
    }));

    track(listen<CovListenerErrorEvent>("cov-listener-error", (event) => {
      onErrorRef.current?.(event.payload.message);
    }));

    return () => {
      cancelled = true;
      for (const u of unlisteners) u();
    };
  }, [deviceKey, appendPoint, fallBackToPoll]);

  // ── COV: subscribe/unsubscribe diff for cov-source objects ──
  // The object each key was subscribed with is remembered here, because by the time a
  // removed series is noticed it is already gone from `series`.
  const subscribedRef = useRef<Map<string, { type: string; instance: number }>>(new Map());
  useEffect(() => {
    if (!connected) {
      subscribedRef.current = new Map();
      return;
    }
    const want = new Set<string>();
    for (const e of series.values()) if (e.source === "cov") want.add(e.key);

    // Subscribe new cov entries.
    for (const e of series.values()) {
      if (e.source === "cov" && !subscribedRef.current.has(e.key)) {
        subscribedRef.current.set(e.key, { type: e.obj.type, instance: e.obj.instance });
        invoke("subscribe_cov_cmd", {
          objectType: e.obj.type,
          objectInstance: e.obj.instance,
          lifetime: COV_LIFETIME_SEC,
          confirmed: false,
        }).catch((err) => fallBackToPoll(e.key, String(err)));
      }
    }
    // Unsubscribe entries no longer cov (removed or switched to poll).
    for (const [key, obj] of Array.from(subscribedRef.current)) {
      if (!want.has(key)) {
        subscribedRef.current.delete(key);
        invoke("unsubscribe_cov_cmd", {
          objectType: obj.type,
          objectInstance: obj.instance,
        }).catch((err) => {
          onErrorRef.current?.(`COV unsubscribe for ${key} failed: ${err}`);
        });
      }
    }
  }, [connected, series, fallBackToPoll]);

  // Errors only describe live series.
  useEffect(() => {
    setErrors((prev) => {
      const stale = Array.from(prev.keys()).filter((k) => !series.has(k));
      if (stale.length === 0) return prev;
      const next = new Map(prev);
      for (const k of stale) next.delete(k);
      return next;
    });
  }, [series]);

  return {
    series: Array.from(series.values()),
    hasSeries: series.size > 0,
    /** Per-series notice, keyed by series key: set when COV could not be used and the series fell back to polling. */
    errors,
    pollMs,
    setPollMs,
    windowSec,
    setWindowSec,
    toggle,
    remove,
    setSource,
  };
}
