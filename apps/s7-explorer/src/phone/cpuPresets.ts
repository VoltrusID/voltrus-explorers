export interface CpuPreset {
  id: string;
  label: string;
  /** Where the CPU usually sits. */
  hint: string;
  rack: number;
  slot: number;
}

/** Rack and slot that reach the CPU of each controller family over ISO-on-TCP. */
export const CPU_PRESETS: readonly CpuPreset[] = [
  { id: "s7-1200-1500", label: "S7-1200 / S7-1500", hint: "Rack 0, slot 1", rack: 0, slot: 1 },
  { id: "s7-300", label: "S7-300", hint: "Rack 0, slot 2", rack: 0, slot: 2 },
  { id: "s7-400", label: "S7-400", hint: "Rack 0, slot 3 in most racks", rack: 0, slot: 3 },
];

export const CUSTOM_CPU = "custom";

/** The preset whose rack and slot match, or `custom`. */
export function cpuPresetFor(rack: number, slot: number): string {
  return CPU_PRESETS.find((p) => p.rack === rack && p.slot === slot)?.id ?? CUSTOM_CPU;
}

/** What to tell the user about reaching this family; empty for a custom rack and slot. */
export function cpuAccessHint(presetId: string): string {
  if (presetId === "s7-1200-1500") return "Needs “Permit access with PUT/GET” on in the CPU, and data blocks with optimized block access off.";
  if (presetId === "s7-300") return "Reads work as is. Put the CPU in slot 2 of rack 0.";
  if (presetId === "s7-400") return "Check the slot in the hardware configuration.";
  return "";
}
