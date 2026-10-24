//! The `tower-lsp` server: lifecycle, full-text document sync, diagnostics,
//! hover, definition, completion.

use std::sync::RwLock;

use lsp_types::{
    CompletionItem, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, Location,
    MarkupContent, MarkupKind, OneOf, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};
use tower_lsp::jsonrpc::Result;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::convert;
use crate::index::Hit;

const SERVER_NAME: &str = "voltrus-lsp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Backend {
    client: Client,
    index: RwLock<crate::index::WorkspaceIndex>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            index: RwLock::new(crate::index::WorkspaceIndex::default()),
        }
    }

    /// Compile the document with mt-stc and publish mapped diagnostics.
    async fn publish_diagnostics(&self, uri: Url, text: &str) {
        let out = mt_stc::compile(text);
        let diags: Vec<_> = out
            .diagnostics
            .iter()
            .map(|d| convert::to_lsp(d, text))
            .collect();
        self.client.publish_diagnostics(uri, diags, None).await;
    }
}

fn workspace_paths(params: &InitializeParams) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(folders) = &params.workspace_folders {
        for f in folders {
            if f.uri.scheme() == "file" {
                if let Ok(p) = f.uri.to_file_path() {
                    paths.push(p);
                }
            }
        }
    }
    if paths.is_empty() {
        // Older clients: root_uri fallback.
        if let Some(root) = &params.root_uri {
            if root.scheme() == "file" {
                if let Ok(p) = root.to_file_path() {
                    paths.push(p);
                }
            }
        }
    }
    paths
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let folders = workspace_paths(&params);
        self.index
            .write()
            .expect("index lock poisoned")
            .set_workspace(folders);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: SERVER_NAME.to_string(),
                version: Some(SERVER_VERSION.to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.index
            .write()
            .expect("index lock poisoned")
            .open(uri.clone(), text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full-text sync: the last change event carries the whole document.
        let Some(text) = params.content_changes.last().map(|c| c.text.clone()) else {
            return;
        };
        let uri = params.text_document.uri;
        self.index
            .write()
            .expect("index lock poisoned")
            .change(&uri, text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.index
            .write()
            .expect("index lock poisoned")
            .close(&params.text_document.uri);
        // Clear diagnostics; the symbol table stays for cross-file lookups.
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let doc = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let hit = {
            let index = self.index.read().expect("index lock poisoned");
            index.resolve(&doc, pos)
        };
        Ok(hit.map(|h| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_markdown(&h, &doc),
            }),
            range: Some(h.symbol.name_range),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let doc = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let hit = {
            let index = self.index.read().expect("index lock poisoned");
            index.resolve(&doc, pos)
        };
        Ok(hit.map(|h| {
            GotoDefinitionResponse::Scalar(Location {
                uri: h.uri,
                range: h.symbol.name_range,
            })
        }))
    }

    async fn completion(&self, _: CompletionParams) -> Result<Option<CompletionResponse>> {
        let items: Vec<CompletionItem> = {
            let index = self.index.read().expect("index lock poisoned");
            index.completions()
        };
        Ok(Some(CompletionResponse::Array(items)))
    }
}

fn hover_markdown(hit: &Hit, open_uri: &Url) -> String {
    let s = &hit.symbol;
    let file = hit
        .uri
        .path()
        .rsplit('/')
        .next()
        .unwrap_or(hit.uri.as_str());
    let origin = if hit.uri == *open_uri {
        format!("declared in `{file}`, line {}", s.decl_line)
    } else {
        format!("declared in `{file}` (other file), line {}", s.decl_line)
    };
    match (&s.type_name, &s.owner) {
        (Some(t), Some(o)) => format!(
            "```iecst\n{} : {}\n```\n**{}** · in `{o}` · {origin}",
            s.name,
            t,
            s.kind.label()
        ),
        (Some(t), None) => format!(
            "```iecst\n{} : {}\n```\n**{}** · {origin}",
            s.name,
            t,
            s.kind.label()
        ),
        (None, Some(o)) => format!(
            "```iecst\n{}\n```\n**{}** · in `{o}` · {origin}",
            s.name,
            s.kind.label()
        ),
        (None, None) => format!("```iecst\n{} {}\n```\n{origin}", s.kind.label(), s.name),
    }
}

/// Run the server over stdio (the `voltrus-lsp` binary entry point).
pub async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
