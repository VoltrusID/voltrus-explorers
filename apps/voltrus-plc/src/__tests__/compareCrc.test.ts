import { describe, it, expect } from "vitest";
import { compareCrc, formatCrcHex } from "../components/Sidebar/compareCrc";

describe("compareCrc", () => {
  it("reports sync when both CRCs match", () => {
    expect(compareCrc(0xdeadbeef, 0xdeadbeef)).toBe("sync");
  });

  it("reports drift when the CRCs differ", () => {
    expect(compareCrc(0xdeadbeef, 0x12345678)).toBe("drift");
  });

  it("reports unknown when either side is missing", () => {
    expect(compareCrc(null, 0xdeadbeef)).toBe("unknown");
    expect(compareCrc(0xdeadbeef, null)).toBe("unknown");
    expect(compareCrc(undefined, undefined)).toBe("unknown");
  });

  it("treats 0 as unknown (nothing loaded)", () => {
    expect(compareCrc(0, 0xdeadbeef)).toBe("unknown");
    expect(compareCrc(0xdeadbeef, 0)).toBe("unknown");
    expect(compareCrc(0, 0)).toBe("unknown");
  });
});

describe("formatCrcHex", () => {
  it("renders 8-digit uppercase hex", () => {
    expect(formatCrcHex(0xdeadbeef)).toBe("0xDEADBEEF");
  });

  it("zero-pads short values", () => {
    expect(formatCrcHex(0x1)).toBe("0x00000001");
  });
});
