import { describe, expect, it } from "vitest";
import { cloneWithNewIds } from "../src";
import { symbol } from "./fixtures";

describe("cloneWithNewIds", () => {
  it("assigns a fresh unique id to every clone", () => {
    const originals = [
      symbol({ id: "a" }),
      symbol({ id: "b" }),
      symbol({ id: "c" }),
    ];
    const clones = cloneWithNewIds(originals);
    expect(clones).toHaveLength(3);
    const ids = clones.map((s) => s.id);
    for (const id of ids) expect(originals.map((o) => o.id)).not.toContain(id);
    expect(new Set(ids).size).toBe(3);
  });

  it("applies the offset to x/y", () => {
    const clones = cloneWithNewIds([symbol({ id: "a", x: 100, y: 50 })], {
      dx: 20,
      dy: 30,
    });
    expect(clones[0].x).toBe(120);
    expect(clones[0].y).toBe(80);
  });

  it("defaults the offset to (0, 0)", () => {
    const clones = cloneWithNewIds([symbol({ id: "a", x: 100, y: 50 })]);
    expect(clones[0].x).toBe(100);
    expect(clones[0].y).toBe(50);
  });

  it("carries over all other fields", () => {
    const clones = cloneWithNewIds([
      symbol({
        id: "a",
        type: "valve",
        w: 70,
        h: 30,
        rotation: 90,
        zIndex: 3,
        label: "Intake",
        binding: "tank_1",
        bindings: ["s1", "s2"],
        config: { color: "red", min: 0, max: 100 },
      }),
    ]);
    const c = clones[0];
    expect(c).toMatchObject({
      type: "valve",
      w: 70,
      h: 30,
      rotation: 90,
      zIndex: 3,
      label: "Intake",
      binding: "tank_1",
    });
    expect(c.config).toEqual({ color: "red", min: 0, max: 100 });
    expect(c.bindings).toEqual(["s1", "s2"]);
  });

  it("deep-isolates config — mutating a clone leaves the original untouched", () => {
    const cfg = { color: "red", nested: { x: 1 } };
    const originals = [symbol({ id: "a", config: cfg })];
    const clones = cloneWithNewIds(originals);
    // engine stores config as JsonValue — cast back to the fixture's shape
    const cloneCfg = clones[0].config as typeof cfg;
    cloneCfg.nested.x = 5;
    cloneCfg.color = "blue";
    expect(cfg.nested.x).toBe(1);
    expect(cfg.color).toBe("red");
  });

  it("clones array fields — mutating a clone's bindings leaves the original untouched", () => {
    const originals = [symbol({ id: "a", bindings: ["s1"] })];
    const clones = cloneWithNewIds(originals);
    clones[0].bindings!.push("s2");
    expect(originals[0].bindings).toEqual(["s1"]);
  });

  it("does not mutate or alias the input", () => {
    const originals = [symbol({ id: "a", config: { k: 1 } })];
    const clones = cloneWithNewIds(originals, { dx: 5, dy: 5 });
    expect(clones[0]).not.toBe(originals[0]);
    expect(clones[0].config).not.toBe(originals[0].config);
    expect(originals).toHaveLength(1);
    expect(originals[0].id).toBe("a");
  });
});
