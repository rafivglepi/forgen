use anyhow::{Context, Result};
use cargo_metadata::MetadataCommand;
use clap::Parser;
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{ChangeWithProcMacros, Crate, HasSource, HirDisplay, ModuleDef, Semantics};
use ra_ap_ide_db::base_db::SourceDatabase;
use ra_ap_ide_db::{FileId, RootDatabase};
use ra_ap_load_cargo::{load_workspace_at, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::{CargoConfig, ProjectManifest, RustLibSource};
use ra_ap_syntax::{ast, ast::HasName, AstNode, Edition, SourceFile};
use ra_ap_vfs::{Vfs, VfsPath};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::mpsc::channel,
    time::Duration,
};

// Helper function to skip serializing if value is None
fn skip_if_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}

// Helper to skip empty vecs
fn skip_if_empty<T>(vec: &Vec<T>) -> bool {
    vec.is_empty()
}

// Helper to skip if value is "<inferred>"
fn skip_if_inferred(s: &str) -> bool {
    s == "<inferred>"
}

// Helper to serialize bool as 0 or 1
fn serialize_bool_as_int<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u8(if *value { 1 } else { 0 })
}

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

    /// Include external library type information (dependencies from crates.io, etc.)
    #[arg(short = 'e', long)]
    include_external: bool,
}

// Data structures for serializing type information

/// Root structure containing all extracted type information
#[derive(Debug, Serialize, Deserialize)]
struct ForgenOutput {
    /// Metadata about crates
    crates: Vec<CrateMetadata>,
    /// Type information per file
    files: Vec<FileTypeInfo>,
}

/// Type information for a single file
#[derive(Debug, Serialize, Deserialize)]
struct FileTypeInfo {
    /// Path to the original source file (relative to project root)
    #[serde(rename = "path")]
    source_file: String,
    /// Items declared in this file
    items: Vec<ItemInfo>,
}

// Helper to skip if value is false (for local field)
fn skip_if_false(b: &bool) -> bool {
    !*b
}

/// Metadata about the entire crate
#[derive(Debug, Serialize, Deserialize)]
struct CrateMetadata {
    /// Crate name
    name: String,
    /// Crate version
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    /// Enabled features
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    features: Vec<String>,
    /// Is this a local workspace crate? (1 = local, omitted = external)
    #[serde(
        rename = "local",
        serialize_with = "serialize_bool_as_int",
        skip_serializing_if = "skip_if_false"
    )]
    is_local: bool,
}

