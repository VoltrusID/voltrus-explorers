import { describe, it, expect } from "vitest";
import { FREE_CONNECTIONS, FREE_VALUES } from "../constants";
import { buildReadPlan } from "../helpers";
import {
  effectivePlan, exceedsFreeValues, limitReadPlan, mayOpenConnection, polledLimit, profileLoadNotice, valueCapNotice, valueCount,
} from "../limits";
import { buildTableRows } from "../utils/rows";
import type { ReadBlock } from "../types";

const block = (name: string, quantity: number, type = "uint16", address = 0): ReadBlock => ({ name, address, quantity, type, area: "holding" });
const opts = { useOneBased: false, displayFormat: "dec" as const };

describe("value cap", () => {
  it("counts values, not registers", () => {
    expect(valueCount([block("a", 3, "float32"), block("b", 2, "float64")])).toBe(5);
  });

  it("free polls the first FREE_VALUES values, Pro polls everything", () => {
    const blocks = [block("a", 15), block("b", 15, "uint16", 100)];
    expect(effectivePlan(blocks, false).tags).toHaveLength(FREE_VALUES);
    expect(effectivePlan(blocks, true).tags).toHaveLength(30);
    expect(polledLimit(true)).toBe(Infinity);
    expect(polledLimit(false)).toBe(FREE_VALUES);
  });

  it("the free plan is a prefix of the full plan, across blocks", () => {
    const blocks = [block("a", 15), block("b", 15, "uint16", 100)];
    const full = buildReadPlan(blocks);
    const free = effectivePlan(blocks, false);
    expect(free.tags).toEqual(full.tags.slice(0, FREE_VALUES));
    // second block is cut after 5 of its 15 values
    expect(free.tags.filter((t) => t.name.startsWith("b_"))).toHaveLength(5);
  });

  it("exactly FREE_VALUES is not over the cap", () => {
    const blocks = [block("a", FREE_VALUES)];
    expect(exceedsFreeValues(blocks, false)).toBe(false);
    expect(effectivePlan(blocks, false)).toEqual(buildReadPlan(blocks));
    expect(exceedsFreeValues([block("a", FREE_VALUES + 1)], false)).toBe(true);
  });

  it("limitReadPlan returns the same plan when it already fits", () => {
    const plan = buildReadPlan([block("a", 3)]);
    expect(limitReadPlan(plan, 20)).toBe(plan);
  });

  it("rows past the cap are marked limited and carry no value", () => {
    const blocks = [block("a", 15), block("b", 15, "uint16", 100)];
    const plan = effectivePlan(blocks, false);
    const values = plan.tags.map((_, i) => ({ Float64: i }));
    const rows = buildTableRows(blocks, plan, values, opts);
    expect(rows).toHaveLength(30);
    expect(rows.slice(0, FREE_VALUES).every((r) => !r.limited && r.rawValue !== null)).toBe(true);
    const limited = rows.slice(FREE_VALUES);
    expect(limited.every((r) => r.limited === true && r.value === "Pro" && r.rawValue === null)).toBe(true);
  });

  it("the default PM5560 map (16 values) is under the cap", () => {
    const pm5560 = [block("V", 3, "float32"), block("I", 3, "float32"), block("P", 3, "float32"), block("Q", 4, "float32"), block("T", 1, "float32"), block("E", 2, "float64")];
    expect(valueCount(pm5560)).toBe(16);
    expect(valueCapNotice(pm5560, false)).toBeNull();
  });
});

describe("cap notices", () => {
  const big = [block("a", 34)];

  it("tells a free user how many values are polled", () => {
    expect(valueCapNotice(big, false)).toBe("Polling the first 20 of 34 values. Pro polls all of them.");
  });

  it("is silent for Pro and for small lists", () => {
    expect(valueCapNotice(big, true)).toBeNull();
    expect(valueCapNotice([block("a", 5)], false)).toBeNull();
  });

  it("loading a profile over the cap says so, naming the profile", () => {
    const message = profileLoadNotice("Plant A", big, false);
    expect(message).toContain("Plant A");
    expect(message).toContain("34 values");
    expect(message).toContain("first 20");
    expect(profileLoadNotice("Plant A", big, true)).toBeNull();
    expect(profileLoadNotice("Plant A", [block("a", 20)], false)).toBeNull();
  });
});

describe("connection cap", () => {
  it("free allows exactly one connection", () => {
    expect(FREE_CONNECTIONS).toBe(1);
    expect(mayOpenConnection(0, false)).toBe(true);
    expect(mayOpenConnection(1, false)).toBe(false);
    expect(mayOpenConnection(3, false)).toBe(false);
  });

  it("Pro allows any number", () => {
    expect(mayOpenConnection(1, true)).toBe(true);
    expect(mayOpenConnection(12, true)).toBe(true);
  });
});
