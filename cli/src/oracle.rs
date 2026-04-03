use forgen_api::{
    query::{SemanticHandle, SemanticQuery, SemanticResult},
    LazyValue, LetBinding, TextRange,
};
use ra_ap_hir::{DisplayTarget, HirDisplay, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_ide_db::EditionedFileId;
use ra_ap_syntax::{ast, ast::AstNode, ast::HasName};
use ra_ap_vfs::Vfs;
use std::collections::HashMap;
use std::sync::Arc;

/// Wraps rust-analyzer handles behind raw pointers so the oracle can be
/// stored in an `Arc` and passed through lazy closures.
///
/// # Safety
///
/// The pointers are only dereferenced while the `attach_db_allow_change`
/// scope is active in `run_plugins`, which encompasses the entire plugin
/// execution including `plugin.run()`.  The oracle is dropped before
/// `run_plugins` returns, so the referenced data always outlives it.
pub(crate) struct CliOracle {
    pub db: *const RootDatabase,
    pub vfs: *const Vfs,
    /// Maps workspace-relative path (forward slashes) → EditionedFileId.
    pub file_map: HashMap<String, EditionedFileId>,
    /// Absolute workspace root (forward slashes, no trailing `/`).
    pub root_norm: String,
    /// When true, emit `[oracle]` lines to stderr for every RA inference call.
    pub verbose: bool,
}

// SAFETY: CliOracle is only used inside the `attach_db_allow_change` scope,
// which is single-threaded in our usage.  The raw pointers are valid for the
// lifetime of that scope.
unsafe impl Send for CliOracle {}
unsafe impl Sync for CliOracle {}

impl CliOracle {
    /// Wrap `self` in a `SemanticHandle` that plugins can use.
    pub fn into_handle(self: Arc<Self>) -> SemanticHandle {
        SemanticHandle {
            oracle: Arc::new(move |q| self.dispatch(q)),
        }
    }

    fn dispatch(&self, q: SemanticQuery) -> SemanticResult {
        let db = unsafe { &*self.db };
        let sema = Semantics::new(db);

        match q {
            SemanticQuery::InferTypeAt { file, range } => {
                let result = self
                    .file_map
                    .get(&file)
                    .and_then(|&eid| infer_type_at_range(&sema, db, eid, range, &file, self.verbose));
                SemanticResult::InferredType(result)
            }

            SemanticQuery::LetBindings { file } => {
                let result = self
                    .file_map
                    .get(&file)
                    .map(|&eid| self.compute_let_bindings_all(&sema, db, eid, &file))
                    .unwrap_or_default();
                SemanticResult::LetBindings(result)
            }

            SemanticQuery::LetBindingsInScope { file, scope } => {
                let result = self
                    .file_map
                    .get(&file)
                    .map(|&eid| self.compute_let_bindings_in_scope(&sema, db, eid, scope, &file))
                    .unwrap_or_default();
                SemanticResult::LetBindings(result)
            }

            SemanticQuery::ResolveItemAt { .. } => SemanticResult::ResolvedPath(None),

            SemanticQuery::TraitImplementors { .. } => {
                // Workspace-wide trait-impl search is left as a future extension.
                SemanticResult::Impls(vec![])
            }

            _ => SemanticResult::Unsupported,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn compute_let_bindings_all(
        &self,
        sema: &Semantics<RootDatabase>,
        db: &RootDatabase,
        eid: EditionedFileId,
        file_path: &str,
    ) -> Vec<LetBinding> {
        let bindings = extract_let_bindings_syntax(sema, eid);

        // For the eager LetBindings query, run inference for all unannotated
        // bindings immediately.
        bindings
            .into_iter()
            .map(|b| {
                let inferred_type = if b.explicit_type.is_some() {
                    LazyValue::from_value(None)
                } else if let Some(init_range) = b.initializer_range {
                    let ty = infer_type_at_range(sema, db, eid, init_range, file_path, self.verbose);
                    LazyValue::from_value(ty)
                } else {
                    LazyValue::from_value(None)
                };
                LetBinding { inferred_type, ..b }
            })
            .collect()
    }

    fn compute_let_bindings_in_scope(
        &self,
        sema: &Semantics<RootDatabase>,
        db: &RootDatabase,
        eid: EditionedFileId,
        scope: TextRange,
        file_path: &str,
    ) -> Vec<LetBinding> {
        let all = self.compute_let_bindings_all(sema, db, eid, file_path);
        all.into_iter()
            .filter(|b| range_contains(scope, b.range))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Free helpers (no self needed)
// ---------------------------------------------------------------------------

/// Infer the type of the expression whose byte range matches `range`.
/// `range` should be the `initializer_range` stored in `LetBinding`.
///
/// When `verbose` is true, emits a debug line to stderr:
/// ```text
/// [oracle] 445-455 `p2.x - p1.x`  →  f64
/// ```
pub(crate) fn infer_type_at_range(
    sema: &Semantics<RootDatabase>,
    db: &RootDatabase,
    eid: EditionedFileId,
    range: TextRange,
    file_path: &str,
    verbose: bool,
) -> Option<String> {
    let parsed = sema.parse(eid);
    let syntax = parsed.syntax();
    // We'll need the raw source text for verbose tracing.
    let src = syntax.text().to_string();

    // Walk every LetStmt in the file looking for one whose initializer
    // has the exact byte range we were given.
    for node in syntax.descendants() {
        let Some(let_stmt) = ast::LetStmt::cast(node) else {
            continue;
        };
        let Some(init) = let_stmt.initializer() else {
            continue;
        };
        let init_range = to_api_range(init.syntax().text_range());
        if init_range == range {
            let type_info = sema.type_of_expr(&init)?;
            let scope = sema.scope(let_stmt.syntax())?;
            let ty_str = type_info
                .original
                .display(
                    db,
                    DisplayTarget::from_crate(db, scope.krate().into()),
                )
                .to_string();

            if verbose {
                // Extract the source snippet, normalise whitespace, cap length.
                let snippet = src
                    .get(range.start as usize..range.end as usize)
                    .unwrap_or("?")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let snippet = if snippet.len() > 60 {
                    format!("{}…", &snippet[..60])
                } else {
                    snippet
                };
                eprintln!(
                    "[oracle] {} {}-{} `{}`  \u{2192}  {}",
                    file_path, range.start, range.end, snippet, ty_str
                );
            }

            return Some(ty_str);
        }
    }
    None
}

/// Extract all `let` binding stubs via a pure syntax parse (no RA type queries).
/// Each binding gets `inferred_type = LazyValue::from_value(None)`; the caller
/// is responsible for replacing that field with a real lazy closure.
pub(crate) fn extract_let_bindings_syntax(
    sema: &Semantics<RootDatabase>,
    eid: EditionedFileId,
) -> Vec<LetBinding> {
    let parsed = sema.parse(eid);
    let syntax = parsed.syntax();
    extract_let_bindings_from_syntax(syntax)
}

/// Pure syntax-pass over a `SyntaxNode` — no RA type queries.
pub(crate) fn extract_let_bindings_from_syntax(
    syntax: &ra_ap_syntax::SyntaxNode,
) -> Vec<LetBinding> {
    let mut bindings = Vec::new();

    for node in syntax.descendants() {
        let Some(let_stmt) = ast::LetStmt::cast(node) else {
            continue;
        };
        let Some(pat) = let_stmt.pat() else { continue };

        let (name, is_mut) = match &pat {
            ast::Pat::IdentPat(ident_pat) => {
                let Some(n) = ident_pat.name() else { continue };
                (n.to_string(), ident_pat.mut_token().is_some())
            }
            _ => continue,
        };

        if name.is_empty() || name == "_" {
            continue;
        }

        let explicit_type = let_stmt
            .ty()
            .map(|ty| ty.syntax().text().to_string().trim().to_owned());

        let initializer_range = let_stmt
            .initializer()
            .map(|init| to_api_range(init.syntax().text_range()));

        bindings.push(LetBinding {
            name,
            explicit_type,
            range: to_api_range(let_stmt.syntax().text_range()),
            initializer_range,
            is_mut,
            inferred_type: LazyValue::from_value(None),
        });
    }

    bindings
}

#[inline]
pub(crate) fn to_api_range(r: ra_ap_syntax::TextRange) -> TextRange {
    TextRange {
        start: u32::from(r.start()),
        end: u32::from(r.end()),
    }
}

/// Returns `true` if `inner` is fully contained within `outer`.
fn range_contains(outer: TextRange, inner: TextRange) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}
