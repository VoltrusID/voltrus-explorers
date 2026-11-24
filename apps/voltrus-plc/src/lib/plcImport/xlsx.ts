// Minimal .xlsx reader: enough of ZIP (stored and deflated entries, via DecompressionStream) and
// of SpreadsheetML (shared strings, inline strings, numbers) to read the rows of each sheet.

interface ZipEntry {
  name: string;
  method: number;
  compressedSize: number;
  localOffset: number;
}

function u16(b: Uint8Array, o: number) {
  return b[o] | (b[o + 1] << 8);
}
function u32(b: Uint8Array, o: number) {
  return (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
}

function zipEntries(b: Uint8Array): ZipEntry[] {
  // End of central directory: signature 0x06054b50 within the last 64 KiB + 22 bytes.
  let eocd = -1;
  for (let i = b.length - 22; i >= Math.max(0, b.length - 65_557); i--) {
    if (u32(b, i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error("Not a ZIP file (is it really .xlsx?)");
  const count = u16(b, eocd + 10);
  let p = u32(b, eocd + 16);
  const entries: ZipEntry[] = [];
  const dec = new TextDecoder();
  for (let i = 0; i < count; i++) {
    if (u32(b, p) !== 0x02014b50) throw new Error("Damaged ZIP directory");
    const method = u16(b, p + 10);
    const compressedSize = u32(b, p + 20);
    const nameLen = u16(b, p + 28);
    const extraLen = u16(b, p + 30);
    const commentLen = u16(b, p + 32);
    const localOffset = u32(b, p + 42);
    const name = dec.decode(b.subarray(p + 46, p + 46 + nameLen));
    entries.push({ name, method, compressedSize, localOffset });
    p += 46 + nameLen + extraLen + commentLen;
  }
  return entries;
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const source = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(data);
      controller.close();
    },
  });
  const stream = source.pipeThrough(new DecompressionStream("deflate-raw") as unknown as TransformStream<Uint8Array, Uint8Array>);
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

async function readEntry(b: Uint8Array, e: ZipEntry): Promise<string> {
  const p = e.localOffset;
  if (u32(b, p) !== 0x04034b50) throw new Error(`Damaged ZIP entry ${e.name}`);
  const start = p + 30 + u16(b, p + 26) + u16(b, p + 28);
  const raw = b.subarray(start, start + e.compressedSize);
  let data: Uint8Array;
  if (e.method === 0) data = raw;
  else if (e.method === 8) data = await inflateRaw(raw);
  else throw new Error(`Unsupported ZIP compression ${e.method} in ${e.name}`);
  return new TextDecoder().decode(data);
}

function elements(doc: Document | Element, name: string): Element[] {
  return Array.from(doc.getElementsByTagName("*")).filter((e) => e.localName === name);
}

function childrenNamed(el: Element, name: string): Element[] {
  return Array.from(el.children).filter((c) => c.localName === name);
}

/** Column index of a cell reference such as `C12` (A = 0). */
function columnOf(ref: string): number {
  const letters = ref.match(/^[A-Z]+/)?.[0] ?? "A";
  let n = 0;
  for (const ch of letters) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n - 1;
}

export interface SheetRows {
  name: string;
  rows: string[][];
}

export async function readXlsxRows(bytes: Uint8Array): Promise<SheetRows[]> {
  const entries = zipEntries(bytes);
  const byName = new Map(entries.map((e) => [e.name, e]));
  const text = async (name: string) => {
    const e = byName.get(name);
    return e ? readEntry(bytes, e) : null;
  };
  const parse = (xml: string) => new DOMParser().parseFromString(xml, "application/xml");

  const shared: string[] = [];
  const sst = await text("xl/sharedStrings.xml");
  if (sst) {
    for (const si of elements(parse(sst), "si")) {
      shared.push(elements(si, "t").map((t) => t.textContent ?? "").join(""));
    }
  }

  // Sheet names and their files, through the workbook relationships.
  const workbook = await text("xl/workbook.xml");
  const rels = await text("xl/_rels/workbook.xml.rels");
  const targets = new Map<string, string>();
  if (rels) {
    for (const r of elements(parse(rels), "Relationship")) {
      const target = r.getAttribute("Target") ?? "";
      targets.set(r.getAttribute("Id") ?? "", target.startsWith("/") ? target.slice(1) : `xl/${target}`);
    }
  }
  const sheets: { name: string; file: string }[] = [];
  if (workbook) {
    for (const s of elements(parse(workbook), "sheet")) {
      const rid = s.getAttribute("r:id") ?? s.getAttributeNS("http://schemas.openxmlformats.org/officeDocument/2006/relationships", "id") ?? "";
      const file = targets.get(rid);
      if (file) sheets.push({ name: s.getAttribute("name") ?? file, file });
    }
  }
  if (sheets.length === 0) {
    for (const e of entries) if (/^xl\/worksheets\/sheet\d+\.xml$/.test(e.name)) sheets.push({ name: e.name, file: e.name });
  }

  const out: SheetRows[] = [];
  for (const sheet of sheets) {
    const xml = await text(sheet.file);
    if (!xml) continue;
    const rows: string[][] = [];
    for (const row of elements(parse(xml), "row")) {
      const cells: string[] = [];
      for (const c of childrenNamed(row, "c")) {
        const col = columnOf(c.getAttribute("r") ?? "");
        const type = c.getAttribute("t");
        let value = "";
        if (type === "inlineStr") {
          value = elements(c, "t").map((t) => t.textContent ?? "").join("");
        } else {
          const v = childrenNamed(c, "v")[0]?.textContent ?? "";
          value = type === "s" ? (shared[Number(v)] ?? "") : v;
        }
        cells[col] = value;
      }
      rows.push(Array.from(cells, (v) => v ?? ""));
    }
    out.push({ name: sheet.name, rows });
  }
  return out;
}
