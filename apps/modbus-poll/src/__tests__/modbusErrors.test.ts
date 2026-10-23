import { describe, expect, it } from "vitest";
import { explainModbusError } from "../utils/format";

describe("explainModbusError", () => {
  it("says the device has no registers in the range and what to check, for exception 02", () => {
    const out = explainModbusError("Read failed: Illegal Data Address (0x02) reading holding 2999..3034");
    expect(out.startsWith("Read failed: Illegal Data Address (0x02) reading holding 2999..3034: ")).toBe(true);
    expect(out).toContain("the device has no registers in that range");
    // 0-based vs 1-based: the neighbours of the start address
    expect(out).toContain("try 3000 or 2998");
    // the other register table
    expect(out).toContain("try input instead of holding");
  });

  it("is case-insensitive (the same text lower-cased by a gateway or a log filter)", () => {
    expect(explainModbusError("read failed: illegal data address (0x02) reading input 10..19")).toContain("try holding instead of input");
  });

  it("only offers an address above 0 when the range starts at 0", () => {
    const out = explainModbusError("Illegal Data Address (0x02) reading holding 0..9");
    expect(out).toContain("so try 1,");
    expect(out).not.toContain("-1");
  });

  it("talks about bits for coils and discrete inputs", () => {
    const out = explainModbusError("Illegal Data Address (0x02) reading coil 5..8");
    expect(out).toContain("the device has no bits in that range");
    expect(out).toContain("try discrete instead of coil");
  });

  it("explains every failing request of a joined message, once each", () => {
    const out = explainModbusError("Read failed: Illegal Data Address (0x02) reading holding 2999..3034; Illegal Data Address (0x02) reading input 100..109");
    expect(out.match(/the device has no registers/g)).toHaveLength(2);
    expect(out).toContain("; Illegal Data Address (0x02) reading input 100..109: ");
  });

  it("leaves every other message alone", () => {
    for (const m of [
      "Read failed: Illegal Function (0x01) reading holding 0..9",
      "Read failed: Timeout: no reply",
      "Server Device Busy (0x06) reading holding 0..9",
      "Connection failed: refused",
      "",
    ]) {
      expect(explainModbusError(m)).toBe(m);
    }
  });
});
