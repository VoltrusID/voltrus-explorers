import type { DisplayFormat, ReadBlock } from "../types";
import { blockStep, blockType, isBitArea, valueName, type ReadPlan } from "../helpers";
import { displayAddr, extractError, extractRaw, formatValue } from "./format";

// A type alias (not an interface) so rows satisfy DataTable's `Record<string, unknown>` rows.
export type TableRow = {
  name: string;
  /** Address as displayed (0- or 1-based). */
  address: string;
  /** Protocol address. */
  rawAddress: number;
  hex: string;
  type: string;
  area: string;
  /** Formatted for display. */
  value: string;
  /** The polled number itself (never a formatted string); null if nothing usable was read. */
  rawValue: number | null;
  /** Beyond the free polling cap: not polled, shown as "Pro". */
  limited?: true;
  /** The device answered the request covering this row with an exception. */
  error?: string;
  _blockIdx: number;
};

interface RowOptions {
  useOneBased: boolean;
  displayFormat: DisplayFormat;
}

/**
 * One row per value of every block. `values` is what `read_tags` returned for `plan.tags`:
 * one entry per *tag* (a float32 is one entry spanning two registers), so a row's value is
 * `values[plan.firstTag[blockIdx] + i]` — not a register offset. A plan cut short by the free cap
 * (see `limitReadPlan`) leaves the rows past its end marked `limited` and unread.
 */
export function buildTableRows(blocks: ReadBlock[], plan: ReadPlan, values: unknown[], options: RowOptions): TableRow[] {
  return blocks.flatMap((b, blockIdx) => {
    const step = blockStep(b);
    const bit = isBitArea(b.area);
    return Array.from({ length: b.quantity }, (_, i): TableRow => {
      const index = plan.firstTag[blockIdx] + i;
      const address = b.address + i * step;
      if (index >= plan.tags.length) {
        return {
          name: valueName(b, i),
          address: displayAddr(address, b.area, options.useOneBased),
          rawAddress: address,
          hex: "---",
          type: bit ? "bit" : blockType(b),
          area: b.area,
          value: "Pro",
          rawValue: null,
          limited: true,
          _blockIdx: blockIdx,
        };
      }
      const value = values[index];
      const raw = extractRaw(value);
      const error = extractError(value);
      const text = error || isNaN(raw) ? "—" : bit ? (raw !== 0 ? "ON" : "OFF") : formatValue(raw, b.type, options.displayFormat);
      return {
        name: valueName(b, i),
        address: displayAddr(address, b.area, options.useOneBased),
        rawAddress: address,
        hex: isNaN(raw) ? "---" : "0x" + ((raw | 0) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0"),
        type: bit ? "bit" : blockType(b),
        area: b.area,
        value: text,
        rawValue: isNaN(raw) ? null : raw,
        ...(error ? { error } : {}),
        _blockIdx: blockIdx,
      };
    });
  });
}