/// Reference to an item defined elsewhere (for cross-file references)
#[derive(Debug, Serialize, Deserialize)]
struct ItemRef {
    /// Path to the item (e.g., "std::vec::Vec" or "crate::module::Type")
    path: String,
    /// HIR id for precise lookup
    id: String,
    /// File where this is defined (for local items)
    #[serde(skip_serializing_if = "skip_if_none")]
    defined_in: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ItemInfo {
    Function {
        name: String,
        /// HIR id for cross-referencing
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        params: Vec<ParamInfo>,
        #[serde(rename = "ret")]
        return_type: String,
        /// Information about the function body
        #[serde(skip_serializing_if = "skip_if_empty_body")]
        body: Option<FunctionBodyInfo>,
    },
    Struct {
        name: String,
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        fields: Vec<FieldInfo>,
    },
    Enum {
        name: String,
        id: String,
        variants: Vec<VariantInfo>,
    },
    Trait {
        name: String,
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        items: Vec<TraitItemInfo>,
    },
    TypeAlias {
        name: String,
        id: String,
        target: String,
    },
    Const {
        name: String,
        id: String,
        ty: String,
    },
    Static {
        name: String,
        id: String,
        ty: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ParamInfo {
    name: String,
    #[serde(skip_serializing_if = "skip_if_inferred")]
    ty: String,
    /// Reference to the type definition if available
    #[serde(rename = "ref", skip_serializing_if = "skip_if_none")]
    type_ref: Option<ItemRef>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FieldInfo {
    name: String,
    ty: String,
    #[serde(rename = "ref", skip_serializing_if = "skip_if_none")]
    type_ref: Option<ItemRef>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VariantInfo {
    name: String,
    #[serde(skip_serializing_if = "skip_if_empty")]
    fields: Vec<FieldInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TraitItemInfo {
    Function {
        name: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        params: Vec<ParamInfo>,
        #[serde(rename = "ret")]
        return_type: String,
    },
    TypeAlias {
        name: String,
    },
    Const {
        name: String,
        ty: String,
    },
}

/// Information about a function's body - locals, closures, etc.
#[derive(Debug, Serialize, Deserialize)]
struct FunctionBodyInfo {
    /// Local variables in the function
    #[serde(skip_serializing_if = "skip_if_empty")]
    locals: Vec<LocalVarInfo>,
    /// Closures defined in the function
    #[serde(skip_serializing_if = "skip_if_empty")]
    closures: Vec<ClosureInfo>,
}

impl FunctionBodyInfo {
    fn is_empty(&self) -> bool {
        self.locals.is_empty() && self.closures.is_empty()
    }
}

// Helper to skip empty body
fn skip_if_empty_body(body: &Option<FunctionBodyInfo>) -> bool {
    body.as_ref().map(|b| b.is_empty()).unwrap_or(true)
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalVarInfo {
    /// Variable name (if available)
    #[serde(skip_serializing_if = "skip_if_none")]
    name: Option<String>,
    /// Type of the variable (skip if inferred)
    #[serde(skip_serializing_if = "skip_if_inferred")]
    ty: String,
    /// Unique identifier for this local within the function (just a number)
    id: usize,
    /// Is this variable mutable? (0 = false, 1 = true)
    #[serde(rename = "mut", serialize_with = "serialize_bool_as_int")]
    mutable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClosureInfo {
    /// Unique identifier for this closure within the function (just a number)
    id: usize,
    /// Parameters of the closure
    #[serde(skip_serializing_if = "skip_if_empty")]
    params: Vec<ParamInfo>,
    /// Return type
    #[serde(rename = "ret", skip_serializing_if = "skip_if_inferred")]
    return_type: String,
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
        analyze_and_save(
            &host,
            &vfs,
            &project_dir,
            args.include_external,
            &manifest_path_abs,
        )?;

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
                                match analyze_and_save(
                                    &host,
                                    &vfs,
                                    &project_dir,
                                    args.include_external,
                                    &manifest_path_abs,
                                ) {
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
        analyze_and_save(
            &host,
            &vfs,
            &project_dir,
            args.include_external,
            &manifest_path_abs,
        )?;
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
    let project_dir_ref: &std::path::Path = project_dir.as_ref();
    let (host, vfs, _proc_macro) =
        load_workspace_at(project_dir_ref, &cargo_config, &load_config, &progress)
            .with_context(|| "Failed to load workspace")?;

    println!("✅ Workspace loaded successfully!\n");

    Ok((host, vfs))
}

fn analyze_and_save(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    include_external: bool,
    manifest_path: &PathBuf,
) -> Result<()> {
    // Extract type information per file
    let (file_infos, crate_metadata) =
        extract_type_info_by_file(db, vfs, project_dir, include_external, manifest_path)?;

    // Create output directory
    let output_dir = project_dir.join("target");
    fs::create_dir_all(&output_dir)?;

    // Convert the map into a vec of FileTypeInfo
    let files: Vec<FileTypeInfo> = file_infos.into_values().collect();

    // Create the complete output structure
    let output = ForgenOutput {
        crates: crate_metadata,
        files,
    };

    // Save everything to a single .forgen.json file (minified)
    let output_file = output_dir.join(".forgen.json");
    let json = serde_json::to_string(&output)?;
    fs::write(&output_file, json)?;

    println!("💾 Saved type information to: {}", output_file.display());
    println!("✨ Analyzed {} files", output.files.len());

    Ok(())
}

fn load_cargo_metadata(manifest_path: &PathBuf) -> Result<HashMap<String, (String, Vec<String>)>> {
    // Load cargo metadata to get version and features
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .context("Failed to load cargo metadata")?;

    let mut crate_info = HashMap::new();
    for package in metadata.packages {
        // Store version and active features for each package
        let info = (
            package.version.to_string(),
            package.features.keys().cloned().collect(),
        );

        // Insert with original name
        crate_info.insert(package.name.clone(), info.clone());

        // Also insert with underscores (Cargo.toml uses hyphens, Rust uses underscores)
        let underscore_name = package.name.replace('-', "_");
        if underscore_name != package.name {
            crate_info.insert(underscore_name, info);
        }
    }

    Ok(crate_info)
}

fn extract_type_info_by_file(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    include_external: bool,
    manifest_path: &PathBuf,
) -> Result<(BTreeMap<PathBuf, FileTypeInfo>, Vec<CrateMetadata>)> {
    let sema = Semantics::new(db);
    let crates = Crate::all(db);

    // Load cargo metadata for version and features
    let cargo_info = load_cargo_metadata(manifest_path).unwrap_or_default();

    // Separate local and external crates
    let local_crates: Vec<_> = crates
        .iter()
        .filter(|krate| krate.origin(db).is_local())
        .cloned()
        .collect();

    let external_crates: Vec<_> = if include_external {
        crates
            .into_iter()
            .filter(|krate| {
                if krate.origin(db).is_local() {
                    return false;
                }
                let name = krate
                    .display_name(db)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                // Filter out standard library crates to avoid noise
                !matches!(
                    name.as_str(),
                    "std" | "core" | "alloc" | "proc_macro" | "test"
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut file_infos: BTreeMap<PathBuf, FileTypeInfo> = BTreeMap::new();
    let mut crate_metadata_map: HashMap<String, CrateMetadata> = HashMap::new();
    let mut referenced_types: HashSet<String> = HashSet::new();

    println!(
        "📊 Phase 1: Analyzing {} local crate(s)...",
        local_crates.len()
    );

    // Phase 1: Extract all local crates and collect type references
    for krate in &local_crates {
        let display_name = krate
            .display_name(db)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let root_module = krate.root_module();
        crate_metadata_map
            .entry(display_name.clone())
            .or_insert_with(|| {
                let (version, features) = cargo_info
                    .get(&display_name)
                    .map(|(v, f)| (Some(v.clone()), f.clone()))
                    .unwrap_or((None, Vec::new()));

                CrateMetadata {
                    name: display_name.clone(),
                    version,
                    features,
                    is_local: true,
                }
            });

        // Extract imports from all files in the local crate first
        extract_module_imports(db, &root_module, &mut referenced_types);

        extract_module_items_by_file(
            db,
            &sema,
            vfs,
            &root_module,
            &mut file_infos,
            project_dir,
            &mut referenced_types,
            false, // is_external
        )?;
    }

    let type_names: Vec<_> = referenced_types.iter().take(10).collect();
    println!(
        "  Collected {} referenced type names (first 10): {:?}",
        referenced_types.len(),
        type_names
    );

    // Phase 2: Extract external crates, but only public items that are referenced
    if !external_crates.is_empty() {
        println!(
            "📊 Phase 2: Analyzing {} external crate(s) (filtering to reachable types)...",
            external_crates.len()
        );

        let mut extracted_external_files = 0;

        for krate in &external_crates {
            let display_name = krate
                .display_name(db)
                .map(|n| n.to_string())
                .unwrap_or_else(|| "<unnamed>".to_string());

            let root_module = krate.root_module();

            crate_metadata_map
                .entry(display_name.clone())
                .or_insert_with(|| {
                    let (version, features) = cargo_info
                        .get(&display_name)
                        .map(|(v, f)| (Some(v.clone()), f.clone()))
                        .unwrap_or((None, Vec::new()));

                    CrateMetadata {
                        name: display_name.clone(),
                        version,
                        features,
                        is_local: false,
                    }
                });

            let before_count = file_infos.len();
            extract_module_items_by_file_filtered(
                db,
                &sema,
                vfs,
                &root_module,
                &mut file_infos,
                project_dir,
                &mut referenced_types,
                true, // is_external
            )?;
            let after_count = file_infos.len();
            if after_count > before_count {
                extracted_external_files += after_count - before_count;
                println!(
                    "  Found {} items in {}",
                    after_count - before_count,
                    display_name
                );
            }
        }

        println!("  Extracted {} external files", extracted_external_files);
    }

    let crate_metadata: Vec<CrateMetadata> = crate_metadata_map.into_values().collect();
    Ok((file_infos, crate_metadata))
}

fn file_id_to_path(vfs: &Vfs, file_id: FileId, _project_dir: &PathBuf) -> Option<PathBuf> {
    let vfs_path = vfs.file_path(file_id);
    let abs_path = vfs_path.as_path()?;
    Some(PathBuf::from(<ra_ap_paths::AbsPath as AsRef<
        std::path::Path,
    >>::as_ref(abs_path)))
}

/// Helper to extract type names from a type string
fn extract_type_names(type_str: &str, referenced_types: &mut HashSet<String>) {
    // Extract base type names from type strings like "Vec<String>", "&Option<Foo>", etc.
    // This is a simple heuristic - we extract capitalized words that look like type names
    for word in type_str.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty() && word.chars().next().unwrap().is_uppercase() {
            // Filter out common non-types
            if !matches!(
                word,
                "Self" | "Result" | "Option" | "Vec" | "Box" | "Arc" | "Rc" | "Cow"
            ) {
                referenced_types.insert(word.to_string());
            }
        }
    }
}

/// Extract type names from imports in a file
fn extract_imports_from_file(
    db: &RootDatabase,
    file_id: FileId,
    referenced_types: &mut HashSet<String>,
) {
    // Get file text from the database
    let file_text = SourceDatabase::file_text(db, file_id);
    let text = &*file_text;

    // Parse the file to extract use statements
    let parsed = SourceFile::parse(text, Edition::CURRENT);
    let root = parsed.syntax_node();

    // Walk the syntax tree to find use items
    for node in root.descendants() {
        if let Some(use_tree) = ast::UseTree::cast(node.clone()) {
            // Extract the path from the use tree
            if let Some(path) = use_tree.path() {
                // Get each segment of the path (e.g., "serde::Serialize" -> ["serde", "Serialize"])
                for segment in path.segments() {
                    if let Some(name_ref) = segment.name_ref() {
                        let name = name_ref.text().to_string();
                        // Check if it looks like a type (starts with uppercase)
                        if !name.is_empty() && name.chars().next().unwrap().is_uppercase() {
                            referenced_types.insert(name);
                        }
                    }
                }
            }

            // Also check for use tree lists like {Serialize, Deserialize}
            if let Some(use_tree_list) = use_tree.use_tree_list() {
                for sub_tree in use_tree_list.use_trees() {
                    if let Some(path) = sub_tree.path() {
                        for segment in path.segments() {
                            if let Some(name_ref) = segment.name_ref() {
                                let name = name_ref.text().to_string();
                                if !name.is_empty() && name.chars().next().unwrap().is_uppercase() {
                                    referenced_types.insert(name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Recursively extract imports from a module and its submodules
fn extract_module_imports(
    db: &RootDatabase,
    module: &ra_ap_hir::Module,
    referenced_types: &mut HashSet<String>,
) {
    // Get the file for this module
    let module_source = module.definition_source(db);
    if let Some(file_id) = module_source.file_id.file_id() {
        extract_imports_from_file(db, file_id.file_id(), referenced_types);
    }

    // Recurse into submodules
    for child in module.children(db) {
        extract_module_imports(db, &child, referenced_types);
    }
}

/// Check if an item is public (visible outside the crate)
fn is_public(_db: &RootDatabase, def: &ModuleDef) -> bool {
    // Check if the item's visibility is Public
    // We do this by checking if it has a `pub` visibility
    // For external crates, we assume all items we can see are somewhat public
    // So we'll just return true for simplicity - the main filtering will be by name
    match def {
        ModuleDef::Module(_) => true, // Modules are traversable
        _ => {
            // For now, assume all items we can extract from external crates are public enough
            // The real filtering happens by checking if they're referenced
            true
        },
    }
}

/// Filtered version for external crates - only public and referenced types
fn extract_module_items_by_file_filtered(
    db: &RootDatabase,
    sema: &Semantics<RootDatabase>,
    vfs: &Vfs,
    module: &ra_ap_hir::Module,
    file_infos: &mut BTreeMap<PathBuf, FileTypeInfo>,
    project_dir: &PathBuf,
    referenced_types: &mut HashSet<String>,
    is_external: bool,
) -> Result<()> {
    // For external crates, we'll do multiple passes to find transitively referenced types
    let mut newly_found = HashSet::new();
    let initial_count = referenced_types.len();

    // Do extraction, which will add more referenced types
    extract_module_items_by_file(
        db,
        sema,
        vfs,
        module,
        file_infos,
        project_dir,
        referenced_types,
        is_external,
    )?;

    // Check if we found new types
    for ty in referenced_types.iter() {
        if referenced_types.len() > initial_count {
            newly_found.insert(ty.clone());
        }
    }

    // TODO: Could iterate until no new types found for full transitive closure
    // For now, one pass is good enough

    Ok(())
}

fn extract_module_items_by_file(
    db: &RootDatabase,
    sema: &Semantics<RootDatabase>,
    vfs: &Vfs,
    module: &ra_ap_hir::Module,
    file_infos: &mut BTreeMap<PathBuf, FileTypeInfo>,
    project_dir: &PathBuf,
    referenced_types: &mut HashSet<String>,
    is_external: bool,
) -> Result<()> {
    let edition = Edition::CURRENT;

    for def in module.declarations(db) {
        // For external crates, skip non-public items and non-referenced types
        if is_external {
            if !is_public(db, &def) {
                continue; // Skip private items from external crates
            }

            // Check if this type is referenced (only for non-module items)
            if !matches!(def, ModuleDef::Module(_)) {
                let name = match &def {
                    ModuleDef::Function(f) => f.name(db).display(db, edition).to_string(),
                    ModuleDef::Adt(adt) => match adt {
                        ra_ap_hir::Adt::Struct(s) => s.name(db).display(db, edition).to_string(),
                        ra_ap_hir::Adt::Enum(e) => e.name(db).display(db, edition).to_string(),
                        ra_ap_hir::Adt::Union(u) => u.name(db).display(db, edition).to_string(),
                    },
                    ModuleDef::Trait(t) => t.name(db).display(db, edition).to_string(),
                    ModuleDef::TypeAlias(t) => t.name(db).display(db, edition).to_string(),
                    _ => continue,
                };

                if !referenced_types.contains(&name) {
                    continue; // Skip unreferenced types from external crates
                }
            }
            // Always recurse into modules to find types deep in the crate structure
        }

        // Get the file where this definition lives
        let file_id = match def {
            ModuleDef::Function(f) => f
                .source(db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Adt(adt) => match adt {
                ra_ap_hir::Adt::Struct(s) => s
                    .source(db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
                ra_ap_hir::Adt::Enum(e) => e
                    .source(db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
                ra_ap_hir::Adt::Union(u) => u
                    .source(db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
            },
            ModuleDef::Trait(t) => t
                .source(db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::TypeAlias(t) => t
                .source(db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Const(c) => c
                .source(db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Static(s) => s
                .source(db)
                .and_then(|src| src.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Module(m) => {
                // Recursively process submodules
                extract_module_items_by_file(
                    db,
                    sema,
                    vfs,
                    &m,
                    file_infos,
                    project_dir,
                    referenced_types,
                    is_external,
                )?;
                continue;
            },
            _ => None,
        };

        let Some(file_id) = file_id else {
            continue;
        };

        let Some(file_path) = file_id_to_path(vfs, file_id, project_dir) else {
            continue;
        };

        // Get or create the FileTypeInfo for this file
        let file_info = file_infos.entry(file_path.clone()).or_insert_with(|| {
            // For external crates, strip the registry/toolchain path prefix
            let source_file_str = if is_external {
                let path_str = file_path.to_str().unwrap_or("<unknown>");
                // Try to find common external crate path patterns and extract just the relative part
                // Pattern: C:\Users\..\.cargo\registry\src\index.../crate-version\src\file.rs -> src\file.rs
                // Pattern: C:\Users\..\.rustup\toolchains\...\library\crate\src\file.rs -> crate\src\file.rs

                if let Some(registry_pos) = path_str.find("\\.cargo\\registry\\src\\") {
                    // Skip past the registry path and index hash
                    let after_registry = registry_pos + "\\.cargo\\registry\\src\\".len();
                    // Find the next directory separator after the index hash (e.g., "index.crates.io-1949...")
                    if let Some(hash_end) = path_str[after_registry..].find('\\') {
                        let after_hash = after_registry + hash_end + 1;
                        // Now skip the crate-version directory to get to src/...
                        if let Some(crate_end) = path_str[after_hash..].find('\\') {
                            let relative_start = after_hash + crate_end + 1;
                            &path_str[relative_start..]
                        } else {
                            path_str
                        }
                    } else {
                        path_str
                    }
                } else if let Some(toolchain_pos) = path_str.find("\\.rustup\\toolchains\\") {
                    // For stdlib crates, find /library/ and keep from there
                    if let Some(lib_pos) = path_str[toolchain_pos..].find("\\library\\") {
                        let after_lib = toolchain_pos + lib_pos + "\\library\\".len();
                        &path_str[after_lib..]
                    } else {
                        path_str
                    }
                } else {
                    path_str
                }
                .to_string()
            } else {
                // For local files, strip project directory
                file_path
                    .strip_prefix(project_dir)
                    .unwrap_or(&file_path)
                    .to_str()
                    .unwrap_or("<unknown>")
                    .to_string()
            };

            FileTypeInfo {
                source_file: source_file_str,
                items: Vec::new(),
            }
        });

        // Extract item information
        match def {
            ModuleDef::Function(func) => {
                // Minify HIR id by removing spaces
                let hir_id = format!("{:?}", func).replace(" ", "");

                let params: Vec<ParamInfo> = func
                    .params_without_self(db)
                    .into_iter()
                    .enumerate()
                    .map(|(idx, param)| {
                        let ty_str = param.ty().display(db, edition).to_string();
                        extract_type_names(&ty_str, referenced_types);
                        ParamInfo {
                            name: param
                                .name(db)
                                .map(|name| name.display(db, edition).to_string())
                                .unwrap_or_else(|| format!("_{}", idx)),
                            ty: ty_str,
                            type_ref: None, // TODO: extract type references
                        }
                    })
                    .collect();

                let ret_type_str = func.ret_type(db).display(db, edition).to_string();
                extract_type_names(&ret_type_str, referenced_types);

                // Extract function body information (skip for external crates)
                let body = if !is_external {
                    extract_function_body(db, sema, &func, edition)
                } else {
                    None
                };

                file_info.items.push(ItemInfo::Function {
                    name: func.name(db).display(db, edition).to_string(),
                    id: hir_id,
                    params,
                    return_type: ret_type_str,
                    body,
                });
            },

            ModuleDef::Adt(adt) => match adt {
                ra_ap_hir::Adt::Struct(struct_def) => {
                    let hir_id = format!("{:?}", struct_def).replace(" ", "");
                    let fields: Vec<FieldInfo> = struct_def
                        .fields(db)
                        .into_iter()
                        .map(|field| {
                            let ty_str = field.ty(db).display(db, edition).to_string();
                            extract_type_names(&ty_str, referenced_types);
                            FieldInfo {
                                name: field.name(db).display(db, edition).to_string(),
                                ty: ty_str,
                                type_ref: None,
                            }
                        })
                        .collect();

                    file_info.items.push(ItemInfo::Struct {
                        name: struct_def.name(db).display(db, edition).to_string(),
                        id: hir_id,
                        fields,
                    });
                },

                ra_ap_hir::Adt::Enum(enum_def) => {
                    let hir_id = format!("{:?}", enum_def).replace(" ", "");
                    let name = enum_def.name(db).display(db, edition).to_string();
                    let variants = enum_def
                        .variants(db)
                        .into_iter()
                        .map(|variant| {
                            let fields: Vec<FieldInfo> = variant
                                .fields(db)
                                .into_iter()
                                .map(|field| {
                                    let ty_str = field.ty(db).display(db, edition).to_string();
                                    extract_type_names(&ty_str, referenced_types);
                                    FieldInfo {
                                        name: field.name(db).display(db, edition).to_string(),
                                        ty: ty_str,
                                        type_ref: None,
                                    }
                                })
                                .collect();
                            VariantInfo {
                                name: variant.name(db).display(db, edition).to_string(),
                                fields,
                            }
                        })
                        .collect();

                    file_info.items.push(ItemInfo::Enum {
                        name,
                        id: hir_id,
                        variants,
                    });
                },

                ra_ap_hir::Adt::Union(_) => {
                    // Skip unions for now
                },
            },

            ModuleDef::Trait(trait_def) => {
                let hir_id = format!("{:?}", trait_def).replace(" ", "");
                file_info.items.push(ItemInfo::Trait {
                    name: trait_def.name(db).display(db, edition).to_string(),
                    id: hir_id,
                    items: trait_def
                        .items(db)
                        .into_iter()
                        .map(|item| match item {
                            ra_ap_hir::AssocItem::Function(func) => {
                                let params: Vec<ParamInfo> = func
                                    .params_without_self(db)
                                    .into_iter()
                                    .enumerate()
                                    .map(|(idx, param)| {
                                        let ty_str = param.ty().display(db, edition).to_string();
                                        extract_type_names(&ty_str, referenced_types);
                                        ParamInfo {
                                            name: param
                                                .name(db)
                                                .map(|name| name.display(db, edition).to_string())
                                                .unwrap_or_else(|| format!("_{}", idx)),
                                            ty: ty_str,
                                            type_ref: None,
                                        }
                                    })
                                    .collect();
                                let ret_type_str =
                                    func.ret_type(db).display(db, edition).to_string();
                                extract_type_names(&ret_type_str, referenced_types);
                                TraitItemInfo::Function {
                                    name: func.name(db).display(db, edition).to_string(),
                                    params,
                                    return_type: ret_type_str,
                                }
                            },
                            ra_ap_hir::AssocItem::TypeAlias(ty) => TraitItemInfo::TypeAlias {
                                name: ty.name(db).display(db, edition).to_string(),
                            },
                            ra_ap_hir::AssocItem::Const(c) => {
                                let ty_str = c.ty(db).display(db, edition).to_string();
                                extract_type_names(&ty_str, referenced_types);
                                TraitItemInfo::Const {
                                    name: c
                                        .name(db)
                                        .map(|n| n.display(db, edition).to_string())
                                        .unwrap_or_else(|| "_".to_string()),
                                    ty: ty_str,
                                }
                            },
                        })
                        .collect(),
                });
            },

            ModuleDef::TypeAlias(type_alias) => {
                let hir_id = format!("{:?}", type_alias).replace(" ", "");
                let target_str = type_alias.ty(db).display(db, edition).to_string();
                extract_type_names(&target_str, referenced_types);
                file_info.items.push(ItemInfo::TypeAlias {
                    name: type_alias.name(db).display(db, edition).to_string(),
                    id: hir_id,
                    target: target_str,
                });
            },

            ModuleDef::Const(const_def) => {
                let hir_id = format!("{:?}", const_def).replace(" ", "");
                let ty_str = const_def.ty(db).display(db, edition).to_string();
                extract_type_names(&ty_str, referenced_types);
                file_info.items.push(ItemInfo::Const {
                    name: const_def
                        .name(db)
                        .map(|n| n.display(db, edition).to_string())
                        .unwrap_or_else(|| "_".to_string()),
                    id: hir_id,
                    ty: ty_str,
                });
            },

            ModuleDef::Static(static_def) => {
                let hir_id = format!("{:?}", static_def).replace(" ", "");
                let ty_str = static_def.ty(db).display(db, edition).to_string();
                extract_type_names(&ty_str, referenced_types);
                file_info.items.push(ItemInfo::Static {
                    name: static_def.name(db).display(db, edition).to_string(),
                    id: hir_id,
                    ty: ty_str,
                });
            },

            _ => {},
        }
    }

    Ok(())
}

/// Extract information about a function's body, including local variables and closures
/// This is a best-effort extraction that doesn't require full semantic analysis
fn extract_function_body(
    db: &RootDatabase,
    _sema: &Semantics<RootDatabase>,
    func: &ra_ap_hir::Function,
    _edition: Edition,
) -> Option<FunctionBodyInfo> {
    // Get the function's HIR body
    let body_source = func.source(db)?;
    let fn_node = body_source.value;

    // Try to get body expression/block
    let body_expr = fn_node.body()?;

    let mut locals = Vec::new();
    let mut closures = Vec::new();

    // Walk through the syntax tree to find local bindings
    // Note: We're doing a simplified syntax-only extraction here to avoid Semantics issues
    // For full type inference, we would need to use HIR's body map directly
    for node in body_expr.syntax().descendants() {
        // Look for let statements (local variables)
        if let Some(let_stmt) = ast::LetStmt::cast(node.clone()) {
            if let Some(pat) = let_stmt.pat() {
                // Check if it's mutable and get the name
                let (is_mut, name) = match &pat {
                    ast::Pat::IdentPat(ident_pat) => {
                        let is_mut = ident_pat.mut_token().is_some();
                        let name = ident_pat.name().map(|n| n.to_string());
                        (is_mut, name)
                    },
                    _ => (false, None),
                };

                // Try to get explicit type annotation from the let statement
                let ty = if let Some(ty_node) = let_stmt.ty() {
                    ty_node.syntax().text().to_string()
                } else {
                    // For now, we can't infer types without full HIR body analysis
                    // This would require using the function's body_source_map
                    "<inferred>".to_string()
                };

                locals.push(LocalVarInfo {
                    name,
                    ty,
                    id: locals.len(),
                    mutable: is_mut,
                });
            }
        }

        // Look for closures
        if let Some(closure) = ast::ClosureExpr::cast(node) {
            let closure_params: Vec<ParamInfo> = closure
                .param_list()
                .map(|params| {
                    params
                        .params()
                        .enumerate()
                        .map(|(idx, param)| {
                            let name = param
                                .pat()
                                .and_then(|p| {
                                    if let ast::Pat::IdentPat(ident) = p {
                                        ident.name().map(|n| n.to_string())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| format!("_{}", idx));

                            // Get explicit type annotation if present
                            let ty = param
                                .ty()
                                .map(|ty_node| ty_node.syntax().text().to_string())
                                .unwrap_or_else(|| "<inferred>".to_string());

                            ParamInfo {
                                name,
                                ty,
                                type_ref: None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Get explicit return type if present
            let return_type = closure
                .ret_type()
                .map(|ret| ret.syntax().text().to_string())
                .unwrap_or_else(|| "<inferred>".to_string());

            closures.push(ClosureInfo {
                id: closures.len(),
                params: closure_params,
                return_type,
            });
        }
    }

    Some(FunctionBodyInfo { locals, closures })
}
