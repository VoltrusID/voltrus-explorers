import type { ReplayInfo } from "./types";

/** Trace files the app opens and replays. */
export const TRACE_FILE_FILTER = { name: "CAN trace (candump, ASC, TRC, CSV)", extensions: ["log", "txt", "candump", "asc", "trc", "csv"] };

/** Replay settings and progress. Kept by the App so that leaving the Replay tab and coming back shows the running replay. */
export interface ReplayState {
  file: { path: string; text: string } | null;
  speed: number;
  repeat: boolean;
  ids: string;
  info: ReplayInfo | null;
  progress: { pos: number; total: number; pass?: number } | null;
  running: boolean;
  error: string | null;
  /** Why the last replay ended early, when it did not run to the end. */
  stopped: string | null;
}

export const REPLAY_INITIAL: ReplayState = {
  file: null, speed: 1, repeat: false, ids: "", info: null, progress: null, running: false, error: null, stopped: null,
};

/** A `can-replay` event from the backend. */
export interface ReplayEvent {
  done: boolean;
  pos?: number;
  total?: number;
  pass?: number;
  error?: string;
  /** Set when the replay was stopped: "stopped" (by the user) or "disconnected". */
  stopped?: string;
}

/** The state after a `can-replay` event. */
export function applyReplayEvent(s: ReplayState, p: ReplayEvent): ReplayState {
  const next = { ...s };
  if (p.total != null) next.progress = { pos: p.pos ?? 0, total: p.total, pass: p.pass };
  if (p.done) {
    next.running = false;
    if (p.error) next.error = p.error;
    if (p.stopped === "disconnected") next.stopped = "Stopped: the bus was disconnected.";
  }
  return next;
}
