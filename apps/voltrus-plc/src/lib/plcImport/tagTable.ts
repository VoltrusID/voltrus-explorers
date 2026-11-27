// PLC tag tables exported by the engineering software: the XML export (Openness/SimaticML,
// `SW.Tags.PlcTagTable`) and the spreadsheet export (`PLC Tags` sheet: Name, Path, Data Type,
// Logical Address, Comment, ...).
import { mapElementaryType, s7ToIecAddress, s7WatchAddress, toIdentifier, uniqueIdentifiers, watchTypeFor } from "./common";
import { readXlsxRows } from "./xlsx";

export interface PlcTag {
  name: string;
  dataType: string;
  /** Absolute address as exported, e.g. `%I0.0`, `%MW10`. Empty for tags without one. */
  address: string;
  comment: string;
}

export interface PlcConstant {
  name: string;
  dataType: string;
  value: string;
  comment: string;
}

export interface TagTable {
  name: string;
  tags: PlcTag[];
  constants: PlcConstant[];
}

function childText(el: Element, name: string): string {
  for (const c of Array.from(el.children)) {
    if (c.localName === name) return (c.textContent ?? "").trim();
  }
  return "";
}

function attributeList(el: Element): Element | undefined {
  return Array.from(el.children).find((c) => c.localName === "AttributeList");
}

/** The comment of an object: the first non-empty text, preferring English. */
function commentOf(el: Element): string {
  const texts = Array.from(el.getElementsByTagName("*")).filter(
    (c) => c.localName === "MultilingualText" && c.getAttribute("CompositionName") === "Comment",
  );
  const items = texts.flatMap((t) => Array.from(t.getElementsByTagName("*")).filter((c) => c.localName === "MultilingualTextItem"));
  const parsed = items.map((it) => {
    const attrs = attributeList(it);
    return { culture: attrs ? childText(attrs, "Culture") : "", text: attrs ? childText(attrs, "Text") : "" };
  }).filter((x) => x.text);
  return (parsed.find((x) => x.culture.startsWith("en")) ?? parsed[0])?.text ?? "";
}

export function parseTagTableXml(xml: string): TagTable[] {
  const doc = new DOMParser().parseFromString(xml, "application/xml");
  if (doc.getElementsByTagName("parsererror").length > 0) {
    throw new Error("The file is not valid XML");
  }
  const all = Array.from(doc.getElementsByTagName("*"));
  const tables = all.filter((e) => e.localName === "SW.Tags.PlcTagTable");
  if (tables.length === 0) {
    throw new Error("No PLC tag table in this XML (expected SW.Tags.PlcTagTable)");
  }
  return tables.map((table) => {
    const attrs = attributeList(table);
    const name = attrs ? childText(attrs, "Name") : "Tags";
    const descendants = Array.from(table.getElementsByTagName("*"));
    const tags = descendants
      .filter((e) => e.localName === "SW.Tags.PlcTag")
      .map((tag) => {
        const a = attributeList(tag)!;
        return {
          name: childText(a, "Name"),
          dataType: childText(a, "DataTypeName"),
          address: childText(a, "LogicalAddress"),
          comment: commentOf(tag),
        };
      })
      .filter((t) => t.name);
    const constants = descendants
      .filter((e) => e.localName === "SW.Tags.PlcUserConstant")
      .map((c) => {
        const a = attributeList(c)!;
        return { name: childText(a, "Name"), dataType: childText(a, "DataTypeName"), value: childText(a, "Value"), comment: commentOf(c) };
      })
      .filter((c) => c.name);
    return { name, tags, constants };
  });
}

