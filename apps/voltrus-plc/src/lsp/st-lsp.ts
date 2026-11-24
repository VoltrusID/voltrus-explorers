import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { linter, type Diagnostic } from "@codemirror/lint";
import { Facet, type Extension, type Text } from "@codemirror/state";
import { EditorView, hoverTooltip, ViewPlugin } from "@codemirror/view";
import { stCompletions } from "../components/Editor/st-completion";
import { useStore } from "../store";
import { getLspHost } from "./lsp-host";
import type { LspCompletionItem, LspDiagnostic, LspHoverInfo, LspPosition } from "./protocol";

// ── Position/URI conversion (LSP: 0-based line, 0-based UTF-16 character —
// the same unit CodeMirror offsets use within a line) ──

/// Virtual document URI for a project-relative path. The one wire convention
/// both sides of the worker protocol derive identically, so index lookups
/// agree regardless of where the path came from.
export function toDocUri(path: string): string {
  return `file:///${path.replace(/^\//, "")}`;
}

export function toLspPosition(doc: Text, pos: number): LspPosition {
  const line = doc.lineAt(pos);
  return { line: line.number - 1, character: pos - line.from };
}

export function fromLspPosition(doc: Text, p: LspPosition): number {
  const lineNo = Math.min(Math.max(p.line + 1, 1), doc.lines);
  const line = doc.line(lineNo);
  return Math.min(line.from + Math.max(0, p.character), line.to);
}

const SEVERITY_BY_CODE: Record<number, "error" | "warning" | "info"> = {
  1: "error",
  2: "warning",
  3: "info",
  4: "info",
};

export function mapLspDiagnostics(doc: Text, diags: LspDiagnostic[]): Diagnostic[] {
  return diags.map((d) => {
    const from = fromLspPosition(doc, d.range.start);
    // CodeMirror wants a non-empty range where possible (same guard the
    // tauri lint source applies to mt-stc's col counts).
    const to = Math.max(fromLspPosition(doc, d.range.end), Math.min(from + 1, doc.length));
    return {
      from,
      to,
      severity: SEVERITY_BY_CODE[d.severity] ?? "info",
      message: d.message,
      source: "voltrus-lsp",
    };
  });
}

export function formatHover(h: LspHoverInfo): string {
  const signature = h.type_name ? `${h.name} : ${h.type_name}` : h.name;
  const file = h.uri.split("/").pop() ?? h.uri;
  const scope = h.owner ? `in ${h.owner} · ` : "";
  return `${signature}\n${h.kind} · ${scope}declared in ${file}, line ${h.decl_line}`;
}

// ── Document identity ──

// Doc URI of the project file this editor instance edits. Only set in cloud
// mode; empty in tauri mode, which keeps the local compile lint path.
export const lspDocUri = Facet.define<string, string>({
  combine: (values) => values[0] ?? "",
});

// ── Project context ──

let projectSyncInstalled = false;

// Push every open .st tab so hover/completion/definition see cross-file
// symbols. Cloud open + save both land as content updates in the main
// store's openFiles (openCloudFile → openFile; saveFile canonicalization),
// so watching that list covers the refresh-on-open/save requirement.
function pushOpenFiles(): void {
  const host = getLspHost();
  for (const file of useStore.getState().openFiles) {
    if (!file.path.endsWith(".st")) continue;
    void host.request({ type: "sync", uri: toDocUri(file.path), text: file.content }).catch(() => {});
  }
}

function installProjectWatcher(): void {
  useStore.subscribe((state, prev) => {
    const host = getLspHost();
    const before = new Map(prev.openFiles.map((f) => [f.path, f] as const));
    const current = new Set<string>();
    for (const file of state.openFiles) {
      current.add(file.path);
      if (!file.path.endsWith(".st")) continue;
      const was = before.get(file.path);
      if (!was || was.content !== file.content) {
        void host.request({ type: "sync", uri: toDocUri(file.path), text: file.content }).catch(() => {});
      }
    }
    for (const file of prev.openFiles) {
      if (file.path.endsWith(".st") && !current.has(file.path)) {
        void host.request({ type: "close", uri: toDocUri(file.path) }).catch(() => {});
      }
    }
  });
}

