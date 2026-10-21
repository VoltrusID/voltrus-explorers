import { describe, expect, it } from "vitest";
import {
  bytesToHex,
  decodeNumber,
  describeValue,
  hexToBytes,
  plotValue,
  presetToHex,
  toCsv,
  resolveFormat,
  textToHex,
} from "../decode";
import { defaultDecode, gattName, shortUuid } from "../gatt";
import { SampleStore, newId, parseStored, pruneMacros } from "../store";

const bytes = (...b: number[]) => new Uint8Array(b);

describe("hexToBytes", () => {
  it("accepts spaces and 0x prefixes", () => {
    expect(hexToBytes("0x01 0x02 ff")).toEqual(bytes(1, 2, 255));
    expect(hexToBytes("")).toEqual(bytes());
  });
  it("rejects odd length and non-hex", () => {
    expect(hexToBytes("abc")).toBeNull();
    expect(hexToBytes("zz")).toBeNull();
  });
  it("round-trips", () => {
    expect(bytesToHex(bytes(0, 16, 255))).toBe("0010ff");
    expect(textToHex("Hi")).toBe("4869");
  });
});

describe("decodeNumber", () => {
  it("reads integers in both endiannesses", () => {
    const b = bytes(0x34, 0x12, 0xff, 0xff);
    expect(decodeNumber(b, "uint16le")).toBe(0x1234);
    expect(decodeNumber(b, "uint16be")).toBe(0x3412);
    expect(decodeNumber(b, "int16le", 2)).toBe(-1);
    expect(decodeNumber(b, "uint32le")).toBe(0xffff1234);
    expect(decodeNumber(b, "int32le")).toBe(-60876);
    expect(decodeNumber(bytes(0xfe), "int8")).toBe(-2);
    expect(decodeNumber(bytes(0xfe), "uint8")).toBe(254);
  });
  it("reads floats", () => {
    expect(decodeNumber(bytes(0x00, 0x00, 0x80, 0x3f), "float32le")).toBe(1);
    expect(decodeNumber(bytes(0x3f, 0x80, 0x00, 0x00), "float32be")).toBe(1);
    expect(decodeNumber(bytes(0, 0, 0, 0, 0, 0, 0xf0, 0x3f), "float64le")).toBe(1);
  });
  it("returns null when too short or not numeric", () => {
    expect(decodeNumber(bytes(1), "uint16le")).toBeNull();
    expect(decodeNumber(bytes(1, 2), "uint8", 2)).toBeNull();
    expect(decodeNumber(bytes(1), "utf8")).toBeNull();
  });
});

describe("describeValue / plotValue", () => {
  it("applies scale and offset", () => {
    const b = bytes(0, 0, 0xd0, 0x07);
    expect(describeValue(b, { format: "int16le", offset: 2, scale: 0.01 })).toBe("20");
    expect(plotValue(b, { format: "int16le", offset: 2, scale: 0.01 })).toBe(20);
  });
  it("explains short values", () => {
    expect(describeValue(bytes(1), { format: "uint32le", offset: 0, scale: 1 })).toBe("needs 4 bytes, got 1");
  });
  it("decodes text and hex", () => {
    expect(describeValue(bytes(0x68, 0x69, 0), { format: "utf8", offset: 0, scale: 1 })).toBe("hi");
    expect(describeValue(bytes(1, 0xab), { format: "hex", offset: 0, scale: 1 })).toBe("01 ab");
    expect(plotValue(bytes(1), { format: "hex", offset: 0, scale: 1 })).toBeNull();
  });
  it("resolves auto", () => {
    expect(resolveFormat(bytes(0x68, 0x69), "auto")).toBe("utf8");
    expect(resolveFormat(bytes(1, 2), "auto")).toBe("hex");
    expect(resolveFormat(bytes(1, 2), "auto", "uint8")).toBe("uint8");
  });
});

describe("presetToHex / toCsv", () => {
  it("converts presets", () => {
    expect(presetToHex("hex", "0x01 FF")).toBe("01ff");
    expect(presetToHex("hex", "0")).toBeNull();
    expect(presetToHex("text", "OK\n")).toBe("4f4b0a");
  });
  it("quotes cells and neutralises formulas", () => {
    expect(toCsv([["a,b", 'say "hi"', "=1+1", -3]])).toBe('"a,b","say ""hi""",\'=1+1,-3\r\n');
  });
});

describe("gatt names", () => {
  it("maps base UUIDs only", () => {
    expect(shortUuid("0000180f-0000-1000-8000-00805f9b34fb")).toBe("0x180F");
    expect(shortUuid("6e400001-b5a3-f393-e0a9-e50e24dcca9e")).toBe("6e400001-b5a3-f393-e0a9-e50e24dcca9e");
    expect(gattName("00002a19-0000-1000-8000-00805f9b34fb")).toBe("Battery Level");
    expect(gattName("6e400003-b5a3-f393-e0a9-e50e24dcca9e")).toBe("Nordic UART TX (notify)");
    expect(gattName("12342a19-0000-1000-8000-00805f9b34fb")).toBeNull();
    expect(defaultDecode("00002a19-0000-1000-8000-00805f9b34fb")?.format).toBe("uint8");
  });
});

describe("store", () => {
  it("drops malformed entries and clamps numbers", () => {
    const s = parseStored({
      prefs: { decode: false, formats: { a: { format: "int16le", offset: -3, scale: 2 }, b: { format: "nope" } } },
      presets: [
        { id: "p1", name: "On", charUuid: "x", mode: "hex", value: "01" },
        { id: "p2", name: "bad", charUuid: "x", mode: "other", value: "01" },
      ],
      macros: [{ id: "m", name: "M", repeat: 99999, steps: [{ presetId: "p1", delayMs: -5 }, { nope: 1 }] }],
    });
    expect(s.prefs.decode).toBe(false);
    expect(s.prefs.formats).toEqual({ a: { format: "int16le", offset: 0, scale: 2 } });
    expect(s.presets).toHaveLength(1);
    expect(s.macros[0]).toEqual({ id: "m", name: "M", repeat: 1000, steps: [{ presetId: "p1", delayMs: 0 }] });
  });
  it("tolerates garbage", () => {
    expect(parseStored("x").presets).toEqual([]);
    expect(parseStored(null).prefs.gattNames).toBe(true);
  });
  it("prunes macro steps of deleted presets", () => {
    const macros = [{ id: "m", name: "M", repeat: 1, steps: [{ presetId: "a", delayMs: 0 }, { presetId: "b", delayMs: 0 }] }];
    const kept = pruneMacros(macros, [{ id: "a", name: "A", charUuid: "x", mode: "hex", value: "" }]);
    expect(kept[0].steps.map((s) => s.presetId)).toEqual(["a"]);
  });
  it("caps sample history", () => {
    const st = new SampleStore<number>(3);
    for (let i = 0; i < 5; i++) st.push("k", i, i);
    expect(st.get("k").map((s) => s.v)).toEqual([2, 3, 4]);
    st.clear("k");
    expect(st.keys()).toEqual([]);
    expect(newId()).not.toBe(newId());
  });
});
