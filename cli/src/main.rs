mod plugin;
mod plugins;
mod workspace;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{Crate, HirDisplay, Semantics};
use ra_ap_ide_db::{base_db::SourceDatabase, EditionedFileId, FileId, RootDatabase};
use ra_ap_paths::AbsPathBuf;
use ra_ap_syntax::{ast, AstNode, Edition};
use ra_ap_vfs::Vfs;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

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

/// Run all registered plugins over every local source file and save the
/// resulting replacements to `target/.forgen/<mirrored-path>.json`.
/// Source files are never touched — a macro will read the JSON and apply
/// the replacements at compile time.
fn run_plugins(db: &RootDatabase, vfs: &Vfs, project_dir: &PathBuf) -> Result<()> {
    let sema = Semantics::new(db);

    // Registered plugins — hardcoded for now, dylib loading comes later.
    let plugins: Vec<Box<dyn plugin::Plugin>> =
        vec![Box::new(plugins::f64_logger::F64LoggerPlugin)];

    // Collect all unique source files reachable from local crates.
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut file_queue: Vec<EditionedFileId> = Vec::new();

    for krate in Crate::all(db) {
        if krate.origin(db).is_local() {
            collect_module_files(db, &krate.root_module(), &mut seen, &mut file_queue);
        }
    }

    println!("🔍 Running plugins on {} file(s)...", file_queue.len());

    let mut total_changes = 0;

    for editioned_id in file_queue {
        let file_id = editioned_id.file_id();

        let Some(path) = workspace::file_id_to_path(vfs, file_id, project_dir) else {
            continue;
        };

        // Source text from the database (consistent with what sema.parse will use).
        let source = String::from(&*SourceDatabase::file_text(db, file_id));

        // Parse through sema so that descendant nodes are registered for type
        // queries (type_of_expr / type_of_pat).
        let parsed = sema.parse(editioned_id);

        // Pre-compute inferred types for every `let` binding that lacks an
        // explicit annotation. We use type_of_expr on the initialiser rather
        // than type_of_pat so we get the concrete, post-inference type.
        let mut pat_types: HashMap<(u32, u32), String> = HashMap::new();
        for node in parsed.syntax().descendants() {
            let Some(let_stmt) = ast::LetStmt::cast(node) else {
                continue;
            };
            // Skip bindings that already have a written type — the plugin
            // will read those directly from the syntax tree.
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
                pat_types.insert((u32::from(r.start()), u32::from(r.end())), ty_str);
            }
        }

        let ctx = plugin::FileContext::new(path.clone(), source.clone(), parsed, pat_types);

        // Gather replacements from every plugin.
        let mut all_replacements: Vec<plugin::Replacement> = Vec::new();
        for p in &plugins {
            all_replacements.extend(p.run(&ctx));
        }

        if all_replacements.is_empty() {
            continue;
        }

        // Keep replacements in source order for readability in the JSON.
        all_replacements.sort_by_key(|r| r.range.start);

        // Mirror the source path under target/.forgen/, appending ".json".
        // e.g. src/lib.rs  →  target/.forgen/src/lib.rs.json
        let relative_path = path.strip_prefix(project_dir).unwrap_or(&path);
        let output_dir = project_dir.join("target").join(".forgen").join(
            relative_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("")),
        );
        fs::create_dir_all(&output_dir)?;

        let mut output_name = relative_path.file_name().unwrap_or_default().to_os_string();
        output_name.push(".json");
        let output_path = output_dir.join(output_name);

        let json = serde_json::to_string_pretty(&all_replacements)?;
        fs::write(&output_path, &json)?;

        println!(
            "  💾 {} → {} replacement(s)",
            relative_path.display(),
            all_replacements.len()
        );
        total_changes += all_replacements.len();
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
