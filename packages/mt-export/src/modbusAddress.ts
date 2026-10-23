import type { ModbusArea } from "./model";

/** Function code a master reads the area with. */
export const READ_FUNCTION: Record<ModbusArea, number> = { coil: 1, discrete: 2, holding: 3, input: 4 };

/** First digit of the conventional 6-digit reference: 0 coils, 1 discrete inputs, 3 input registers, 4 holding registers. */
const TABLE_DIGIT: Record<ModbusArea, string> = { coil: "0", discrete: "1", input: "3", holding: "4" };

/** The 1-based, 6-digit reference most tools show: holding register 0 is `400001`, coil 9 is `000010`. */
export function reference6(area: ModbusArea, address: number): string {
  return TABLE_DIGIT[area] + String(address + 1).padStart(5, "0");
}
