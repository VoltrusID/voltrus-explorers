import type { SymbolDoc, SymbolInstance } from "./model.js";
import { MIN_SIZE } from "./util.js";

/** Editor grid size (px). All resize results land on this grid. */
export const GRID = 10;

/** Snap `v` to the nearest multiple of `grid` (default {@link GRID}). */
export function snap(v: number, grid: number = GRID): number {
  if (!grid || grid <= 0) return v;
  return Math.round(v / grid) * grid;
}

export type ResizeHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

function mapSymbol(
  doc: SymbolDoc,
  id: string,
  fn: (s: SymbolInstance) => SymbolInstance,
): SymbolDoc {
  let touched = false;
  const symbols = doc.symbols.map((s) => {
    if (s.id !== id) return s;
    touched = true;
    return fn(s);
  });
  return touched ? { ...doc, symbols } : doc;
}

/** Translate a symbol by `(dx, dy)`. Pure; unknown id → doc unchanged. */
export function moveSymbol(
  doc: SymbolDoc,
  id: string,
  dx: number,
  dy: number,
): SymbolDoc {
  return mapSymbol(doc, id, (s) => ({ ...s, x: s.x + dx, y: s.y + dy }));
}

/**
 * Resize a symbol by dragging `handle` by `(dx, dy)`. The anchored edge(s)
 * stay fixed; the result snaps to the grid and clamps to {@link MIN_SIZE}.
 * Pure; unknown id → doc unchanged.
 */
export function resizeSymbol(
  doc: SymbolDoc,
  id: string,
  handle: ResizeHandle,
  dx: number,
  dy: number,
): SymbolDoc {
  return mapSymbol(doc, id, (s) => {
    let left = s.x;
    let top = s.y;
    let right = s.x + s.w;
    let bottom = s.y + s.h;

    if (handle.includes("w")) left = snap(left + dx);
    if (handle.includes("e")) right = snap(right + dx);
    if (handle.includes("n")) top = snap(top + dy);
    if (handle.includes("s")) bottom = snap(bottom + dy);

    // Min-size clamp pins the moving edge so the anchored edge never moves.
    if (right - left < MIN_SIZE) {
      if (handle.includes("w")) left = right - MIN_SIZE;
      else right = left + MIN_SIZE;
    }
    if (bottom - top < MIN_SIZE) {
      if (handle.includes("n")) top = bottom - MIN_SIZE;
      else bottom = top + MIN_SIZE;
    }

    return { ...s, x: left, y: top, w: right - left, h: bottom - top };
  });
}

/**
 * Raise a symbol to the top of the z-order (end of the symbols array).
 * Reindexes `zIndex` to match array order. Pure; unknown id → doc unchanged.
 */
export function bringToFront(doc: SymbolDoc, id: string): SymbolDoc {
  const s = doc.symbols.find((w) => w.id === id);
  if (!s) return doc;
  const rest = doc.symbols.filter((w) => w.id !== id);
  const symbols = [...rest, s].map((w, i) =>
    w.zIndex === i ? w : { ...w, zIndex: i },
  );
  return { ...doc, symbols };
}

/**
 * Sink a symbol to the back of the z-order (start of the symbols array).
 * Reindexes `zIndex` to match array order. Pure; unknown id → doc unchanged.
 */
export function sendToBack(doc: SymbolDoc, id: string): SymbolDoc {
  const s = doc.symbols.find((w) => w.id === id);
  if (!s) return doc;
  const rest = doc.symbols.filter((w) => w.id !== id);
  const symbols = [s, ...rest].map((w, i) =>
    w.zIndex === i ? w : { ...w, zIndex: i },
  );
  return { ...doc, symbols };
}
