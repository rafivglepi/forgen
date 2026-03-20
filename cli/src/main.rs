mod loader;
mod plugins;
mod workspace;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use forgen_api::{
    EnumDef, FieldDef, FileContext as ApiFileContext, FnDef, FnParam, ImplDef, LetBinding, Plugin,
    StructDef, TextRange as ApiTextRange, VariantDef, WorkspaceContext,
};
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{Crate, HirDisplay, Semantics};
use ra_ap_ide_db::{base_db::SourceDatabase, EditionedFileId, FileId, RootDatabase};
use ra_ap_paths::AbsPathBuf;
use ra_ap_syntax::{ast, ast::HasName, ast::HasVisibility, AstNode, Edition};
use ra_ap_vfs::Vfs;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Forgen - compile-time codegen for Rust
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, bin_name = "cargo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the Forgen plugin runner
    Forgen(Args),
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to Cargo.toml (defaults to ./Cargo.toml in current directory)
    #[arg(value_name = "MANIFEST")]
    manifest: Option<PathBuf>,

    /// Watch for file changes and re-run plugins (development mode)
    #[arg(short, long)]
    watch: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Command::Forgen(args) = cli.command;

    println!("🚀 Forgen");
    println!("=========================================\n");

    let manifest_path = args.manifest.unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    println!("📦 Loading project: {}", manifest_path.display());

    let manifest_path_abs = manifest_path.canonicalize()?;
    let manifest_path_str = manifest_path_abs
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;
    let manifest_path = AbsPathBuf::try_from(manifest_path_str)
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

    let workspace_info = workspace::get_workspace_info(&manifest_path_abs)?;
    let (mut host, mut vfs) = workspace::load_workspace(&manifest_path)?;

    if args.watch {
        println!("👀 Watch mode enabled - monitoring for changes...\n");
        println!("Press Ctrl+C to stop\n");

        run_plugins(&host, &vfs, &workspace_info.root)?;

        let (tx, rx) = channel();
        let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;

        if workspace_info.members.is_empty() {
            anyhow::bail!("No source directories found to watch");
        }

        for src_path in &workspace_info.members {
            debouncer
                .watcher()
                .watch(src_path, RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch {:?}", src_path))?;
            println!("📁 Watching: {}", src_path.display());
        }
        println!();

        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    let changed_files: Vec<_> = events
                        .iter()
                        .filter(|e| e.path.extension().and_then(|s| s.to_str()) == Some("rs"))
                        .map(|e| e.path.clone())
                        .collect();

                    if !changed_files.is_empty() {
                        println!("🔄 File change detected, re-running plugins...");

                        match workspace::apply_file_changes(&mut host, &mut vfs, &changed_files) {
                            Ok(_) => match run_plugins(&host, &vfs, &workspace_info.root) {
                                Ok(_) => println!("✅ Done\n"),
                                Err(e) => eprintln!("❌ Plugin error: {}\n", e),
                            },
                            Err(e) => eprintln!("❌ Error applying file changes: {}\n", e),
                        }
                    }
                }
                Ok(Err(e)) => eprintln!("Watch error: {:?}", e),
                Err(e) => {
                    eprintln!("Channel error: {:?}", e);
                    break;
                }
            }
        }
    } else {
        run_plugins(&host, &vfs, &workspace_info.root)?;
        println!("\n✨ Done!");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Plugin runner
// ---------------------------------------------------------------------------

/// Build the workspace context, run every plugin against it, and persist the
/// resulting `FileReplacement` lists to `target/.forgen/<path>.json`.
///
/// Source files are **never** modified — macros read the JSON at compile time.
fn run_plugins(db: &RootDatabase, vfs: &Vfs, project_dir: &PathBuf) -> Result<()> {
    // Collect built-in plugins first.
    let mut plugins: Vec<Box<dyn Plugin>> = vec![Box::new(plugins::f64_logger::F64LoggerPlugin)];

    // Load any dylib plugins from <workspace>/forgen-plugins/.
    let dylib_dir = project_dir.join("forgen-plugins");
    if dylib_dir.exists() {
        let dylib_plugins = loader::load_plugins_from_dir(&dylib_dir);
        if !dylib_plugins.is_empty() {
            println!("  📦 Loaded {} dylib plugin(s)\n", dylib_plugins.len());
        }
        plugins.extend(dylib_plugins);
    }

    // Enumerate every source file reachable from a local crate.
    let sema = Semantics::new(db);
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut file_queue: Vec<EditionedFileId> = Vec::new();

    for krate in Crate::all(db) {
        if krate.origin(db).is_local() {
            collect_module_files(db, &krate.root_module(), &mut seen, &mut file_queue);
        }
    }

    println!(
        "🔍 Building workspace context from {} file(s)...",
        file_queue.len()
    );

    // Build the full WorkspaceContext *before* calling any plugin.
    let workspace_ctx = build_workspace_context(&sema, db, vfs, project_dir, file_queue)?;

    println!("🧩 Running {} plugin(s)...\n", plugins.len());

    let mut total_changes: usize = 0;

    for plugin in &plugins {
        let file_replacements = plugin.run(&workspace_ctx);

        for mut fr in file_replacements {
            if fr.replacements.is_empty() {
                continue;
            }

            // Sort in source order for readable JSON diffs.
            fr.replacements.sort_by_key(|r| r.range.start);

            // Mirror the source path under target/.forgen/, appending ".json".
            // e.g.  "test/src/lib.rs"  →  target/.forgen/test/src/lib.rs.json
            let rel_path = std::path::Path::new(&fr.path);
            let output_dir = project_dir.join("target").join(".forgen").join(
                rel_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("")),
            );
            fs::create_dir_all(&output_dir)?;

            let mut output_name = rel_path.file_name().unwrap_or_default().to_os_string();
            output_name.push(".json");
            let output_path = output_dir.join(output_name);

            let json = serde_json::to_string_pretty(&fr.replacements)?;
            fs::write(&output_path, &json)?;

            println!(
                "  💾 {} → {} replacement(s)  [{}]",
                fr.path,
                fr.replacements.len(),
                plugin.name(),
            );
            total_changes += fr.replacements.len();
        }
    }

    println!();
    if total_changes > 0 {
        println!(
            "✅ Saved {} total replacement(s) to target/.forgen/",
            total_changes
        );
    } else {
        println!("✅ No replacements generated");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// WorkspaceContext builder
// ---------------------------------------------------------------------------

fn build_workspace_context(
    sema: &Semantics<RootDatabase>,
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    file_queue: Vec<EditionedFileId>,
) -> Result<WorkspaceContext> {
    let mut files: Vec<ApiFileContext> = Vec::new();

    for editioned_id in file_queue {
        let file_id = editioned_id.file_id();

        let Some(path) = workspace::file_id_to_path(vfs, file_id, project_dir) else {
            continue;
        };

        // Read source text from the database (consistent with sema.parse).
        let source = String::from(&*SourceDatabase::file_text(db, file_id));

        // Parse through sema so that descendant nodes are registered for type
        // queries (type_of_expr / type_of_pat).
        let parsed = sema.parse(editioned_id);
        let syntax = parsed.syntax();

        // Pre-compute inferred types for unannotated `let` bindings.
        // We use type_of_expr on the initialiser to get the concrete, post-
        // inference type even when no annotation is written.
        let mut pat_type_cache: HashMap<(u32, u32), String> = HashMap::new();
        for node in syntax.descendants() {
            let Some(let_stmt) = ast::LetStmt::cast(node) else {
                continue;
            };
            // Annotated bindings don't need inference — the plugin reads the
            // explicit type directly from the pre-computed LetBinding.
            if let_stmt.ty().is_some() {
                continue;
            }
            let Some(pat) = let_stmt.pat() else { continue };
            let Some(init) = let_stmt.initializer() else {
                continue;
            };

            if let Some(type_info) = sema.type_of_expr(&init) {
                let ty_str = type_info.original.display(db, Edition::CURRENT).to_string();
                let r = pat.syntax().text_range();
                pat_type_cache.insert((u32::from(r.start()), u32::from(r.end())), ty_str);
            }
        }

        // Workspace-relative path using forward slashes on all platforms.
        let rel_path = path
            .strip_prefix(project_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        files.push(ApiFileContext {
            path: rel_path,
            source,
            let_bindings: extract_let_bindings(syntax, &pat_type_cache),
            functions: extract_functions(syntax),
            structs: extract_structs(syntax),
            enums: extract_enums(syntax),
            impls: extract_impls(syntax),
        });
    }

    Ok(WorkspaceContext {
        workspace_root: project_dir.to_string_lossy().replace('\\', "/"),
        files,
    })
}

// ---------------------------------------------------------------------------
// AST → API type converters
// ---------------------------------------------------------------------------

/// Convert a ra_ap_syntax `TextRange` to the API's `TextRange`.
#[inline]
fn to_api_range(r: ra_ap_syntax::TextRange) -> ApiTextRange {
    ApiTextRange {
        start: u32::from(r.start()),
        end: u32::from(r.end()),
    }
}

/// Extract every `let` binding in `syntax` (all scopes, flattened).
///
/// Only simple identifier patterns (`let [mut] name [: T] = …`) are captured;
/// destructuring patterns are skipped.
fn extract_let_bindings(
    syntax: &ra_ap_syntax::SyntaxNode,
    pat_types: &HashMap<(u32, u32), String>,
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

        // Skip anonymous / underscore patterns.
        if name.is_empty() || name == "_" {
            continue;
        }

        let explicit_type = let_stmt
            .ty()
            .map(|ty| ty.syntax().text().to_string().trim().to_owned());

        let inferred_type = if explicit_type.is_none() {
            let r = pat.syntax().text_range();
            pat_types
                .get(&(u32::from(r.start()), u32::from(r.end())))
                .cloned()
        } else {
            None
        };

        bindings.push(LetBinding {
            name,
            explicit_type,
            inferred_type,
            range: to_api_range(let_stmt.syntax().text_range()),
            is_mut,
        });
    }

    bindings
}

/// Build an [`FnDef`] from an `ast::Fn` node.
///
/// Returns `None` if the function has no name (shouldn't happen in valid
/// Rust, but defensive programming is cheap here).
fn extract_fn_def(fn_node: &ast::Fn) -> Option<FnDef> {
    let name = fn_node.name()?.to_string();

    let has_self = fn_node
        .param_list()
        .and_then(|pl| pl.self_param())
        .is_some();

    let params: Vec<FnParam> = fn_node
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| {
                    let name = p
                        .pat()
                        .and_then(|pat| match pat {
                            ast::Pat::IdentPat(ip) => ip.name().map(|n| n.to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "_".to_string());
                    let ty = p
                        .ty()
                        .map(|t| t.syntax().text().to_string().trim().to_owned());
                    FnParam { name, ty }
                })
                .collect()
        })
        .unwrap_or_default();

    let return_type = fn_node
        .ret_type()
        .and_then(|rt| rt.ty())
        .map(|t| t.syntax().text().to_string().trim().to_owned());

    let is_pub = fn_node
        .visibility()
        .map(|v| v.syntax().text().to_string().starts_with("pub"))
        .unwrap_or(false);

    let is_async = fn_node.async_token().is_some();

    Some(FnDef {
        name,
        params,
        has_self,
        return_type,
        range: to_api_range(fn_node.syntax().text_range()),
        is_pub,
        is_async,
    })
}

/// Extract all function definitions in `syntax` (top-level and impl methods).
fn extract_functions(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<FnDef> {
    syntax
        .descendants()
        .filter_map(ast::Fn::cast)
        .filter_map(|fn_node| extract_fn_def(&fn_node))
        .collect()
}

/// Extract named fields from a `RecordFieldList`.
fn extract_record_fields(list: &ast::RecordFieldList) -> Vec<FieldDef> {
    list.fields()
        .filter_map(|f| {
            let name = f.name()?.to_string();
            let ty = f
                .ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())
                .unwrap_or_default();
            let is_pub = f
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            Some(FieldDef { name, ty, is_pub })
        })
        .collect()
}

/// Extract positional fields from a `TupleFieldList`, naming them by index.
fn extract_tuple_fields(list: &ast::TupleFieldList) -> Vec<FieldDef> {
    list.fields()
        .enumerate()
        .map(|(i, f)| {
            let ty = f
                .ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())
                .unwrap_or_default();
            let is_pub = f
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            FieldDef {
                name: i.to_string(),
                ty,
                is_pub,
            }
        })
        .collect()
}

