mod loader;
mod oracle;
mod replacements;
mod workspace;

use anyhow::{Context, Result};
use cargo_metadata::DependencyKind as CargoDep;
use clap::{Parser, Subcommand};
use forgen_api::Replacement;
use forgen_api::{
    syntax::raw::{Child as SyntaxChild, RawNode, RawToken},
    syntax::SyntaxKind,
    Dependency, DependencySource, DirNode, EnumDef, FieldDef, FileContext as ApiFileContext,
    FileRef, FnDef, FnParam, FsEntry, ImplDef, LazyValue, LetBinding, PackageManifest,
    SemanticHandle, StructDef, SuiteRuntime, TextRange as ApiTextRange, VariantDef,
    WorkspaceContext, WorkspaceManifest,
};
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{attach_db_allow_change, Crate, Semantics};
use ra_ap_ide_db::{base_db::SourceDatabase, EditionedFileId, FileId, RootDatabase};
use ra_ap_paths::AbsPathBuf;
use ra_ap_syntax::{ast, ast::HasName, ast::HasVisibility, AstNode, SourceFile, SyntaxElement};
use ra_ap_vfs::Vfs;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use std::path::{Path, PathBuf};
use std::sync::{mpsc::channel, Arc, OnceLock};
use std::time::{Duration, Instant};

