export interface RegisterDef {
  name: string;
  unit: string;
  description: string;
  initial: number;
  min: number;
  max: number;
  drift: number;
  isCounter: boolean;
  counterRate: number;
  locked?: boolean;
}

export interface DeviceProfile {
  id: string;
  manufacturer: string;
  model: string;
  description: string;
  baseAddress: number;
  registers: RegisterDef[];
}

// ── Built-in device profiles ──

export const SMA_INVERTER: DeviceProfile = {
  id: "sma-inverter",
  manufacturer: "SMA",
  model: "STP 60-US-10",
  description: "Sunny Tripower 60TL-US 60kW String Inverter",
  baseAddress: 30769,
  registers: [
    { name: "DC_Voltage_A", unit: "V", description: "DC input voltage string A", initial: 580, min: 0, max: 800, drift: 2, isCounter: false, counterRate: 0 },
    { name: "DC_Voltage_B", unit: "V", description: "DC input voltage string B", initial: 575, min: 0, max: 800, drift: 2, isCounter: false, counterRate: 0 },
    { name: "DC_Current_A", unit: "A", description: "DC input current string A", initial: 52, min: 0, max: 110, drift: 1.5, isCounter: false, counterRate: 0 },
    { name: "DC_Current_B", unit: "A", description: "DC input current string B", initial: 49, min: 0, max: 110, drift: 1.5, isCounter: false, counterRate: 0 },
    { name: "DC_Power", unit: "W", description: "Total DC input power", initial: 58000, min: 0, max: 62000, drift: 500, isCounter: false, counterRate: 0 },
    { name: "AC_Voltage_L1", unit: "V", description: "AC output voltage L1-N", initial: 230.5, min: 207, max: 253, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "AC_Voltage_L2", unit: "V", description: "AC output voltage L2-N", initial: 229.8, min: 207, max: 253, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "AC_Voltage_L3", unit: "V", description: "AC output voltage L3-N", initial: 230.2, min: 207, max: 253, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "AC_Current_L1", unit: "A", description: "AC output current L1", initial: 86.5, min: 0, max: 100, drift: 1, isCounter: false, counterRate: 0 },
    { name: "AC_Current_L2", unit: "A", description: "AC output current L2", initial: 84.2, min: 0, max: 100, drift: 1, isCounter: false, counterRate: 0 },
    { name: "AC_Current_L3", unit: "A", description: "AC output current L3", initial: 85.8, min: 0, max: 100, drift: 1, isCounter: false, counterRate: 0 },
    { name: "AC_Power", unit: "W", description: "Total AC active power output", initial: 56500, min: 0, max: 60000, drift: 400, isCounter: false, counterRate: 0 },
    { name: "AC_Frequency", unit: "Hz", description: "Grid frequency", initial: 50.01, min: 49.8, max: 50.2, drift: 0.003, isCounter: false, counterRate: 0 },
    { name: "Efficiency", unit: "%", description: "Current conversion efficiency", initial: 97.4, min: 90, max: 98.5, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "PowerFactor", unit: "", description: "AC power factor", initial: 0.99, min: 0.9, max: 1, drift: 0.002, isCounter: false, counterRate: 0 },
    { name: "Temperature", unit: "\u00b0C", description: "Inverter heatsink temperature", initial: 45.2, min: 10, max: 75, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "Irradiance", unit: "W/m\u00b2", description: "Plane-of-array irradiance", initial: 856, min: 0, max: 1100, drift: 15, isCounter: false, counterRate: 0 },
    { name: "Daily_Yield", unit: "kWh", description: "Energy produced today", initial: 185, min: 0, max: 400, drift: 0, isCounter: true, counterRate: 56.5 / 3600 },
    { name: "Total_Yield", unit: "kWh", description: "Lifetime energy produced", initial: 452380, min: 0, max: 9999999, drift: 0, isCounter: true, counterRate: 56.5 / 3600 },
    { name: "Operating_Time", unit: "h", description: "Total operating hours", initial: 8920, min: 0, max: 999999, drift: 0, isCounter: true, counterRate: 1 / 3600 },
  ],
};

