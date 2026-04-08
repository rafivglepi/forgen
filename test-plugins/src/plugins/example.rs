//! Inline example plugin for the `test-plugins` plugin suite.
//!
//! This module contains the same logic that used to live in the standalone
//! `example-plugin` crate, but is now embedded directly inside the plugin suite
//! crate.  No separate workspace member is needed.
//!
//! This plugin demonstrates the full plugin API:
//!   - Reading workspace manifest data (package names, editions, dependencies)
//!   - Traversing the file tree (`DirNode` / `FsEntry`)
//!   - Walking the raw CST (`RawNode` / `Child`) including comments
//!   - Using `SyntaxKind` to pattern-match on node/token types
//!
//! It does not modify any files — it just prints an analysis report to stderr
//! so you can verify it loaded and received the workspace context correctly.

use forgen_api::syntax::raw::{Child, RawNode};
use forgen_api::syntax::SyntaxKind;
use forgen_api::{
    DirNode, FileReplacement, FsEntry, Plugin, PluginRuntime, TextRange, WorkspaceContext,
};

// ---------------------------------------------------------------------------
// Plugin declaration
// ---------------------------------------------------------------------------

/// The example plugin struct.
pub struct ExamplePlugin;

impl Plugin for ExamplePlugin {
    fn name(&self) -> &str {
        "example-plugin"
    }

    fn run(
        &self,
        ctx: &WorkspaceContext,
        _runtime: &mut PluginRuntime<'_>,
    ) -> Vec<FileReplacement> {
        let sep = "=".repeat(56);
        eprintln!("[example-plugin] {sep}");
        eprintln!("[example-plugin]  Workspace : {}", ctx.workspace_root);

        // ── Manifest info ────────────────────────────────────────────────
        let m = &ctx.manifest;
        eprintln!("[example-plugin]  Packages  :");
        for pkg in &m.members {
            eprintln!(
                "[example-plugin]    · {} v{}  (edition {})",
                pkg.name, pkg.version, pkg.edition
            );
            eprintln!(
                "[example-plugin]      {} dep(s), {} dev-dep(s)",
                pkg.dependencies.len(),
                pkg.dev_dependencies.len()
            );
        }

        // forgen-specific metadata, if present
        if let Some(plugins) = m.forgen_metadata::<Vec<String>>("plugins") {
            eprintln!("[example-plugin]  Configured plugins : {plugins:?}");
        }

        // ── File tree ────────────────────────────────────────────────────
        eprintln!("[example-plugin]  File tree :");
        print_dir_tree(ctx.file_tree(), 1);

        // ── Per-file analysis ─────────────────────────────────────────────
        eprintln!("[example-plugin]  File analysis :");

        let mut total_nodes = 0usize;
        let mut total_tokens = 0usize;
        let mut total_comments = 0usize;
        let mut total_todo = 0usize;
        let mut total_pub_fns = 0usize;

        for file in &ctx.files {
            let stats = analyse_file(file.tree());

            total_nodes += stats.nodes;
            total_tokens += stats.tokens;
            total_comments += stats.comments;
            total_todo += stats.todo_comments.len();
            total_pub_fns += stats.pub_fns;

            eprintln!(
                "[example-plugin]    {:<40}  nodes={:4}  tokens={:4}  \
                 comments={:3}  pub_fns={:2}",
                file.path, stats.nodes, stats.tokens, stats.comments, stats.pub_fns,
            );

            for (range, text) in &stats.todo_comments {
                let snippet = text.trim().chars().take(60).collect::<String>();
                eprintln!(
                    "[example-plugin]      TODO @ [{:5}..{:5}]  {snippet}",
                    range.start, range.end,
                );
            }
        }

        eprintln!("[example-plugin]  ── Totals ──────────────────────────────────");
        eprintln!("[example-plugin]    CST nodes   : {total_nodes}");
        eprintln!("[example-plugin]    CST tokens  : {total_tokens}");
        eprintln!("[example-plugin]    Comments    : {total_comments}");
        eprintln!("[example-plugin]    TODO items  : {total_todo}");
        eprintln!("[example-plugin]    pub fn defs : {total_pub_fns}");
        eprintln!("[example-plugin] {sep}");

        // This plugin is read-only — it only reports, never modifies.
        vec![]
    }
}

// ---------------------------------------------------------------------------
// File-tree printer
// ---------------------------------------------------------------------------

fn print_dir_tree(dir: &DirNode, depth: usize) {
    let pad = "  ".repeat(depth);
    let display_name = if dir.name.is_empty() {
        "<workspace root>"
    } else {
        &dir.name
    };
    eprintln!("[example-plugin]    {pad}[dir] {display_name}/");

    for entry in &dir.entries {
        match entry {
            FsEntry::Dir(child) => print_dir_tree(child, depth + 1),
            FsEntry::File(f) => {
                eprintln!("[example-plugin]    {pad}  [file] {}", f.name);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CST analysis
// ---------------------------------------------------------------------------

struct FileStats {
    nodes: usize,
    tokens: usize,
    comments: usize,
    todo_comments: Vec<(TextRange, String)>,
    pub_fns: usize,
}

/// Recursively walk the full CST and compute statistics.
fn analyse_file(root: &RawNode) -> FileStats {
    let mut stats = FileStats {
        nodes: 0,
        tokens: 0,
        comments: 0,
        todo_comments: Vec::new(),
        pub_fns: 0,
    };
    walk(root, &mut stats, /*inside_fn=*/ false);
    stats
}

fn walk(node: &RawNode, stats: &mut FileStats, inside_fn: bool) {
    stats.nodes += 1;

    let is_fn_node = node.kind == SyntaxKind::FN;

    // Check whether this FN has a pub visibility.
    if is_fn_node && !inside_fn && has_pub_visibility(node) {
        stats.pub_fns += 1;
    }

    for child in &node.children {
        match child {
            Child::Token(tok) => {
                stats.tokens += 1;

                if tok.kind == SyntaxKind::COMMENT {
                    stats.comments += 1;

                    // Highlight TODO / FIXME / HACK comments.
                    let upper = tok.text.to_uppercase();
                    if upper.contains("TODO") || upper.contains("FIXME") || upper.contains("HACK") {
                        stats
                            .todo_comments
                            .push((tok.range.clone(), tok.text.clone()));
                    }
                }
            }
            Child::Node(n) => {
                // Track whether we're now inside a function body so we don't
                // double-count nested `fn` definitions as top-level pub fns.
                walk(n, stats, inside_fn || is_fn_node);
            }
        }
    }
}

/// Returns `true` when `fn_node` has a direct VISIBILITY child that contains
/// a `pub` keyword token.
fn has_pub_visibility(fn_node: &RawNode) -> bool {
    for child in &fn_node.children {
        if let Child::Node(vis_node) = child {
            if vis_node.kind == SyntaxKind::VISIBILITY {
                return vis_node
                    .child_tokens()
                    .any(|t| t.kind == SyntaxKind::PUB_KW);
            }
        }
    }
    false
}
