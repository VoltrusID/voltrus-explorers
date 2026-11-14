import { describe, expect, it } from "vitest";
import { TEMPLATES, simValue } from "../simTemplates";

describe("simulator templates", () => {
  it("keeps values inside their limits and counters rising", () => {
    const level = TEMPLATES.fins.points.find((p) => p.address === "D104")!;
    for (let t = 0; t < 200; t += 7) {
      const v = simValue(level, t);
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(100);
    }
    const good = TEMPLATES.mc.points.find((p) => p.address === "D102")!;
    expect(simValue(good, 10)).toBeGreaterThan(simValue(good, 0));
  });
  it("blinks bools", () => {
    const fault = TEMPLATES.mc.points.find((p) => p.address === "M1")!;
    expect(simValue(fault, 0)).toBe(0);
    expect(simValue(fault, 31)).toBe(1);
  });
});

import { isAnimated } from "../simTemplates";
describe("fixed points", () => {
  it("are not animated, so client writes stick", () => {
    expect(isAnimated(TEMPLATES.mc.points.find((p) => p.address === "D120")!)).toBe(false);
    expect(isAnimated(TEMPLATES.mc.points.find((p) => p.address === "D100")!)).toBe(true);
  });
});
