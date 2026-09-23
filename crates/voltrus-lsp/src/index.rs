//! Per-document symbol tables and the workspace-wide index backing
//! hover / definition / completion.
//!
//! Indexing strategy (honest summary):
//! - Primary path: the `iecst` AST directly (`parse_type_block` for
//!   TYPE/STRUCT/ENUM declarations, `parse_pou` for POUs and their VAR
//!   blocks). This is the same parser `mt-stc` uses, so spans agree.
//! - Fallback path: while a program is mid-edit, `parse_pou` fails on almost
//!   every keystroke. When AST parsing fails, a small line-oriented
//!   declaration scanner still extracts POU headers, `TYPE name` lines, and
//!   `name : type;` lines inside VAR blocks, so hover/definition keep working
//!   on broken sources. Scanned scopes are best-effort (a POU's scope extends
//!   to the next header or EOF rather than its true END_ keyword).
//!
//! Identifier resolution is case-insensitive (IEC 61131-3 identifiers are
//! case-insensitive), exact-case matches preferred. Owner-scoped symbols
//! (variables declared inside a POU or TYPE) only resolve when the cursor is
//! inside that owner's span in the same file. Cross-file, any declaration
//! may be referenced (POUs, types, VAR_GLOBALs, FB fields like `m.speed` —
//! no type inference; field resolution is name-based).

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use lsp_types::Url;

use crate::convert::LineIndex;

// ---------------------------------------------------------------------------
// Symbols
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolKind {
    Program,
    Function,
    FunctionBlock,
    StructType,
    EnumType,
    AliasType,
    SubrangeType,
    ArrayType,
    Var,
    VarInput,
    VarOutput,
    VarInOut,
    VarTemp,
    VarGlobal,
    VarExternal,
    StructField,
    EnumValue,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Program => "PROGRAM",
            SymbolKind::Function => "FUNCTION",
            SymbolKind::FunctionBlock => "FUNCTION_BLOCK",
            SymbolKind::StructType => "STRUCT type",
            SymbolKind::EnumType => "ENUM type",
            SymbolKind::AliasType => "TYPE alias",
            SymbolKind::SubrangeType => "Subrange type",
            SymbolKind::ArrayType => "ARRAY type",
            SymbolKind::Var => "VAR",
            SymbolKind::VarInput => "VAR_INPUT",
            SymbolKind::VarOutput => "VAR_OUTPUT",
            SymbolKind::VarInOut => "VAR_IN_OUT",
            SymbolKind::VarTemp => "VAR_TEMP",
            SymbolKind::VarGlobal => "VAR_GLOBAL",
            SymbolKind::VarExternal => "VAR_EXTERNAL",
            SymbolKind::StructField => "STRUCT field",
            SymbolKind::EnumValue => "ENUM value",
        }
    }

}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Rendered declared type (vars, fields, aliases), when known.
    pub type_name: Option<String>,
    /// Enclosing POU or TYPE name for nested declarations.
    pub owner: Option<String>,
    /// LSP range covering the identifier itself.
    pub name_range: lsp_types::Range,
    /// 1-based source line of the declaration (mt-stc's line convention).
    pub decl_line: u32,
}

impl Symbol {
    fn completion_kind(&self) -> lsp_types::CompletionItemKind {
        use lsp_types::CompletionItemKind as K;
        match self.kind {
            SymbolKind::Program | SymbolKind::Function | SymbolKind::FunctionBlock => K::FUNCTION,
            SymbolKind::Var
            | SymbolKind::VarInput
            | SymbolKind::VarOutput
            | SymbolKind::VarInOut
            | SymbolKind::VarTemp
            | SymbolKind::VarGlobal
            | SymbolKind::VarExternal => K::VARIABLE,
            SymbolKind::StructType
            | SymbolKind::AliasType
            | SymbolKind::SubrangeType
            | SymbolKind::ArrayType => K::STRUCT,
            SymbolKind::EnumType => K::ENUM,
            SymbolKind::StructField => K::FIELD,
            SymbolKind::EnumValue => K::ENUM_MEMBER,
        }
    }
}

/// Span of an owner (POU / TYPE) used to scope nested symbol lookups.
#[derive(Clone, Debug)]
struct Scope {
    name: String,
    range: lsp_types::Range,
}

#[derive(Debug)]
struct DocumentEntry {
    text: String,
    symbols: Vec<Symbol>,
    scopes: Vec<Scope>,
    /// True while the client has the document open (overrides the disk copy).
    is_open: bool,
}

