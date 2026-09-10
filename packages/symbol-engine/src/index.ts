// @mactools/symbol-engine — shared 2D symbol-editor engine.
// Standalone: zero runtime deps, pure functions + one History class.

// ── Model ──
export type { JsonValue, SymbolInstance, SymbolDoc } from "./model.js";

// ── Geometry ──
export {
  GRID,
  snap,
  moveSymbol,
  resizeSymbol,
  bringToFront,
  sendToBack,
} from "./geometry.js";
export type { ResizeHandle } from "./geometry.js";

// ── History ──
export { History } from "./history.js";

// ── Clipboard ──
export { cloneWithNewIds } from "./clipboard.js";

// ── Util ──
export { newId, clamp } from "./util.js";