const MAX_PLUGIN_PASSES: usize = 10;

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

    /// Disable rust-analyzer proc-macro expansion while loading the workspace
    #[arg(long)]
    no_proc_macros: bool,

    /// Re-enable build-script / out-dir loading while loading the workspace
    #[arg(long)]
    with_build_scripts: bool,

    /// Re-enable rust-analyzer cache prefill while loading the workspace
    #[arg(long)]
    with_prefill_caches: bool,

    /// Print oracle inference traces (binding text + inferred type) to stderr
    #[arg(short, long)]
    verbose: bool,
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

    let total_start = Instant::now();

    let manifest_path_abs = manifest_path.canonicalize()?;
    let manifest_path_str = manifest_path_abs
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;
    let manifest_path = AbsPathBuf::try_from(manifest_path_str)
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

    let metadata_start = Instant::now();
    let workspace_info = workspace::get_workspace_info(&manifest_path_abs)?;
    replacements::clear_saved_replacements(&workspace_info.root)?;
    println!(
        "⏱ cargo metadata + workspace discovery took {:.2?}",
        metadata_start.elapsed()
    );

    let load_start = Instant::now();
    println!(
        "🧪 Workspace load config: proc_macros={}, build_scripts={}, prefill_caches={}",
        if args.no_proc_macros {
            "disabled"
        } else {
            "sysroot"
        },
        if args.with_build_scripts {
            "enabled"
        } else {
            "disabled"
        },
        if args.with_prefill_caches {
            "enabled"
        } else {
            "disabled"
        },
    );
    let (mut host, mut vfs) = workspace::load_workspace(
        &manifest_path,
        workspace::WorkspaceLoadOptions {
            proc_macro_server: if args.no_proc_macros {
                ra_ap_load_cargo::ProcMacroServerChoice::None
            } else {
                ra_ap_load_cargo::ProcMacroServerChoice::Sysroot
            },
            load_out_dirs_from_check: args.with_build_scripts,
            prefill_caches: args.with_prefill_caches,
        },
    )?;
    println!("⏱ workspace load took {:.2?}", load_start.elapsed());

    let mut suite_runtime = SuiteRuntime::new();
    println!("🎲 Suite runtime seed: {}", suite_runtime.seed());

    if args.watch {
        println!("👀 Watch mode enabled - monitoring for changes...\n");
        println!("Press Ctrl+C to stop\n");

        run_plugins(
            &mut host,
            &mut vfs,
            &workspace_info,
            true,
            args.verbose,
            &mut suite_runtime,
        )?;

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
                    // Collect every .rs path that was touched — this covers
                    // creates, deletes, renames, and plain modifications since
                    // notify-debouncer-mini collapses all of them into a single
                    // path-level event.
                    let changed_files: Vec<_> = events
                        .iter()
                        .filter(|e| e.path.extension().and_then(|s| s.to_str()) == Some("rs"))
                        .map(|e| e.path.clone())
                        .collect();

                    if !changed_files.is_empty() {
                        // Summarise what happened so the user knows why a
                        // re-run was triggered (created / deleted / modified).
                        let created: Vec<_> = changed_files
                            .iter()
                            .filter(|p| {
                                // A file that exists now but the VFS doesn't
                                // know about yet is effectively "new".
                                p.exists()
                            })
                            .collect();
                        let deleted: Vec<_> =
                            changed_files.iter().filter(|p| !p.exists()).collect();

                        if !created.is_empty() {
                            for p in &created {
                                println!(
                                    "  📝 {}",
                                    p.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("(unknown)")
                                );
                            }
                        }
                        if !deleted.is_empty() {
                            for p in &deleted {
                                println!(
                                    "  🗑  {}",
                                    p.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("(unknown)")
                                );
                            }
                        }

                        println!("🔄 File system change detected, re-running plugins...");
                        match workspace::apply_file_changes(&mut host, &mut vfs, &changed_files) {
                            Ok(_) => {
                                match run_plugins(
                                    &mut host,
                                    &mut vfs,
                                    &workspace_info,
                                    false,
                                    args.verbose,
                                    &mut suite_runtime,
                                ) {
                                    Ok(_) => println!("✅ Done\n"),
                                    Err(e) => eprintln!("❌ Plugin error: {}\n", e),
                                }
                            }
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
        let run_start = Instant::now();
        run_plugins(
            &mut host,
            &mut vfs,
            &workspace_info,
            true,
            args.verbose,
            &mut suite_runtime,
        )?;
        println!("⏱ plugin run took {:.2?}", run_start.elapsed());
        println!("⏱ total CLI time {:.2?}", total_start.elapsed());
        println!("\n✨ Done!");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Plugin runner
// ---------------------------------------------------------------------------

fn run_plugins(
    db: &mut RootDatabase,
    vfs: &mut Vfs,
    workspace_info: &workspace::WorkspaceInfo,
    build: bool,
    verbose: bool,
    suite_runtime: &mut SuiteRuntime,
) -> Result<()> {
    let run_start = Instant::now();
    let project_dir = &workspace_info.root;
    let cargo_meta = &workspace_info.cargo_metadata;

    replacements::clear_saved_replacements(project_dir)?;

    // Plugin suite: `[workspace.metadata.forgen] suite = "..."`
    let suite_start = Instant::now();
    let suite = loader::load_suite(cargo_meta, build);
    if suite.is_some() {
        println!();
    }
    println!("⏱ plugin suite load took {:.2?}", suite_start.elapsed());

    let Some(suite) = suite else {
        let write_start = Instant::now();
        let total_saved = replacements::write_final_file_replacements(project_dir, &[])?;
        println!(
            "⏱ replacement JSON write took {:.2?}",
            write_start.elapsed()
        );

        println!();
        if total_saved > 0 {
            println!(
                "✅ Saved {} total replacement patch(es) to target/.forgen/",
                total_saved
            );
        } else {
            println!("✅ No replacements generated");
        }

        println!("⏱ run_plugins total took {:.2?}", run_start.elapsed());
        return Ok(());
    };

    let mut working_suite_runtime = suite_runtime.clone();

    let file_queue = enumerate_workspace_file_queue(db, vfs, project_dir, cargo_meta)?;
    let snapshots = snapshot_workspace_sources(db, vfs, project_dir, &file_queue)?;
    let original_sources: HashMap<String, String> = snapshots
        .iter()
        .map(|snapshot| (snapshot.rel_path.clone(), snapshot.source.clone()))
        .collect();
    let mut file_models = replacements::build_file_models(&original_sources);
    let abs_paths_by_rel: HashMap<String, PathBuf> = snapshots
        .iter()
        .map(|snapshot| (snapshot.rel_path.clone(), snapshot.abs_path.clone()))
        .collect();

    let fixed_point_result = run_fixed_point_passes(
        original_sources.clone(),
        |pass, current_sources| {
            println!("🧩 Pass {pass}/{MAX_PLUGIN_PASSES}");
            apply_source_snapshot(db, vfs, &abs_paths_by_rel, current_sources, true)?;

            let pass_start = Instant::now();
            let pass_output = attach_db_allow_change(db, || -> Result<_> {
                println!("  entering semantics init");
                let sema = Semantics::new(db);
                println!("  finished semantics init");

                println!(
                    "  Building workspace context from {} file(s)...",
                    file_queue.len()
                );

                let workspace_ctx_start = Instant::now();
                let ctx = build_workspace_context(
                    &sema,
                    db,
                    vfs,
                    project_dir,
                    file_queue.clone(),
                    cargo_meta,
                    verbose,
                )?;
                println!(
                    "  workspace context build took {:.2?}",
                    workspace_ctx_start.elapsed()
                );

                let suite_exec_start = Instant::now();
                let file_replacements = suite.run(&ctx, &mut working_suite_runtime);
                let mut replacements_by_path: HashMap<String, Vec<Replacement>> = HashMap::new();
                let mut generated_replacements = 0usize;

                for fr in file_replacements {
                    if fr.replacements.is_empty() {
                        continue;
                    }

                    generated_replacements += fr.replacements.len();
                    replacements_by_path
                        .entry(fr.path)
                        .or_default()
                        .extend(fr.replacements);
                }

                println!(
                    "  suite execution took {:.2?} ({} replacement(s))",
                    suite_exec_start.elapsed(),
                    generated_replacements
                );

                Ok(PassOutput {
                    replacements_by_path,
                    generated_replacements,
                })
            })?;

            let changed_paths = replacements::changed_paths_from_replacements(
                current_sources,
                &pass_output.replacements_by_path,
            )?;
            let mut changed_paths: Vec<_> = changed_paths.into_iter().collect();
            changed_paths.sort();

            if changed_paths.is_empty() {
                println!(
                    "  pass {pass} converged in {:.2?} ({} replacement(s), no source changes)",
                    pass_start.elapsed(),
                    pass_output.generated_replacements
                );
            } else {
                for path in &changed_paths {
                    let model = file_models
                        .get_mut(path)
                        .ok_or_else(|| anyhow::anyhow!("Missing file model for `{path}`"))?;
                    let current_source = current_sources.get(path).ok_or_else(|| {
                        anyhow::anyhow!("Missing current source snapshot for `{path}`")
                    })?;
                    let replacements = pass_output
                        .replacements_by_path
                        .get(path)
                        .ok_or_else(|| anyhow::anyhow!("Missing replacements for `{path}`"))?;

                    replacements::apply_replacements_to_file_model(
                        model,
                        current_source,
                        replacements,
                    )
                    .with_context(|| {
                        format!(
                            "Failed to record original-source replacements for `{path}` in pass {pass}"
                        )
                    })?;
                }

                let changed_files = replacements::collect_changed_files(&file_models)?;
                let saved_count =
                    replacements::replace_saved_replacements(project_dir, &changed_files)?;
                println!(
                    "  saved intermediate replacement JSON after pass {pass} ({} patch(es))",
                    saved_count
                );

                println!(
                    "  pass {pass} changed {} file(s) in {:.2?}: {}",
                    changed_paths.len(),
                    pass_start.elapsed(),
                    changed_paths.join(", ")
                );
            }

            Ok(pass_output)
        },
    );

    let restore_result =
        apply_source_snapshot(db, vfs, &abs_paths_by_rel, &original_sources, false);

    let fixed_point_result = match (fixed_point_result, restore_result) {
        (Ok(result), Ok(())) => result,
        (Err(err), Ok(())) => return Err(err),
        (Ok(_), Err(err)) => return Err(err),
        (Err(run_err), Err(restore_err)) => {
            return Err(run_err.context(format!(
                "Additionally failed to restore original workspace snapshot: {restore_err}"
            )))
        }
    };

    let write_start = Instant::now();
    let changed_files = replacements::collect_changed_files(&file_models)?;
    let total_saved = replacements::replace_saved_replacements(project_dir, &changed_files)?;
    println!(
        "⏱ replacement JSON write took {:.2?}",
        write_start.elapsed()
    );

    println!();
    if total_saved > 0 {
        println!(
            "✅ Saved {} total replacement patch(es) to target/.forgen/",
            total_saved
        );
    } else if fixed_point_result.total_generated_replacements > 0 {
        println!("✅ Replacements were generated, but all serialised patch sets were empty");
    } else {
        println!("✅ No replacements generated");
    }

    println!("⏱ run_plugins total took {:.2?}", run_start.elapsed());

    *suite_runtime = working_suite_runtime;

    Ok(())
}

#[derive(Debug)]
struct PassOutput {
    replacements_by_path: HashMap<String, Vec<Replacement>>,
    generated_replacements: usize,
}

#[derive(Debug)]
struct FixedPointResult {
    total_generated_replacements: usize,
}

#[derive(Debug, Clone)]
struct WorkspaceFileSnapshot {
    rel_path: String,
    abs_path: PathBuf,
    source: String,
}

fn run_fixed_point_passes<F>(
    initial_sources: HashMap<String, String>,
    mut run_pass: F,
) -> Result<FixedPointResult>
where
    F: FnMut(usize, &HashMap<String, String>) -> Result<PassOutput>,
{
    let mut current_sources = initial_sources;
    let mut total_generated_replacements = 0usize;

    for pass in 1..=MAX_PLUGIN_PASSES {
        let pass_output = run_pass(pass, &current_sources)?;
        total_generated_replacements += pass_output.generated_replacements;

        let changed_paths = replacements::changed_paths_from_replacements(
            &current_sources,
            &pass_output.replacements_by_path,
        )?;

        if changed_paths.is_empty() {
            return Ok(FixedPointResult {
                total_generated_replacements,
            });
        }

        let mut changed_paths: Vec<_> = changed_paths.into_iter().collect();
        changed_paths.sort();

        if pass == MAX_PLUGIN_PASSES {
            anyhow::bail!(
                "Plugin execution did not converge after {} passes; files still changing: {}",
                MAX_PLUGIN_PASSES,
                changed_paths.join(", ")
            );
        }

        let mut next_sources = current_sources.clone();
        for path in changed_paths {
            let source = current_sources
                .get(&path)
                .ok_or_else(|| anyhow::anyhow!("Replacement references unknown file `{path}`"))?;
            let replacements = pass_output
                .replacements_by_path
                .get(&path)
                .ok_or_else(|| anyhow::anyhow!("Missing replacement set for `{path}`"))?;
            let rewritten = replacements::apply_replacements_to_source(source, replacements)
                .with_context(|| format!("Failed to rewrite `{path}` during pass {pass}"))?;
            next_sources.insert(path, rewritten);
        }
        current_sources = next_sources;
    }

    unreachable!("fixed-point pass loop should either converge or bail")
}

fn enumerate_workspace_file_queue(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    cargo_meta: &cargo_metadata::Metadata,
) -> Result<Vec<EditionedFileId>> {
    let enumerate_start = Instant::now();
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut file_queue: Vec<EditionedFileId> = Vec::new();
    let root_norm = normalize_path_str(&project_dir.to_string_lossy());

    let crate_all = Crate::all(db);
    for krate in &crate_all {
        if !krate.origin(db).is_local() {
            continue;
        }
        for file_id in db
            .source_root(db.file_source_root(krate.root_file(db)).source_root_id(db))
            .source_root(db)
            .iter()
        {
            if seen.insert(file_id) {
                file_queue.push(EditionedFileId::new(db, file_id, krate.edition(db)));
            }
        }
    }

    let valid_roots: Vec<String> = cargo_meta
        .workspace_packages()
        .into_iter()
        .filter(|p| p.dependencies.iter().any(|d| d.name == "forgen"))
        .map(|p| {
            let mut rp = normalize_path_str(&p.manifest_path.parent().unwrap().to_string());
            if !rp.ends_with('/') {
                rp.push('/');
            }
            rp
        })
        .collect();

    let path_cache: HashMap<FileId, Option<String>> = file_queue
        .iter()
        .map(|ef| {
            let fid = ef.file_id(db);
            let norm = workspace::file_id_to_path(vfs, fid, project_dir)
                .map(|p| normalize_path_str(&p.to_string_lossy()));
            (fid, norm)
        })
        .collect();

    let mut local_vfs_file_count = 0usize;
    let mut skipped_non_local_files = 0usize;
    file_queue.retain(|ef| {
        let fid = ef.file_id(db);
        let Some(path_norm) = path_cache.get(&fid).and_then(|p| p.as_deref()) else {
            skipped_non_local_files += 1;
            return false;
        };

        if path_norm.starts_with(&root_norm) && path_norm.ends_with(".rs") {
            let is_valid = valid_roots.is_empty()
                || valid_roots
                    .iter()
                    .any(|r| path_norm.starts_with(r.as_str()));
            if is_valid {
                local_vfs_file_count += 1;
                true
            } else {
                skipped_non_local_files += 1;
                false
            }
        } else {
            skipped_non_local_files += 1;
            false
        }
    });

    file_queue.sort_by_cached_key(|ef| {
        path_cache
            .get(&ef.file_id(db))
            .and_then(|p| p.clone())
            .unwrap_or_default()
    });
    file_queue.dedup_by_key(|ef| ef.file_id(db));

    println!(
        "  crate-graph file enumeration took {:.2?} (local={}, skipped={})",
        enumerate_start.elapsed(),
        local_vfs_file_count,
        skipped_non_local_files
    );

    Ok(file_queue)
}

fn snapshot_workspace_sources(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    file_queue: &[EditionedFileId],
) -> Result<Vec<WorkspaceFileSnapshot>> {
    let root_norm = normalize_path_str(&project_dir.to_string_lossy());
    let mut snapshots = Vec::with_capacity(file_queue.len());

    for editioned_id in file_queue {
        let file_id = editioned_id.file_id(db);
        let abs_path = workspace::file_id_to_path(vfs, file_id, project_dir)
            .ok_or_else(|| anyhow::anyhow!("Could not resolve path for file id {:?}", file_id))?;
        let rel_path = relative_workspace_path(&root_norm, &abs_path);
        let source = SourceDatabase::file_text(db, file_id).text(db).to_string();
        snapshots.push(WorkspaceFileSnapshot {
            rel_path,
            abs_path,
            source,
        });
    }

    snapshots.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(snapshots)
}

fn apply_source_snapshot(
    db: &mut RootDatabase,
    vfs: &mut Vfs,
    abs_paths_by_rel: &HashMap<String, PathBuf>,
    sources_by_rel: &HashMap<String, String>,
    neutralize_forgen_attr: bool,
) -> Result<()> {
    let mut rel_paths: Vec<_> = abs_paths_by_rel.keys().cloned().collect();
    rel_paths.sort();

    let mut updates = Vec::with_capacity(rel_paths.len());
    for rel_path in rel_paths {
        let abs_path = abs_paths_by_rel
            .get(&rel_path)
            .ok_or_else(|| anyhow::anyhow!("Missing absolute path for `{rel_path}`"))?;
        let source = sources_by_rel
            .get(&rel_path)
            .ok_or_else(|| anyhow::anyhow!("Missing source snapshot for `{rel_path}`"))?;
        let source = if neutralize_forgen_attr {
            neutralize_forgen_file_attr(source)
        } else {
            source.clone()
        };
        updates.push((abs_path.clone(), Some(source)));
    }

    workspace::apply_text_updates(db, vfs, &updates)
}

fn neutralize_forgen_file_attr(source: &str) -> String {
    let needle = "#![forgen::file(";
    let Some(attr_index) = source.find(needle) else {
        return source.to_owned();
    };

    let line_start = source[..attr_index]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = source[attr_index..]
        .find('\n')
        .map(|offset| attr_index + offset)
        .unwrap_or(source.len());

    let mut out = source.as_bytes().to_vec();
    for byte in &mut out[line_start..line_end] {
        *byte = b' ';
    }

    String::from_utf8(out).expect("source text should remain valid UTF-8")
}

fn relative_workspace_path(root_norm: &str, abs_path: &Path) -> String {
    let path_norm = normalize_path_str(&abs_path.to_string_lossy());
    path_norm
        .strip_prefix(root_norm)
        .map(|s| s.trim_start_matches('/').to_owned())
        .unwrap_or(path_norm)
}

// ---------------------------------------------------------------------------
// WorkspaceContext builder
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct SharedRuntime {
    db: *const RootDatabase,
    vfs: *const Vfs,
    project_dir: *const PathBuf,
    _not_send_sync: PhantomData<fn() -> ()>,
}

unsafe impl Send for SharedRuntime {}
unsafe impl Sync for SharedRuntime {}

impl SharedRuntime {
    fn db(self) -> &'static RootDatabase {
        unsafe { &*self.db }
    }

    fn vfs(self) -> &'static Vfs {
        unsafe { &*self.vfs }
    }

    fn project_dir(self) -> &'static PathBuf {
        unsafe { &*self.project_dir }
    }
}

