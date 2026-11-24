import { describe, expect, it } from "vitest";
import { mkdirSync, writeFileSync } from "node:fs";
import { dbRows, planImport } from "../lib/plcImport";
import { layoutDb, parseDbSource } from "../lib/plcImport/dbSource";
import { s7ToIecAddress, s7WatchAddress, toIdentifier } from "../lib/plcImport/common";
import { sclToSt } from "../lib/plcImport/scl";

const enc = (s: string) => new TextEncoder().encode(s);

// An XML tag table export as the engineering software writes it (Openness document).
const TAG_TABLE_XML = `<?xml version="1.0" encoding="utf-8"?>
<Document>
  <Engineering version="V18" />
  <SW.Tags.PlcTagTable ID="0">
    <AttributeList><Name>Conveyor</Name></AttributeList>
    <ObjectList>
      <SW.Tags.PlcTag ID="1" CompositionName="Tags">
        <AttributeList>
          <DataTypeName>Bool</DataTypeName>
          <LogicalAddress>%I0.0</LogicalAddress>
          <Name>Start button</Name>
        </AttributeList>
        <ObjectList>
          <MultilingualText ID="2" CompositionName="Comment">
            <ObjectList>
              <MultilingualTextItem ID="3" CompositionName="Items">
                <AttributeList><Culture>de-DE</Culture><Text>Starttaster</Text></AttributeList>
              </MultilingualTextItem>
              <MultilingualTextItem ID="4" CompositionName="Items">
                <AttributeList><Culture>en-US</Culture><Text>Green start button</Text></AttributeList>
              </MultilingualTextItem>
            </ObjectList>
          </MultilingualText>
        </ObjectList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcTag ID="5" CompositionName="Tags">
        <AttributeList>
          <DataTypeName>Int</DataTypeName>
          <LogicalAddress>%MW10</LogicalAddress>
          <Name>Speed_SP</Name>
        </AttributeList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcTag ID="6" CompositionName="Tags">
        <AttributeList>
          <DataTypeName>Real</DataTypeName>
          <LogicalAddress>%MD20</LogicalAddress>
          <Name>Motor.Current</Name>
        </AttributeList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcTag ID="7" CompositionName="Tags">
        <AttributeList>
          <DataTypeName>Bool</DataTypeName>
          <LogicalAddress>%Q4.7</LogicalAddress>
          <Name>Lamp</Name>
        </AttributeList>
      </SW.Tags.PlcTag>
      <SW.Tags.PlcUserConstant ID="8" CompositionName="UserConstants">
        <AttributeList>
          <DataTypeName>Int</DataTypeName>
          <Name>MAX_SPEED</Name>
          <Value>1500</Value>
        </AttributeList>
      </SW.Tags.PlcUserConstant>
    </ObjectList>
  </SW.Tags.PlcTagTable>
</Document>`;

const DB_SOURCE = `TYPE "Motor_T"
VERSION : 0.1
   STRUCT
      Running : Bool;   // feedback
      Fault : Bool;
      Speed : Int;
   END_STRUCT;

END_TYPE

DATA_BLOCK "Line Settings"
{ S7_Optimized_Access := 'FALSE' }
VERSION : 0.1
NON_RETAIN
   STRUCT
      Enable : Bool;   // master enable
      Mode : Byte;
      Setpoint : Int := 100;
      Ratio : Real := 1.5;
      Flags : Array[0..9] of Bool;
      Label : String[10];
      Counts : Array[1..3] of DInt;
      M1 : "Motor_T";
      Last : Bool;
      Total : LReal;
   END_STRUCT;

BEGIN
   Setpoint := 250;
END_DATA_BLOCK

DATA_BLOCK "Fast"
{ S7_Optimized_Access := 'TRUE' }
   STRUCT
      x : Int;
   END_STRUCT;
BEGIN
END_DATA_BLOCK
`;

const SCL = `FUNCTION_BLOCK "Conveyor Control"
{ S7_Optimized_Access := 'TRUE' }
VERSION : 0.1
   VAR_INPUT
      start : Bool;
      stop : Bool;
   END_VAR
   VAR_OUTPUT
      run : Bool;
   END_VAR
   VAR
      delay {InstructionName := 'TON_TIME'; LibVersion := '1.0'} : TON_TIME;
      count : Int;
      speeds : Array[0..3] of Int;
   END_VAR

BEGIN
	REGION Start logic
	    // latch the motor
	    IF #start AND NOT #stop THEN
	        #run := TRUE;
	    ELSIF #stop THEN
	        #run := FALSE;
	    END_IF;
	END_REGION

	#delay(IN := #run, PT := S5T#2S);
	IF #delay.Q THEN
	    #count := #count + 16#1;
	    "Lamp" := TRUE;
	    %Q4.7 := #run;
	END_IF;
END_FUNCTION_BLOCK
`;

