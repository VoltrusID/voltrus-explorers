import { describe, expect, it } from "vitest";
import { planImport } from "../lib/plcImport";
import {
  appendToWatchDoc,
  emptyWatchDoc,
  isDeviceDoc,
  nextDocName,
  offsetText,
  parseTagDoc,
  parseWatchDoc,
  resolveSymbol,
  serializeTagDoc,
  serializeWatchDoc,
  symbolTable,
  tagDocFromDb,
  tagDocFromTable,
  watchRowsForTags,
  withDbNumber,
} from "../lib/s7Docs";

const enc = (s: string) => new TextEncoder().encode(s);

const TAGS_XML = `<?xml version="1.0" encoding="utf-8"?>
<Document>
  <SW.Tags.PlcTagTable ID="0">
    <AttributeList><Name>Conveyor</Name></AttributeList>
    <ObjectList>
      <SW.Tags.PlcTag ID="1" CompositionName="Tags">
        <AttributeList><DataTypeName>Bool</DataTypeName><LogicalAddress>%I0.0</LogicalAddress><Name>Start button</Name></AttributeList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcTag ID="2" CompositionName="Tags">
        <AttributeList><DataTypeName>Real</DataTypeName><LogicalAddress>%MD20</LogicalAddress><Name>Motor_Current</Name></AttributeList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcTag ID="3" CompositionName="Tags">
        <AttributeList><DataTypeName>"Motor_T"</DataTypeName><Name>No_Address</Name></AttributeList>
      </SW.Tags.PlcTag>
    </ObjectList>
  </SW.Tags.PlcTagTable>
</Document>`;

const DB_SOURCE = `DATA_BLOCK "Line"
{ S7_Optimized_Access := 'FALSE' }
   STRUCT
      Enable : Bool;
      Setpoint : Int;
      Ratio : Real;
   END_STRUCT;
BEGIN
END_DATA_BLOCK
`;

describe("device documents", () => {
  it("tells device documents from program source", () => {
    expect(isDeviceDoc("s7:online")).toBe(true);
    expect(isDeviceDoc("/p/Watch table_1.s7watch")).toBe(true);
    expect(isDeviceDoc("/p/Tags.s7tags")).toBe(true);
    expect(isDeviceDoc("/p/Main.st")).toBe(false);
  });

  it("round-trips a watch table and tolerates missing fields", () => {
    const doc = emptyWatchDoc([{ name: "Speed", address: "MW10", dataType: "INT", modify: "5", modifyOn: true }]);
    expect(parseWatchDoc(serializeWatchDoc(doc))).toEqual(doc);
    expect(parseWatchDoc("").rows).toEqual([]);
    expect(parseWatchDoc('{"rows":[{"address":"M0.0"}]}').rows).toEqual([
      { name: undefined, address: "M0.0", dataType: "", modify: "", modifyOn: undefined, comment: undefined },
    ]);
    expect(() => parseWatchDoc("{not json")).toThrow();
  });

  it("turns an imported tag table into a tag document with symbols", async () => {
    const plan = await planImport("tags.xml", enc(TAGS_XML));
    const doc = tagDocFromTable(plan.tagTables[0]);
    expect(doc.rows.map((r) => [r.name, r.address])).toEqual([
      ["Start button", "I0.0"],
      ["Motor_Current", "MD20"],
      ["No_Address", ""],
    ]);
    const symbols = symbolTable([parseTagDoc(serializeTagDoc(doc))]);
    expect(resolveSymbol('"start button"', symbols)).toEqual({ name: "Start button", address: "I0.0", dataType: "BOOL" });
    expect(resolveSymbol("Motor_Current", symbols)?.dataType).toBe("REAL");
    expect(resolveSymbol("No_Address", symbols)).toBeUndefined();
    // Tags without an address cannot be watched.
    expect(watchRowsForTags(doc, doc.rows).map((r) => r.address)).toEqual(["I0.0", "MD20"]);
  });

  it("gives DB members addresses once the DB number is known", async () => {
    const plan = await planImport("line.db", enc(DB_SOURCE));
    const doc = tagDocFromDb(plan.dbs[0]);
    expect(doc.kind).toBe("db");
    expect(doc.rows.map(offsetText)).toEqual(["0.0", "2.0", "4.0"]);
    expect(doc.rows.every((r) => r.address === "")).toBe(true);

    const numbered = withDbNumber(doc, 7);
    expect(numbered.rows.map((r) => r.address)).toEqual(["DB7.DBX0.0", "DB7.DBW2", "DB7.DBD4"]);
    expect(withDbNumber(numbered, undefined).rows[1].address).toBe("");
    const back = parseTagDoc(serializeTagDoc(numbered));
    expect(back).toEqual(numbered);
    expect(resolveSymbol("Line.Setpoint", symbolTable([back]))?.address).toBe("DB7.DBW2");
  });

  it("adds tags to a watch table once", () => {
    const start = emptyWatchDoc([{ address: "MW10", dataType: "INT", modify: "" }]);
    const { doc, added } = appendToWatchDoc(start, [
      { address: "mw10", dataType: "INT", modify: "" },
      { address: "MW10", dataType: "WORD", modify: "" },
      { address: "I0.0", dataType: "BOOL", modify: "" },
      { address: "I0.0", dataType: "BOOL", modify: "" },
    ]);
    expect(added).toBe(2);
    expect(doc.rows.map((r) => `${r.address}:${r.dataType}`)).toEqual(["MW10:INT", "MW10:WORD", "I0.0:BOOL"]);
  });

  it("names new documents after the ones that exist", () => {
    expect(nextDocName("Watch table", ".s7watch", [])).toBe("Watch table_1.s7watch");
    expect(nextDocName("Watch table", ".s7watch", ["/p/watch table_1.s7watch", "Watch table_2.s7watch"])).toBe("Watch table_3.s7watch");
  });
});
