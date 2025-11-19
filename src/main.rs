use anyhow::{Context, Result};
use clap::Parser;
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{ChangeWithProcMacros, Crate, HirDisplay, ModuleDef};
use ra_ap_ide_db::RootDatabase;
use ra_ap_load_cargo::{load_workspace_at, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::{CargoConfig, ProjectManifest, RustLibSource};
use ra_ap_syntax::Edition;
use ra_ap_vfs::{Vfs, VfsPath};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::mpsc::channel, time::Duration};

/// Forgen - Enhanced compile-time macro information with type awareness
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to Cargo.toml (defaults to ./Cargo.toml in current directory)
    #[arg(value_name = "MANIFEST")]
    manifest: Option<PathBuf>,

    /// Watch for file changes and re-analyze (development mode)
    #[arg(short, long)]
    watch: bool,
}

// Data structures for serializing type information
#[derive(Debug, Serialize, Deserialize)]
struct TypeInfo {
    crates: Vec<CrateInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CrateInfo {
    name: String,
    items: Vec<ItemInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ItemInfo {
    Function {
        name: String,
        params: Vec<(String, String)>,
        return_type: String,
    },
    Struct {
        name: String,
        fields: Vec<(String, String)>,
    },
    Enum {
        name: String,
        variants: Vec<VariantInfo>,
    },
    Trait {
        name: String,
        items: Vec<String>,
    },
    TypeAlias {
        name: String,
        target: String,
    },
    Const {
        name: String,
        ty: String,
    },
    Static {
        name: String,
        ty: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct VariantInfo {
    name: String,
    fields: Vec<(String, String)>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("🚀 Forgen - Enhanced Macro Compiler Info");
    println!("=========================================\n");

    // Default to current directory's Cargo.toml if not specified
    let manifest_path = args.manifest.unwrap_or_else(|| PathBuf::from("Cargo.toml"));

    println!("📦 Loading project: {}", manifest_path.display());

    // Convert to absolute path
    let manifest_path_abs = manifest_path.canonicalize()?;
    let manifest_path_str = manifest_path_abs
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;
    let manifest_path = AbsPathBuf::try_from(manifest_path_str)
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

    // Load the workspace
    let (mut host, mut vfs) = load_workspace(&manifest_path)?;

    // Get the project directory for watching
    let project_dir = manifest_path_abs.parent().unwrap().to_path_buf();

    if args.watch {
        println!("👀 Watch mode enabled - monitoring for changes...\n");
        println!("Press Ctrl+C to stop\n");

        // Initial analysis
        analyze_and_save(&host, &vfs, &project_dir)?;

        // Setup file watcher
        let (tx, rx) = channel();
        let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;

        // Watch the src directory
        let src_path = project_dir.join("src");
        debouncer
            .watcher()
            .watch(&src_path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch {:?}", src_path))?;

        println!("📁 Watching: {}\n", src_path.display());

        // Watch loop
        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    // Check if any Rust files changed
                    let changed_files: Vec<_> = events
                        .iter()
                        .filter(|e| e.path.extension().and_then(|s| s.to_str()) == Some("rs"))
                        .map(|e| e.path.clone())
                        .collect();

                    if !changed_files.is_empty() {
                        println!("🔄 File change detected, re-analyzing...");

                        // Apply file changes incrementally (fast!)
                        match apply_file_changes(&mut host, &mut vfs, &changed_files) {
                            Ok(_) => {
                                // Re-analyze with the updated database
                                match analyze_and_save(&host, &vfs, &project_dir) {
                                    Ok(_) => println!("✅ Re-analysis complete\n"),
                                    Err(e) => eprintln!("❌ Error during re-analysis: {}\n", e),
                                }
                            },
                            Err(e) => eprintln!("❌ Error applying file changes: {}\n", e),
                        }
                    }
                },
                Ok(Err(e)) => eprintln!("Watch error: {:?}", e),
                Err(e) => {
                    eprintln!("Channel error: {:?}", e);
                    break;
                },
            }
        }
    } else {
        // Single run mode
        analyze_and_save(&host, &vfs, &project_dir)?;
        println!("\n✨ Analysis complete!");
    }

    Ok(())
}

fn apply_file_changes(
    host: &mut RootDatabase,
    vfs: &mut Vfs,
    changed_files: &[PathBuf],
) -> Result<()> {
    // Update VFS with the changed file contents
    for file_path in changed_files {
        // Read the new file contents
        let contents = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {:?}", file_path))?;

        // Convert PathBuf to AbsPathBuf (required by VfsPath)
        let abs_path = AbsPathBuf::try_from(
            file_path
                .canonicalize()?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?,
        )
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

        let vfs_path = VfsPath::from(abs_path);

        // Tell VFS about the change - it will track it internally
        vfs.set_file_contents(vfs_path, Some(contents.into_bytes()));
    }

    // Take all accumulated changes from VFS
    let vfs_changes = vfs.take_changes();

    // Apply changes to the database incrementally
    if !vfs_changes.is_empty() {
        let mut change = ChangeWithProcMacros::new();

        for (file_id, vfs_change) in vfs_changes {
            match vfs_change.change {
                ra_ap_vfs::Change::Create(contents, _) | ra_ap_vfs::Change::Modify(contents, _) => {
                    let text = String::from_utf8(contents)
                        .with_context(|| format!("File {:?} is not valid UTF-8", file_id))?;
                    change.change_file(file_id, Some(text));
                },
                ra_ap_vfs::Change::Delete => {
                    change.change_file(file_id, None);
                },
            }
        }

        // Apply all changes to the database at once - Salsa will handle incremental recomputation
        host.apply_change(change);
    }

    Ok(())
}

fn load_workspace(manifest_path: &AbsPathBuf) -> Result<(RootDatabase, Vfs)> {
    let _manifest = ProjectManifest::from_manifest_file(manifest_path.clone())
        .with_context(|| format!("Failed to load manifest at {:?}", manifest_path))?;

    // Configure cargo loading
    let cargo_config = CargoConfig {
        sysroot: Some(RustLibSource::Discover),
        ..Default::default()
    };

    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: true,
        with_proc_macro_server: ProcMacroServerChoice::Sysroot,
        prefill_caches: true,
    };

    let progress = |msg: String| {
        println!("  {}", msg);
    };

    // Load the workspace using rust-analyzer
    let project_dir = manifest_path.parent().unwrap();
    let (host, vfs, _proc_macro) =
        load_workspace_at(project_dir.as_ref(), &cargo_config, &load_config, &progress)
            .with_context(|| "Failed to load workspace")?;

    println!("✅ Workspace loaded successfully!\n");

    Ok((host, vfs))
}

fn analyze_and_save(db: &RootDatabase, _vfs: &Vfs, project_dir: &PathBuf) -> Result<()> {
    // Extract type information
    let type_info = extract_type_info(db)?;

    // Create output directory
    let output_dir = project_dir.join("target/forgen");
    fs::create_dir_all(&output_dir)?;

    // Save to JSON
    let output_file = output_dir.join("types.json");
    let json = serde_json::to_string_pretty(&type_info)?;
    fs::write(&output_file, json)?;

    println!("💾 Saved type information to: {}", output_file.display());

    Ok(())
}

fn extract_type_info(db: &RootDatabase) -> Result<TypeInfo> {
    let crates = Crate::all(db);

    // Filter to only workspace crates
    let workspace_crates: Vec<_> = crates
        .into_iter()
        .filter(|krate| krate.origin(db).is_local())
        .collect();

    let mut crate_infos = Vec::new();

    for krate in workspace_crates {
        let display_name = krate
            .display_name(db)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let mut items = Vec::new();
        let root_module = krate.root_module();

        extract_module_items(db, &root_module, &mut items)?;

        crate_infos.push(CrateInfo {
            name: display_name,
            items,
        });
    }

    Ok(TypeInfo {
        crates: crate_infos,
    })
}

fn extract_module_items(
    db: &RootDatabase,
    module: &ra_ap_hir::Module,
    items: &mut Vec<ItemInfo>,
) -> Result<()> {
    let edition = Edition::CURRENT;

    for def in module.declarations(db) {
        match def {
            ModuleDef::Function(func) => {
                items.push(ItemInfo::Function {
                    name: func.name(db).display(db, edition).to_string(),
                    params: func
                        .params_without_self(db)
                        .into_iter()
                        .enumerate()
                        .map(|(idx, param)| {
                            (
                                param
                                    .name(db)
                                    .map(|name| name.display(db, edition).to_string())
                                    .unwrap_or_else(|| idx.to_string()),
                                param.ty().display(db, edition).to_string(),
                            )
                        })
                        .collect(),
                    return_type: func.ret_type(db).display(db, edition).to_string(),
                });
            },

            ModuleDef::Adt(adt) => match adt {
                ra_ap_hir::Adt::Struct(struct_def) => {
                    items.push(ItemInfo::Struct {
                        name: struct_def.name(db).display(db, edition).to_string(),
                        fields: struct_def
                            .fields(db)
                            .into_iter()
                            .map(|field| {
                                (
                                    field.name(db).display(db, edition).to_string(),
                                    field.ty(db).display(db, edition).to_string(),
                                )
                            })
                            .collect(),
                    });
                },

                ra_ap_hir::Adt::Enum(enum_def) => {
                    let name = enum_def.name(db).display(db, edition).to_string();
                    let variants = enum_def
                        .variants(db)
                        .into_iter()
                        .map(|variant| VariantInfo {
                            name: variant.name(db).display(db, edition).to_string(),
                            fields: variant
                                .fields(db)
                                .into_iter()
                                .map(|field| {
                                    (
                                        field.name(db).display(db, edition).to_string(),
                                        field.ty(db).display(db, edition).to_string(),
                                    )
                                })
                                .collect(),
                        })
                        .collect();

                    items.push(ItemInfo::Enum { name, variants });
                },

                ra_ap_hir::Adt::Union(_) => {
                    // Skip unions for now
                },
            },

            ModuleDef::Trait(trait_def) => {
                items.push(ItemInfo::Trait {
                    name: trait_def.name(db).display(db, edition).to_string(),
                    items: trait_def
                        .items(db)
                        .into_iter()
                        .map(|item| match item {
                            ra_ap_hir::AssocItem::Function(func) => {
                                format!("fn {}", func.name(db).display(db, edition))
                            },
                            ra_ap_hir::AssocItem::TypeAlias(ty) => {
                                format!("type {}", ty.name(db).display(db, edition))
                            },
                            ra_ap_hir::AssocItem::Const(c) => {
                                format!(
                                    "const {}",
                                    c.name(db)
                                        .map(|n| n.display(db, edition).to_string())
                                        .unwrap_or_else(|| "_".to_string())
                                )
                            },
                        })
                        .collect(),
                });
            },

            ModuleDef::TypeAlias(type_alias) => {
                items.push(ItemInfo::TypeAlias {
                    name: type_alias.name(db).display(db, edition).to_string(),
                    target: type_alias.ty(db).display(db, edition).to_string(),
                });
            },

            ModuleDef::Const(const_def) => {
                items.push(ItemInfo::Const {
                    name: const_def
                        .name(db)
                        .map(|n| n.display(db, edition).to_string())
                        .unwrap_or_else(|| "_".to_string()),
                    ty: const_def.ty(db).display(db, edition).to_string(),
                });
            },

            ModuleDef::Static(static_def) => {
                items.push(ItemInfo::Static {
                    name: static_def.name(db).display(db, edition).to_string(),
                    ty: static_def.ty(db).display(db, edition).to_string(),
                });
            },

            ModuleDef::Module(submodule) => {
                // Recursively extract from submodules
                extract_module_items(db, &submodule, items)?;
            },

            _ => {},
        }
    }

    Ok(())
}
