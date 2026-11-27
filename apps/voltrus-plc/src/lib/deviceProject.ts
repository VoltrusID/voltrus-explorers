// Reading, creating and updating the device documents (watch and tag tables) in the open project,
// or among the open tabs when no project folder is open.
import { invoke } from "../api/bridge";
import { useStore } from "../store";
import {
  emptyWatchDoc,
  isTagDoc,
  isWatchDoc,
  nextDocName,
  ONLINE_PATH,
  newDbDoc,
  parseTagDoc,
  serializeTagDoc,
  serializeWatchDoc,
  TAGS_EXT,
  WATCH_EXT,
  type TagDoc,
  type WatchDocRow,
} from "./s7Docs";
import { emptyHwDoc, familyFromOrderCode, HW_EXT, isHwDoc, serializeHwDoc } from "./s7Hw";
import { useS7 } from "./s7Store";

export interface DocRef {
  path: string;
  name: string;
}

/** Device documents in the project folder and among the open tabs, sorted by name. */
export function listDeviceDocs(kind: "watch" | "tags" | "hw"): DocRef[] {
  const { projectFiles, openFiles } = useStore.getState();
  const test = kind === "watch" ? isWatchDoc : kind === "hw" ? isHwDoc : isTagDoc;
  const byPath = new Map<string, DocRef>();
  for (const f of projectFiles) if (!f.is_dir && test(f.path)) byPath.set(f.path, { path: f.path, name: f.name });
  for (const f of openFiles) if (test(f.path)) byPath.set(f.path, { path: f.path, name: f.name });
  return [...byPath.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
}

/** Open a tab for a generated file, saving it into the project folder when one is open. */
export async function writeProjectFile(name: string, content: string, open = true) {
  const store = useStore.getState();
  const path = store.projectDir ? `${store.projectDir}/${name}` : name;
  if (store.projectDir) {
    await invoke("save_file", { path, content });
    await store.refreshProjectFiles();
    if (open) await store.openFile(path);
    return path;
  }
  const { openFiles } = useStore.getState();
  const existing = openFiles.findIndex((f) => f.path === path);
  if (existing >= 0) {
    const next = [...openFiles];
    next[existing] = { ...next[existing], content, modified: true };
    useStore.setState({ openFiles: next, ...(open ? { activeFileIndex: existing } : {}) });
  } else {
    useStore.setState({
      openFiles: [...openFiles, { name, path, content, modified: true }],
      ...(open ? { activeFileIndex: openFiles.length } : {}),
    });
  }
  return path;
}

/** Content of a document: the open tab's (possibly unsaved) text, else the file on disk. */
export async function readDoc(path: string): Promise<string> {
  const open = useStore.getState().openFiles.find((f) => f.path === path);
  if (open) return open.content;
  return invoke<string>("open_file", { path });
}

/** Replace a document's content: in its tab if open (marked unsaved), else on disk. */
export async function updateDoc(path: string, content: string) {
  const store = useStore.getState();
  const index = store.openFiles.findIndex((f) => f.path === path);
  if (index >= 0) {
    store.updateFileContent(index, content);
    return;
  }
  await invoke("save_file", { path, content });
}

/** Open a project document in a tab, or switch to its tab. */
export async function openDoc(path: string) {
  const store = useStore.getState();
  const index = store.openFiles.findIndex((f) => f.path === path);
  if (index >= 0) store.setActiveFile(index);
  else await store.openFile(path);
}

export function openOnlineTab() {
  const { openFiles } = useStore.getState();
  const index = openFiles.findIndex((f) => f.path === ONLINE_PATH);
  if (index >= 0) {
    useStore.setState({ activeFileIndex: index });
    return;
  }
  useStore.setState({
    openFiles: [...openFiles, { name: "Online & diagnostics", path: ONLINE_PATH, content: "", modified: false }],
    activeFileIndex: openFiles.length,
  });
}

function takenNames(): string[] {
  const { projectFiles, openFiles } = useStore.getState();
  return [...projectFiles.map((f) => f.name), ...openFiles.map((f) => f.name)];
}

export async function createWatchTable(rows: WatchDocRow[] = [], base = "Watch table") {
  const name = nextDocName(base, WATCH_EXT, takenNames());
  return writeProjectFile(name, serializeWatchDoc(emptyWatchDoc(rows)));
}

/** A free file name for a tag table: `Motor_DB.s7tags`, else `Motor_DB_1.s7tags`, ... */
export function tagDocName(base: string): string {
  const taken = new Set(takenNames().map((n) => n.toLowerCase()));
  const plain = `${base}${TAGS_EXT}`;
  return taken.has(plain.toLowerCase()) ? nextDocName(base, TAGS_EXT, [...taken]) : plain;
}

/** Every tag table in the project, parsed; unreadable ones are skipped. */
export async function loadTagDocs(): Promise<TagDoc[]> {
  const docs: TagDoc[] = [];
  for (const ref of listDeviceDocs("tags")) {
    try {
      docs.push(parseTagDoc(await readDoc(ref.path)));
    } catch { /* skip */ }
  }
  return docs;
}

/** Open the device configuration, creating it (for the connected CPU's family) if there is none. */
export async function openDeviceConfig() {
  const existing = listDeviceDocs("hw")[0];
  if (existing) return openDoc(existing.path);
  const order = useS7.getState().info?.cpu?.order_code ?? "";
  const doc = emptyHwDoc(familyFromOrderCode(order) ?? "S7-1200");
  return writeProjectFile(`Device${HW_EXT}`, serializeHwDoc(doc));
}

/** A new, empty PLC tag table or standard-access DB in the project. */
export async function createTagDoc(kind: "tags" | "db") {
  const name = tagDocName(kind === "db" ? "Data_block" : "PLC_tags").replace(/\.s7tags$/, "");
  const doc = kind === "db"
    ? newDbDoc(name)
    : { format: "voltrus-s7-tags" as const, version: 1 as const, kind: "tags" as const, rows: [{ name: "Tag_1", dataType: "Bool", address: "M0.0", comment: "" }] };
  return writeProjectFile(`${name}${TAGS_EXT}`, serializeTagDoc(doc));
}