export const PM5560: DeviceProfile = {
  id: "pm5560",
  manufacturer: "Schneider Electric",
  model: "PM5560",
  description: "PM5560 3-Phase Power Quality Meter (0.2S, CT/VT)",
  baseAddress: 2999,
  registers: [
    { name: "Voltage_L1N", unit: "V", description: "L1-N voltage", initial: 230.2, min: 0, max: 347, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "Voltage_L2N", unit: "V", description: "L2-N voltage", initial: 229.8, min: 0, max: 347, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "Voltage_L3N", unit: "V", description: "L3-N voltage", initial: 230.5, min: 0, max: 347, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "Current_L1", unit: "A", description: "L1 current (via CT)", initial: 142.5, min: 0, max: 6000, drift: 1, isCounter: false, counterRate: 0 },
    { name: "Current_L2", unit: "A", description: "L2 current (via CT)", initial: 138.7, min: 0, max: 6000, drift: 1, isCounter: false, counterRate: 0 },
    { name: "Current_L3", unit: "A", description: "L3 current (via CT)", initial: 145.3, min: 0, max: 6000, drift: 1, isCounter: false, counterRate: 0 },
    { name: "Active_Power", unit: "W", description: "3-phase active power", initial: 95000, min: -999999, max: 999999, drift: 500, isCounter: false, counterRate: 0 },
    { name: "Reactive_Power", unit: "var", description: "3-phase reactive power", initial: -12000, min: -999999, max: 999999, drift: 200, isCounter: false, counterRate: 0 },
    { name: "Apparent_Power", unit: "VA", description: "3-phase apparent power", initial: 98000, min: 0, max: 999999, drift: 500, isCounter: false, counterRate: 0 },
    { name: "Power_Factor", unit: "", description: "Overall power factor", initial: 0.96, min: -1, max: 1, drift: 0.003, isCounter: false, counterRate: 0 },
    { name: "Frequency", unit: "Hz", description: "Grid frequency", initial: 50, min: 45, max: 65, drift: 0.002, isCounter: false, counterRate: 0 },
    { name: "THD_Voltage", unit: "%", description: "Avg voltage THD", initial: 1.8, min: 0, max: 50, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "THD_Current", unit: "%", description: "Avg current THD", initial: 6.2, min: 0, max: 50, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Temperature", unit: "\u00b0C", description: "Internal temperature", initial: 38.5, min: -40, max: 85, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "Energy_Import", unit: "kWh", description: "Cumulative active energy import", initial: 1587329, min: 0, max: 999999999, drift: 0, isCounter: true, counterRate: 95 / 3600 },
    { name: "Energy_Export", unit: "kWh", description: "Cumulative active energy export", initial: 2451, min: 0, max: 999999999, drift: 0, isCounter: true, counterRate: 0.5 / 3600 },
  ],
};

export const EH_FLOWMETER: DeviceProfile = {
  id: "eh-flowmeter",
  manufacturer: "Endress+Hauser",
  model: "Promag 10W DN50",
  description: "Electromagnetic flowmeter, DN50, 0-50 m\u00b3/h",
  baseAddress: 100,
  registers: [
    { name: "Flow_Rate", unit: "m\u00b3/h", description: "Instantaneous volume flow rate", initial: 12.5, min: 0, max: 50, drift: 0.3, isCounter: false, counterRate: 0 },
    { name: "Flow_Velocity", unit: "m/s", description: "Flow velocity in pipe", initial: 1.77, min: 0, max: 7, drift: 0.05, isCounter: false, counterRate: 0 },
    { name: "Volume_Total", unit: "m\u00b3", description: "Cumulative volume totalizer", initial: 245891, min: 0, max: 999999999, drift: 0, isCounter: true, counterRate: 12.5 / 3600 },
    { name: "Conductivity", unit: "\u00b5S/cm", description: "Fluid conductivity", initial: 520, min: 0, max: 5000, drift: 5, isCounter: false, counterRate: 0 },
    { name: "Temperature", unit: "\u00b0C", description: "Process fluid temperature", initial: 15.2, min: -20, max: 60, drift: 0.05, isCounter: false, counterRate: 0 },
    { name: "Pipe_Volume", unit: "%", description: "Pipe fill level", initial: 100, min: 0, max: 100, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Signal_Quality", unit: "%", description: "Measurement signal quality", initial: 98.5, min: 0, max: 100, drift: 0.1, isCounter: false, counterRate: 0 },
  ],
};

export const ABB_ANALYZER: DeviceProfile = {
  id: "abb-analyzer",
  manufacturer: "ABB",
  model: "AZL402",
  description: "Water Quality Analyzer (pH, ORP, Turbidity, DO, Cond, Temp)",
  baseAddress: 200,
  registers: [
    { name: "pH", unit: "pH", description: "pH value", initial: 7.2, min: 0, max: 14, drift: 0.02, isCounter: false, counterRate: 0 },
    { name: "ORP", unit: "mV", description: "Oxidation-reduction potential", initial: 245, min: -2000, max: 2000, drift: 2, isCounter: false, counterRate: 0 },
    { name: "Turbidity", unit: "NTU", description: "Water turbidity", initial: 0.45, min: 0, max: 100, drift: 0.02, isCounter: false, counterRate: 0 },
    { name: "Dissolved_Oxygen", unit: "mg/L", description: "Dissolved oxygen concentration", initial: 8.2, min: 0, max: 20, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "Conductivity", unit: "\u00b5S/cm", description: "Electrical conductivity", initial: 485, min: 0, max: 5000, drift: 3, isCounter: false, counterRate: 0 },
    { name: "Chlorine_Free", unit: "mg/L", description: "Free chlorine residual", initial: 0.85, min: 0, max: 5, drift: 0.02, isCounter: false, counterRate: 0 },
    { name: "Temperature", unit: "\u00b0C", description: "Water temperature", initial: 15.8, min: 0, max: 40, drift: 0.02, isCounter: false, counterRate: 0 },
  ],
};

export const DANIEL_FLOW: DeviceProfile = {
  id: "daniel-flow",
  manufacturer: "Emerson",
  model: "Daniel 1000",
  description: "Gas Flow Computer (orifice meter, AGA3/AGA8)",
  baseAddress: 3000,
  registers: [
    { name: "Diff_Pressure", unit: "inH2O", description: "Orifice differential pressure", initial: 85, min: 0, max: 400, drift: 1, isCounter: false, counterRate: 0 },
    { name: "Static_Pressure", unit: "psig", description: "Line static pressure", initial: 745, min: 0, max: 1500, drift: 2, isCounter: false, counterRate: 0 },
    { name: "Temperature", unit: "\u00b0F", description: "Gas temperature", initial: 95, min: -40, max: 200, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "Flow_Rate", unit: "MSCFH", description: "Instantaneous gas flow rate", initial: 1250, min: 0, max: 5000, drift: 20, isCounter: false, counterRate: 0 },
    { name: "Volume_Today", unit: "MSCF", description: "Accumulated gas volume today", initial: 18500, min: 0, max: 120000, drift: 0, isCounter: true, counterRate: 1250 / 3600 },
    { name: "Volume_Total", unit: "MMSCF", description: "Lifetime accumulated volume", initial: 8452.3, min: 0, max: 999999, drift: 0, isCounter: true, counterRate: 1.25 / 3600 },
    { name: "Gas_Gravity", unit: "", description: "Specific gravity (air=1)", initial: 0.62, min: 0.5, max: 1, drift: 0.001, isCounter: false, counterRate: 0 },
    { name: "Heating_Value", unit: "BTU/SCF", description: "Higher heating value", initial: 1025, min: 900, max: 1200, drift: 1, isCounter: false, counterRate: 0 },
    { name: "CH4", unit: "mol%", description: "Methane fraction", initial: 88.5, min: 0, max: 100, drift: 0.05, isCounter: false, counterRate: 0 },
    { name: "CO2", unit: "mol%", description: "Carbon dioxide fraction", initial: 1.8, min: 0, max: 30, drift: 0.02, isCounter: false, counterRate: 0 },
    { name: "H2S", unit: "ppm", description: "Hydrogen sulfide", initial: 12, min: 0, max: 5000, drift: 0.5, isCounter: false, counterRate: 0 },
  ],
};

export const RARITAN_PDU: DeviceProfile = {
  id: "raritan-pdu",
  manufacturer: "Raritan",
  model: "PX3-5902V",
  description: "Intelligent Rack PDU, 3-phase 32A, 24 outlets",
  baseAddress: 1,
  registers: [
    { name: "Voltage_L1", unit: "V", description: "L1-N voltage at PDU inlet", initial: 208.5, min: 180, max: 240, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Voltage_L2", unit: "V", description: "L2-N voltage at PDU inlet", initial: 209.1, min: 180, max: 240, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Voltage_L3", unit: "V", description: "L3-N voltage at PDU inlet", initial: 208.8, min: 180, max: 240, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Current_L1", unit: "A", description: "L1 branch circuit current", initial: 18.5, min: 0, max: 32, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Current_L2", unit: "A", description: "L2 branch circuit current", initial: 17.2, min: 0, max: 32, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Current_L3", unit: "A", description: "L3 branch circuit current", initial: 19.1, min: 0, max: 32, drift: 0.2, isCounter: false, counterRate: 0 },
    { name: "Active_Power", unit: "W", description: "Total PDU active power", initial: 8650, min: 0, max: 13300, drift: 50, isCounter: false, counterRate: 0 },
    { name: "Power_Factor", unit: "", description: "PDU overall power factor", initial: 0.98, min: 0, max: 1, drift: 0.005, isCounter: false, counterRate: 0 },
    { name: "Energy_Total", unit: "kWh", description: "Cumulative energy", initial: 42580, min: 0, max: 999999999, drift: 0, isCounter: true, counterRate: 8.65 / 3600 },
    { name: "Temperature_Inlet", unit: "\u00b0C", description: "PDU inlet air temperature", initial: 24.5, min: 10, max: 50, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "Temperature_Outlet", unit: "\u00b0C", description: "PDU outlet air temperature", initial: 32.8, min: 10, max: 60, drift: 0.1, isCounter: false, counterRate: 0 },
    { name: "Humidity", unit: "%", description: "Rack relative humidity", initial: 45, min: 10, max: 90, drift: 0.5, isCounter: false, counterRate: 0 },
  ],
};

export const BUILTIN_PROFILES: DeviceProfile[] = [
  PM5560,
  SMA_INVERTER,
  EH_FLOWMETER,
  ABB_ANALYZER,
  DANIEL_FLOW,
  RARITAN_PDU,
];

// ── Custom template storage ──

const STORAGE_KEY = "modbus-poll-custom-templates";

export function loadCustomTemplates(): DeviceProfile[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveCustomTemplates(templates: DeviceProfile[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(templates));
}

// ── Simulation logic ──

export function initialValues(profile: DeviceProfile): number[] {
  return profile.registers.map((r) => r.initial);
}

export function numRegisters(profile: DeviceProfile): number {
  return profile.registers.reduce((sum, r) => sum + (r.isCounter ? 4 : 2), 0);
}

export function tickValues(
  profile: DeviceProfile,
  values: number[],
  loadFactor: number
): { values: number[]; loadFactor: number } {
  const next = [...values];
  const newLoad = (loadFactor + (2 * Math.PI) / 900 + (Math.random() - 0.5) * 0.002) % (2 * Math.PI);
  const load = 1 + 0.15 * Math.sin(newLoad);

  for (let i = 0; i < profile.registers.length; i++) {
    const def = profile.registers[i];
    if (def.isCounter) {
      const variation = (Math.random() - 0.5) * def.counterRate * 0.1;
      next[i] = next[i] + def.counterRate + variation;
    } else {
      const drift = (Math.random() * 2 - 1) * def.drift;
      let loadEffect = 0;
      if (def.unit === "W" || def.unit === "A" || def.unit === "VA" || def.unit === "var") {
        loadEffect = (load - 1) * Math.abs(def.initial) * 0.1;
      } else if (def.unit === "V" || def.unit === "kV") {
        loadEffect = -(load - 1) * 0.5;
      }
      next[i] = Math.min(Math.max(next[i] + drift + loadEffect, def.min), def.max);
    }
  }

  return { values: next, loadFactor: newLoad };
}

export interface RegWrite {
  address: number;
  value: number;
}

export function valuesToRegisters(values: number[], profile: DeviceProfile): RegWrite[] {
  const writes: RegWrite[] = [];
  let offset = 0;
  const base = profile.baseAddress;

  for (let i = 0; i < profile.registers.length; i++) {
    const def = profile.registers[i];
    if (def.isCounter) {
      // u64 stored as 4 u16 (big-endian)
      const raw = Math.floor(values[i]);
      const hi32 = Math.floor(raw / 4294967296) >>> 0;
      const lo32 = (raw >>> 0) & 0xffffffff;
      writes.push(
        { address: base + offset, value: (hi32 >>> 16) & 0xffff },
        { address: base + offset + 1, value: hi32 & 0xffff },
        { address: base + offset + 2, value: (lo32 >>> 16) & 0xffff },
        { address: base + offset + 3, value: lo32 & 0xffff }
      );
      offset += 4;
    } else {
      // Float32 stored as 2 u16 (big-endian IEEE 754)
      const buf = new ArrayBuffer(4);
      new Float32Array(buf)[0] = values[i];
      const bits = new Uint32Array(buf)[0];
      writes.push(
        { address: base + offset, value: (bits >>> 16) & 0xffff },
        { address: base + offset + 1, value: bits & 0xffff }
      );
      offset += 2;
    }
  }

  return writes;
}

/** Decode raw u16 register array into labeled values based on a profile. */
export interface DecodedReg {
  address: number;
  count: number;        // how many u16 regs this value spans
  name: string;
  unit: string;
  raw: string;          // hex representation
  decimals: number[];   // raw u16 decimal values
  type: string;         // "Float32 BE" or "UInt64 Counter BE"
  decoded: string;      // human-readable value
}

export function decodeRegisters(regs: number[], profile: DeviceProfile): DecodedReg[] {
  const result: DecodedReg[] = [];
  const base = profile.baseAddress;
  let offset = 0;

  for (let i = 0; i < profile.registers.length; i++) {
    const def = profile.registers[i];
    const addr = base + offset;

    if (def.isCounter) {
      const r = [regs[offset] ?? 0, regs[offset + 1] ?? 0, regs[offset + 2] ?? 0, regs[offset + 3] ?? 0];
      const hi32 = (r[0] << 16) | r[1];
      const lo32 = (r[2] << 16) | r[3];
      const value = hi32 * 4294967296 + lo32;
      result.push({
        address: addr,
        count: 4,
        name: def.name,
        unit: def.unit,
        raw: r.map((v) => v.toString(16).toUpperCase().padStart(4, "0")).join(" "),
        decimals: r,
        type: "UInt64 Counter BE",
        decoded: value.toLocaleString() + (def.unit ? ` ${def.unit}` : ""),
      });
      offset += 4;
    } else {
      const hi = regs[offset] ?? 0;
      const lo = regs[offset + 1] ?? 0;
      const bits = (hi << 16) | lo;
      const buf = new ArrayBuffer(4);
      new Uint32Array(buf)[0] = bits >>> 0;
      const value = new Float32Array(buf)[0];
      const decimals = def.drift < 0.01 ? 3 : def.drift < 0.1 ? 2 : 1;
      result.push({
        address: addr,
        count: 2,
        name: def.name,
        unit: def.unit,
        raw: `${hi.toString(16).toUpperCase().padStart(4, "0")} ${lo.toString(16).toUpperCase().padStart(4, "0")}`,
        decimals: [hi, lo],
        type: "Float32 BE",
        decoded: value.toFixed(decimals) + (def.unit ? ` ${def.unit}` : ""),
      });
      offset += 2;
    }
  }

  return result;
}