/** The spreadsheet export: one table per distinct Path column value, `PLC Tags` sheet first. */
export async function parseTagTableXlsx(bytes: Uint8Array): Promise<TagTable[]> {
  const sheets = await readXlsxRows(bytes);
  const sheet = sheets.find((s) => /plc tags/i.test(s.name)) ?? sheets[0];
  if (!sheet || sheet.rows.length === 0) throw new Error("The spreadsheet has no rows");
  const header = sheet.rows[0].map((h) => h.trim().toLowerCase());
  const col = (...names: string[]) => header.findIndex((h) => names.includes(h));
  const iName = col("name");
  const iType = col("data type", "datatype");
  const iAddr = col("logical address", "address");
  const iPath = col("path");
  const iComment = col("comment");
  if (iName < 0 || iType < 0) {
    throw new Error("Expected Name and Data Type columns in the spreadsheet");
  }
  const tables = new Map<string, TagTable>();
  for (const row of sheet.rows.slice(1)) {
    const name = (row[iName] ?? "").trim();
    if (!name) continue;
    const path = iPath >= 0 ? (row[iPath] ?? "").trim() || "Tags" : "Tags";
    const table = tables.get(path) ?? { name: path, tags: [], constants: [] };
    table.tags.push({
      name,
      dataType: (row[iType] ?? "").trim(),
      address: iAddr >= 0 ? (row[iAddr] ?? "").trim() : "",
      comment: iComment >= 0 ? (row[iComment] ?? "").trim() : "",
    });
    tables.set(path, table);
  }
  return [...tables.values()];
}

function stComment(text: string): string {
  return text ? ` (* ${text.replace(/\*\)/g, "* )")} *)` : "";
}

export interface TagTableSt {
  fileName: string;
  st: string;
  warnings: string[];
}

/**
 * A tag table as ST: a PROGRAM whose VAR block holds every tag as a located variable
 * (`Start AT %IX0.0 : BOOL;`), and its constants in a VAR CONSTANT block. Copy the
 * declarations into the programs that use them.
 */
export function tagTableToSt(table: TagTable): TagTableSt {
  const warnings: string[] = [];
  const ids = uniqueIdentifiers([...table.tags.map((t) => t.name), ...table.constants.map((c) => c.name)]);
  const pou = `${toIdentifier(table.name)}_Tags`;
  const lines = [
    `(* PLC tags imported from tag table "${table.name}".`,
    `   Copy the declarations you need into your programs. *)`,
    `PROGRAM ${pou}`,
  ];
  if (table.tags.length > 0) {
    lines.push("VAR");
    table.tags.forEach((tag, i) => {
      const type = mapElementaryType(tag.dataType || "Bool");
      if (type.warning) warnings.push(`${tag.name}: ${type.warning}`);
      const iec = tag.address ? s7ToIecAddress(tag.address) : null;
      if (tag.address && !iec) warnings.push(`${tag.name}: address ${tag.address} has no IEC form, declared without AT`);
      const renamed = ids[i] !== tag.name ? ` "${tag.name}"` : "";
      const note = [renamed.trim(), tag.comment].filter(Boolean).join(": ");
      lines.push(`  ${ids[i]}${iec ? ` AT ${iec}` : ""} : ${type.st};${stComment(note)}`);
    });
    lines.push("END_VAR");
  }
  if (table.constants.length > 0) {
    lines.push("VAR CONSTANT");
    table.constants.forEach((c, j) => {
      const type = mapElementaryType(c.dataType || "Int");
      if (type.warning) warnings.push(`${c.name}: ${type.warning}`);
      lines.push(`  ${ids[table.tags.length + j]} : ${type.st} := ${c.value};${stComment(c.comment)}`);
    });
    lines.push("END_VAR");
  }
  lines.push(`END_PROGRAM`, "");
  return { fileName: `${pou}.st`, st: lines.join("\n"), warnings };
}

export interface ImportedWatchRow {
  name: string;
  address: string;
  dataType: string;
}

/** Tags that the Online watch table can monitor: those with an absolute address and a known type. */
export function tagTableWatchRows(table: TagTable): ImportedWatchRow[] {
  return table.tags.flatMap((t) => {
    const address = t.address ? s7WatchAddress(t.address) : null;
    const dataType = watchTypeFor(t.dataType);
    return address && dataType ? [{ name: t.name, address, dataType }] : [];
  });
}
