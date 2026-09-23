// Cloud-mode language-service worker: owns the wasm voltrus-lsp core
// (mt-stc diagnostics + symbol index for hover/completion/definition) so
// language features run entirely in the browser, next to the sim worker.
// All ops are full-text; the core is synchronous, so one op per message.
import init, { WasmLsp } from "@voltrus/lsp-wasm";
import type { LspRequest, LspResponse } from "../lsp/protocol";

let ready: Promise<WasmLsp> | null = null;

function core(): Promise<WasmLsp> {
  ready ??= init().then(() => new WasmLsp());
  return ready;
}

function post(msg: LspResponse): void {
  self.postMessage(msg);
}

self.onmessage = (e: MessageEvent<LspRequest>) => {
  const req = e.data;
  void (async () => {
    try {
      const lsp = await core();
      switch (req.type) {
        case "sync":
          post({
            id: req.id,
            ok: true,
            type: "sync",
            diagnostics: JSON.parse(lsp.sync(req.uri, req.text)),
          });
          break;
        case "close":
          lsp.close(req.uri);
          post({ id: req.id, ok: true, type: "close" });
          break;
        case "hover":
          post({
            id: req.id,
            ok: true,
            type: "hover",
            hover: JSON.parse(lsp.hover(req.uri, req.line, req.character)),
          });
          break;
        case "completion":
          post({
            id: req.id,
            ok: true,
            type: "completion",
            items: JSON.parse(lsp.completion(req.uri, req.line, req.character)),
          });
          break;
        case "definition":
          post({
            id: req.id,
            ok: true,
            type: "definition",
            location: JSON.parse(lsp.definition(req.uri, req.line, req.character)),
          });
          break;
      }
    } catch (err) {
      post({
        id: req.id,
        ok: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  })();
};
