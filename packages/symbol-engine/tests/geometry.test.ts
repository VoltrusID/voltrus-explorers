import { describe, expect, it } from "vitest";
import {
  GRID,
  bringToFront,
  moveSymbol,
  resizeSymbol,
  sendToBack,
  snap,
} from "../src";
import { abcDoc, doc, symbol } from "./fixtures";

describe("snap", () => {
  it("rounds to the nearest multiple of GRID", () => {
    expect(GRID).toBe(10);
    expect(snap(15)).toBe(20);
    expect(snap(14)).toBe(10);
    expect(snap(24)).toBe(20);
    expect(snap(-14)).toBe(-10);
    expect(snap(-16)).toBe(-20);
    expect(snap(0)).toBe(0);
  });

  it("honors a custom grid", () => {
    expect(snap(7, 5)).toBe(5);
    expect(snap(13, 5)).toBe(15);
    expect(snap(12, 20)).toBe(20);
  });

  it("is a no-op for a non-positive grid", () => {
    expect(snap(33, 0)).toBe(33);
    expect(snap(33, -5)).toBe(33);
  });
});

describe("moveSymbol", () => {
  it("translates the symbol by (dx, dy)", () => {
    const d = doc([symbol({ id: "a", x: 100, y: 50 })]);
    const moved = moveSymbol(d, "a", -30, 15);
    expect(moved.symbols[0].x).toBe(70);
    expect(moved.symbols[0].y).toBe(65);
  });

  it("leaves other symbols untouched", () => {
    const d = abcDoc();
    const moved = moveSymbol(d, "b", 5, 5);
    expect(moved.symbols.find((s) => s.id === "a")!.x).toBe(0);
    expect(moved.symbols.find((s) => s.id === "c")!.y).toBe(20);
  });

  it("is immutable — input doc and symbols are not mutated", () => {
    const d = doc([symbol({ id: "a", x: 100, y: 50 })]);
    const moved = moveSymbol(d, "a", 10, 10);
    expect(moved).not.toBe(d);
    expect(moved.symbols[0]).not.toBe(d.symbols[0]);
    expect(d.symbols[0].x).toBe(100);
    expect(d.symbols[0].y).toBe(50);
  });

  it("returns the same doc for an unknown id", () => {
    const d = abcDoc();
    expect(moveSymbol(d, "nope", 10, 10)).toBe(d);
  });
});

