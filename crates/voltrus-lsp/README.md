# voltrus-lsp

Language Server Protocol (LSP) server for Voltrus PLC Structured Text (ST)
projects, implementing the Atur PLC-13 toolchain integration. It compiles open
documents with the same engine as `mt-stc` / `mt-plc-cli check` and indexes
POUs, TYPE declarations, and variables for hover, go-to-definition, and
completion.

## Running

```sh
cargo run -p voltrus-lsp
```

The server speaks LSP over stdio (Content-Length framed JSON-RPC). Any LSP
client that can launch a binary works.

## Supported capabilities

| Capability | Notes |
|---|---|
| `textDocument/didOpen` / `didChange` / `didClose` | **Full-text sync** (`TextDocumentSyncKind.Full`). Each change recompiles the document via `mt_stc::compile` and publishes diagnostics. |
| `textDocument/publishDiagnostics` | Ranges are mapped from mt-stc's (1-based line, char-count column) output to LSP (0-based line, UTF-16 column). Severity is Error/Warning per mt-stc; source is tagged `mt-stc:analysis` / `mt-stc:parser` / `mt-stc:codegen`. |
| `textDocument/hover` | Identifier under the cursor → kind, declared type, owning POU/TYPE, file and 1-based declaration line (markdown). |
| `textDocument/definition` | Variable / POU / TYPE reference → declaration location, including **cross-file** through the workspace folder index. |
| `textDocument/completion` | Indexed declarations (POUs, types, vars, struct fields, enum values) + `mt-stc` stdlib functions + IEC 61131-3 keywords. |

Workspace files: on `initialize` the server scans `workspaceFolders` (fallback:
`rootUri`) for `*.st` files **directly inside the folder** (flat project model,
no recursion) and indexes them; open documents always override the on-disk
copies.

## Pointing VS Code at the binary

No extension build required — the server is a plain stdio binary, so any
generic LSP client can launch it. Search the VS Code marketplace for a generic
"LSP client" extension (several exist; none are Voltrus-specific) and point it
at the binary for `st`/`iecst` files, following that extension's schema —
conceptually:

```jsonc
// settings.json of your chosen generic LSP client extension
{
  "<client>.servers": {
    "voltrus-st": {
      "command": "/absolute/path/to/MacTools/target/debug/voltrus-lsp",
      "fileTypes": ["st"],
      "languageIds": ["iecst", "st"]
    }
  }
}
```

The lightest fully-scriptable path is Neovim's built-in LSP client:

```lua
-- ~/.config/nvim/lua/voltrus.lua (call :lua require'voltrus'.setup() )
require('lspconfig.configs').voltrus = {
  default_config = {
    name = 'voltrus-lsp',
    cmd = { vim.env.HOME .. '/Documents/JoyoDigitama/voltrus/MacTools/target/debug/voltrus-lsp' },
    filetypes = { 'iecst', 'st' },
    root_markers = { '.git' },
  },
}
vim.lsp.enable('voltrus')
```

Both routes speak plain stdio LSP to the binary; no extension packaging is
involved.

## Design notes

- **Compiler parity**: diagnostics come from `mt_stc::compile` — the identical
  function used by `mt-plc-cli check` — so the editor agrees with the CLI.
- **Indexing**: primary path is the `iecst` 0.5 AST (`parse_type_block` /
  `parse_pou`), the same parser mt-stc uses. While a program is mid-edit and
  no longer parses, a small line-oriented declaration scanner keeps hover /
  definition / completion working on the broken source.
- **Resolution**: case-insensitive (IEC identifiers are case-insensitive),
  exact-case preferred. Same-file variables resolve only inside their owning
  POU/TYPE span; cross-file, any declaration may be referenced.

## Honest limitations

- **No rename** (`textDocument/rename`), **no semantic tokens**, no document
  symbols, no references, no formatting.
- **Full-text sync only** — every keystroke resends the whole document and
  recompiles it (fine for typical PLC file sizes; not incremental).
- Parser errors are reported at line 1, col 1 — that coarse position comes
  straight from mt-stc/iecst, the server maps it faithfully rather than
  guessing better.
- Field access is name-based: `m.speed` resolves to a declaration *named*
  `speed`; the server does not verify that `m`'s declared type actually has
  that field.
- Only the first POU per file is modeled (matches the mt-stc project model of
  one POU per `.st` file); additional POUs in one file are visible only
  through the fallback scanner.
- Workspace scanning happens once at `initialize`; files added or deleted on
  disk afterwards are not picked up (no `workspace/didChangeWatchedFiles`
  support).
- A file containing only a `TYPE` block still gets mt-stc's parser error
  diagnostic ("expected PROGRAM..."), because `mt_stc::compile` requires a
  POU per file. Mirroring the compiler means showing the same diagnostic.
