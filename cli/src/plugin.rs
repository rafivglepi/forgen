use ra_ap_syntax::{ast, AstNode};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// A text replacement (or insertion) at a byte-offset range in a source file.
///
/// Plugins return a list of these; forgen applies them to the source file.
/// The JSON shape mirrors the future dylib contract:
///   { "range": { "start": 312, "end": 347 }, "text": "safe_unwrap(x)" }
#[derive(Debug, Clone, Serialize)]
pub struct Replacement {
    pub range: TextRange,
    pub text: String,
}

/// A byte-offset range within a source file.
#[derive(Debug, Clone, Serialize)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl Replacement {
    /// Creates a zero-width insertion at `offset` (i.e. nothing is replaced, text is inserted).
    pub fn insert(offset: u32, text: String) -> Self {
        Self {
            range: TextRange {
                start: offset,
                end: offset,
            },
            text,
        }
    }
}

/// Context provided to a plugin for a single source file.
///
/// Exposes the syntax tree and pre-resolved type information so plugins
/// don't need to depend on rust-analyzer directly. When dylib plugins land
/// this struct (or a stable ABI-friendly equivalent) will be what forgen
/// hands to each plugin.
pub struct FileContext {
    /// Absolute path to the source file.
    #[allow(dead_code)]
    pub path: PathBuf,
    /// Raw source text of the file (same bytes that were parsed).
    pub source: String,
    /// Parsed syntax tree of the file.
    pub syntax: ast::SourceFile,
    /// Pre-computed inferred types for `let` patterns that carry no explicit
    /// type annotation, keyed by (start_offset, end_offset) of the pattern node.
    pub(crate) pat_types: HashMap<(u32, u32), String>,
}

impl FileContext {
    pub fn new(
        path: PathBuf,
        source: String,
        syntax: ast::SourceFile,
        pat_types: HashMap<(u32, u32), String>,
    ) -> Self {
        Self {
            path,
            source,
            syntax,
            pat_types,
        }
    }

    /// Returns the inferred type of `pat` as a string, or `None` if it could
    /// not be resolved (e.g. the pattern has no initialiser or inference
    /// failed).
    ///
    /// Explicit type annotations are intentionally *not* included here; the
    /// plugin can read those directly from the syntax tree via
    /// `ast::LetStmt::ty()`.
    pub fn type_of_pat(&self, pat: &ast::Pat) -> Option<&str> {
        let range = pat.syntax().text_range();
        let key = (u32::from(range.start()), u32::from(range.end()));
        self.pat_types.get(&key).map(String::as_str)
    }
}

/// A plugin that can inspect and transform source files.
///
/// Implementing this trait is all that is needed to write a forgen plugin.
/// Each `run` call receives a read-only view of one source file and returns
/// a (possibly empty) list of replacements to apply.
pub trait Plugin: Send + Sync {
    /// Human-readable name used in log output.
    #[allow(dead_code)]
    fn name(&self) -> &str;

    /// Analyse `ctx` and return any replacements to apply.
    ///
    /// Replacements are applied in reverse offset order by the runner, so
    /// plugins do not need to account for position shifts caused by earlier
    /// insertions.
    fn run(&self, ctx: &FileContext) -> Vec<Replacement>;
}
