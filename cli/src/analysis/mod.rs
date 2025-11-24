mod extensions;
mod extractors;
mod hir_ext;

pub use extensions::*;
pub use extractors::*;
pub use hir_ext::*;

use crate::models::{CrateMetadata, FileTypeInfo};
use crate::workspace::{file_id_to_path, load_cargo_metadata};
use anyhow::{Context as AnyhowContext, Result};
use ra_ap_hir::{Crate, HirDisplay, ModuleDef};
use ra_ap_ide_db::base_db::SourceDatabase;
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{ast, AstNode, Edition, SourceFile};
use ra_ap_vfs::Vfs;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

pub type FileInfoMap = BTreeMap<PathBuf, FileTypeInfo>;
pub type CrateMetadataMap = HashMap<String, CrateMetadata>;
pub type CargoInfoMap = HashMap<String, (String, Vec<String>)>;

/// Standard library crates to exclude from external analysis
const STD_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

pub struct AnalysisContext<'a> {
    pub db: &'a RootDatabase,
    pub vfs: &'a Vfs,
    pub referenced_types: &'a mut HashSet<String>,
    pub project_dir: &'a PathBuf,
    pub edition: Edition,
}

impl<'a> AnalysisContext<'a> {
    pub fn new(
        db: &'a RootDatabase,
        vfs: &'a Vfs,
        referenced_types: &'a mut HashSet<String>,
        project_dir: &'a PathBuf,
    ) -> Self {
        Self {
            db,
            vfs,
            referenced_types,
            project_dir,
            edition: Edition::CURRENT,
        }
    }

    pub fn display<T: HirDisplay>(&self, item: T) -> String {
        item.display(self.db, self.edition).to_string()
    }

    pub fn display_name(&self, name: ra_ap_hir::Name) -> String {
        name.display(self.db, self.edition).to_string()
    }
}

/// Extraction state to group mutable state
///
/// This struct consolidates the various pieces of state that are updated
/// during the extraction process, reducing the number of mutable references
/// that need to be passed between functions.
pub struct ExtractionState {
    pub file_infos: FileInfoMap,
    pub crate_metadata: CrateMetadataMap,
    pub referenced_types: HashSet<String>,
}

impl ExtractionState {
    pub fn new() -> Self {
        Self {
            file_infos: BTreeMap::new(),
            crate_metadata: HashMap::new(),
            referenced_types: HashSet::new(),
        }
    }
}