/// Extract all struct definitions in `syntax`.
fn extract_structs(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<StructDef> {
    syntax
        .descendants()
        .filter_map(ast::Struct::cast)
        .filter_map(|s| {
            let name = s.name()?.to_string();
            let is_pub = s
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);

            let (fields, tuple_fields) = match s.field_list() {
                Some(ast::FieldList::RecordFieldList(list)) => {
                    (extract_record_fields(&list), vec![])
                }
                Some(ast::FieldList::TupleFieldList(list)) => (vec![], extract_tuple_fields(&list)),
                None => (vec![], vec![]),
            };

            Some(StructDef {
                name,
                fields,
                tuple_fields,
                range: to_api_range(s.syntax().text_range()),
                is_pub,
            })
        })
        .collect()
}

/// Extract all enum definitions in `syntax`.
fn extract_enums(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<EnumDef> {
    syntax
        .descendants()
        .filter_map(ast::Enum::cast)
        .filter_map(|e| {
            let name = e.name()?.to_string();
            let is_pub = e
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);

            let variants: Vec<VariantDef> = e
                .variant_list()
                .map(|vl| {
                    vl.variants()
                        .filter_map(|v| {
                            let name = v.name()?.to_string();
                            let (fields, tuple_fields) = match v.field_list() {
                                Some(ast::FieldList::RecordFieldList(list)) => {
                                    (extract_record_fields(&list), vec![])
                                }
                                Some(ast::FieldList::TupleFieldList(list)) => {
                                    (vec![], extract_tuple_fields(&list))
                                }
                                None => (vec![], vec![]),
                            };
                            Some(VariantDef {
                                name,
                                fields,
                                tuple_fields,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            Some(EnumDef {
                name,
                variants,
                range: to_api_range(e.syntax().text_range()),
                is_pub,
            })
        })
        .collect()
}

/// Extract all `impl` blocks in `syntax`.
///
/// Methods are also included in the file-level `functions` list for flat
/// iteration; the `ImplDef` gives the structural view.
fn extract_impls(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<ImplDef> {
    syntax
        .descendants()
        .filter_map(ast::Impl::cast)
        .filter_map(|impl_node| {
            let self_ty = impl_node
                .self_ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())?;

            let trait_ = impl_node
                .trait_()
                .map(|t| t.syntax().text().to_string().trim().to_owned());

            let methods: Vec<FnDef> = impl_node
                .assoc_item_list()
                .map(|list| {
                    list.assoc_items()
                        .filter_map(|item| {
                            if let ast::AssocItem::Fn(fn_node) = item {
                                extract_fn_def(&fn_node)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            Some(ImplDef {
                self_ty,
                trait_,
                methods,
                range: to_api_range(impl_node.syntax().text_range()),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Module file collector
// ---------------------------------------------------------------------------

/// Recursively collect the `EditionedFileId` of every module in the subtree
/// rooted at `module`, deduplicating via the plain `FileId`.
fn collect_module_files(
    db: &RootDatabase,
    module: &ra_ap_hir::Module,
    seen: &mut HashSet<FileId>,
    queue: &mut Vec<EditionedFileId>,
) {
    if let Some(editioned_id) = module.definition_source(db).file_id.file_id() {
        let file_id = editioned_id.file_id();
        if seen.insert(file_id) {
            queue.push(editioned_id);
        }
    }
    for child in module.children(db) {
        collect_module_files(db, &child, seen, queue);
    }
}