#[derive(Clone, Copy)]
struct FileRuntime {
    shared: SharedRuntime,
    editioned_id: EditionedFileId,
}

unsafe impl Send for FileRuntime {}
unsafe impl Sync for FileRuntime {}

impl FileRuntime {
    fn path(self) -> Option<PathBuf> {
        workspace::file_id_to_path(
            self.shared.vfs(),
            self.editioned_id.file_id(self.shared.db()),
            self.shared.project_dir(),
        )
    }

    fn rel_path(self, root_norm: &str) -> Option<String> {
        let path = self.path()?;
        let p = normalize_path_str(&path.to_string_lossy());
        Some(
            p.strip_prefix(root_norm)
                .map(|s| s.trim_start_matches('/').to_owned())
                .unwrap_or(p),
        )
    }

    fn source(self) -> String {
        SourceDatabase::file_text(
            self.shared.db(),
            self.editioned_id.file_id(self.shared.db()),
        )
        .text(self.shared.db())
        .to_string()
    }

    fn syntax_from_source(self) -> ra_ap_syntax::SyntaxNode {
        let (_file_id, edition) = self.editioned_id.unpack(self.shared.db());
        SourceFile::parse(&self.source(), edition)
            .tree()
            .syntax()
            .clone()
    }

    fn tree(self) -> RawNode {
        let syntax = self.syntax_from_source();
        build_raw_node(&syntax)
    }