describe("resizeSymbol", () => {
  // Fixture edges: left=100 top=100 right=160 bottom=140.
  const d = () => doc([symbol({ id: "a", x: 100, y: 100, w: 60, h: 40 })]);

  it("se grows width and height, snapping to the grid", () => {
    const r = resizeSymbol(d(), "a", "se", 7, -3);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 100, w: 70, h: 40 });
  });

  it("e moves only the right edge", () => {
    const r = resizeSymbol(d(), "a", "e", -25, 99);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 100, w: 40, h: 40 });
  });

  it("s moves only the bottom edge", () => {
    const r = resizeSymbol(d(), "a", "s", 99, 15);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 100, w: 60, h: 60 });
  });

  it("n moves the top edge down and shrinks height", () => {
    const r = resizeSymbol(d(), "a", "n", 0, 10);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 110, w: 60, h: 30 });
  });

  it("ne moves the top-right corner", () => {
    const r = resizeSymbol(d(), "a", "ne", 10, 8);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 110, w: 70, h: 30 });
  });

  it("nw moves the top-left corner", () => {
    const r = resizeSymbol(d(), "a", "nw", 5, -6);
    expect(r.symbols[0]).toMatchObject({ x: 110, y: 90, w: 50, h: 50 });
  });

  it("sw moves the bottom-left corner", () => {
    const r = resizeSymbol(d(), "a", "sw", -10, -5);
    expect(r.symbols[0]).toMatchObject({ x: 90, y: 100, w: 70, h: 40 });
  });

  it("w moves only the left edge", () => {
    const r = resizeSymbol(d(), "a", "w", -10, 99);
    expect(r.symbols[0]).toMatchObject({ x: 90, y: 100, w: 70, h: 40 });
  });

  it("clamps to min size 20 at the moving edge, anchoring the fixed edge (se)", () => {
    const r = resizeSymbol(d(), "a", "se", -100, -100);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 100, w: 20, h: 20 });
  });

  it("clamps to min size 20 at the moving edge, anchoring the fixed edge (nw)", () => {
    const r = resizeSymbol(d(), "a", "nw", 55, 55);
    expect(r.symbols[0]).toMatchObject({ x: 140, y: 120, w: 20, h: 20 });
  });

  it("clamps a single-axis handle (w)", () => {
    const r = resizeSymbol(d(), "a", "w", 55, 0);
    expect(r.symbols[0]).toMatchObject({ x: 140, y: 100, w: 20, h: 40 });
  });

  it("clamps a single-axis handle (n)", () => {
    const r = resizeSymbol(d(), "a", "n", 0, 55);
    expect(r.symbols[0]).toMatchObject({ x: 100, y: 120, w: 60, h: 20 });
  });

  it("keeps min-size results on the grid", () => {
    const r = resizeSymbol(d(), "a", "se", -1234, -4321);
    const s = r.symbols[0];
    expect(s.w).toBe(20);
    expect(s.h).toBe(20);
    expect(s.x % GRID).toBe(0);
    expect(s.y % GRID).toBe(0);
  });

  it("returns the same doc for an unknown id", () => {
    const dd = d();
    expect(resizeSymbol(dd, "nope", "se", 10, 10)).toBe(dd);
  });

  it("does not mutate the input", () => {
    const dd = d();
    resizeSymbol(dd, "a", "se", 50, 50);
    expect(dd.symbols[0]).toMatchObject({ x: 100, y: 100, w: 60, h: 40 });
  });
});

describe("bringToFront", () => {
  it("moves the symbol to the end and reindexes zIndex", () => {
    const r = bringToFront(abcDoc(), "b");
    expect(r.symbols.map((s) => s.id)).toEqual(["a", "c", "b"]);
    expect(r.symbols.map((s) => s.zIndex)).toEqual([0, 1, 2]);
  });

  it("is order-preserving when already on top", () => {
    const dd = abcDoc();
    const r = bringToFront(dd, "c");
    expect(r.symbols.map((s) => s.id)).toEqual(["a", "b", "c"]);
    expect(r.symbols.map((s) => s.zIndex)).toEqual([0, 1, 2]);
  });

  it("returns the same doc for an unknown id", () => {
    const dd = abcDoc();
    expect(bringToFront(dd, "nope")).toBe(dd);
  });

  it("does not mutate the input", () => {
    const dd = abcDoc();
    bringToFront(dd, "a");
    expect(dd.symbols.map((s) => s.id)).toEqual(["a", "b", "c"]);
  });
});

describe("sendToBack", () => {
  it("moves the symbol to the front of the array and reindexes zIndex", () => {
    const r = sendToBack(abcDoc(), "b");
    expect(r.symbols.map((s) => s.id)).toEqual(["b", "a", "c"]);
    expect(r.symbols.map((s) => s.zIndex)).toEqual([0, 1, 2]);
  });

  it("is order-preserving when already at the back", () => {
    const r = sendToBack(abcDoc(), "a");
    expect(r.symbols.map((s) => s.id)).toEqual(["a", "b", "c"]);
    expect(r.symbols.map((s) => s.zIndex)).toEqual([0, 1, 2]);
  });

  it("round-trips with bringToFront", () => {
    const dd = abcDoc();
    const r = sendToBack(bringToFront(dd, "a"), "a");
    expect(r.symbols.map((s) => s.id)).toEqual(["a", "b", "c"]);
  });

  it("returns the same doc for an unknown id", () => {
    const dd = abcDoc();
    expect(sendToBack(dd, "nope")).toBe(dd);
  });
});
