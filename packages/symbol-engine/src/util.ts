/** Smallest allowed symbol size after a resize (px). */
export const MIN_SIZE = 20;

export function newId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  // uuid-v4-shaped fallback for runtimes without crypto.randomUUID
  let out = "";
  for (let i = 0; i < 36; i++) {
    if (i === 8 || i === 13 || i === 18 || i === 23) out += "-";
    else if (i === 14) out += "4";
    else if (i === 19) out += "89ab"[Math.floor(Math.random() * 4)];
    else out += Math.floor(Math.random() * 16).toString(16);
  }
  return out;
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/**
 * Deep-copy plain model data. Prefers structuredClone, falls back to a JSON
 * round-trip on runtimes without it (both inputs are JsonValue-shaped).
 */
export function deepClone<T>(value: T): T {
  if (typeof globalThis.structuredClone === "function") {
    return globalThis.structuredClone(value);
  }
  return JSON.parse(JSON.stringify(value)) as T;
}