/** A stored (uncompressed) ZIP: enough for the reader, which does not check CRCs. */
function storedZip(files: Record<string, string | Uint8Array>, method = 0): Uint8Array {
  const parts: number[] = [];
  const central: number[] = [];
  const le16 = (n: number) => [n & 0xff, (n >> 8) & 0xff];
  const le32 = (n: number) => [n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, (n >>> 24) & 0xff];
  let count = 0;
  for (const [name, text] of Object.entries(files)) {
    const nameBytes = [...enc(name)];
    const data = [...(typeof text === "string" ? enc(text) : text)];
    const offset = parts.length;
    parts.push(...le32(0x04034b50), ...le16(20), ...le16(0), ...le16(method), ...le32(0), ...le32(0),
      ...le32(data.length), ...le32(data.length), ...le16(nameBytes.length), ...le16(0), ...nameBytes, ...data);
    central.push(...le32(0x02014b50), ...le16(20), ...le16(20), ...le16(0), ...le16(method), ...le32(0), ...le32(0),
      ...le32(data.length), ...le32(data.length), ...le16(nameBytes.length), ...le16(0), ...le16(0), ...le16(0),
      ...le16(0), ...le32(0), ...le32(offset), ...nameBytes);
    count++;
  }
  const cdOffset = parts.length;
  parts.push(...central);
  parts.push(...le32(0x06054b50), ...le16(0), ...le16(0), ...le16(count), ...le16(count),
    ...le32(central.length), ...le32(cdOffset), ...le16(0));
  return new Uint8Array(parts);
}

const XLSX = storedZip({
  "xl/workbook.xml": `<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="PLC Tags" sheetId="1" r:id="rId1"/><sheet name="TagTable Properties" sheetId="2" r:id="rId2"/></sheets></workbook>`,
  "xl/_rels/workbook.xml.rels": `<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>`,
  "xl/sharedStrings.xml": `<sst><si><t>Name</t></si><si><t>Path</t></si><si><t>Data Type</t></si><si><t>Logical Address</t></si><si><t>Comment</t></si><si><t>Pump run</t></si><si><t>Default tag table</t></si><si><t>Bool</t></si><si><t>%Q0.1</t></si><si><t>Runs the pump</t></si></sst>`,
  "xl/worksheets/sheet1.xml": `<worksheet><sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c><c r="D1" t="s"><v>3</v></c><c r="E1" t="s"><v>4</v></c></row>
    <row r="2"><c r="A2" t="s"><v>5</v></c><c r="B2" t="s"><v>6</v></c><c r="C2" t="s"><v>7</v></c><c r="D2" t="s"><v>8</v></c><c r="E2" t="s"><v>9</v></c></row>
    <row r="3"><c r="A3" t="inlineStr"><is><t>Level</t></is></c><c r="B3" t="s"><v>6</v></c><c r="C3" t="inlineStr"><is><t>Word</t></is></c><c r="D3" t="inlineStr"><is><t>%IW64</t></is></c></row>
  </sheetData></worksheet>`,
  "xl/worksheets/sheet2.xml": `<worksheet><sheetData/></worksheet>`,
});

