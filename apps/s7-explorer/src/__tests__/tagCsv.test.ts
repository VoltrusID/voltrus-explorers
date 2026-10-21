import { describe, it, expect } from "vitest";
import { normalizeAddress, parseTagCsv, tagsToCsv } from "../tagCsv";

describe("normalizeAddress", () => {
  it("accepts the forms the S7 client parses", () => {
    expect(normalizeAddress("DB1.DBD0")).toEqual({ address: "DB1.DBD0", type: "DWord" });
    expect(normalizeAddress("db10.dbx4.2")).toEqual({ address: "DB10.DBX4.2", type: "Bool" });
    expect(normalizeAddress("IW100")).toEqual({ address: "IW100", type: "Word" });
    expect(normalizeAddress("Q0.1")).toEqual({ address: "Q0.1", type: "Bool" });
  });

  it("strips the TIA Portal % prefix and IEC X", () => {
    expect(normalizeAddress("%MW20")).toEqual({ address: "MW20", type: "Word" });
    expect(normalizeAddress("%I0.0")).toEqual({ address: "I0.0", type: "Bool" });
    expect(normalizeAddress("%QX1.3")).toEqual({ address: "Q1.3", type: "Bool" });
    expect(normalizeAddress("%MD30")).toEqual({ address: "MD30", type: "DWord" });
    expect(normalizeAddress("%IB5")).toEqual({ address: "IB5", type: "Byte" });
  });

  it("strips the padding classic STEP 7 symbol tables use", () => {
    expect(normalizeAddress("MW    10")).toEqual({ address: "MW10", type: "Word" });
    expect(normalizeAddress("I      0.0")).toEqual({ address: "I0.0", type: "Bool" });
    expect(normalizeAddress("DB1.DBD    0")).toEqual({ address: "DB1.DBD0", type: "DWord" });
  });

  it("gives a DB bit address a bit number", () => {
    expect(normalizeAddress("DB1.DBX8")).toEqual({ address: "DB1.DBX8.0", type: "Bool" });
  });

  it("rejects what the client cannot read", () => {
    for (const a of ["T5", "C3", "PIW256", "M5", "MW", "I0.8", "DB1.DBW2.1", "MW70000", "Motor_1", ""]) {
      expect(normalizeAddress(a)).toBeNull();
    }
  });
});

describe("parseTagCsv", () => {
  it("reads a simple Name,Address,Type file", () => {
    const text = "Name,Address,Type\nVoltage,DB1.DBD0,Real\nRun,DB1.DBX8.0,Bool\n";
    expect(parseTagCsv(text)).toEqual({
      tags: [
        { name: "Voltage", address: "DB1.DBD0", data_type: "Real" },
        { name: "Run", address: "DB1.DBX8.0", data_type: "Bool" },
      ],
      errors: [],
      notes: [],
    });
  });

  it("reads what tagsToCsv wrote, including names that need quoting", () => {
    const tags = [
      { name: 'Pump "A", speed', address: "MD10", data_type: "Real" },
      { name: "Run", address: "I0.0", data_type: "Bool" },
    ];
    expect(parseTagCsv(tagsToCsv(tags))).toEqual({ tags, errors: [], notes: [] });
  });

  it("infers the type from the address when there is no type column or cell", () => {
    const { tags } = parseTagCsv("Name,Address\nA,MW4\nB,%Q1.0\n");
    expect(tags.map((t) => t.data_type)).toEqual(["Word", "Bool"]);
    expect(parseTagCsv("Name,Address,Type\nA,DB2.DBB3,\n").tags[0].data_type).toBe("Byte");
  });

  it("reads a TIA Portal PLC tag table export saved as semicolon CSV", () => {
    const text = [
      "Name;Path;Data Type;Logical Address;Comment;Hmi Visible;Hmi Accessible;Hmi Writeable;Typeobject ID;Version ID",
      "Motor_On;Default tag table;Bool;%Q0.0;Starts the motor;True;True;True;;",
      "Speed;Default tag table;Real;%MD100;rpm;True;True;True;;",
      "Level;Default tag table;Int;%IW64;;True;True;True;;",
      "Total;Default tag table;DInt;%MD104;;True;True;True;;",
    ].join("\r\n");
    const result = parseTagCsv("\uFEFF" + text);
    expect(result.errors).toEqual([]);
    expect(result.tags).toEqual([
      { name: "Motor_On", address: "Q0.0", data_type: "Bool" },
      { name: "Speed", address: "MD100", data_type: "Real" },
      { name: "Level", address: "IW64", data_type: "Word" },
      { name: "Total", address: "MD104", data_type: "DWord" },
    ]);
    expect(result.notes).toHaveLength(1);
    expect(result.notes[0]).toContain("2 signed");
  });

  it("reads a comma-separated TIA export with quoted comments that span lines", () => {
    const text = 'Name,Path,Data Type,Logical Address,Comment\nA,T,Word,%MW0,"first\nsecond"\nB,T,Word,%MW2,\n';
    expect(parseTagCsv(text).tags.map((t) => t.name)).toEqual(["A", "B"]);
  });

  it("reads a classic STEP 7 symbol table export", () => {
    const text = 'Symbol,Address,Data type,Comment\n"Motor_1","I      0.0","BOOL","start"\n"Setpoint","MD    20","REAL",""\n';
    expect(parseTagCsv(text).tags).toEqual([
      { name: "Motor_1", address: "I0.0", data_type: "Bool" },
      { name: "Setpoint", address: "MD20", data_type: "Real" },
    ]);
  });

  it("accepts columns in any order and tab delimiters", () => {
    const { tags } = parseTagCsv("Type\tAddress\tName\nWord\tMW2\tA\n");
    expect(tags).toEqual([{ name: "A", address: "MW2", data_type: "Word" }]);
  });

  it("rejects bad rows with line numbers and keeps the good ones", () => {
    const text = [
      "Name,Address,Type",
      "good,MW0,Word",
      "timer,T5,Word",
      ",MW2,Word",
      "noaddr,,Word",
      "lreal,MD4,LReal",
      "mismatch,MW6,Bool",
      "bitword,I0.0,Word",
      "tail,MW8,Word",
    ].join("\n");
    const result = parseTagCsv(text);
    expect(result.tags.map((t) => t.name)).toEqual(["good", "tail"]);
    expect(result.errors).toHaveLength(6);
    expect(result.errors[0]).toMatch(/^Line 3: .*timer/);
    expect(result.errors[1]).toMatch(/^Line 4: missing name/);
    expect(result.errors[5]).toMatch(/^Line 8: .*bitword/);
  });

  it("needs a header with name and address columns", () => {
    expect(parseTagCsv("Foo,Bar\n1,2\n").errors[0]).toMatch(/^Line 1:/);
    expect(parseTagCsv("").errors).toEqual(["The file is empty"]);
  });
});
