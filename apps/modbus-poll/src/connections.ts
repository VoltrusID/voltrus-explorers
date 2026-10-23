import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { appendTextFile, saveTextFile } from "@mactools/ui";
import { CSV_LOG_HEADER, csvLogFileName, csvLogLines } from "./csvLog";
import { readPlanKey, type ReadPlan, type ReadTag } from "./helpers";
import { effectivePlan } from "./limits";
import { fieldsOf } from "./sessions";
import { useStore } from "./store";
import { appendTrendPoints } from "./trend";
import { dropTraffic } from "./trafficStore";
import type { ModbusConfig, ReadBlock } from "./types";
import { explainModbusError } from "./utils/format";
import { buildTableRows } from "./utils/rows";

/**
 * Live state of every master connection, keyed by connection id (the store's session ids).
 * Each connection polls on its own timer, whether or not its tab is in front, and keeps its
 * own comm log, trend series and CSV log up to date from that poll.
 */
export interface ConnRuntime {
  connected: boolean;
  /** A connect is waiting for the device. */
  loading: boolean;
  values: unknown[];
  /** The most recent read failed, so `values` no longer reflect the device. */
  stale: boolean;
  error: string | null;
  pollCount: number;
  latency: number;
  /** Poll interval of the live session; null while not connected. */
  pollIntervalMs: number | null;
  /** Identity of the plan the running poll was started with; values only line up with rows while it matches. */
  polledKey: string | null;
  /** File the polled values are being logged to, or null. */
  csvLogPath: string | null;
  csvLogRows: number;
}

export const IDLE: ConnRuntime = {
  connected: false, loading: false, values: [], stale: false, error: null, pollCount: 0, latency: 0,
  pollIntervalMs: null, polledKey: null, csvLogPath: null, csvLogRows: 0,
};

export const useConnRuntime = create<{ byId: Record<string, ConnRuntime> }>(() => ({ byId: {} }));

export const runtimeOf = (id: string): ConnRuntime => useConnRuntime.getState().byId[id] ?? IDLE;

const patch = (id: string, change: Partial<ConnRuntime>) =>
  useConnRuntime.setState((s) => ({ byId: { ...s.byId, [id]: { ...(s.byId[id] ?? IDLE), ...change } } }));

interface PollContext {
  blocks: ReadBlock[];
  plan: ReadPlan;
  intervalMs: number;
}

interface CsvLogger {
  path: string;
  label: string;
  /** Appends run one after the other so lines never interleave. */
  queue: Promise<void>;
}

const timers = new Map<string, number>();
/** Bumped whenever polling starts, restarts or stops; results of older reads are dropped. */
const generations = new Map<string, number>();
const aborted = new Map<string, boolean>();
const contexts = new Map<string, PollContext>();
const loggers = new Map<string, CsvLogger>();
const lastFailures = new Map<string, string>();

const log = (id: string, direction: "tx" | "rx" | "info" | "error", message: string) =>
  useStore.getState().addLogFor(id, direction, message);

const bump = (id: string) => {
  const gen = (generations.get(id) ?? 0) + 1;
  generations.set(id, gen);
  return gen;
};

/** Stop the poll timer and drop reads in flight. The session stays open: `pollIntervalMs` is kept so polling can resume. */
export function pausePolling(id: string) {
  bump(id);
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
}

/** Stop polling for good (disconnect). */
function endPolling(id: string) {
  pausePolling(id);
  contexts.delete(id);
  lastFailures.delete(id);
  patch(id, { pollIntervalMs: null, polledKey: null });
}

/** Rows of one poll as the logs and trends see them: decimal numbers, whatever the display format is. */
function pollRows(ctx: PollContext, values: unknown[]) {
  return buildTableRows(ctx.blocks, ctx.plan, values, { useOneBased: false, displayFormat: "dec" });
}

/** Everything a finished poll feeds besides the table: comm log, trend series and the CSV log. */
function onPoll(id: string, ctx: PollContext, values: unknown[], pollCount: number, latency: number) {
  const rows = pollRows(ctx, values);
  const polled = rows.filter((r) => !r.limited);
  const first = polled[0];
  const last = polled[polled.length - 1];
  if (first && last) {
    const range = polled.length === 1 ? `${first.name}=${first.value}` : `${first.name}…${last.name} (${polled.length} values)`;
    log(id, "rx", `Poll #${pollCount}: ${range} (${latency}ms)`);
  }

  // Exception responses: say which request the device rejected (once per distinct set, not every poll).
  const failures = Array.from(new Set(polled.flatMap((r) => (r.error ? [r.error] : [])))).join("\n");
  if (failures !== (lastFailures.get(id) ?? "")) {
    lastFailures.set(id, failures);
    if (failures) failures.split("\n").forEach((m) => log(id, "error", explainModbusError(m)));
  }

  const now = Date.now();
  const { trendWindow, patchSession } = useStore.getState();
  patchSession(id, (f) => {
    const trendSeries = appendTrendPoints(f.trendSeries, polled, now, trendWindow);
    return trendSeries === f.trendSeries ? {} : { trendSeries };
  });

  const logger = loggers.get(id);
  if (logger) {
    const lines = csvLogLines(logger.label, now, polled);
    if (lines) {
      logger.queue = logger.queue
        .then(() => appendTextFile(logger.path, lines))
        .then(() => patch(id, { csvLogRows: runtimeOf(id).csvLogRows + polled.length }))
        .catch((e) => {
          // Dropped here so one failure is reported once and later polls do not keep failing.
          if (loggers.get(id) !== logger) return;
          loggers.delete(id);
          patch(id, { csvLogPath: null });
          log(id, "error", `CSV logging stopped, could not write ${logger.path}: ${String(e)}`);
        });
    }
  }
}

