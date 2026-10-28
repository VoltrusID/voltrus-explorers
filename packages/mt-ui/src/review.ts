import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

/** Idle time after launch before a rating request may fire. */
export const MIN_UPTIME_MS = 30_000;
/** A success waits this long before firing; a read error in the window cancels it. */
export const SETTLE_MS = 2_000;
export const STORAGE_KEY = "voltrus-review-requested-version";

export interface ReviewDeps {
  now: () => number;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
  /** App Store build (`store_status().store`). */
  isStoreBuild: () => Promise<boolean>;
  version: () => Promise<string>;
  getStored: () => string | null;
  setStored: (version: string) => void;
  /** A modal dialog is open (paywall, connect, settings). */
  modalOpen: () => boolean;
  /** Native request; resolves true when StoreKit was asked. */
  request: () => Promise<boolean>;
}

export interface ReviewGate {
  noteDeviceSuccess: () => void;
  noteDeviceError: () => void;
}

/**
 * Gating for the App Store rating request. A success schedules the request {@link SETTLE_MS} later when the app
 * has run {@link MIN_UPTIME_MS}, no error was seen in the last {@link SETTLE_MS} and this version has not asked
 * yet. Successes before the uptime threshold are ignored (polling brings another). An error cancels a pending
 * request. At most one request per session.
 */
export function createReviewGate(deps: ReviewDeps): ReviewGate {
  const startedAt = deps.now();
  let lastErrorAt = Number.NEGATIVE_INFINITY;
  let pending: unknown = null;
  let spent = false;
  let storePromise: Promise<boolean> | null = null;

  const fire = async () => {
    pending = null;
    if (spent) return;
    if (deps.now() - lastErrorAt < SETTLE_MS || deps.modalOpen()) return;
    try {
      storePromise ??= deps.isStoreBuild();
      if (!(await storePromise)) {
        spent = true;
        return;
      }
      const version = await deps.version();
      if (deps.getStored() === version) {
        spent = true;
        return;
      }
      // An error may have arrived while the checks above awaited.
      if (deps.now() - lastErrorAt < SETTLE_MS || deps.modalOpen()) return;
      spent = true;
      if (await deps.request()) deps.setStored(version);
    } catch {
      spent = true;
    }
  };

  return {
    noteDeviceSuccess() {
      if (spent || pending !== null) return;
      const now = deps.now();
      if (now - startedAt < MIN_UPTIME_MS || now - lastErrorAt < SETTLE_MS) return;
      pending = deps.setTimer(() => void fire(), SETTLE_MS);
    },
    noteDeviceError() {
      lastErrorAt = deps.now();
      if (pending !== null) {
        deps.clearTimer(pending);
        pending = null;
      }
    },
  };
}

const defaultGate = createReviewGate({
  now: () => Date.now(),
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  isStoreBuild: () => invoke<{ store: boolean }>("store_status").then((s) => s.store),
  version: getVersion,
  getStored: () => {
    try {
      return localStorage.getItem(STORAGE_KEY);
    } catch {
      return null;
    }
  },
  setStored: (v) => {
    try {
      localStorage.setItem(STORAGE_KEY, v);
    } catch {
      /* storage unavailable: the session cap still holds */
    }
  },
  modalOpen: () => typeof document !== "undefined" && document.querySelector('[aria-modal="true"], dialog[open]') !== null,
  request: () => invoke<boolean>("request_review"),
});

/** A device read or discovery succeeded (call at every real success point; cheap and idempotent). */
export const noteDeviceSuccess = defaultGate.noteDeviceSuccess;
/** A device read or discovery failed. */
export const noteDeviceError = defaultGate.noteDeviceError;