pub fn extract_type_info_by_file(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    manifest_path: &PathBuf,
) -> Result<(FileInfoMap, Vec<CrateMetadata>)> {
    let crates = Crate::all(db);
    let cargo_info = load_cargo_metadata(manifest_path).unwrap_or_default();

    let local_crates: Vec<_> = crates
        .iter()
        .filter(|krate| krate.origin(db).is_local())
        .cloned()
        .collect();

    let external_crates: Vec<_> = crates
        .into_iter()
        .filter(|krate| !{
            krate.origin(db).is_local()
                || STD_CRATES.contains(
                    &krate
                        .display_name(db)
                        .map(|n| n.to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
        })
        .collect();

    let mut state = ExtractionState::new();

    println!(
        "📊 Phase 1: Analyzing {} local crate(s)...",
        local_crates.len()
    );

    // Phase 1: Extract all local crates and collect type references
    for krate in &local_crates {
        process_crate(db, vfs, project_dir, krate, &cargo_info, &mut state, true).with_context(
            || {
                format!(
                    "Failed to process local crate '{}'",
                    krate
                        .display_name(db)
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "<unnamed>".to_string())
                )
            },
        )?;
    }

    let type_names: Vec<_> = state.referenced_types.iter().take(10).collect();
    println!(
        "  Collected {} referenced type names (first 10): {:?}",
        state.referenced_types.len(),
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
            let before_count = state.file_infos.len();
            process_crate(db, vfs, project_dir, krate, &cargo_info, &mut state, false)
                .with_context(|| format!("Failed to process external crate '{}'", display_name))?;
            let after_count = state.file_infos.len();

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

    let crate_metadata: Vec<CrateMetadata> = state.crate_metadata.into_values().collect();
    Ok((state.file_infos, crate_metadata))
}

/// Process a single crate (local or external)
///
/// For local crates:
/// - Extracts all type imports to track referenced types
/// - Analyzes all items in the crate
///
/// For external crates:
/// - Only analyzes items that are referenced by local code
/// - Applies visibility filtering (public items only)
fn process_crate(
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    krate: &Crate,
    cargo_info: &CargoInfoMap,
    state: &mut ExtractionState,
    is_local: bool,
) -> Result<()> {
    let root_module = krate.root_module();

    register_crate_metadata(db, krate, cargo_info, is_local, &mut state.crate_metadata);

    if is_local {
        extract_module_imports(db, &root_module, &mut state.referenced_types);
    }

    // Extract module items
    let mut ctx = AnalysisContext::new(db, vfs, &mut state.referenced_types, project_dir);
    extract_module_items_by_file(&mut ctx, &root_module, &mut state.file_infos, !is_local)?;

    Ok(())
}

fn register_crate_metadata(
    db: &RootDatabase,
    krate: &Crate,
    cargo_info: &CargoInfoMap,
    is_local: bool,
    crate_metadata_map: &mut CrateMetadataMap,
) {
    let display_name = krate
        .display_name(db)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "<unnamed>".to_string());

    crate_metadata_map
        .entry(display_name.clone())
        .or_insert_with(|| {
            let (version, features) = cargo_info
                .get(&display_name)
                .map(|(v, f)| (Some(v.clone()), f.clone()))
                .unwrap_or((None, Vec::new()));

            CrateMetadata::new(display_name.clone(), version, features, is_local)
        });
}

/// Recursively extract imports from a module and its submodules
fn extract_module_imports(
    db: &RootDatabase,
    module: &ra_ap_hir::Module,
    referenced_types: &mut HashSet<String>,
) {
    if let Some(file_id) = module.definition_source(db).file_id.file_id() {
        extract_imports_from_file(db, file_id.file_id(), referenced_types);
    }

    for child in module.children(db) {
        extract_module_imports(db, &child, referenced_types);
    }
}

/// Extract type names from imports in a file
fn extract_imports_from_file(
    db: &RootDatabase,
    file_id: ra_ap_ide_db::FileId,
    referenced_types: &mut HashSet<String>,
) {
    for node in SourceFile::parse(&*SourceDatabase::file_text(db, file_id), Edition::CURRENT)
        .syntax_node()
        .descendants()
    {
        if let Some(use_tree) = ast::UseTree::cast(node.clone()) {
            extract_use_tree_types(&use_tree, referenced_types);
        }
    }
}

/// Extract type names from a use tree
fn extract_use_tree_types(use_tree: &ast::UseTree, referenced_types: &mut HashSet<String>) {
    if let Some(path) = use_tree.path() {
        for segment in path.segments() {
            if let Some(name_ref) = segment.name_ref() {
                let name = name_ref.text().to_string();
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

/// Check if we should extract this definition
///
/// For local crates: Extract all definitions
/// For external crates: Only extract public definitions that are referenced
/// by local code (modules are always extracted for traversal)
fn should_extract_def(ctx: &AnalysisContext, def: &ModuleDef, is_external: bool) -> bool {
    if !is_external {
        return true;
    }

    // Modules are always extracted for traversal
    if matches!(def, ModuleDef::Module(_)) {
        return true;
    }

    if let Some(name) = def.display_name(ctx) {
        ctx.referenced_types.contains(&name)
    } else {
        false
    }
}

fn extract_module_items_by_file(
    ctx: &mut AnalysisContext,
    module: &ra_ap_hir::Module,
    file_infos: &mut FileInfoMap,
    is_external: bool,
) -> Result<()> {
    for def in module.declarations(ctx.db) {
        if !should_extract_def(ctx, &def, is_external) {
            continue;
        }

        if let ModuleDef::Module(m) = def {
            extract_module_items_by_file(ctx, &m, file_infos, is_external)?;
            continue;
        }

        let Some(file_id) = def.file_id(ctx) else {
            continue;
        };

        let Some(file_path) = file_id_to_path(ctx.vfs, file_id, ctx.project_dir) else {
            continue;
        };

        extract_item(
            ctx,
            &def,
            get_or_create_file_info(&file_path, ctx.project_dir, is_external, file_infos),
            is_external,
        )?;
    }

    Ok(())
}

fn get_or_create_file_info<'a>(
    file_path: &PathBuf,
    project_dir: &PathBuf,
    is_external: bool,
    file_infos: &'a mut FileInfoMap,
) -> &'a mut FileTypeInfo {
    file_infos
        .entry(file_path.clone())
        .or_insert_with(|| FileTypeInfo {
            source_file: normalize_file_path(file_path, project_dir, is_external),
            items: Vec::new(),
        })
}

/// Extract a single item based on its type
fn extract_item(
    ctx: &mut AnalysisContext,
    def: &ModuleDef,
    file_info: &mut FileTypeInfo,
    is_external: bool,
) -> Result<()> {
    match def {
        ModuleDef::Function(func) => {
            FunctionExtractor.extract(ctx, func, file_info, is_external)?;
        },
        ModuleDef::Adt(adt) => match adt {
            ra_ap_hir::Adt::Struct(s) => {
                StructExtractor.extract(ctx, s, file_info, is_external)?;
            },
            ra_ap_hir::Adt::Enum(e) => {
                EnumExtractor.extract(ctx, e, file_info, is_external)?;
            },
            ra_ap_hir::Adt::Union(_) => {}, // Skip unions for now
        },
        ModuleDef::Trait(t) => {
            TraitExtractor.extract(ctx, t, file_info, is_external)?;
        },
        ModuleDef::TypeAlias(t) => {
            TypeAliasExtractor.extract(ctx, t, file_info, is_external)?;
        },
        ModuleDef::Const(c) => {
            ConstExtractor.extract(ctx, c, file_info, is_external)?;
        },
        ModuleDef::Static(s) => {
            StaticExtractor.extract(ctx, s, file_info, is_external)?;
        },
        _ => {},
    }
    Ok(())
}
