// EtherNet/IP UDT import (issue PLC-7): browse a controller for UDT tags,
// preview the generated IEC 61131-3 TYPE file, and write it into the project
// via the same in-editor file path the New File flow uses.
import { invoke } from "../api/bridge";
import { useStore } from "../store";

export const EIP_TYPES_FILENAME = "eip_types.st";

export interface EipImportMember {
  name: string;
  type: string;
}

export interface EipImportTag {
  name: string;
  template: string;
  members: EipImportMember[];
}

export interface EipImportResult {
  tags: EipImportTag[];
  generated_st: string;
}

const BINDINGS_HEADER = "(* Suggested VAR_EXTERNAL bindings *)";

export async function fetchEipUdts(
  host: string,
  port: number,
  tagGlob: string
): Promise<EipImportResult> {
  return invoke<EipImportResult>("eip_import_udts", { host, port, tagGlob });
}

// Keep only the TYPE blocks of selected tags' templates and the VAR_EXTERNAL
// binding comment lines of selected tags. The Rust generator emits a fixed
// line format, so line-shape parsing here is stable.
export function filterGeneratedSt(
  st: string,
  selectedTags: Set<string>,
  tags: EipImportTag[]
): string {
  const lines = st.split("\n");
  const firstType = lines.findIndex((l) => l.startsWith("TYPE "));
  if (firstType < 0) return st;

  const selectedTemplates = new Set(
    tags.filter((t) => selectedTags.has(t.name)).map((t) => t.template)
  );

  const header = lines.slice(0, firstType).filter((l) => l.length > 0);

  const blocks: string[][] = [];
  let i = firstType;
  while (i < lines.length && lines[i] !== BINDINGS_HEADER) {
    if (lines[i].startsWith("TYPE ")) {
      const block: string[] = [];
      while (i < lines.length && lines[i] !== "END_TYPE") {
        block.push(lines[i]);
        i++;
      }
      if (i < lines.length) block.push(lines[i]);
      blocks.push(block);
    }
    i++;
  }
  const bindings = lines.slice(i).filter((l) => l.length > 0);
  const keptBindings = bindings.filter(
    (l) =>
      l === BINDINGS_HEADER ||
      selectedTags.has((l.match(/^\(\*\s+(.+?) : /) ?? [])[1] ?? "")
  );

  const kept = blocks
    .filter((b) => selectedTemplates.has((b[0].split(/\s+/)[1] ?? "")))
    .map((b) => b.join("\n"));

  const parts = [header.join("\n")];
  if (kept.length > 0) parts.push(kept.join("\n\n"));
  parts.push(keptBindings.join("\n"));
  return parts.filter((p) => p.length > 0).join("\n\n") + "\n";
}

// Accept: write eip_types.st into the project. With an open project folder the
// file is persisted next to the sources and opened from disk; otherwise it is
// added to the editor like store.newFile does.
export async function acceptEipImport(
  result: EipImportResult,
  selectedTags: Set<string>
): Promise<string> {
  const st = filterGeneratedSt(result.generated_st, selectedTags, result.tags);
  const store = useStore.getState();
  const path = store.projectDir
    ? `${store.projectDir}/${EIP_TYPES_FILENAME}`
    : EIP_TYPES_FILENAME;

  if (store.projectDir) {
    await invoke("save_file", { path, content: st });
    await store.openFile(path);
  } else {
    const { openFiles } = useStore.getState();
    const existing = openFiles.findIndex((f) => f.path === path);
    if (existing >= 0) {
      const next = [...openFiles];
      next[existing] = { ...next[existing], content: st, modified: true };
      useStore.setState({ openFiles: next, activeFileIndex: existing });
    } else {
      useStore.setState({
        openFiles: [
          ...openFiles,
          { name: EIP_TYPES_FILENAME, path, content: st, modified: false },
        ],
        activeFileIndex: openFiles.length,
      });
    }
  }
  store.addLog(
    "system",
    `Imported UDT types into ${EIP_TYPES_FILENAME} (${selectedTags.size} tag(s))`
  );
  return path;
}
