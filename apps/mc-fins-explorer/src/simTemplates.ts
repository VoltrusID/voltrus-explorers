import type { Protocol } from "./types";

/** One animated point of the simulator. Numbers follow a sine around `base`; bools are static or blink. */
export interface SimPoint {
  key: string;
  address: string;
  data_type: string;
  base: number;
  variance?: number;
  noise?: number;
  period?: number;
  min?: number;
  max?: number;
  /** Counts up by this much per second (production counters). */
  ramp?: number;
  /** Bool: toggles every this many seconds (0 or unset = static). */
  blink?: number;
}

export interface SimTemplate {
  protocol: Protocol;
  name: string;
  description: string;
  points: SimPoint[];
}

/** One template per protocol; the simulator serves both at once. */
export const TEMPLATES: Record<Protocol, SimTemplate> = {
  mc: {
    protocol: "mc",
    name: "Mitsubishi packaging line (Q / iQ-R)",
    description: "Conveyor and case packer on MC protocol: speed, motor current, temperatures, counters, run and fault bits.",
    points: [
      { key: "Line speed (pcs/min)", address: "D100", data_type: "Int16", base: 120, variance: 6, noise: 1, period: 20, min: 0, max: 200 },
      { key: "Good count", address: "D102", data_type: "UInt32", base: 15234, ramp: 2 },
      { key: "Reject count", address: "D104", data_type: "UInt16", base: 37, ramp: 0.05 },
      { key: "Motor current (A)", address: "D110", data_type: "Float32", base: 12.4, variance: 0.8, noise: 0.2, period: 12 },
      { key: "Sealer temp (°C)", address: "D112", data_type: "Float32", base: 182.5, variance: 2.5, noise: 0.4, period: 45 },
      { key: "Recipe no. (BCD)", address: "D120", data_type: "BCD16", base: 1203 },
      { key: "Status word", address: "D130", data_type: "Hex16", base: 0x0011 },
      { key: "Line running", address: "M0", data_type: "Bool", base: 1 },
      { key: "Fault", address: "M1", data_type: "Bool", base: 0, blink: 30 },
      { key: "Photo eye", address: "X0", data_type: "Bool", base: 0, blink: 1 },
      { key: "Conveyor motor", address: "Y20", data_type: "Bool", base: 1 },
      { key: "Link register", address: "W0", data_type: "Int16", base: 500, variance: 50, period: 30 },
    ],
  },
  fins: {
    protocol: "fins",
    name: "Omron filling machine (CJ2M)",
    description: "Bottle filler on FINS: fill weight, flow, tank level, counters, valve and alarm bits.",
    points: [
      { key: "Fill weight (g)", address: "D100", data_type: "Float32", base: 502.3, variance: 1.5, noise: 0.4, period: 9 },
      { key: "Flow (l/min)", address: "D102", data_type: "Float32", base: 38.2, variance: 2, noise: 0.3, period: 15 },
      { key: "Tank level (%)", address: "D104", data_type: "Int16", base: 72, variance: 8, period: 90, min: 0, max: 100 },
      { key: "Bottles filled", address: "D110", data_type: "Int32", base: 88120, ramp: 1.5 },
      { key: "Speed setpoint", address: "D120", data_type: "UInt16", base: 90 },
      { key: "Timer T0 (BCD)", address: "T0", data_type: "BCD16", base: 50, variance: 50, period: 10, min: 0, max: 100 },
      { key: "Machine running", address: "CIO0.00", data_type: "Bool", base: 1 },
      { key: "Fill valve", address: "CIO0.01", data_type: "Bool", base: 0, blink: 2 },
      { key: "Low level alarm", address: "W0.00", data_type: "Bool", base: 0, blink: 40 },
      { key: "Batch number", address: "H0", data_type: "UInt16", base: 42 },
    ],
  },
};

/** The value of a point `t` seconds after the simulator started. */
export function simValue(p: SimPoint, t: number): number {
  if (p.data_type === "Bool") {
    if (!p.blink) return p.base >= 0.5 ? 1 : 0;
    const on = Math.floor(t / p.blink) % 2 === 1;
    return (p.base >= 0.5) !== on ? 1 : 0;
  }
  let v = p.base;
  if (p.ramp) v += Math.floor(t * p.ramp);
  if (p.variance && p.period) v += Math.sin((t / p.period) * Math.PI * 2) * p.variance;
  if (p.noise) v += (Math.random() - 0.5) * p.noise;
  if (p.min !== undefined) v = Math.max(p.min, v);
  if (p.max !== undefined) v = Math.min(p.max, v);
  return p.data_type === "Float32" ? Math.round(v * 100) / 100 : Math.round(v);
}

/** Points that change over time; the others keep whatever was last written. */
export function isAnimated(p: SimPoint): boolean {
  return Boolean(p.blink || p.ramp || p.noise || (p.variance && p.period));
}
