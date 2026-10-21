import { describe, it, expect } from "vitest";
import {
  BAUD_PRESETS,
  PARITY_OPTIONS,
  DATA_BITS_OPTIONS,
  STOP_BITS_OPTIONS,
  TEMPLATES,
} from "../constants";

describe("BAUD_PRESETS", () => {
  it("is sorted in ascending order", () => {
    const sorted = [...BAUD_PRESETS].sort((a, b) => a - b);
    expect(BAUD_PRESETS).toEqual(sorted);
  });

  it("has no duplicates", () => {
    expect(new Set(BAUD_PRESETS).size).toBe(BAUD_PRESETS.length);
  });

  it("includes minimum standard rates", () => {
    expect(BAUD_PRESETS).toContain(9600);
    expect(BAUD_PRESETS).toContain(115200);
  });

  it("all values are positive integers", () => {
    for (const rate of BAUD_PRESETS) {
      expect(Number.isInteger(rate)).toBe(true);
      expect(rate).toBeGreaterThan(0);
    }
  });
});

describe("PARITY_OPTIONS", () => {
  it("includes required values", () => {
    expect(PARITY_OPTIONS).toContain("none");
    expect(PARITY_OPTIONS).toContain("even");
    expect(PARITY_OPTIONS).toContain("odd");
  });

  it("has no duplicates", () => {
    expect(new Set(PARITY_OPTIONS).size).toBe(PARITY_OPTIONS.length);
  });
});

describe("DATA_BITS_OPTIONS", () => {
  it("range covers 5–8", () => {
    expect(DATA_BITS_OPTIONS).toContain(5);
    expect(DATA_BITS_OPTIONS).toContain(8);
  });

  it("all values are valid serial data bits", () => {
    for (const bits of DATA_BITS_OPTIONS) {
      expect(bits).toBeGreaterThanOrEqual(5);
      expect(bits).toBeLessThanOrEqual(8);
    }
  });
});

describe("STOP_BITS_OPTIONS", () => {
  it("includes 1 and 2", () => {
    expect(STOP_BITS_OPTIONS).toContain(1);
    expect(STOP_BITS_OPTIONS).toContain(2);
  });
});

describe("TEMPLATES", () => {
  it("has modbus_master and modbus_slave entries", () => {
    expect(Object.keys(TEMPLATES)).toContain("modbus_master");
    expect(Object.keys(TEMPLATES)).toContain("modbus_slave");
  });

  it("each template has name, description, and non-empty frames", () => {
    for (const [key, tmpl] of Object.entries(TEMPLATES)) {
      expect(tmpl.name.length).toBeGreaterThan(0);
      expect(tmpl.description.length).toBeGreaterThan(0);
      expect(tmpl.frames.length).toBeGreaterThan(0);
    }
  });

  it("each frame has valid direction, label, hexData, and intervalMs", () => {
    for (const tmpl of Object.values(TEMPLATES)) {
      for (const frame of tmpl.frames) {
        expect(["TX", "RX"]).toContain(frame.direction);
        expect(frame.label.length).toBeGreaterThan(0);
        expect(frame.hexData.length).toBeGreaterThan(0);
        expect(frame.intervalMs).toBeGreaterThan(0);
      }
    }
  });

  it("hexData in all frames are valid hex strings", () => {
    const hexRegex = /^([0-9A-Fa-f]{2}\s*)+$/;
    for (const tmpl of Object.values(TEMPLATES)) {
      for (const frame of tmpl.frames) {
        expect(
          hexRegex.test(frame.hexData),
          `Invalid hex: "${frame.hexData}" in ${tmpl.name}/${frame.label}`
        ).toBe(true);
      }
    }
  });

  it("modbus_master frames alternate TX/RX for realistic polling", () => {
    const frames = TEMPLATES.modbus_master.frames;
    for (let i = 0; i < frames.length; i++) {
      expect(frames[i].direction).toBe(i % 2 === 0 ? "TX" : "RX");
    }
  });
});