/// Where a resolved symbol declaration lives.
#[derive(Clone, Debug)]
pub struct Hit {
    pub uri: Url,
    pub symbol: Symbol,
}

// ---------------------------------------------------------------------------
// Workspace index
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct WorkspaceIndex {
    docs: HashMap<Url, DocumentEntry>,
    /// Workspace folders supplied by the client; siblings are scanned once.
    #[cfg(not(target_arch = "wasm32"))]
    folders: Vec<PathBuf>,
}

impl WorkspaceIndex {
    /// Scan workspace folders for sibling `*.st` files. Native only: the wasm
    /// build has no filesystem, every document arrives via open/change.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_workspace(&mut self, folders: Vec<PathBuf>) {
        self.folders = folders;
        for dir in self.folders.clone() {
            for path in st_files_in(&dir) {
                let Ok(uri) = Url::from_file_path(&path) else {
                    continue;
                };
                if self.docs.contains_key(&uri) {
                    continue; // an open document overrides the disk copy
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    self.docs.insert(uri, index_text(&text, false));
                }
            }
        }
    }

    /// wasm build: no filesystem; documents arrive via open/change.
    #[cfg(target_arch = "wasm32")]
    pub fn set_workspace(&mut self, _folders: Vec<std::path::PathBuf>) {}

    pub fn open(&mut self, uri: Url, text: String) {
        self.docs.insert(uri, index_text(&text, true));
    }

    pub fn change(&mut self, uri: &Url, text: String) {
        self.docs.insert(uri.clone(), index_text(&text, true));
    }

    /// The client closed the document: clear liveness; the last-known symbol
    /// table stays (the file normally still exists on disk).
    pub fn close(&mut self, uri: &Url) {
        if let Some(entry) = self.docs.get_mut(uri) {
            entry.is_open = false;
        }
    }

    pub fn text(&self, uri: &Url) -> Option<&str> {
        self.docs.get(uri).map(|e| e.text.as_str())
    }

    /// Identifier under the cursor as (name, range covering it).
    pub fn word_at(
        &self,
        uri: &Url,
        position: lsp_types::Position,
    ) -> Option<(String, lsp_types::Range)> {
        let text = self.text(uri)?;
        let line = text.split('\n').nth(position.line as usize)?;
        let line_chars: Vec<char> = line.chars().collect();

        // UTF-16 column -> char index.
        let mut utf16 = 0usize;
        let mut start_char = 0usize;
        let mut placed = false;
        for (i, ch) in line_chars.iter().enumerate() {
            if utf16 >= position.character as usize {
                start_char = i;
                placed = true;
                break;
            }
            utf16 += ch.len_utf16();
            start_char = i + 1;
        }
        let _ = placed; // start_char == line_chars.len() when not placed
        if start_char >= line_chars.len() || !is_ident_char(line_chars[start_char]) {
            return None;
        }
        let mut lo = start_char;
        let mut hi = start_char + 1;
        while lo > 0 && is_ident_char(line_chars[lo - 1]) {
            lo -= 1;
        }
        while hi < line_chars.len() && is_ident_char(line_chars[hi]) {
            hi += 1;
        }
        let name: String = line_chars[lo..hi].iter().collect();
        let start16: usize = line_chars[..lo].iter().map(|c| c.len_utf16()).sum();
        let end16: usize = line_chars[..hi].iter().map(|c| c.len_utf16()).sum();
        Some((
            name,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: position.line,
                    character: start16 as u32,
                },
                end: lsp_types::Position {
                    line: position.line,
                    character: end16 as u32,
                },
            },
        ))
    }

    /// Resolve the identifier under `position` to a declaration.
    pub fn resolve(&self, uri: &Url, position: lsp_types::Position) -> Option<Hit> {
        let (word, _) = self.word_at(uri, position)?;
        let exact = self.resolve_named(uri, position, &word, true);
        exact.or_else(|| self.resolve_named(uri, position, &word, false))
    }

    fn resolve_named(
        &self,
        uri: &Url,
        position: lsp_types::Position,
        word: &str,
        exact_case: bool,
    ) -> Option<Hit> {
        let matches = |name: &str| {
            if exact_case {
                name == word
            } else {
                name.eq_ignore_ascii_case(word)
            }
        };

        // Same document first: owner-scoped symbols match inside their scope.
        if let Some(entry) = self.docs.get(uri) {
            for s in &entry.symbols {
                if matches(&s.name) && symbol_in_scope(s, &entry.scopes, position) {
                    return Some(Hit {
                        uri: uri.clone(),
                        symbol: s.clone(),
                    });
                }
            }
        }

        // Other documents: any declaration kind may be referenced cross-file
        // (POUs, types, VAR_GLOBALs, and FB fields like `m.speed` — the
        // canonical PLC pattern). Same-name collisions resolve to the
        // URI-sorted first match. Deterministic order: open documents first,
        // then URI string order.
        let mut others: Vec<&Url> = self.docs.keys().filter(|u| *u != uri).collect();
        others.sort_by_key(|u| (!self.docs[*u].is_open, u.as_str().to_string()));
        for other in others {
            let entry = &self.docs[other];
            for s in &entry.symbols {
                if matches(&s.name) {
                    return Some(Hit {
                        uri: other.clone(),
                        symbol: s.clone(),
                    });
                }
            }
        }
        None
    }

    /// All indexed declarations plus stdlib functions and IEC keywords.
    pub fn completions(&self) -> Vec<lsp_types::CompletionItem> {
        use lsp_types::CompletionItemKind as K;

        // Own declarations, deduplicated by (name, kind). Open documents
        // first, then URI order — same determinism rule as resolve.
        let mut uris: Vec<&Url> = self.docs.keys().collect();
        uris.sort_by_key(|u| (!self.docs[*u].is_open, u.as_str().to_string()));
        let mut seen: Vec<(String, SymbolKind)> = Vec::new();
        let mut items = Vec::new();
        for uri in uris {
            for s in &self.docs[uri].symbols {
                let key = (s.name.clone(), s.kind);
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                items.push(lsp_types::CompletionItem {
                    label: s.name.clone(),
                    kind: Some(s.completion_kind()),
                    detail: Some(completion_detail(s)),
                    ..Default::default()
                });
            }
        }

        for f in mt_stc::stdlib::standard_functions() {
            items.push(lsp_types::CompletionItem {
                label: f.name.to_string(),
                kind: Some(K::FUNCTION),
                detail: Some(format!("{} · {} argument(s)", f.description, f.arg_count)),
                ..Default::default()
            });
        }

        for kw in IEC_KEYWORDS {
            items.push(lsp_types::CompletionItem {
                label: (*kw).to_string(),
                kind: Some(K::KEYWORD),
                ..Default::default()
            });
        }
        items
    }
}

