import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CompletionContext } from "@codemirror/autocomplete";
import { forEachDiagnostic } from "@codemirror/lint";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { LspDiagnostic, LspRequestBody, LspResponse } from "./protocol";
import { defaultLspWorkerFactory, resetLspHost, setLspHost, setLspWorkerFactory, type LspHost } from "./lsp-host";
import {
  ensureLspProjectSynced,
  formatHover,
  fromLspPosition,
  lspDocUri,
  mapLspDiagnostics,
  stLspCompletionSource,
  stLspExtensions,
  toDocUri,
  toLspPosition,
} from "./st-lsp";

const BROKEN_MAIN_ST = "PROGRAM Main\nVAR\n    m : Motor;\nEND_VAR\n    boom := 1;\nEND_PROGRAM\n";

// Spy host: records requests, answers them from canned data. Enough of the
// LspHost surface for the extension code under test.
function recordingHost(respond: (req: LspRequestBody) => Promise<LspResponse> | LspResponse) {
  const calls: LspRequestBody[] = [];
  const host = {
    request: async (req: LspRequestBody) => {
      calls.push(req);
      return respond(req);
    },
  } as unknown as LspHost;
  return { calls, host };
}

function brokenSync(): LspResponse {
  return {
    id: 0,
    ok: true,
    type: "sync",
    diagnostics: [
      {
        range: { start: { line: 4, character: 4 }, end: { line: 4, character: 8 } },
        severity: 1,
        message: "Unknown variable 'boom'",
      },
    ],
  } satisfies Extract<LspResponse, { type: "sync" }>;
}

beforeEach(() => {
  setLspWorkerFactory(() => ({ postMessage: () => {}, terminate: () => {} }) as unknown as Worker);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  setLspWorkerFactory(defaultLspWorkerFactory);
  resetLspHost();
});

describe("lsp position/uri mapping", () => {
  const doc = EditorState.create({ doc: BROKEN_MAIN_ST }).doc;

  it("converts CodeMirror offsets to LSP positions and back", () => {
    const pos = doc.line(5).from + 4; // `boom` on line index 4
    expect(toLspPosition(doc, pos)).toEqual({ line: 4, character: 4 });
    expect(fromLspPosition(doc, { line: 4, character: 4 })).toBe(pos);
  });

  it("clamps out-of-range LSP positions into the document", () => {
    const lastLine = doc.line(doc.lines);
    expect(fromLspPosition(doc, { line: 99, character: 0 })).toBe(lastLine.from);
    // character beyond the line content clamps to that line's end…
    expect(fromLspPosition(doc, { line: 4, character: 999 })).toBe(doc.line(5).to);
    // …and a line beyond the document clamps to the last line.
    expect(fromLspPosition(doc, { line: 4 + 99, character: 0 })).toBe(lastLine.from);
  });

  it("derives stable virtual doc uris", () => {
    expect(toDocUri("Main.st")).toBe("file:///Main.st");
    expect(toDocUri("/Main.st")).toBe("file:///Main.st");
    expect(toDocUri("src/Main.st")).toBe("file:///src/Main.st");
  });
});

describe("mapLspDiagnostics", () => {
  const doc = EditorState.create({ doc: BROKEN_MAIN_ST }).doc;

  it("maps LSP severity/range onto CodeMirror diagnostics", () => {
    const diags: LspDiagnostic[] = [
      {
        range: { start: { line: 4, character: 4 }, end: { line: 4, character: 8 } },
        severity: 1,
        message: "Unknown variable 'boom'",
      },
      {
        range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
        severity: 2,
        message: "empty program name",
      },
    ];
    const mapped = mapLspDiagnostics(doc, diags);
    expect(mapped[0].severity).toBe("error");
    expect(mapped[0].message).toBe("Unknown variable 'boom'");
    expect(mapped[0].from).toBe(doc.line(5).from + 4);
    expect(mapped[0].to).toBe(doc.line(5).from + 8);
    // Zero-width LSP range widens to one character, like the tauri lint path.
    expect(mapped[1].to).toBeGreaterThan(mapped[1].from);
    expect(mapped[1].severity).toBe("warning");
  });
});

