import type { SymbolInstance } from "./model.js";
import { deepClone, newId } from "./util.js";

/**
 * Duplicate symbols for a paste action: every copy gets a fresh id and a
 * fully isolated deep clone (config, bindings, …), optionally offset by
 * `(dx, dy)`. Pure — the input array and its symbols are untouched.
 */
export function cloneWithNewIds(
  symbols: SymbolInstance[],
  offset: { dx: number; dy: number } = { dx: 0, dy: 0 },
): SymbolInstance[] {
  return symbols.map((s) => ({
    ...deepClone(s),
    id: newId(),
    x: s.x + offset.dx,
    y: s.y + offset.dy,
  }));
}
