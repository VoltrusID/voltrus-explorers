import { describe, expect, it } from "vitest";
import { connectConfig, formatWord, guessType, newDevice, offsetAddress } from "../helpers";
import { parseTagCsv, tagsToCsv } from "../tagCsv";

describe("connect payload", () => {
  it("matches the Rust ConnectConfig for MC", () => {
    const d = newDevice("mc");
    d.mc = { ...d.mc, frame: "4E", encoding: "ascii", transport: "udp", xyOctal: true };
    expect(connectConfig(d)).toMatchObject({
      protocol: "mc", port: 5000, frame: "4E", encoding: "ascii", transport: "udp",
      route: { network: 0, pc: 255, io: 1023, station: 0 }, xy_octal: true,
    });
  });
  it("matches the Rust ConnectConfig for FINS", () => {
    const d = newDevice("fins");
    d.fins.destNode = 1;
    expect(connectConfig(d)).toMatchObject({ protocol: "fins", port: 9600, transport: "udp", dest_node: 1, src_node: 0 });
  });
});

describe("tag helpers", () => {
  it("guesses Bool for bit addresses", () => {
    expect(guessType("mc", "M100", "Int16")).toBe("Bool");
    expect(guessType("mc", "X1F", "Int16")).toBe("Bool");
    expect(guessType("mc", "D100", "Bool")).toBe("Int16");
    expect(guessType("mc", "SD0", "Bool")).toBe("Int16");
    expect(guessType("mc", "D100.F", "Int16")).toBe("Bool");
    expect(guessType("fins", "CIO0.05", "Int16")).toBe("Bool");
    expect(guessType("fins", "D100", "Float32")).toBe("Float32");
  });
  it("labels monitor rows", () => {
    expect(offsetAddress("mc", "D100", 3)).toBe("D103");
    expect(offsetAddress("mc", "M0", 2)).toBe("M32");
    expect(offsetAddress("mc", "W1A", 1)).toBe("W1B");
    expect(offsetAddress("mc", "X0", 1)).toBe("X10");
    expect(offsetAddress("fins", "CIO100", 2)).toBe("CIO102");
    expect(offsetAddress("fins", "E0_5", 1)).toBe("E0_6");
  });
  it("formats words", () => {
    expect(formatWord(0xffff, "sdec")).toBe("-1");
    expect(formatWord(0x2a, "hex")).toBe("002A");
    expect(formatWord(5, "bin")).toBe("0000 0000 0000 0101");
  });
});

describe("tag CSV", () => {
  it("round-trips and maps other tools' type names", () => {
    const csv = tagsToCsv([{ name: "Speed, rpm", address: "D100", data_type: "Int16" }]);
    expect(parseTagCsv(csv).tags).toEqual([{ name: "Speed, rpm", address: "D100", data_type: "Int16" }]);
    const r = parseTagCsv("Name;Address;Type\nTemp;d200;REAL\nRun;M0;BOOL\nBad;D1;STRING\n;;\n");
    expect(r.tags).toEqual([
      { name: "Temp", address: "D200", data_type: "Float32" },
      { name: "Run", address: "M0", data_type: "Bool" },
    ]);
    expect(r.errors).toHaveLength(2);
  });
});