    fn functions(self) -> Vec<FnDef> {
        let syntax = self.syntax_from_source();
        extract_functions(&syntax)
    }

    fn structs(self) -> Vec<StructDef> {
        let syntax = self.syntax_from_source();
        extract_structs(&syntax)
    }

    fn enums(self) -> Vec<EnumDef> {
        let syntax = self.syntax_from_source();
        extract_enums(&syntax)
    }

    fn impls(self) -> Vec<ImplDef> {
        let syntax = self.syntax_from_source();
        extract_impls(&syntax)
    }
}

fn build_workspace_context(
    _sema: &Semantics<RootDatabase>,
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    file_queue: Vec<EditionedFileId>,
    cargo_meta: &cargo_metadata::Metadata,
    verbose: bool,
) -> Result<WorkspaceContext> {
    let ctx_start = Instant::now();

    let root_norm = normalize_path_str(&project_dir.to_string_lossy());
    let manifest_start = Instant::now();
    let manifest = build_manifest(cargo_meta);
    println!("⏱ manifest build took {:.2?}", manifest_start.elapsed());

    // Build the CliOracle (shared across all file contexts for this run).
    let mut file_map: HashMap<String, EditionedFileId> = HashMap::new();
    // We'll populate file_map as we iterate the file queue below, then build
    // the oracle after (two-pass). Pre-populate to avoid a clone of file_queue.
    for editioned_id in &file_queue {
        let runtime_tmp = FileRuntime {
            shared: SharedRuntime {
                db: db as *const RootDatabase,
                vfs: vfs as *const Vfs,
                project_dir: project_dir as *const PathBuf,
                _not_send_sync: PhantomData,
            },
            editioned_id: *editioned_id,
        };
        if let Some(rel) = runtime_tmp.rel_path(&root_norm) {
            file_map.insert(rel, *editioned_id);
        }
    }

    let oracle = Arc::new(oracle::CliOracle {
        db: db as *const RootDatabase,
        vfs: vfs as *const Vfs,
        file_map,
        root_norm: root_norm.clone(),
        verbose,
    });
    let workspace_handle: SemanticHandle = oracle.clone().into_handle();

    let shared = SharedRuntime {
        db: db as *const RootDatabase,
        vfs: vfs as *const Vfs,
        project_dir: project_dir as *const PathBuf,
        _not_send_sync: PhantomData,
    };

    let mut files: Vec<ApiFileContext> = Vec::new();
    let mut paths_for_tree: Vec<String> = Vec::new();
    let mut skipped_files = 0usize;

    for editioned_id in file_queue {
        let runtime = FileRuntime {
            shared,
            editioned_id,
        };

        let Some(rel_path) = runtime.rel_path(&root_norm) else {
            skipped_files += 1;
            continue;
        };

        paths_for_tree.push(rel_path.clone());

        // ── Syntax pass (no RA) ───────────────────────────────────────────
        // Extract binding stubs from the CST, then enrich each unannotated
        // binding with a per-binding lazy closure that fires RA on first
        // `.ty()` call.  This is safe because the oracle (and therefore `db`)
        // remains valid for the lifetime of the `attach_db_allow_change` scope
        // that wraps both this build step AND `plugin.run()`.
        let syntax_bindings =
            oracle::extract_let_bindings_from_syntax(&runtime.syntax_from_source());
        let let_bindings: Vec<LetBinding> = {
            let oracle_for_bindings = Arc::clone(&oracle);
            let rel_for_bindings = rel_path.clone();
            syntax_bindings
                .into_iter()
                .map(move |b| {
                    let inferred_type = if b.explicit_type.is_some() {
                        // Annotated — no RA ever needed.
                        LazyValue::from_value(None)
                    } else if let Some(init_range) = b.initializer_range {
                        // Unannotated — defer to oracle on first `.ty()` call.
                        let o = Arc::clone(&oracle_for_bindings);
                        let fp = rel_for_bindings.clone();
                        LazyValue::new(move || {
                            let db = unsafe { &*o.db };
                            let sema_inner = ra_ap_hir::Semantics::new(db);
                            o.file_map.get(&fp).and_then(|&eid| {
                                oracle::infer_type_at_range(
                                    &sema_inner,
                                    db,
                                    eid,
                                    init_range,
                                    &fp,
                                    o.verbose,
                                )
                            })
                        })
                    } else {
                        LazyValue::from_value(None)
                    };
                    LetBinding { inferred_type, ..b }
                })
                .collect()
        };

        let file_handle: SemanticHandle = Arc::clone(&oracle).into_handle();

        let source_runtime = runtime;
        let generated_regions_runtime = runtime;
        let tree_runtime = runtime;
        let functions_runtime = runtime;
        let structs_runtime = runtime;
        let enums_runtime = runtime;
        let impls_runtime = runtime;

        files.push(ApiFileContext::new(
            rel_path,
            LazyValue::new(move || source_runtime.source()),
            LazyValue::new(move || {
                forgen_api::parse_generated_regions(&generated_regions_runtime.source())
            }),
            LazyValue::new(move || tree_runtime.tree()),
            LazyValue::from_value(let_bindings),
            LazyValue::new(move || functions_runtime.functions()),
            LazyValue::new(move || structs_runtime.structs()),
            LazyValue::new(move || enums_runtime.enums()),
            LazyValue::new(move || impls_runtime.impls()),
            Some(file_handle),
        ));
    }

    let file_tree_start = Instant::now();
    let file_tree_paths = paths_for_tree.clone();
    let file_tree = LazyValue::new(move || build_file_tree(&file_tree_paths));
    println!(
        "⏱ workspace context file handle build took {:.2?} (files={}, skipped={})",
        file_tree_start.elapsed(),
        files.len(),
        skipped_files
    );

    println!(
        "⏱ build_workspace_context total took {:.2?} (files={}, skipped={})",
        ctx_start.elapsed(),
        files.len(),
        skipped_files
    );

    Ok(WorkspaceContext::new(
        root_norm,
        files,
        manifest,
        file_tree,
        Some(workspace_handle),
    ))
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Collapse backslashes to forward slashes and strip the Windows extended-path
/// prefix (`\\?\` or `\\?\UNC\`) so that two paths referring to the same
/// location always compare equal as strings.
fn normalize_path_str(raw: &str) -> String {
    let s = raw.replace('\\', "/");
    // Strip \\?\ (becomes //?/ after backslash replacement)
    if let Some(rest) = s.strip_prefix("//?/UNC/") {
        return format!("//{}", rest);
    }
    if let Some(rest) = s.strip_prefix("//?/") {
        return rest.to_owned();
    }
    s
}

// ---------------------------------------------------------------------------
// CST converter  (ra_ap_syntax → forgen_api::syntax)
// ---------------------------------------------------------------------------

/// Converts a `ra_ap_syntax::SyntaxKind` to our `SyntaxKind` via the debug-
/// string name. This avoids binding to the internal numeric representation of
/// ra_ap_syntax and degrades gracefully (unknown → `ERROR`) when using an
/// older or newer version of ra_ap_syntax that has different variants.
fn convert_kind(k: ra_ap_syntax::SyntaxKind) -> SyntaxKind {
    type A = SyntaxKind;
    static MAP: OnceLock<HashMap<&'static str, SyntaxKind>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut m: HashMap<&'static str, SyntaxKind> = HashMap::with_capacity(320);
        macro_rules! ins {
            ($($n:ident),* $(,)?) => { $(m.insert(stringify!($n), A::$n);)* }
        }
        ins!(
            // Punctuation
            DOLLAR,
            SEMICOLON,
            COMMA,
            L_PAREN,
            R_PAREN,
            L_CURLY,
            R_CURLY,
            L_BRACK,
            R_BRACK,
            L_ANGLE,
            R_ANGLE,
            AT,
            POUND,
            TILDE,
            QUESTION,
            AMP,
            PIPE,
            PLUS,
            STAR,
            SLASH,
            CARET,
            PERCENT,
            UNDERSCORE,
            DOT,
            DOT2,
            DOT3,
            DOT2EQ,
            COLON,
            COLON2,
            EQ,
            EQ2,
            FAT_ARROW,
            BANG,
            NEQ,
            MINUS,
            THIN_ARROW,
            LTEQ,
            GTEQ,
            PLUSEQ,
            MINUSEQ,
            PIPEEQ,
            AMPEQ,
            CARETEQ,
            SLASHEQ,
            STAREQ,
            PERCENTEQ,
            AMP2,
            PIPE2,
            SHL,
            SHR,
            SHLEQ,
            SHREQ,
            // Strict keywords
            SELF_TYPE_KW,
            ABSTRACT_KW,
            AS_KW,
            BECOME_KW,
            BOX_KW,
            BREAK_KW,
            CONST_KW,
            CONTINUE_KW,
            CRATE_KW,
            DO_KW,
            ELSE_KW,
            ENUM_KW,
            EXTERN_KW,
            FALSE_KW,
            FINAL_KW,
            FN_KW,
            FOR_KW,
            IF_KW,
            IMPL_KW,
            IN_KW,
            LET_KW,
            LOOP_KW,
            MACRO_KW,
            MATCH_KW,
            MOD_KW,
            MOVE_KW,
            MUT_KW,
            OVERRIDE_KW,
            PRIV_KW,
            PUB_KW,
            REF_KW,
            RETURN_KW,
            SELF_KW,
            STATIC_KW,
            STRUCT_KW,
            SUPER_KW,
            TRAIT_KW,
            TRUE_KW,
            TYPE_KW,
            TYPEOF_KW,
            UNSAFE_KW,
            UNSIZED_KW,
            USE_KW,
            VIRTUAL_KW,
            WHERE_KW,
            WHILE_KW,
            YIELD_KW,
            // Contextual keywords
            ASM_KW,
            ASYNC_KW,
            ATT_SYNTAX_KW,
            AUTO_KW,
            BUILTIN_KW,
            CLOBBER_ABI_KW,
            DEFAULT_KW,
            DYN_KW,
            FORMAT_ARGS_KW,
            GEN_KW,
            GLOBAL_ASM_KW,
            LABEL_KW,
            MACRO_RULES_KW,
            NAKED_ASM_KW,
            OFFSET_OF_KW,
            OPTIONS_KW,
            PRESERVES_FLAGS_KW,
            PURE_KW,
            RAW_KW,
            READONLY_KW,
            SAFE_KW,
            SYM_KW,
            TRY_KW,
            UNION_KW,
            YEET_KW,
            // Literals
            BYTE,
            BYTE_STRING,
            CHAR,
            C_STRING,
            FLOAT_NUMBER,
            INT_NUMBER,
            STRING,
            // Trivia / special tokens
            COMMENT,
            ERROR,
            FRONTMATTER,
            IDENT,
            LIFETIME_IDENT,
            NEWLINE,
            SHEBANG,
            WHITESPACE,
            TOMBSTONE,
            // Composite node kinds
            ABI,
            ARG_LIST,
            ARRAY_EXPR,
            ARRAY_TYPE,
            ASM_CLOBBER_ABI,
            ASM_CONST,
            ASM_DIR_SPEC,
            ASM_EXPR,
            ASM_LABEL,
            ASM_OPERAND_EXPR,
            ASM_OPERAND_NAMED,
            ASM_OPTION,
            ASM_OPTIONS,
            ASM_REG_OPERAND,
            ASM_REG_SPEC,
            ASM_SYM,
            ASSOC_ITEM_LIST,
            ASSOC_TYPE_ARG,
            ATTR,
            AWAIT_EXPR,
            BECOME_EXPR,
            BIN_EXPR,
            BLOCK_EXPR,
            BOX_PAT,
            BREAK_EXPR,
            CALL_EXPR,
            CAST_EXPR,
            CLOSURE_EXPR,
            CONST,
            CONST_ARG,
            CONST_BLOCK_PAT,
            CONST_PARAM,
            CONTINUE_EXPR,
            DYN_TRAIT_TYPE,
            ENUM,
            EXPR_STMT,
            EXTERN_BLOCK,
            EXTERN_CRATE,
            EXTERN_ITEM_LIST,
            FIELD_EXPR,
            FN,
            FN_PTR_TYPE,
            FOR_BINDER,
            FOR_EXPR,
            FOR_TYPE,
            FORMAT_ARGS_ARG,
            FORMAT_ARGS_ARG_NAME,
            FORMAT_ARGS_EXPR,
            GENERIC_ARG_LIST,
            GENERIC_PARAM_LIST,
            IDENT_PAT,
            IF_EXPR,
            IMPL,
            IMPL_TRAIT_TYPE,
            INDEX_EXPR,
            INFER_TYPE,
            ITEM_LIST,
            LABEL,
            LET_ELSE,
            LET_EXPR,
            LET_STMT,
            LIFETIME,
            LIFETIME_ARG,
            LIFETIME_PARAM,
            LITERAL,
            LITERAL_PAT,
            LOOP_EXPR,
            MACRO_CALL,
            MACRO_DEF,
            MACRO_EXPR,
            MACRO_ITEMS,
            MACRO_PAT,
            MACRO_RULES,
            MACRO_STMTS,
            MACRO_TYPE,
            MATCH_ARM,
            MATCH_ARM_LIST,
            MATCH_EXPR,
            MATCH_GUARD,
            META,
            METHOD_CALL_EXPR,
            MODULE,
            NAME,
            NAME_REF,
            NEVER_TYPE,
            OFFSET_OF_EXPR,
            OR_PAT,
            PARAM,
            PARAM_LIST,
            PAREN_EXPR,
            PAREN_PAT,
            PAREN_TYPE,
            PARENTHESIZED_ARG_LIST,
            PATH,
            PATH_EXPR,
            PATH_PAT,
            PATH_SEGMENT,
            PATH_TYPE,
            PREFIX_EXPR,
            PTR_TYPE,
            RANGE_EXPR,
            RANGE_PAT,
            RECORD_EXPR,
            RECORD_EXPR_FIELD,
            RECORD_EXPR_FIELD_LIST,
            RECORD_FIELD,
            RECORD_FIELD_LIST,
            RECORD_PAT,
            RECORD_PAT_FIELD,
            RECORD_PAT_FIELD_LIST,
            REF_EXPR,
            REF_PAT,
            REF_TYPE,
            RENAME,
            REST_PAT,
            RET_TYPE,
            RETURN_EXPR,
            RETURN_TYPE_SYNTAX,
            SELF_PARAM,
            SLICE_PAT,
            SLICE_TYPE,
            SOURCE_FILE,
            STATIC,
            STMT_LIST,
            STRUCT,
            TOKEN_TREE,
            TRAIT,
            TRAIT_ALIAS,
            TRY_BLOCK_MODIFIER,
            TRY_EXPR,
            TUPLE_EXPR,
            TUPLE_FIELD,
            TUPLE_FIELD_LIST,
            TUPLE_PAT,
            TUPLE_STRUCT_PAT,
            TUPLE_TYPE,
            TYPE_ALIAS,
            TYPE_ANCHOR,
            TYPE_ARG,
            TYPE_BOUND,
            TYPE_BOUND_LIST,
            TYPE_PARAM,
            UNDERSCORE_EXPR,
            UNION,
            USE,
            USE_BOUND_GENERIC_ARGS,
            USE_TREE,
            USE_TREE_LIST,
            VARIANT,
            VARIANT_LIST,
            VISIBILITY,
            WHERE_CLAUSE,
            WHERE_PRED,
            WHILE_EXPR,
            WILDCARD_PAT,
            YEET_EXPR,
            YIELD_EXPR,
        );
        m
    });
    let s = format!("{k:?}");
    map.get(s.as_str()).copied().unwrap_or(SyntaxKind::ERROR)
}