fn completion_detail(s: &Symbol) -> String {
    match (&s.type_name, &s.owner) {
        (Some(t), Some(o)) => format!("{} {} : {} · in {}", s.kind.label(), s.name, t, o),
        (Some(t), None) => format!("{} {} : {}", s.kind.label(), s.name, t),
        (None, Some(o)) => format!("{} {} · in {}", s.kind.label(), s.name, o),
        (None, None) => format!("{} {}", s.kind.label(), s.name),
    }
}

fn symbol_in_scope(s: &Symbol, scopes: &[Scope], position: lsp_types::Position) -> bool {
    match &s.owner {
        None => true,
        Some(owner) => scopes
            .iter()
            .any(|sc| sc.name.eq_ignore_ascii_case(owner) && range_contains(&sc.range, position)),
    }
}

fn range_contains(r: &lsp_types::Range, p: lsp_types::Position) -> bool {
    r.start <= p && p <= r.end
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `*.st` files directly inside `dir` (flat project model — no recursion).
#[cfg(not(target_arch = "wasm32"))]
fn st_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("st")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Text indexing
// ---------------------------------------------------------------------------

fn index_text(text: &str, is_open: bool) -> DocumentEntry {
    let li = LineIndex::new(text);
    let mut symbols = Vec::new();
    let mut scopes = Vec::new();

    // 1. TYPE declarations (from the original text, like mt-stc does).
    //    parse_type_block stops after the FIRST block, so parse repeatedly,
    //    advancing past each block (spans end before the closing END_TYPE).
    //    Spans come back relative to the slice, so every use shifts them by
    //    the slice base.
    let mut base = 0usize;
    loop {
        let Ok(decls) = iecst::parse_type_block(&text[base..]) else {
            break;
        };
        if decls.is_empty() {
            break;
        }
        for t in &decls {
            process_type_decl(t, base, text, &li, &mut symbols, &mut scopes);
        }
        // The parsed span ends BEFORE the block's closing END_TYPE; skip past
        // it so the next iteration sees the following TYPE block.
        match text[base..].to_uppercase().find("END_TYPE") {
            Some(pos) => base += pos + "END_TYPE".len(),
            None => break,
        }
    }

    // 2. POU + variable declarations (AST on TYPE-stripped source).
    let pou_source = blank_type_blocks(text);
    match iecst::parse_pou(&pou_source) {
        Ok(pou) => {
            symbols.push(Symbol {
                name: pou.name.clone(),
                kind: pou_kind(pou.kind),
                type_name: pou.return_type.as_ref().map(render_type),
                owner: None,
                name_range: name_range(pou.span, &pou.name, text, &li),
                decl_line: li.line_of(pou.span.start),
            });
            scopes.push(Scope {
                name: pou.name.clone(),
                range: li.range_from_span(pou.span),
            });
            for block in &pou.var_blocks {
                let kind = var_block_kind(block.kind);
                for decl in &block.vars {
                    symbols.push(var_symbol(decl, 0, kind, Some(pou.name.clone()), text, &li));
                }
            }
        }
        Err(_) => {
            // AST unavailable (mid-edit source): documented fallback scanner.
            scan_declarations(text, &mut symbols, &mut scopes);
        }
    }

    DocumentEntry {
        text: text.to_string(),
        symbols,
        scopes,
        is_open,
    }
}

/// Index one TYPE declaration whose spans are relative to `text[shift..]`.
fn process_type_decl(
    t: &iecst::TypeDecl,
    shift: usize,
    text: &str,
    li: &LineIndex,
    symbols: &mut Vec<Symbol>,
    scopes: &mut Vec<Scope>,
) {
    let type_symbol = |kind| Symbol {
        name: t.name.clone(),
        kind,
        type_name: None,
        owner: None,
        name_range: name_range(shift_span(t.span, shift), &t.name, text, li),
        decl_line: li.line_of(t.span.start + shift),
    };
    match &t.definition {
        iecst::TypeDef::Struct { fields } => {
            for f in fields {
                symbols.push(var_symbol(
                    f,
                    shift,
                    SymbolKind::StructField,
                    Some(t.name.clone()),
                    text,
                    li,
                ));
            }
            symbols.push(type_symbol(SymbolKind::StructType));
        }
        iecst::TypeDef::Enum { values } => {
            for v in values {
                symbols.push(Symbol {
                    name: v.name.clone(),
                    kind: SymbolKind::EnumValue,
                    type_name: Some(t.name.clone()),
                    owner: Some(t.name.clone()),
                    name_range: name_range(shift_span(v.span, shift), &v.name, text, li),
                    decl_line: li.line_of(v.span.start + shift),
                });
            }
            symbols.push(type_symbol(SymbolKind::EnumType));
        }
        iecst::TypeDef::Alias(spec) => {
            symbols.push(Symbol {
                type_name: Some(render_type(spec)),
                ..type_symbol(SymbolKind::AliasType)
            });
        }
        iecst::TypeDef::Subrange { base, .. } => {
            symbols.push(Symbol {
                type_name: Some(base.clone()),
                ..type_symbol(SymbolKind::SubrangeType)
            });
        }
        iecst::TypeDef::Array { .. } => {
            symbols.push(type_symbol(SymbolKind::ArrayType));
        }
    }
    scopes.push(Scope {
        name: t.name.clone(),
        range: li.range_from_span(shift_span(t.span, shift)),
    });
}

fn shift_span(span: iecst::Span, shift: usize) -> iecst::Span {
    iecst::Span::new(span.start + shift, span.end + shift)
}

/// Symbol for one variable declaration whose span is relative to
/// `source[shift..]`.
fn var_symbol(
    decl: &iecst::VarDecl,
    shift: usize,
    kind: SymbolKind,
    owner: Option<String>,
    source: &str,
    li: &LineIndex,
) -> Symbol {
    Symbol {
        name: decl.name.clone(),
        kind,
        type_name: Some(render_type(&decl.var_type)),
        owner,
        name_range: name_range(shift_span(decl.span, shift), &decl.name, source, li),
        decl_line: li.line_of(decl.span.start + shift),
    }
}

/// LSP range covering the identifier of a declaration. The declaration span
/// usually starts at the header keyword (`TYPE MotorConfig`, `VAR_INPUT
/// speed`), so the identifier is located by its first case-insensitive
/// occurrence inside the span (ASCII ST sources; same trick as everywhere
/// else in this module).
fn name_range(
    span: iecst::Span,
    name: &str,
    source: &str,
    li: &LineIndex,
) -> lsp_types::Range {
    let start = span.start.min(source.len());
    let end = span.end.min(source.len()).max(start);
    let rel = source[start..end]
        .to_uppercase()
        .find(&name.to_uppercase())
        .map_or(0, |b| start + b);
    lsp_types::Range {
        start: li.position_from_offset(rel),
        end: li.position_from_offset(rel + name.len()),
    }
}

fn pou_kind(k: iecst::PouKind) -> SymbolKind {
    match k {
        iecst::PouKind::Program => SymbolKind::Program,
        iecst::PouKind::Function => SymbolKind::Function,
        iecst::PouKind::FunctionBlock => SymbolKind::FunctionBlock,
    }
}

fn var_block_kind(k: iecst::VarBlockKind) -> SymbolKind {
    match k {
        iecst::VarBlockKind::Var => SymbolKind::Var,
        iecst::VarBlockKind::VarInput => SymbolKind::VarInput,
        iecst::VarBlockKind::VarOutput => SymbolKind::VarOutput,
        iecst::VarBlockKind::VarInOut => SymbolKind::VarInOut,
        iecst::VarBlockKind::VarTemp => SymbolKind::VarTemp,
        iecst::VarBlockKind::VarGlobal => SymbolKind::VarGlobal,
        iecst::VarBlockKind::VarExternal => SymbolKind::VarExternal,
    }
}

/// Render an `iecst::TypeSpec` as the text a user would recognize.
pub fn render_type(spec: &iecst::TypeSpec) -> String {
    match &spec.kind {
        iecst::TypeKind::Simple(name) => name.clone(),
        iecst::TypeKind::Array { element, .. } => format!("ARRAY[..] OF {}", render_type(element)),
        iecst::TypeKind::String { length: Some(n) } => format!("STRING[{n}]"),
        iecst::TypeKind::String { length: None } => "STRING".to_string(),
        iecst::TypeKind::WString { length: Some(n) } => format!("WSTRING[{n}]"),
        iecst::TypeKind::WString { length: None } => "WSTRING".to_string(),
        iecst::TypeKind::Subrange { base, .. } => format!("{base} (subrange)"),
    }
}

/// Blank `TYPE ... END_TYPE` blocks (spaces, newlines kept) so the POU parse
/// happens on text whose byte offsets are identical to the original source.
/// mt-stc strips TYPE blocks by removal, which shifts offsets; we must not.
fn blank_type_blocks(source: &str) -> String {
    let upper = source.to_uppercase();
    let mut out: Vec<char> = source.chars().collect();
    let upper_chars: Vec<char> = upper.chars().collect();
    let mut in_type = false;

    let is_word_before =
        |i: usize| i > 0 && upper_chars[i - 1].is_alphanumeric();
    let is_word_after = |i: usize, n: usize| {
        i + n < upper_chars.len() && upper_chars[i + n].is_alphanumeric()
    };

    let mut i = 0;
    while i < out.len() {
        if !in_type
            && i + 4 <= upper_chars.len()
            && upper_chars[i..i + 4] == ['T', 'Y', 'P', 'E']
            && !is_word_before(i)
            && !is_word_after(i, 4)
        {
            in_type = true;
            blank(&mut out, i, 4);
            i += 4;
            continue;
        }
        if in_type
            && i + 8 <= upper_chars.len()
            && upper_chars[i..i + 8] == ['E', 'N', 'D', '_', 'T', 'Y', 'P', 'E']
            && !is_word_before(i)
            && !is_word_after(i, 8)
        {
            in_type = false;
            blank(&mut out, i, 8);
            i += 8;
            continue;
        }
        if in_type && out[i] != '\n' {
            out[i] = ' ';
        }
        i += 1;
    }
    out.into_iter().collect()
}

fn blank(out: &mut [char], start: usize, len: usize) {
    for c in &mut out[start..start + len] {
        *c = ' ';
    }
}

// ---------------------------------------------------------------------------
// Fallback declaration scanner (best-effort, broken-source mode)
// ---------------------------------------------------------------------------

/// Longest prefixes first so `VAR_INPUT` is not eaten by `VAR`.
const VAR_BLOCK_PREFIXES: [&str; 7] = [
    "VAR_INPUT",
    "VAR_OUTPUT",
    "VAR_IN_OUT",
    "VAR_EXTERNAL",
    "VAR_TEMP",
    "VAR_GLOBAL",
    "VAR",
];

fn scan_declarations(text: &str, symbols: &mut Vec<Symbol>, scopes: &mut Vec<Scope>) {
    let mut current_scope: Option<(String, lsp_types::Position)> = None;
    let mut var_kind: Option<SymbolKind> = None;

    for (line_no, raw) in text.split('\n').enumerate() {
        let line_no = line_no as u32;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("(*") {
            continue;
        }
        let upper = trimmed.to_uppercase();

        // Scope/block closers.
        if upper.starts_with("END_PROGRAM")
            || upper.starts_with("END_FUNCTION_BLOCK")
            || upper.starts_with("END_FUNCTION")
            || upper.starts_with("END_TYPE")
        {
            close_scope(&mut current_scope, scopes, line_no);
            var_kind = None;
            continue;
        }
        if upper.starts_with("END_VAR") {
            var_kind = None;
            continue;
        }

        // POU headers. The name is cut from the original line (upper is only
        // a case-folded view; its char positions match trimmed).
        let mut header: Option<(String, SymbolKind)> = None;
        for (prefix, kind) in [
            ("FUNCTION_BLOCK", SymbolKind::FunctionBlock),
            ("PROGRAM", SymbolKind::Program),
            ("FUNCTION", SymbolKind::Function),
        ] {
            if let Some(rest) = upper.strip_prefix(prefix) {
                let orig = &trimmed[trimmed.len() - rest.len()..];
                let word: String =
                    orig.trim_start().chars().take_while(|c| is_ident_char(*c)).collect();
                if !word.is_empty() {
                    header = Some((word, kind));
                }
                break;
            }
        }
        if let Some((name, kind)) = header {
            close_scope(&mut current_scope, scopes, line_no);
            let col = col_of_word(trimmed, &name) as u32;
            symbols.push(Symbol {
                name: name.clone(),
                kind,
                type_name: None,
                owner: None,
                name_range: lsp_types::Range {
                    start: lsp_types::Position { line: line_no, character: col },
                    end: lsp_types::Position { line: line_no, character: col + name.len() as u32 },
                },
                decl_line: line_no + 1,
            });
            current_scope = Some((
                name,
                lsp_types::Position {
                    line: line_no,
                    character: 0,
                },
            ));
            var_kind = None;
            continue;
        }

        // `TYPE MotorConfig : ...` headers.
        if upper == "TYPE" || upper.starts_with("TYPE ") {
            var_kind = None;
            let orig = &trimmed[trimmed.len()
                - upper.strip_prefix("TYPE").unwrap_or("").len()..];
            let word: String =
                orig.trim_start().chars().take_while(|c| is_ident_char(*c)).collect();
            if !word.is_empty() {
                let col = col_of_word(trimmed, &word) as u32;
                symbols.push(Symbol {
                    name: word.clone(),
                    kind: SymbolKind::StructType, // body unknown to the scanner
                    type_name: None,
                    owner: None,
                    name_range: lsp_types::Range {
                        start: lsp_types::Position { line: line_no, character: col },
                        end: lsp_types::Position { line: line_no, character: col + word.len() as u32 },
                    },
                    decl_line: line_no + 1,
                });
            }
            continue;
        }

        // VAR block openers.
        for prefix in VAR_BLOCK_PREFIXES {
            if upper == *prefix || upper.starts_with(&format!("{prefix} ")) {
                var_kind = Some(var_block_kind_str(prefix));
                break;
            }
        }
        let Some(kind) = var_kind else {
            continue;
        };

        // Declaration line inside a VAR block: `name : type...;`
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let raw_name = trimmed[..colon].trim();
        let name_part = raw_name.split_whitespace().last().unwrap_or(raw_name);
        if name_part.is_empty() || !name_part.chars().all(is_ident_char) {
            continue;
        }
        let type_part = trimmed[colon + 1..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let col = col_of_word(trimmed, name_part) as u32;
        let decl_start = lsp_types::Position { line: line_no, character: col };
        if symbols.iter().any(|s| {
            s.name.eq_ignore_ascii_case(name_part) && s.name_range.start == decl_start
        }) {
            continue; // AST already indexed this declaration
        }
        symbols.push(Symbol {
            name: name_part.to_string(),
            kind,
            type_name: (!type_part.is_empty()).then_some(type_part),
            owner: current_scope.as_ref().map(|(n, _)| n.clone()),
            name_range: lsp_types::Range {
                start: lsp_types::Position { line: line_no, character: col },
                end: lsp_types::Position { line: line_no, character: col + name_part.len() as u32 },
            },
            decl_line: line_no + 1,
        });
    }
    close_scope(&mut current_scope, scopes, u32::MAX);
}

fn var_block_kind_str(prefix: &str) -> SymbolKind {
    match prefix {
        "VAR_INPUT" => SymbolKind::VarInput,
        "VAR_OUTPUT" => SymbolKind::VarOutput,
        "VAR_IN_OUT" => SymbolKind::VarInOut,
        "VAR_TEMP" => SymbolKind::VarTemp,
        "VAR_GLOBAL" => SymbolKind::VarGlobal,
        "VAR_EXTERNAL" => SymbolKind::VarExternal,
        _ => SymbolKind::Var,
    }
}

fn close_scope(
    current: &mut Option<(String, lsp_types::Position)>,
    scopes: &mut Vec<Scope>,
    end_line: u32,
) {
    if let Some((name, start)) = current.take() {
        scopes.push(Scope {
            name,
            range: lsp_types::Range {
                start,
                end: lsp_types::Position {
                    line: end_line,
                    character: 0,
                },
            },
        });
    }
}

/// Column (chars) of `word`'s first occurrence in `line`, case-insensitive.
fn col_of_word(line: &str, word: &str) -> usize {
    if word.is_empty() {
        return 0;
    }
    line.to_uppercase()
        .find(&word.to_uppercase())
        .map(|byte_idx| line[..byte_idx].chars().count())
        .unwrap_or(0)
}

/// IEC 61131-3 keywords offered in completion.
pub const IEC_KEYWORDS: &[&str] = &[
    "PROGRAM", "END_PROGRAM", "FUNCTION", "END_FUNCTION", "FUNCTION_BLOCK",
    "END_FUNCTION_BLOCK", "TYPE", "END_TYPE", "STRUCT", "END_STRUCT", "VAR",
    "VAR_INPUT", "VAR_OUTPUT", "VAR_IN_OUT", "VAR_TEMP", "VAR_GLOBAL",
    "VAR_EXTERNAL", "END_VAR", "IF", "THEN", "ELSIF", "ELSE", "END_IF", "CASE",
    "OF", "END_CASE", "FOR", "TO", "BY", "DO", "END_FOR", "WHILE", "END_WHILE",
    "REPEAT", "UNTIL", "END_REPEAT", "EXIT", "RETURN", "AND", "OR", "XOR",
    "NOT", "MOD", "TRUE", "FALSE", "ARRAY", "AT", "RETAIN", "NON_RETAIN",
    "CONSTANT", "BOOL", "BYTE", "WORD", "DWORD", "SINT", "INT", "DINT", "USINT",
    "UINT", "UDINT", "REAL", "LREAL", "STRING", "WSTRING", "TIME", "DATE",
];

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_ST: &str = "\
TYPE MotorConfig :
STRUCT
    ratio : INT;
END_STRUCT
END_TYPE

PROGRAM Main
VAR
    cfg : MotorConfig;
END_VAR
VAR_INPUT
    setpoint : REAL;
END_VAR
    cfg.ratio := 1;
END_PROGRAM
";

    #[test]
    fn indexes_types_pous_and_vars() {
        let doc = index_text(MAIN_ST, true);
        let find = |name: &str| doc.symbols.iter().find(|s| s.name == name).unwrap();

        let cfg_type = find("MotorConfig");
        assert_eq!(cfg_type.kind, SymbolKind::StructType);
        assert_eq!(cfg_type.decl_line, 1);

        let ratio = find("ratio");
        assert_eq!(ratio.kind, SymbolKind::StructField);
        assert_eq!(ratio.type_name.as_deref(), Some("INT"));
        assert_eq!(ratio.owner.as_deref(), Some("MotorConfig"));

        let main = find("Main");
        assert_eq!(main.kind, SymbolKind::Program);

        let cfg_var = find("cfg");
        assert_eq!(cfg_var.kind, SymbolKind::Var);
        assert_eq!(cfg_var.type_name.as_deref(), Some("MotorConfig"));
        assert_eq!(cfg_var.owner.as_deref(), Some("Main"));
        assert_eq!(cfg_var.name_range.start.line, 8);

        let sp = find("setpoint");
        assert_eq!(sp.kind, SymbolKind::VarInput);
        assert_eq!(sp.type_name.as_deref(), Some("REAL"));
        assert_eq!(sp.decl_line, 12);
    }

    #[test]
    fn broken_source_falls_back_to_scanner() {
        let broken = "PROGRAM Main\nVAR\n    x : INT\nEND_VAR\n    x := ;\nEND_PROGRAM";
        assert!(iecst::parse_pou(&blank_type_blocks(broken)).is_err());
        let doc = index_text(broken, true);
        let find = |name: &str| doc.symbols.iter().find(|s| s.name == name).unwrap();
        assert_eq!(find("Main").kind, SymbolKind::Program);
        assert_eq!(find("x").kind, SymbolKind::Var);
        assert_eq!(find("x").type_name.as_deref(), Some("INT"));
        assert_eq!(find("x").owner.as_deref(), Some("Main"));
    }

    #[test]
    fn cross_file_definition_via_index() {
        let motor_st = "\
FUNCTION_BLOCK Motor
VAR_INPUT
    speed : INT;
END_VAR
END_FUNCTION_BLOCK
";
        let main_st = "\
PROGRAM Main
VAR
    m : Motor;
END_VAR
    m.speed := 100;
END_PROGRAM
";
        let mut idx = WorkspaceIndex::default();
        let motor_uri = Url::parse("file:///proj/motor.st").unwrap();
        let main_uri = Url::parse("file:///proj/main.st").unwrap();
        idx.open(motor_uri.clone(), motor_st.to_string());
        idx.open(main_uri.clone(), main_st.to_string());

        // Cursor inside `Motor` on main.st line 2 ("    m : Motor;").
        let hit = idx
            .resolve(&main_uri, lsp_types::Position { line: 2, character: 11 })
            .unwrap();
        assert_eq!(hit.uri, motor_uri);
        assert_eq!(hit.symbol.kind, SymbolKind::FunctionBlock);
        assert_eq!(hit.symbol.name_range.start.line, 0);

        // `speed` field use on line 4 resolves cross-file to the VAR_INPUT.
        let hit = idx
            .resolve(&main_uri, lsp_types::Position { line: 4, character: 7 })
            .unwrap();
        assert_eq!(hit.uri, motor_uri);
        assert_eq!(hit.symbol.kind, SymbolKind::VarInput);

        // Same-file variable resolves within its owner scope.
        let hit = idx
            .resolve(&main_uri, lsp_types::Position { line: 4, character: 4 })
            .unwrap();
        assert_eq!(hit.uri, main_uri);
        assert_eq!(hit.symbol.kind, SymbolKind::Var);
        // The `.` member-access operator carries no identifier -> None.
        assert!(idx.resolve(&main_uri, lsp_types::Position { line: 4, character: 5 }).is_none());
    }

    #[test]
    fn scope_keeps_same_named_fields_apart() {
        // One POU per file is the project model, so same-file scope
        // separation is exercised with two TYPE declarations instead.
        let text = "\
TYPE A :
STRUCT
    shared : INT;
END_STRUCT
END_TYPE

TYPE B :
STRUCT
    shared : BOOL;
END_STRUCT
END_TYPE
";
        let mut idx = WorkspaceIndex::default();
        let uri = Url::parse("file:///s.st").unwrap();
        idx.open(uri.clone(), text.to_string());
        // Cursor on A's field declaration (line 2) vs B's (line 8).
        let in_a = idx.resolve(&uri, lsp_types::Position { line: 2, character: 6 }).unwrap();
        assert_eq!(in_a.symbol.type_name.as_deref(), Some("INT"));
        let in_b = idx.resolve(&uri, lsp_types::Position { line: 8, character: 6 }).unwrap();
        assert_eq!(in_b.symbol.type_name.as_deref(), Some("BOOL"));
    }
    #[test]
    fn word_at_spans_identifier() {
        let mut idx = WorkspaceIndex::default();
        let uri = Url::parse("file:///w.st").unwrap();
        idx.open(uri.clone(), "    undefined_thing := 1;".to_string());
        let (word, range) = idx
            .word_at(&uri, lsp_types::Position { line: 0, character: 8 })
            .unwrap();
        assert_eq!(word, "undefined_thing");
        assert_eq!(range.start.character, 4);
        assert_eq!(range.end.character, 19);
    }

    #[test]
    fn completions_include_symbols_stdlib_and_keywords() {
        let mut idx = WorkspaceIndex::default();
        let uri = Url::parse("file:///c.st").unwrap();
        idx.open(
            uri,
            "PROGRAM Pump\nVAR\n    psi : REAL;\nEND_VAR\nEND_PROGRAM".to_string(),
        );
        let labels: Vec<String> = idx.completions().into_iter().map(|i| i.label).collect();
        assert!(labels.iter().any(|l| l == "Pump"));
        assert!(labels.iter().any(|l| l == "psi"));
        assert!(labels.iter().any(|l| l == "SQRT"));
        assert!(labels.iter().any(|l| l == "END_FOR"));
    }
}
