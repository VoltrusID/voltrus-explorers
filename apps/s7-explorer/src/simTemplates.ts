/** Points of the built-in S7 simulator templates. */
export interface SimPoint {
  key: string;
  address: string;
  data_type: string;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  decimals?: number;
  isBool?: boolean;
  invert?: number; // seconds between boolean toggles (0 = static)
}

export const TEMPLATES: Record<string, { name: string; description: string; points: SimPoint[] }> = {
  motor: {
    name: "S7-1500 Motor Control (DB1)",
    description: "VFD motor control data block: speed, current, temp, torque, power, status flags.",
    points: [
      { key: "speed", address: "DB1.DBD0", data_type: "Real", base: 1450, variance: 12, noise: 3, period: 15, min: 1400, max: 1500, decimals: 1 },
      { key: "current", address: "DB1.DBD4", data_type: "Real", base: 12.4, variance: 0.8, noise: 0.2, period: 12, min: 8, max: 18, decimals: 1 },
      { key: "temp", address: "DB1.DBD8", data_type: "Real", base: 67.2, variance: 1.5, noise: 0.3, period: 60, min: 50, max: 90, decimals: 1 },
      { key: "torque", address: "DB1.DBD12", data_type: "Real", base: 45.8, variance: 2.5, noise: 0.8, period: 14, min: 30, max: 60, decimals: 1 },
      { key: "power", address: "DB1.DBD16", data_type: "Real", base: 7.5, variance: 0.4, noise: 0.1, period: 13, min: 5, max: 10, decimals: 2 },
      { key: "running", address: "DB1.DBX20.0", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "fault", address: "DB1.DBX20.1", data_type: "Bool", base: 0, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "runtime", address: "DB1.DBW22", data_type: "Word", base: 15234, variance: 0, noise: 0, period: 3600, decimals: 0 },
      { key: "freq", address: "DB1.DBD24", data_type: "Real", base: 50, variance: 0.02, noise: 0.01, period: 10, min: 49.9, max: 50.1, decimals: 2 },
    ],
  },
  process: {
    name: "S7-300 Process Control (DB1)",
    description: "Chemical reactor: temperatures, pressures, levels, flow, pH, agitator/valve states.",
    points: [
      { key: "rtemp", address: "DB1.DBD0", data_type: "Real", base: 185.4, variance: 1.2, noise: 0.3, period: 25, min: 175, max: 195, decimals: 1 },
      { key: "rpress", address: "DB1.DBD4", data_type: "Real", base: 4.21, variance: 0.08, noise: 0.02, period: 20, min: 3.5, max: 5, decimals: 2 },
      { key: "rlvl", address: "DB1.DBD8", data_type: "Real", base: 72.5, variance: 1.5, noise: 0.5, period: 30, min: 50, max: 90, decimals: 1 },
      { key: "feed", address: "DB1.DBD12", data_type: "Real", base: 12.4, variance: 0.3, noise: 0.1, period: 18, min: 10, max: 15, decimals: 1 },
      { key: "ph", address: "DB1.DBD16", data_type: "Real", base: 7.12, variance: 0.05, noise: 0.02, period: 15, min: 6.5, max: 7.8, decimals: 2 },
      { key: "agitator", address: "DB1.DBX20.0", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "valve_in", address: "DB1.DBX20.1", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "valve_out", address: "DB1.DBX20.2", data_type: "Bool", base: 0, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "cycle", address: "DB1.DBW24", data_type: "Word", base: 1247, variance: 0, noise: 0, period: 60, decimals: 0 },
    ],
  },
};