/// Recursively serialise a `ra_ap_syntax::SyntaxNode` into a [`RawNode`],
/// preserving the full CST including whitespace and comment tokens.
fn build_raw_node(node: &ra_ap_syntax::SyntaxNode) -> RawNode {
    RawNode {
        kind: convert_kind(node.kind()),
        range: to_api_range(node.text_range()),
        children: node
            .children_with_tokens()
            .map(|child| match child {
                SyntaxElement::Node(n) => SyntaxChild::Node(build_raw_node(&n)),
                SyntaxElement::Token(t) => SyntaxChild::Token(RawToken {
                    kind: convert_kind(t.kind()),
                    text: t.text().to_string(),
                    range: to_api_range(t.text_range()),
                }),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Cargo manifest builder
// ---------------------------------------------------------------------------

fn build_manifest(meta: &cargo_metadata::Metadata) -> WorkspaceManifest {
    let members: Vec<PackageManifest> = meta
        .workspace_packages()
        .iter()
        .map(|pkg| {
            let mut deps = Vec::new();
            let mut dev_deps = Vec::new();
            let mut build_deps = Vec::new();

            for dep in &pkg.dependencies {
                let converted = convert_dependency(dep);
                match dep.kind {
                    CargoDep::Normal => deps.push(converted),
                    CargoDep::Development => dev_deps.push(converted),
                    CargoDep::Build => build_deps.push(converted),
                    _ => deps.push(converted),
                }
            }

            let features: HashMap<String, Vec<String>> = pkg
                .features
                .iter()
                .map(|(k, v): (&String, &Vec<String>)| (k.clone(), v.clone()))
                .collect();

            PackageManifest {
                name: pkg.name.clone(),
                version: pkg.version.to_string(),
                edition: pkg.edition.to_string(),
                authors: pkg.authors.clone(),
                description: pkg.description.clone(),
                license: pkg.license.clone(),
                repository: pkg.repository.clone(),
                dependencies: deps,
                dev_dependencies: dev_deps,
                build_dependencies: build_deps,
                features,
                metadata: pkg.metadata.clone(),
            }
        })
        .collect();

    WorkspaceManifest {
        members,
        workspace_root: meta.workspace_root.to_string().replace('\\', "/"),
        target_directory: meta.target_directory.to_string().replace('\\', "/"),
        metadata: meta.workspace_metadata.clone(),
    }
}

fn convert_dependency(dep: &cargo_metadata::Dependency) -> Dependency {
    let source = if let Some(path) = &dep.path {
        DependencySource::Path {
            path: path.to_string().replace('\\', "/"),
        }
    } else {
        match dep.source.as_deref() {
            Some(s) if s.starts_with("git+") => DependencySource::Git {
                url: s.to_string(),
                rev: None,
            },
            Some(_) => DependencySource::Registry,
            None => DependencySource::Unknown,
        }
    };

    Dependency {
        name: dep.name.clone(),
        rename: dep.rename.clone(),
        req: dep.req.to_string(),
        features: dep.features.clone(),
        optional: dep.optional,
        default_features: dep.uses_default_features,
        source,
    }
}

// ---------------------------------------------------------------------------
// File-tree builder
// ---------------------------------------------------------------------------

fn build_file_tree(paths: &[String]) -> DirNode {
    let mut root = DirNode {
        name: String::new(),
        path: String::new(),
        entries: Vec::new(),
    };
    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        insert_into_tree(&mut root, &parts, path);
    }
    sort_dir(&mut root);
    root
}

fn insert_into_tree(dir: &mut DirNode, remaining: &[&str], full_path: &str) {
    if remaining.is_empty() {
        return;
    }
    if remaining.len() == 1 {
        dir.entries.push(FsEntry::File(FileRef {
            name: remaining[0].to_string(),
            path: full_path.to_string(),
        }));
        return;
    }

    let dir_name = remaining[0];
    let existing_idx = dir
        .entries
        .iter()
        .position(|e| matches!(e, FsEntry::Dir(d) if d.name == dir_name));

    let idx = if let Some(i) = existing_idx {
        i
    } else {
        let dir_path = if dir.path.is_empty() {
            dir_name.to_string()
        } else {
            format!("{}/{}", dir.path, dir_name)
        };
        dir.entries.push(FsEntry::Dir(DirNode {
            name: dir_name.to_string(),
            path: dir_path,
            entries: Vec::new(),
        }));
        dir.entries.len() - 1
    };

    if let FsEntry::Dir(subdir) = &mut dir.entries[idx] {
        insert_into_tree(subdir, &remaining[1..], full_path);
    }
}

fn sort_dir(dir: &mut DirNode) {
    dir.entries.sort_by(|a, b| {
        let name_a = match a {
            FsEntry::Dir(d) => d.name.as_str(),
            FsEntry::File(f) => f.name.as_str(),
        };
        let name_b = match b {
            FsEntry::Dir(d) => d.name.as_str(),
            FsEntry::File(f) => f.name.as_str(),
        };
        name_a.cmp(name_b)
    });
    for entry in &mut dir.entries {
        if let FsEntry::Dir(subdir) = entry {
            sort_dir(subdir);
        }
    }
}

// ---------------------------------------------------------------------------
// AST → API type converters  (typed helper fields on FileContext)
// ---------------------------------------------------------------------------

#[inline]
fn to_api_range(r: ra_ap_syntax::TextRange) -> ApiTextRange {
    ApiTextRange {
        start: u32::from(r.start()),
        end: u32::from(r.end()),
    }
}

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

fn extract_functions(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<FnDef> {
    syntax
        .descendants()
        .filter_map(ast::Fn::cast)
        .filter_map(|fn_node| extract_fn_def(&fn_node))
        .collect()
}

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