// Set PLC_IMPORT_OUT=/some/dir to write every generated ST file there and check it with
// `voltrus-plc check /some/dir`.
function keep(name: string, content: string) {
  const dir = process.env.PLC_IMPORT_OUT;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/${name}`, content);
}

describe("names and addresses", () => {
  it("makes IEC identifiers", () => {
    expect(toIdentifier("Start button")).toBe("Start_button");
    expect(toIdentifier('Motor "1" -- Speed')).toBe("Motor_1_Speed");
    expect(toIdentifier("1st")).toBe("T_1st");
  });

  it("converts absolute addresses", () => {
    expect(s7ToIecAddress("%I0.0")).toBe("%IX0.0");
    expect(s7ToIecAddress("%MW10")).toBe("%MW10");
    expect(s7ToIecAddress("%QD4")).toBe("%QD4");
    expect(s7ToIecAddress("E1.2")).toBe("%IX1.2");
    expect(s7ToIecAddress("%DB1.DBX0.0")).toBeNull();
    expect(s7WatchAddress("%Q4.7")).toBe("Q4.7");
    expect(s7WatchAddress("%DB2.DBW4")).toBe("DB2.DBW4");
    expect(s7WatchAddress("%IW64:P")).toBe("IW64");
  });
});

describe("tag tables", () => {
  it("reads the XML export into ST and watch rows", async () => {
    const plan = await planImport("Conveyor.xml", enc(TAG_TABLE_XML));
    expect(plan.kind).toBe("tag-table");
    expect(plan.files).toHaveLength(1);
    const st = plan.files[0].content;
    expect(plan.files[0].name).toBe("Conveyor_Tags.st");
    expect(st).toContain('Start_button AT %IX0.0 : BOOL; (* "Start button": Green start button *)');
    expect(st).toContain("Speed_SP AT %MW10 : INT;");
    expect(st).toContain("Motor_Current AT %MD20 : REAL;");
    expect(st).toContain("MAX_SPEED : INT := 1500;");
    expect(plan.watchRows).toEqual([
      { name: "Start button", address: "I0.0", dataType: "BOOL" },
      { name: "Speed_SP", address: "MW10", dataType: "INT" },
      { name: "Motor.Current", address: "MD20", dataType: "REAL" },
      { name: "Lamp", address: "Q4.7", dataType: "BOOL" },
    ]);
    keep(plan.files[0].name, st);
  });

  it("reads the spreadsheet export", async () => {
    const plan = await planImport("PLCTags.xlsx", XLSX);
    expect(plan.summary).toContain("2 tag(s)");
    expect(plan.files[0].content).toContain("Pump_run AT %QX0.1 : BOOL;");
    expect(plan.files[0].content).toContain("Level AT %IW64 : WORD;");
    expect(plan.watchRows.map((r) => r.address)).toEqual(["Q0.1", "IW64"]);
    keep(plan.files[0].name, plan.files[0].content);
  });

  it("inflates deflated entries, as real spreadsheets have", async () => {
    const deflate = async (text: string) =>
      new Uint8Array(await new Response(new ReadableStream({ start(c) { c.enqueue(enc(text)); c.close(); } }).pipeThrough(new CompressionStream("deflate-raw"))).arrayBuffer());
    const zip = storedZip({
      "xl/worksheets/sheet1.xml": await deflate(
        `<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="B1" t="inlineStr"><is><t>Data Type</t></is></c><c r="C1" t="inlineStr"><is><t>Logical Address</t></is></c></row>` +
        `<row r="2"><c r="A2" t="inlineStr"><is><t>Valve</t></is></c><c r="B2" t="inlineStr"><is><t>Bool</t></is></c><c r="C2" t="inlineStr"><is><t>%Q1.0</t></is></c></row></sheetData></worksheet>`,
      ),
    }, 8);
    const plan = await planImport("tags.xlsx", zip);
    expect(plan.watchRows).toEqual([{ name: "Valve", address: "Q1.0", dataType: "BOOL" }]);
  });

  it("explains files that are not tag tables", async () => {
    await expect(planImport("x.xml", enc("<Document><Other/></Document>"))).rejects.toThrow(/No PLC tag table/);
    await expect(planImport("x.xlsx", enc("not a zip"))).rejects.toThrow(/ZIP/);
  });
});

describe("DB sources", () => {
  it("lays out a standard-access DB like the CPU does", () => {
    const blocks = parseDbSource(DB_SOURCE);
    const db = blocks.find((b) => b.name === "Line Settings")!;
    const layout = layoutDb(db, blocks);
    const at = (path: string) => {
      const m = layout.members.find((x) => x.path === path)!;
      return m.bit !== undefined ? `${m.byte}.${m.bit}` : `${m.byte}`;
    };
    expect(layout.warnings).toEqual([]);
    expect(at("Enable")).toBe("0.0");
    expect(at("Mode")).toBe("1");
    expect(at("Setpoint")).toBe("2");
    expect(at("Ratio")).toBe("4");
    expect(at("Flags[0]")).toBe("8.0");
    expect(at("Flags[9]")).toBe("9.1");
    expect(at("Label")).toBe("10"); // array of 10 bits = 2 bytes, padded even
    expect(at("Counts[1]")).toBe("22"); // String[10] = 12 bytes
    expect(at("Counts[3]")).toBe("30");
    expect(at("M1.Running")).toBe("34.0");
    expect(at("M1.Fault")).toBe("34.1");
    expect(at("M1.Speed")).toBe("36");
    expect(at("Last")).toBe("38.0"); // UDT size 4, even
    expect(at("Total")).toBe("40");
    expect(layout.size).toBe(48);
  });

  it("emits ST types and watch rows once the DB number is known", async () => {
    const plan = await planImport("Line.db", enc(DB_SOURCE));
    expect(plan.kind).toBe("db-source");
    const st = plan.files[0].content;
    expect(st).toContain("TYPE Motor_T :");
    expect(st).toContain("TYPE Line_Settings_T :");
    expect(st).toContain("Setpoint : INT := 250;"); // actual value from BEGIN wins
    expect(st).toContain("Flags : ARRAY[0..9] OF BOOL;");
    expect(st).toContain("Label : STRING[10];");
    expect(st).toContain("M1 : Motor_T;");
    expect(st).toMatch(/PROGRAM Line_Data\nVAR\n  Line_Settings : Line_Settings_T;\n  Fast : Fast_T;\nEND_VAR\nEND_PROGRAM/);
    expect(plan.warnings.some((w) => w.includes("Fast uses optimized access"))).toBe(true);

    const rows = dbRows(plan.dbs, { "Line Settings": 5 });
    expect(rows).toContainEqual({ name: "Line Settings.Setpoint", address: "DB5.DBW2", dataType: "INT" });
    expect(rows).toContainEqual({ name: "Line Settings.Flags[9]", address: "DB5.DBX9.1", dataType: "BOOL" });
    expect(rows).toContainEqual({ name: "Line Settings.Label", address: "DB5.DBB10", dataType: "STRING" });
    expect(rows).toContainEqual({ name: "Line Settings.Total", address: "DB5.DBD40", dataType: "LREAL" });
    expect(dbRows(plan.dbs, {})).toEqual([]); // no number, no rows
    keep(plan.files[0].name, st);
  });

  it("reads classic sources with a DB number", () => {
    const blocks = parseDbSource(`DATA_BLOCK DB 12
TITLE = Recipe
VERSION : 0.1
  STRUCT
    A : WORD;
    B : DINT;
  END_STRUCT ;
BEGIN
  A := W#16#0;
END_DATA_BLOCK`);
    expect(blocks[0].number).toBe(12);
    expect(layoutDb(blocks[0], blocks).members.map((m) => m.byte)).toEqual([0, 2]);
  });
});

describe("SCL", () => {
  it("converts to ST", async () => {
    const plan = await planImport("Conveyor.scl", enc(SCL));
    const st = plan.files[0].content;
    expect(plan.files[0].name).toBe("Conveyor_Control.st");
    expect(st).toContain("FUNCTION_BLOCK Conveyor_Control");
    for (const gone of ["S7_Optimized_Access", "VERSION", "InstructionName"]) expect(st).not.toContain(gone);
    expect(st.replace(/\/\/.*$/gm, "")).not.toMatch(/(^|[^A-Za-z0-9_])#[A-Za-z_]/m); // no #locals left
    expect(st).toContain("delay : TON;");
    expect(st).toContain("(* REGION Start logic *)");
    expect(st).toContain("IF start AND NOT stop THEN");
    expect(st).toContain("delay(IN := run, PT := T#2S);");
    expect(st).toContain("count := count + 16#1;");
    expect(st).toContain("Lamp := TRUE;");
    expect(st).toContain("%QX4.7 := run;");
    expect(st).toContain("speeds : ARRAY[0..3] OF Int;");
    expect(st).not.toMatch(/^\s*BEGIN\s*$/m);
    expect(st).toContain("// latch the motor");
    expect(plan.warnings.join("\n")).toMatch(/global tags or DBs: Lamp\./);
    keep(plan.files[0].name, st);
  });

  it("turns organization blocks into programs and reports what it cannot convert", () => {
    const r = sclToSt(`ORGANIZATION_BLOCK "Main"
TITLE = "Main Program Sweep (Cycle)"
BEGIN
  "IEC_Timer_0_DB".TON(IN := TRUE, PT := T#1s);
  #x := PEEK(area := 16#84, dbNumber := 1, byteOffset := 0);
  GOTO lbl;
END_ORGANIZATION_BLOCK`);
    expect(r.st).toContain("PROGRAM Main");
    expect(r.st).toContain("END_PROGRAM");
    expect(r.st).toContain("IEC_Timer_0_DB(IN := TRUE, PT := T#1s);");
    expect(r.st).not.toContain("TITLE");
    expect(r.warnings.join("\n")).toMatch(/single instance of TON/);
    expect(r.warnings.join("\n")).toMatch(/PEEK\(\)/);
    expect(r.warnings.join("\n")).toMatch(/GOTO/);
  });

  it("leaves strings and comments alone", () => {
    const r = sclToSt(`FUNCTION_BLOCK FB1
VAR s : String; END_VAR
BEGIN
  #s := 'say "#hi" {x}'; // "quoted" #name
END_FUNCTION_BLOCK`);
    expect(r.st).toContain(`s := 'say "#hi" {x}'; // "quoted" #name`);
  });
});
