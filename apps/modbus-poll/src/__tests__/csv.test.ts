import { describe, it, expect } from "vitest";
import { blocksToCsv, parseBlocksCsv } from "../csv";
import type { ReadBlock } from "../types";

const BLOCKS: ReadBlock[] = [
  { name: "Voltage", address: 2999, quantity: 3, type: "float32", area: "holding" },
  { name: "Run, state", address: 10, quantity: 2, type: "uint16", area: "coil" },
];

describe("block CSV", () => {
  it("writes the header and one line per block, quoting names that need it", () => {
    expect(blocksToCsv(BLOCKS)).toBe(
      'name,address,quantity,type,area\nVoltage,2999,3,float32,holding\n"Run, state",10,2,uint16,coil\n',
    );
  });

  it("reads back what it wrote", () => {
    expect(parseBlocksCsv(blocksToCsv(BLOCKS))).toEqual({ blocks: BLOCKS, errors: [] });
  });

  it("accepts CRLF line endings and a BOM", () => {
    const text = "\uFEFFname,address,quantity,type,area\r\nA,1,2,uint16,input\r\n";
    expect(parseBlocksCsv(text).blocks).toEqual([{ name: "A", address: 1, quantity: 2, type: "uint16", area: "input" }]);
  });

  it("rejects bad lines with their line numbers and keeps the good ones", () => {
    const text = [
      "name,address,quantity,type,area",
      "good,0,1,uint16,holding",
      "text,0,1,string,holding",
      "short,0,1",
      "empty,,1,uint16,holding",
      "badarea,0,1,uint16,register",
      "",
      "good2,5,1,int16,input",
    ].join("\n");
    const { blocks, errors } = parseBlocksCsv(text);
    expect(blocks.map((b) => b.name)).toEqual(["good", "good2"]);
    expect(errors).toHaveLength(4);
    expect(errors[0]).toMatch(/^Line 3: Unsupported type 'string'/);
    expect(errors[1]).toMatch(/^Line 4: expected/);
    expect(errors[2]).toMatch(/^Line 5:/);
    expect(errors[3]).toMatch(/^Line 6: Unsupported area/);
  });
});
