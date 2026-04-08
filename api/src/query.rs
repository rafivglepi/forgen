use crate::{ImplDef, LetBinding, TextRange};
use std::sync::Arc;

/// A semantic query issued by a plugin to the Forgen runtime.
///
/// The runtime dispatches these to rust-analyzer.
/// Plugins never import `ra_ap_*`.
#[non_exhaustive]
pub enum SemanticQuery {
    // ── File-scoped ──────────────────────────────────────────────────────
    /// All `let` bindings in the file, with `inferred_type` populated.
    /// (Prefer `file.let_bindings()` + `binding.ty()` for per-binding laziness.)
    LetBindings { file: String },

    /// Let bindings whose pattern range is contained within `scope`.
    /// Useful for "only bindings in this function body" — get the body range
    /// from `file.tree()` and pass it here.
    LetBindingsInScope { file: String, scope: TextRange },

    /// Infer the type of the expression at `range` (the initializer RHS).
    InferTypeAt { file: String, range: TextRange },

    /// Resolve the item at `range` to its fully-qualified path.
    /// E.g. a usage of `HashMap` resolves to `"std::collections::HashMap"`.
    ResolveItemAt { file: String, range: TextRange },

    // ── Workspace-scoped ─────────────────────────────────────────────────
    /// All `impl` blocks across the workspace that implement `trait_path`.
    /// `trait_path` is matched by suffix (e.g. `"Display"` matches
    /// `std::fmt::Display`).
    TraitImplementors { trait_path: String },
}

/// Result of a [`SemanticQuery`].
#[non_exhaustive]
pub enum SemanticResult {
    LetBindings(Vec<LetBinding>),
    InferredType(Option<String>),
    ResolvedPath(Option<String>),
    Impls(Vec<ImplDef>),
    /// Returned for unrecognised or unimplemented query variants.
    Unsupported,
}

/// A handle for issuing semantic (RA-backed) queries from within a plugin.
///
/// Obtained via `FileContext::semantics()` or `WorkspaceContext::semantics()`.
/// Returns `None` from those methods when no oracle is active (e.g. tests).
///
/// `SemanticHandle` is `Clone` — store it in closures or helper structs freely.
#[derive(Clone)]
pub struct SemanticHandle {
    pub oracle: Arc<dyn Fn(SemanticQuery) -> SemanticResult + Send + Sync>,
}

impl std::fmt::Debug for SemanticHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SemanticHandle(oracle)")
    }
}

impl SemanticHandle {
    /// Issue a raw query. Prefer the typed helpers below.
    pub fn query(&self, q: SemanticQuery) -> SemanticResult {
        (self.oracle)(q)
    }

    // ── File-scoped helpers ───────────────────────────────────────────────

    /// All `let` bindings in the file, with inferred types populated.
    ///
    /// For maximum laziness, prefer `file.let_bindings()` + `binding.ty()`.
    /// This method runs inference for every unannotated binding up front.
    pub fn let_bindings(&self, file: &str) -> Vec<LetBinding> {
        match self.query(SemanticQuery::LetBindings {
            file: file.to_owned(),
        }) {
            SemanticResult::LetBindings(v) => v,
            _ => vec![],
        }
    }

    /// Let bindings whose pattern falls inside `scope`.
    pub fn let_bindings_in(&self, file: &str, scope: TextRange) -> Vec<LetBinding> {
        match self.query(SemanticQuery::LetBindingsInScope {
            file: file.to_owned(),
            scope,
        }) {
            SemanticResult::LetBindings(v) => v,
            _ => vec![],
        }
    }

    /// Infer the type of the expression at `range`.
    pub fn infer_type_at(&self, file: &str, range: TextRange) -> Option<String> {
        match self.query(SemanticQuery::InferTypeAt {
            file: file.to_owned(),
            range,
        }) {
            SemanticResult::InferredType(t) => t,
            _ => None,
        }
    }

    /// Resolve the item at `range` to its fully-qualified path.
    pub fn resolve_item_at(&self, file: &str, range: TextRange) -> Option<String> {
        match self.query(SemanticQuery::ResolveItemAt {
            file: file.to_owned(),
            range,
        }) {
            SemanticResult::ResolvedPath(p) => p,
            _ => None,
        }
    }

    // ── Workspace-scoped helpers ──────────────────────────────────────────

    /// All impl blocks that implement `trait_path` across the workspace.
    pub fn trait_implementors(&self, trait_path: &str) -> Vec<ImplDef> {
        match self.query(SemanticQuery::TraitImplementors {
            trait_path: trait_path.to_owned(),
        }) {
            SemanticResult::Impls(v) => v,
            _ => vec![],
        }
    }
}
