import { describe, expect, it } from "vitest";
import { planImport } from "../lib/plcImport";
import { dbToSt, newDbDoc, offsetText, parseTagDoc, relayoutDb, serializeTagDoc, tagDocFromDb, withDbNumber } from "../lib/s7Docs";
import {
  addressConflicts,
  channelsOf,
  channelTags,
  cpuFromOrderCode,
  emptyHwDoc,
  entryOf,
  familyFromOrderCode,
  freeSlotFor,
  parseHwDoc,
  placeModule,
  rangesOf,
  serializeHwDoc,
} from "../lib/s7Hw";
import { compareBlocks } from "../lib/s7Blocks";
import type { BlockDetails } from "../lib/s7";

const enc = (s: string) => new TextEncoder().encode(s);

describe("device configuration", () => {
  it("builds an S7-1200 rack with on-board and module I/O", () => {
    let doc = emptyHwDoc("S7-1200");
    doc = placeModule(doc, entryOf("S7-1200", "cpu1214c")!, 1);
    doc = placeModule(doc, entryOf("S7-1200", "di16")!, freeSlotFor(doc, entryOf("S7-1200", "di16")!)!);
    doc = placeModule(doc, entryOf("S7-1200", "aq2")!, 3);
    const [cpu, di, aq] = [...doc.modules].sort((a, b) => a.slot - b.slot);
    expect(rangesOf("S7-1200", cpu)).toEqual({ i: [[0, 2], [64, 68]], q: [[0, 2]] });
    expect(di.slot).toBe(2);
    expect(rangesOf("S7-1200", di)).toEqual({ i: [[8, 10]], q: [] });
    expect(rangesOf("S7-1200", aq).q).toEqual([[96, 100]]);
    expect(channelsOf("S7-1200", cpu).filter((c) => c.kind === "di").map((c) => c.address).slice(7, 10)).toEqual(["I0.7", "I1.0", "I1.1"]);
    expect(channelsOf("S7-1200", aq).map((c) => c.address)).toEqual(["QW96", "QW98"]);
    expect(addressConflicts(doc)).toEqual([]);
    expect(parseHwDoc(serializeHwDoc(doc))).toEqual({ ...doc, modules: [...doc.modules].sort((a, b) => a.slot - b.slot) });

    const clash = { ...doc, modules: doc.modules.map((m) => (m.slot === 2 ? { ...m, iStart: 1 } : m)) };
    expect(addressConflicts(clash)[0]).toMatch(/overlap/);
  });

  it("gives S7-300 modules their slot addresses", () => {
    let doc = emptyHwDoc("S7-300");
    doc = placeModule(doc, entryOf("S7-300", "di32")!, 4);
    doc = placeModule(doc, entryOf("S7-300", "ai8")!, 5);
    expect(doc.modules.find((m) => m.slot === 4)!.iStart).toBe(0);
    expect(doc.modules.find((m) => m.slot === 5)!.iStart).toBe(272);
  });

  it("turns channels into tags and recognises CPUs by order number", () => {
    let doc = placeModule(emptyHwDoc("S7-1500"), entryOf("S7-1500", "dq8")!, 2);
    const tags = channelTags(doc, doc.modules);
    expect(tags[0]).toMatchObject({ name: "DQ8_1_DQ_0", address: "Q0.0", dataType: "Bool" });
    expect(familyFromOrderCode("6ES7 214-1AG40-0XB0")).toBe("S7-1200");
    expect(cpuFromOrderCode("S7-1200", "6ES7 214-1AG40-0XB0", "")?.id).toBe("cpu1214c");
    expect(cpuFromOrderCode("S7-1500", "6ES7 516-3AN02-0AB0", "CPU 1516-3 PN/DP")?.id).toBe("cpu1516");
    doc = placeModule(doc, entryOf("S7-1500", "cpu1511")!, 1);
    expect(freeSlotFor(doc, entryOf("S7-1500", "cpu1513")!)).toBeUndefined();
  });
});

