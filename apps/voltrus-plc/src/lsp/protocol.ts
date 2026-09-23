// Wire types between the editor-side host (src/lsp/lsp-host.ts) and the wasm
// language-service worker (src/wasm/lsp.worker.ts). The worker wraps the
// voltrus-lsp-wasm core; all ops are full-text and every request carries a
// correlation id echoed on its response.
//
// Positions/ranges follow LSP: 0-based line, 0-based character in UTF-16 code
// units (identical to CodeMirror's offset units per line).

export type LspPosition = { line: number; character: number };

export type LspRange = { start: LspPosition; end: LspPosition };

// severity follows LSP numeric codes: 1 error, 2 warning, 3 info, 4 hint.
export type LspDiagnostic = {
  range: LspRange;
  severity: number;
  message: string;
};

export type LspHoverInfo = {
  name: string;
  type_name: string | null;
  kind: string;
  owner: string | null;
  decl_line: number;
  uri: string;
  range: LspRange;
};

export type LspCompletionItem = {
  label: string;
  detail?: string | null;
  kind?: number | null;
};

export type LspLocation = { uri: string; range: LspRange };

// ── Requests (host → worker) ──

export type LspRequest =
  | { id: number; type: "sync"; uri: string; text: string }
  | { id: number; type: "close"; uri: string }
  | { id: number; type: "hover"; uri: string; line: number; character: number }
  | { id: number; type: "completion"; uri: string; line: number; character: number }
  | { id: number; type: "definition"; uri: string; line: number; character: number };

// Requests without the correlation id — the host assigns ids on send.
// Distributive conditional: a plain Omit over the union would collapse the
// variants into their common keys and lose discrimination.
export type LspRequestBody = LspRequest extends infer T
  ? T extends { id: number }
    ? Omit<T, "id">
    : never
  : never;

// ── Responses (worker → host) ──

export type LspResponse =
  | { id: number; ok: true; type: "sync"; diagnostics: LspDiagnostic[] }
  | { id: number; ok: true; type: "close" }
  | { id: number; ok: true; type: "hover"; hover: LspHoverInfo | null }
  | { id: number; ok: true; type: "completion"; items: LspCompletionItem[] }
  | { id: number; ok: true; type: "definition"; location: LspLocation | null }
  | { id: number; ok: false; error: string };
