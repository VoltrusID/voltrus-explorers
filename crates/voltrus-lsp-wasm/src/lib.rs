//! WASM bindings to the `voltrus-lsp` core: full-text document sync with
//! mt-stc diagnostics, symbol hover, completion, and definition. The
//! tower-lsp/tokio stdio transport stays native-only (the crate's `server`
//! feature); the browser worker (apps/voltrus-plc/src/wasm/lsp.worker.ts)
//! drives this core directly.
//!
//! All calls are synchronous over JSON strings — same binding style as
//! mt-stc-wasm — so the worker needs no async plumbing.

use wasm_bindgen::prelude::*;

use voltrus_lsp::index::{Hit, WorkspaceIndex};

#[wasm_bindgen]
pub struct WasmLsp {
    index: WorkspaceIndex,
}

#[derive(serde::Serialize)]
struct HoverJson {
    name: String,
    type_name: Option<String>,
    kind: &'static str,
    owner: Option<String>,
    decl_line: u32,
    uri: String,
    /// LSP range covering the identifier at the declaration site.
    range: lsp_types::Range,
}

#[wasm_bindgen]
impl WasmLsp {
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmLsp {
        WasmLsp {
            index: WorkspaceIndex::default(),
        }
    }

    /// Full-text document sync (LSP didOpen/didChange collapsed into one op;
    /// `WorkspaceIndex::open` is an upsert). Returns the mt-stc diagnostics
    /// for this document as a JSON array of LSP Diagnostics.
    pub fn sync(&mut self, uri: &str, text: &str) -> Result<String, JsValue> {
        let url = parse_uri(uri)?;
        self.index.open(url, text.to_string());
        let out = mt_stc::compile(text);
        let diags: Vec<lsp_types::Diagnostic> = out
            .diagnostics
            .iter()
            .map(|d| voltrus_lsp::convert::to_lsp(d, text))
            .collect();
        serde_json::to_string(&diags).map_err(js_err)
    }

    /// Drop the open-document flag; the symbol table stays for cross-file
    /// lookups (mirrors the tower-lsp didClose behavior).
    pub fn close(&mut self, uri: &str) -> Result<(), JsValue> {
        let url = parse_uri(uri)?;
        self.index.close(&url);
        Ok(())
    }

    /// Hover payload for the identifier at the position as JSON (null when
    /// nothing resolves). Structured rather than pre-formatted markdown so
    /// the client owns rendering.
    pub fn hover(&self, uri: &str, line: u32, character: u32) -> Result<String, JsValue> {
        let json = self.resolve(uri, line, character)?.map(|h| HoverJson {
            name: h.symbol.name,
            type_name: h.symbol.type_name,
            kind: h.symbol.kind.label(),
            owner: h.symbol.owner,
            decl_line: h.symbol.decl_line,
            uri: h.uri.to_string(),
            range: h.symbol.name_range,
        });
        serde_json::to_string(&json).map_err(js_err)
    }

    /// All indexed declarations plus IEC keywords, as a JSON array of LSP
    /// CompletionItems. Position accepted for protocol symmetry; the core
    /// completion set is position-independent.
    pub fn completion(&self, uri: &str, line: u32, character: u32) -> Result<String, JsValue> {
        let _ = (uri, line, character);
        let items = self.index.completions();
        serde_json::to_string(&items).map_err(js_err)
    }

    /// Declaration location for the identifier at the position, as JSON
    /// (null when unresolved).
    pub fn definition(&self, uri: &str, line: u32, character: u32) -> Result<String, JsValue> {
        let loc = self
            .resolve(uri, line, character)?
            .map(|h: Hit| lsp_types::Location {
                uri: h.uri,
                range: h.symbol.name_range,
            });
        serde_json::to_string(&loc).map_err(js_err)
    }
}

impl WasmLsp {
    fn resolve(&self, uri: &str, line: u32, character: u32) -> Result<Option<Hit>, JsValue> {
        let url = parse_uri(uri)?;
        Ok(self
            .index
            .resolve(&url, lsp_types::Position { line, character }))
    }
}

fn parse_uri(uri: &str) -> Result<lsp_types::Url, JsValue> {
    lsp_types::Url::parse(uri)
        .map_err(|e| JsValue::from_str(&format!("invalid document uri {uri:?}: {e}")))
}

fn js_err(e: serde_json::Error) -> JsValue {
    JsValue::from_str(&e.to_string())
}