const DB_SOURCE = `TYPE "Motor_T"
   STRUCT
      Running : Bool;
      Speed : Int;
   END_STRUCT;
END_TYPE

DATA_BLOCK "Line"
{ S7_Optimized_Access := 'FALSE' }
   STRUCT
      Enable : Bool;   // master enable
      Setpoint : Int := 100;
      Flags : Array[0..9] of Bool;
      M1 : "Motor_T";
      Inner : Struct a : Byte; b : Real; End_Struct;
   END_STRUCT;
BEGIN
   Setpoint := 250;
END_DATA_BLOCK
`;

describe("DB declarations", () => {
  it("re-lays out an imported DB exactly as the import did", async () => {
    const plan = await planImport("line.db", enc(DB_SOURCE));
    const doc = tagDocFromDb(plan.dbs[0], 4);
    expect(doc.decl?.map((r) => `${r.name}:${r.type}`)).toEqual([
      "Enable:Bool", "Setpoint:Int", "Flags:Array[0..9] of Bool", 'M1:"Motor_T"', "Inner:Struct a : Byte; b : Real; End_Struct",
    ]);
    expect(doc.decl?.[1].initial).toBe("250");
    const again = relayoutDb(parseTagDoc(serializeTagDoc(doc)));
    expect(again.error).toBeNull();
    expect(again.doc.rows.map((r) => `${r.name}@${offsetText(r)}:${r.address}`)).toEqual(
      doc.rows.map((r) => `${r.name}@${offsetText(r)}:${r.address}`),
    );
    expect(again.doc.rows.find((r) => r.name === "M1.Speed")?.address).toBe("DB4.DBW8"); // Enable 0.0, Setpoint 2, Flags 4-5, M1.Running 6.0
  });

  it("moves offsets when the declaration changes, and reports bad types", async () => {
    const plan = await planImport("line.db", enc(DB_SOURCE));
    const doc = withDbNumber(tagDocFromDb(plan.dbs[0]), 2);
    const inserted = relayoutDb({ ...doc, decl: [{ name: "Counter", type: "DInt", comment: "" }, ...doc.decl!] });
    expect(inserted.doc.rows.find((r) => r.name === "Setpoint")?.address).toBe("DB2.DBW6");
    const broken = relayoutDb({ ...doc, decl: [{ name: "X", type: "Array[0..", comment: "" }] });
    expect(broken.error).toBeTruthy();
    expect(broken.doc.rows).toEqual(doc.rows);
  });

  it("creates new DBs and writes them as ST", () => {
    const doc = newDbDoc("Recipe");
    expect(doc.rows.map((r) => r.name)).toEqual(["Value"]);
    const { fileName, st } = dbToSt(relayoutDb({ ...doc, decl: [...doc.decl!, { name: "Temp", type: "Real", initial: "20.5", comment: "set" }] }).doc);
    expect(fileName).toMatch(/\.st$/);
    expect(st).toContain("Temp : REAL := 20.5;");
  });
});

describe("block baselines", () => {
  const blk = (block_type: string, block_number: number, checksum: number, code_time = "2024-03-18 20:00:00.000"): BlockDetails => ({
    block_type, block_number, checksum, code_time, language: "AWL", load_size: 100, mc7_size: 10, local_data: 0,
    know_how_protected: false, interface_time: code_time, author: "", family: "", name: "", version: "0.1",
  });

  it("marks blocks the same, changed, new or missing", () => {
    const rows = compareBlocks(
      [blk("OB", 1, 1), blk("FC", 2, 9), blk("DB", 5, 3)],
      [blk("OB", 1, 1), blk("FC", 2, 8, "2024-01-01 00:00:00.000"), blk("FB", 1, 4)],
    );
    expect(rows.map((r) => `${r.key}:${r.status}`)).toEqual(["OB1:same", "FB1:missing", "FC2:changed", "DB5:new"]);
    expect(rows.find((r) => r.key === "FC2")!.diffs).toEqual(["checksum", "code time", "interface time"]);
  });
});
