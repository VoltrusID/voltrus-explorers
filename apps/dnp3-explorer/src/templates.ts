export interface SimPointDef {
  key: string;
  label: string;
  group: number;
  variation: number;
  index: number;
  base: number;
  variance: number;
  noise: number;
  periodMs: number;
  min?: number;
  max?: number;
  decimals?: number;
  unit?: string;
  isBool?: boolean;
}export interface DeviceTemplate {
  id: string;
  name: string;
  description: string;
  points: SimPointDef[];
}

export const BUILTIN_TEMPLATES: DeviceTemplate[] = [
  {
    id: "feeder",
    name: "Feeder-1 (13.8kV Distribution)",
    description: "13.8kV distribution feeder with voltage, current, power, frequency",
    points: [
      { key: "v_ab", label: "Voltage_AB", group: 30, variation: 4, index: 0, base: 13.82, variance: 0.05, noise: 0.02, periodMs: 20, decimals: 2, unit: "kV" },
      { key: "v_bc", label: "Voltage_BC", group: 30, variation: 4, index: 1, base: 13.76, variance: 0.05, noise: 0.02, periodMs: 22, decimals: 2, unit: "kV" },
      { key: "v_ca", label: "Voltage_CA", group: 30, variation: 4, index: 2, base: 13.89, variance: 0.04, noise: 0.02, periodMs: 21, decimals: 2, unit: "kV" },
      { key: "i_a", label: "Current_A", group: 30, variation: 4, index: 3, base: 142.3, variance: 3.5, noise: 1.2, periodMs: 18, min: 100, max: 200, decimals: 1, unit: "A" },
      { key: "i_b", label: "Current_B", group: 30, variation: 4, index: 4, base: 138.7, variance: 3.2, noise: 1.0, periodMs: 19, min: 100, max: 200, decimals: 1, unit: "A" },
      { key: "i_c", label: "Current_C", group: 30, variation: 4, index: 5, base: 145.1, variance: 3.8, noise: 1.3, periodMs: 18, min: 100, max: 200, decimals: 1, unit: "A" },
      { key: "freq", label: "Frequency", group: 30, variation: 4, index: 6, base: 50.02, variance: 0.01, noise: 0.005, periodMs: 10, min: 49.9, max: 50.1, decimals: 2, unit: "Hz" },
      { key: "mw", label: "MW", group: 30, variation: 4, index: 7, base: 3.42, variance: 0.12, noise: 0.03, periodMs: 25, min: 2.5, max: 4.5, decimals: 2, unit: "MW" },
      { key: "mvar", label: "MVAR", group: 30, variation: 4, index: 8, base: 0.85, variance: 0.08, noise: 0.02, periodMs: 22, min: 0, max: 1.5, decimals: 2, unit: "MVAR" },
      { key: "pf", label: "PowerFactor", group: 30, variation: 4, index: 9, base: 0.97, variance: 0.01, noise: 0.003, periodMs: 20, min: 0.9, max: 1.0, decimals: 3 },
      { key: "breaker", label: "Breaker_52_1", group: 1, variation: 2, index: 0, base: 1, variance: 0, noise: 0, periodMs: 100, isBool: true },
      { key: "recloser", label: "Recloser", group: 1, variation: 2, index: 1, base: 1, variance: 0, noise: 0, periodMs: 100, isBool: true },
      { key: "energy_kwh", label: "Energy_kWh", group: 20, variation: 1, index: 0, base: 0, variance: 5, noise: 0, periodMs: 1000, min: 0, max: 999999, decimals: 0, unit: "kWh" },
      { key: "energy_kvarh", label: "Energy_kvarh", group: 20, variation: 1, index: 1, base: 0, variance: 3, noise: 0, periodMs: 1000, min: 0, max: 999999, decimals: 0, unit: "kvarh" },
      { key: "pulse_1", label: "PulseCounter_1", group: 20, variation: 1, index: 2, base: 0, variance: 2, noise: 0, periodMs: 1500, min: 0, max: 99999, decimals: 0 },
      { key: "pulse_2", label: "PulseCounter_2", group: 20, variation: 1, index: 3, base: 0, variance: 2, noise: 0, periodMs: 1500, min: 0, max: 99999, decimals: 0 },
    ],
  },
  {
    id: "transformer",
    name: "Transformer-1 (10MVA 69/13.8kV)",
    description: "Power transformer with oil/winding temp, tap position, load",
    points: [
      { key: "oil_temp", label: "OilTemp", group: 30, variation: 4, index: 0, base: 62.3, variance: 1.5, noise: 0.3, periodMs: 60, min: 50, max: 85, decimals: 1, unit: "\u00b0C" },
      { key: "winding_temp", label: "WindingTemp", group: 30, variation: 4, index: 1, base: 78.5, variance: 2.0, noise: 0.5, periodMs: 45, min: 60, max: 105, decimals: 1, unit: "\u00b0C" },
      { key: "tap_pos", label: "TapPosition", group: 30, variation: 4, index: 2, base: 5, variance: 0, noise: 0, periodMs: 100, decimals: 0 },
      { key: "load_i", label: "LoadCurrent", group: 30, variation: 4, index: 3, base: 425.7, variance: 12, noise: 3, periodMs: 20, min: 300, max: 550, decimals: 1, unit: "A" },
      { key: "load_mva", label: "LoadMVA", group: 30, variation: 4, index: 4, base: 10.2, variance: 0.3, noise: 0.1, periodMs: 25, min: 7, max: 12, decimals: 1, unit: "MVA" },
      { key: "v_hv", label: "Voltage_HV", group: 30, variation: 4, index: 5, base: 69.0, variance: 0.2, noise: 0.05, periodMs: 20, decimals: 1, unit: "kV" },
      { key: "v_lv", label: "Voltage_LV", group: 30, variation: 4, index: 6, base: 13.8, variance: 0.05, noise: 0.02, periodMs: 20, decimals: 2, unit: "kV" },
      { key: "fan1", label: "FanGroup1", group: 1, variation: 2, index: 0, base: 1, variance: 0, noise: 0, periodMs: 100, isBool: true },
      { key: "fan2", label: "FanGroup2", group: 1, variation: 2, index: 1, base: 0, variance: 0, noise: 0, periodMs: 100, isBool: true },
      { key: "alarm", label: "Alarm", group: 1, variation: 2, index: 2, base: 0, variance: 0, noise: 0, periodMs: 100, isBool: true },
    ],
  },
];

const CUSTOM_TEMPLATES_KEY = "dnp3-explorer-custom-templates";

export function loadCustomTemplates(): DeviceTemplate[] {
  try {
    const raw = localStorage.getItem(CUSTOM_TEMPLATES_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveCustomTemplates(templates: DeviceTemplate[]) {
  localStorage.setItem(CUSTOM_TEMPLATES_KEY, JSON.stringify(templates));
}