/// Idempotent cloud-mode startup: install the open-files watcher once, then
/// push the current tabs. Called when a cloud StEditor view is created.
export function ensureLspProjectSynced(): void {
  if (!projectSyncInstalled) {
    projectSyncInstalled = true;
    installProjectWatcher();
  }
  pushOpenFiles();
}

// ── CodeMirror extensions ──

// Diagnostics from the wasm worker's full-text sync; the sync doubles as the
// document update (didOpen/didChange collapsed into one op worker-side).
export const stLspLinter = linter(
  async (view) => {
    const uri = view.state.facet(lspDocUri);
    if (!uri) return [];
    const doc = view.state.doc;
    if (!doc.toString().trim()) return [];
    try {
      const res = await getLspHost().request({ type: "sync", uri, text: doc.toString() });
      // Map against the captured doc; the linter discards stale results
      // when the document moved on while the request was in flight.
      return mapLspDiagnostics(doc, res.diagnostics);
    } catch {
      return [];
    }
  },
  { delay: 300 },
);

export const stLspHover = hoverTooltip(async (view, pos) => {
  const uri = view.state.facet(lspDocUri);
  if (!uri) return null;
  try {
    const at = toLspPosition(view.state.doc, pos);
    const res = await getLspHost().request({ type: "hover", uri, line: at.line, character: at.character });
    if (!res.hover) return null;
    const dom = document.createElement("div");
    dom.className = "st-lsp-hover";
    dom.style.whiteSpace = "pre-wrap";
    dom.textContent = formatHover(res.hover);
    return {
      pos: fromLspPosition(view.state.doc, res.hover.range.start),
      create: () => ({ dom }),
    };
  } catch {
    return null;
  }
});

// LSP CompletionItemKind → CodeMirror completion type (the kinds the core
// emits).
const TYPE_BY_LSP_KIND: Record<number, string> = {
  2: "function", // Method
  3: "function", // Function
  5: "property", // Field
  6: "variable", // Variable
  7: "type", // Class
  13: "type", // Enum
  14: "keyword", // Keyword
  20: "variable", // EnumMember
  21: "constant", // Constant
  22: "type", // Struct
};

function completionFrom(item: LspCompletionItem): Completion {
  return {
    label: item.label,
    detail: item.detail ?? undefined,
    type: TYPE_BY_LSP_KIND[item.kind ?? 0],
  };
}

/// Static keyword/function/snippet completions merged with the wasm index's
/// project symbols. LSP items are deduplicated against the static list,
/// which keeps priority (its boost/apply metadata is richer).
export async function stLspCompletionSource(
  context: CompletionContext,
): Promise<CompletionResult | null> {
  const base = stCompletions(context);
  const uri = context.state.facet(lspDocUri);
  if (!uri) return base;
  const at = toLspPosition(context.state.doc, context.pos);
  let items: LspCompletionItem[] = [];
  try {
    const res = await getLspHost().request({ type: "completion", uri, line: at.line, character: at.character });
    items = res.items;
  } catch {
    return base; // worker unavailable: static list only
  }
  const seen = new Set((base?.options ?? []).map((o) => o.label));
  const extra = items.filter((i) => i.label !== "" && !seen.has(i.label)).map(completionFrom);
  if (!base) {
    const word = context.matchBefore(/[\w]*/);
    if (!word || (word.from === word.to && !context.explicit)) return null;
    return { from: word.from, options: extra };
  }
  return { from: base.from, options: [...base.options, ...extra] };
}

export const stLspCompletion = autocompletion({ override: [stLspCompletionSource] });

// Runs once per cloud editor view creation: brings the worker up and pushes
// the open tabs (extension construction itself stays side-effect free).
const lspBootstrap = ViewPlugin.fromClass(
  class {
    constructor(_view: EditorView) {
      ensureLspProjectSynced();
    }
  },
);

/// Cloud-mode replacement for stCompletion + stLinter: wasm-backed
/// diagnostics, hover tooltips, and project-aware completion. `filePath`
/// identities the edited document on the worker side.
export function stLspExtensions(filePath: string): Extension[] {
  return [
    lspDocUri.of(toDocUri(filePath)),
    lspBootstrap,
    stLspLinter,
    stLspHover,
    stLspCompletion,
  ];
}
