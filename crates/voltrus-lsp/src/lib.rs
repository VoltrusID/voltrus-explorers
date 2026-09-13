//! voltrus-lsp: language server for Voltrus PLC Structured Text projects.
//!
//! Wraps `mt-stc` for headless compilation (diagnostics) and the `iecst` AST
//! for symbol indexing (hover, definition, completion).

pub mod convert;
pub mod index;
pub mod server;

pub use server::run_stdio;