async function readOnce(id: string, ctx: PollContext, gen: number) {
  const start = performance.now();
  try {
    const result = await invoke<unknown[]>("read_tags", { connId: id, tags: ctx.plan.tags });
    // Polling was restarted or stopped while this read was in flight: it describes an old tag list.
    if (gen !== generations.get(id)) return result;
    const pollCount = runtimeOf(id).pollCount + 1;
    const latency = Math.round(performance.now() - start);
    patch(id, { values: result, latency, pollCount, error: null, stale: false });
    onPoll(id, ctx, result, pollCount, latency);
    return result;
  } catch (e) {
    if (gen === generations.get(id)) {
      const message = explainModbusError(String(e));
      if (message !== runtimeOf(id).error) log(id, "error", message);
      patch(id, { error: message, stale: true });
    }
    return [];
  }
}

/**
 * Poll `blocks` every `intervalMs`, measured from the start of one read to the start of the next.
 * A new read is only scheduled once the previous one finished, so slow devices never get
 * overlapping requests. Calling again replaces the running poll. `pro` decides how many values
 * are read (the free version stops at the first FREE_VALUES).
 */
export function startPolling(id: string, blocks: ReadBlock[], intervalMs: number, pro: boolean) {
  pausePolling(id);
  const gen = generations.get(id)!;
  const plan = effectivePlan(blocks, pro);
  const ctx: PollContext = { blocks, plan, intervalMs };
  contexts.set(id, ctx);
  patch(id, { pollIntervalMs: intervalMs, polledKey: readPlanKey(plan) });
  const tick = async () => {
    const began = performance.now();
    await readOnce(id, ctx, gen);
    if (gen !== generations.get(id)) return;
    timers.set(id, setTimeout(tick, Math.max(0, intervalMs - (performance.now() - began))));
  };
  void tick();
}

/** One extra read right now, e.g. after a write, with the plan the poll uses. */
export async function readNow(id: string) {
  const ctx = contexts.get(id);
  if (ctx) await readOnce(id, ctx, generations.get(id) ?? 0);
}

/** Connect over TCP. Resolves true once connected; false if it failed or was cancelled. */
export async function connectTcp(id: string, config: ModbusConfig): Promise<boolean> {
  patch(id, { loading: true, error: null });
  aborted.set(id, false);
  try {
    await invoke("connect", { connId: id, config });
    if (aborted.get(id)) return false;
    patch(id, { connected: true });
    return true;
  } catch (e) {
    if (!aborted.get(id)) patch(id, { error: String(e), connected: false });
    return false;
  } finally {
    if (!aborted.get(id)) patch(id, { loading: false });
  }
}

/** A connection made by other means (RTU) is up. */
export function markConnected(id: string, connected: boolean) {
  patch(id, { connected });
}

/** Give up on a connect still waiting for the device. */
export async function cancelConnect(id: string) {
  aborted.set(id, true);
  patch(id, { loading: false, connected: false, error: null });
  // `disconnect` also aborts a connect still waiting for the device, and drops the
  // client if the connect finished just before the cancel arrived.
  await invoke("disconnect", { connId: id });
}

export async function disconnect(id: string) {
  endPolling(id);
  await stopCsvLog(id);
  try {
    await invoke("disconnect", { connId: id });
  } catch {
    // may not be connected
  } finally {
    patch(id, { connected: false, values: [], stale: false, pollCount: 0, loading: false });
  }
}

/** Disconnect and forget a connection (tab closed). */
export async function closeConnection(id: string) {
  await disconnect(id);
  generations.delete(id);
  aborted.delete(id);
  dropTraffic(id);
  useConnRuntime.setState((s) => {
    const byId = { ...s.byId };
    delete byId[id];
    return { byId };
  });
}

export async function writeTag(id: string, tag: ReadTag, value: number): Promise<{ ok: boolean; error?: string }> {
  try {
    await invoke("write_tag", { connId: id, tag, value });
    patch(id, { error: null });
    return { ok: true };
  } catch (e) {
    const message = String(e);
    patch(id, { error: message });
    return { ok: false, error: message };
  }
}

/**
 * Ask where to save, then log every polled value of connection `id` to that file until stopped.
 * Resolves with the path, or null if the panel was cancelled. Rejects if the file can not be created.
 */
export async function startCsvLog(id: string, label: string): Promise<string | null> {
  if (loggers.has(id)) await stopCsvLog(id);
  const path = await saveTextFile(csvLogFileName(new Date()), CSV_LOG_HEADER);
  if (!path) return null;
  loggers.set(id, { path, label, queue: Promise.resolve() });
  patch(id, { csvLogPath: path, csvLogRows: 0 });
  return path;
}

/** Stop logging; resolves once everything polled so far is in the file. */
export async function stopCsvLog(id: string): Promise<{ path: string; rows: number } | null> {
  const logger = loggers.get(id);
  if (!logger) return null;
  loggers.delete(id);
  await logger.queue;
  const rows = runtimeOf(id).csvLogRows;
  patch(id, { csvLogPath: null });
  return { path: logger.path, rows };
}

/** Label recorded in the CSV log's connection column. */
export function connectionLabel(id: string): string {
  const fields = fieldsOf(useStore.getState(), id);
  return fields ? (fields.activeConnProfile ?? `${fields.tcpFormValues.host}:${fields.tcpFormValues.port}`) : id;
}
