import { describe, it, expect } from "vitest";
import { parseVarUpdate, TYPE_MAP } from "./store";

/**
 * Encode a binary var-update message (msgType 0x01 header + body).
 * The body format and wire type codes match the ESP32 firmware
 * (plc_flatbuf.c / plc_value_type_t):
 *   count(u16le) + [nameLen(u16le) + name(utf8) + typeCode(u8) + value(4 bytes) + timestamp(u32le)]
 *   BOOL=0 BYTE=1 INT=2 DINT=3 REAL=4 TIME=5 STRING=6 WORD=7 DWORD=8
 */
function encodeVarUpdate(
  vars: Array<{ name: string; typeCode: number; value: number }>
): ArrayBuffer {
  const parts: ArrayBuffer[] = [];

  // count
  const count = new ArrayBuffer(2);
  new DataView(count).setUint16(0, vars.length, true);
  parts.push(count);

  for (const v of vars) {
    // name
    const nameBytes = new TextEncoder().encode(v.name);
    const nameLen = new ArrayBuffer(2);
    new DataView(nameLen).setUint16(0, nameBytes.length, true);
    parts.push(nameLen);
    parts.push(nameBytes.buffer);

    // type
    const typeByte = new ArrayBuffer(1);
    new DataView(typeByte).setUint8(0, v.typeCode);
    parts.push(typeByte);

    // value (4 bytes) — REAL is float32, WORD/DWORD raw u32, the rest i32
    const valueBuf = new ArrayBuffer(4);
    if (v.typeCode === 4) {
      new DataView(valueBuf).setFloat32(0, v.value, true);
    } else if (v.typeCode === 7 || v.typeCode === 8) {
      new DataView(valueBuf).setUint32(0, v.value, true);
    } else {
      new DataView(valueBuf).setInt32(0, v.value, true);
    }
    parts.push(valueBuf);

    // timestamp (4 bytes)
    const ts = new ArrayBuffer(4);
    new DataView(ts).setUint32(0, Math.floor(Date.now() / 1000), true);
    parts.push(ts);
  }

  // Concat all
  const total = parts.reduce((sum, b) => sum + b.byteLength, 0);
  const result = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    result.set(new Uint8Array(part), offset);
    offset += part.byteLength;
  }
  return result.buffer;
}

describe("Binary Protocol Parser", () => {
  it("should parse a single BOOL variable", () => {
    const buf = encodeVarUpdate([
      { name: "motor_running", typeCode: 0, value: 1 },
    ]);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(1);
    expect(vars[0].name).toBe("motor_running");
    expect(vars[0].type).toBe("BOOL");
    expect(vars[0].value).toBe(true);
  });

  it("should parse a BOOL variable with value 0 as false", () => {
    const buf = encodeVarUpdate([
      { name: "stopped", typeCode: 0, value: 0 },
    ]);
    const vars = parseVarUpdate(buf);
    expect(vars[0].value).toBe(false);
  });

  it("should parse a single INT variable", () => {
    const buf = encodeVarUpdate([{ name: "speed", typeCode: 2, value: 1500 }]);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(1);
    expect(vars[0].name).toBe("speed");
    expect(vars[0].type).toBe("INT");
    expect(vars[0].value).toBe(1500);
  });

  it("should parse a DINT variable as signed int (not float)", () => {
    const buf = encodeVarUpdate([{ name: "counter", typeCode: 3, value: 123456 }]);
    const vars = parseVarUpdate(buf);
    expect(vars[0].type).toBe("DINT");
    expect(vars[0].value).toBe(123456);
  });

  it("should parse a REAL variable with float encoding", () => {
    const buf = encodeVarUpdate([
      { name: "temperature", typeCode: 4, value: 23.5 },
    ]);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(1);
    expect(vars[0].name).toBe("temperature");
    expect(vars[0].type).toBe("REAL");
    expect(vars[0].value).toBeCloseTo(23.5, 5);
  });

  it("should parse WORD/DWORD as unsigned", () => {
    const buf = encodeVarUpdate([
      { name: "w", typeCode: 7, value: 40000 },
      { name: "dw", typeCode: 8, value: 3_000_000_000 },
    ]);
    const vars = parseVarUpdate(buf);
    expect(vars[0].type).toBe("WORD");
    expect(vars[0].value).toBe(40000);
    expect(vars[1].type).toBe("DWORD");
    expect(vars[1].value).toBe(3_000_000_000);
  });

  it("should parse multiple variables", () => {
    const buf = encodeVarUpdate([
      { name: "speed", typeCode: 2, value: 1500 },
      { name: "temp", typeCode: 4, value: 45.2 },
      { name: "running", typeCode: 0, value: 1 },
    ]);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(3);
    expect(vars[0].name).toBe("speed");
    expect(vars[0].type).toBe("INT");
    expect(vars[0].value).toBe(1500);
    expect(vars[1].name).toBe("temp");
    expect(vars[1].value).toBeCloseTo(45.2, 4);
    expect(vars[2].name).toBe("running");
    expect(vars[2].value).toBe(true);
  });

  it("should handle empty variable list", () => {
    const buf = encodeVarUpdate([]);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(0);
  });

  it("should handle truncated buffer gracefully", () => {
    // Provide a count of 1 but no actual data
    const buf = new ArrayBuffer(2);
    new DataView(buf).setUint16(0, 1, true);
    const vars = parseVarUpdate(buf);
    expect(vars).toHaveLength(0);
  });

  it("should parse all supported type codes", () => {
    const typeCodes = Object.keys(TYPE_MAP).map(Number);
    const vars = typeCodes.map((tc) => ({
      name: `var_${tc}`,
      typeCode: tc,
      value: tc === 4 ? 1.5 : (tc * 10) % 1000,
    }));
    const buf = encodeVarUpdate(vars);
    const parsed = parseVarUpdate(buf);
    expect(parsed).toHaveLength(typeCodes.length);
    for (let i = 0; i < typeCodes.length; i++) {
      expect(parsed[i].name).toBe(`var_${typeCodes[i]}`);
      expect(parsed[i].type).toBe(TYPE_MAP[typeCodes[i]]);
    }
  });
});