describe("formatHover", () => {
  it("renders name/type/kind/origin as plain text", () => {
    expect(
      formatHover({
        name: "speed",
        type_name: "INT",
        kind: "VAR_INPUT",
        owner: "Motor",
        decl_line: 3,
        uri: "file:///Motor.st",
        range: { start: { line: 2, character: 4 }, end: { line: 2, character: 9 } },
      }),
    ).toBe("speed : INT\nVAR_INPUT · in Motor · declared in Motor.st, line 3");
  });
});

describe("stLspCompletionSource", () => {
  it("merges project symbols from the worker into the static list, deduped", async () => {
    const { host } = recordingHost((req) => {
      if (req.type !== "completion") throw new Error(`unexpected ${req.type}`);
      return {
        id: 0,
        ok: true,
        type: "completion",
        items: [
          { label: "Motor", kind: 3, detail: "FUNCTION_BLOCK Motor" },
          { label: "MOD", kind: 14 }, // duplicate of the static list → dropped
        ],
      } satisfies Extract<LspResponse, { type: "completion" }>;
    });
    setLspHost(host);
    const state = EditorState.create({ doc: "m", extensions: [lspDocUri.of("file:///Main.st")] });
    const result = await stLspCompletionSource(new CompletionContext(state, 1, true));
    expect(result).not.toBeNull();
    const labels = result!.options.map((o) => o.label);
    expect(labels).toContain("Motor"); // from the wasm index
    expect(labels).toContain("MOD"); // static keyword/function list
    expect(labels.filter((l) => l === "MOD")).toHaveLength(1); // deduped
    expect(labels).not.toContain("IF"); // static list filters by prefix "m"
    expect(result!.from).toBe(0);
  });

  it("falls back to the static list when the worker fails", async () => {
    const { host } = recordingHost(() => {
      throw new Error("worker gone");
    });
    setLspHost(host);
    const state = EditorState.create({ doc: "TO", extensions: [lspDocUri.of("file:///Main.st")] });
    const result = await stLspCompletionSource(new CompletionContext(state, 2, true));
    expect(result).not.toBeNull();
    expect(result!.options.map((o) => o.label)).toContain("TON");
  });
});

describe("cloud editor extension wiring", () => {
  it("syncs the edited document through the host on mount and on change", async () => {
    const { calls, host } = recordingHost((req) => {
      if (req.type !== "sync") throw new Error(`unexpected ${req.type}`);
      return brokenSync();
    });
    setLspHost(host);
    ensureLspProjectSynced();

    const view = new EditorView({
      state: EditorState.create({ doc: BROKEN_MAIN_ST, extensions: stLspExtensions("Main.st") }),
    });
    await vi.advanceTimersByTimeAsync(0); // flush the mount push microtasks
    // Mount pushes the open tabs from the main store (project context).
    expect(calls.some((c) => c.type === "sync" && c.uri === "file:///Main.st")).toBe(true);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: "PROGRAM Other\nEND_PROGRAM\n" },
    });
    await vi.advanceTimersByTimeAsync(300); // lint delay

    // The doc change re-syncs with the edited text (doubles as didChange).
    expect(calls.some((c) => c.type === "sync" && c.uri === "file:///Main.st" && c.text.startsWith("PROGRAM Other"))).toBe(true);
    view.destroy();
  });

  it("publishes worker diagnostics into the view's lint set", async () => {
    const { host } = recordingHost((req) => {
      if (req.type !== "sync") throw new Error(`unexpected ${req.type}`);
      return brokenSync();
    });
    setLspHost(host);
    const view = new EditorView({
      state: EditorState.create({ doc: BROKEN_MAIN_ST, extensions: stLspExtensions("Main.st") }),
    });
    // The lint source runs on document change (debounced 300ms).
    view.dispatch({ changes: { from: 0, to: 0, insert: "\n" } });
    await vi.advanceTimersByTimeAsync(300);

    const messages: string[] = [];
    forEachDiagnostic(view.state, (d) => messages.push(d.message));
    expect(messages.some((m) => m.includes("boom"))).toBe(true);
    view.destroy();
  });
});
