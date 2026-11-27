// Block baselines: the details of the CPU's blocks saved at one moment, to compare the CPU with
// later. Kept in the project folder (`cpu_blocks.s7baseline`), or per CPU address in local storage
// when no folder is open.
import { invoke } from "../api/bridge";
import { useStore } from "../store";
import type { BlockDetails } from "./s7";

export interface Baseline {
  savedAt: string;
  host: string;
  blocks: BlockDetails[];
}

export type CompareStatus = "same" | "changed" | "new" | "missing";

export interface CompareRow {
  key: string;
  status: CompareStatus;
  now?: BlockDetails;
  then?: BlockDetails;
  /** What differs, for changed blocks: `checksum`, `code time`, `size`, ... */
  diffs: string[];
}

const FILE = "cpu_blocks.s7baseline";
const keyOf = (b: { block_type: string; block_number: number }) => `${b.block_type}${b.block_number}`;

export async function saveBaseline(baseline: Baseline): Promise<string> {
  const text = `${JSON.stringify(baseline, null, 2)}\n`;
  const dir = useStore.getState().projectDir;
  if (dir) {
    await invoke("save_file", { path: `${dir}/${FILE}`, content: text });
    return `${dir}/${FILE}`;
  }
  localStorage.setItem(`vplc_s7_baseline:${baseline.host}`, text);
  return "local storage";
}

export async function loadBaseline(host: string): Promise<Baseline | null> {
  const dir = useStore.getState().projectDir;
  let text: string | null = null;
  if (dir) {
    try { text = await invoke<string>("open_file", { path: `${dir}/${FILE}` }); } catch { text = null; }
  } else {
    text = localStorage.getItem(`vplc_s7_baseline:${host}`);
  }
  if (!text) return null;
  try {
    const raw = JSON.parse(text);
    return Array.isArray(raw?.blocks) ? (raw as Baseline) : null;
  } catch {
    return null;
  }
}

/** Compare the blocks now with a baseline: same, changed (and how), new, or missing. */
export function compareBlocks(now: BlockDetails[], then: BlockDetails[]): CompareRow[] {
  const before = new Map(then.map((b) => [keyOf(b), b]));
  const rows: CompareRow[] = now.map((b) => {
    const old = before.get(keyOf(b));
    before.delete(keyOf(b));
    if (!old) return { key: keyOf(b), status: "new", now: b, diffs: [] };
    const diffs: string[] = [];
    if (old.checksum !== b.checksum) diffs.push("checksum");
    if (old.code_time !== b.code_time) diffs.push("code time");
    if (old.interface_time !== b.interface_time) diffs.push("interface time");
    if (old.load_size !== b.load_size) diffs.push("size");
    if (old.version !== b.version) diffs.push("version");
    return { key: keyOf(b), status: diffs.length ? "changed" : "same", now: b, then: old, diffs };
  });
  for (const [key, old] of before) rows.push({ key, status: "missing", then: old, diffs: [] });
  const order = ["OB", "FB", "FC", "DB", "SDB", "SFB", "SFC"];
  return rows.sort((a, b) => {
    const x = a.now ?? a.then!;
    const y = b.now ?? b.then!;
    return order.indexOf(x.block_type) - order.indexOf(y.block_type) || x.block_number - y.block_number;
  });
}
