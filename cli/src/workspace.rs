use anyhow::{Context, Result};
use cargo_metadata::MetadataCommand;
use ra_ap_hir::ChangeWithProcMacros;
use ra_ap_ide_db::RootDatabase;
use ra_ap_load_cargo::{load_workspace_at, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_vfs::{Change, Vfs, VfsPath};
use std::{collections::HashMap, fs, path::PathBuf};

pub struct WorkspaceInfo {
    pub root: PathBuf,
    pub members: Vec<PathBuf>,
}

pub fn get_workspace_info(manifest_path: &PathBuf) -> Result<WorkspaceInfo> {
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .context("Failed to load cargo metadata")?;

    let mut members = Vec::new();
    for package in metadata.workspace_packages() {
        let src_path = package.manifest_path.parent().unwrap().join("src");
        if src_path.exists() {
            members.push(src_path.into_std_path_buf());
        }
    }

    let root = metadata.workspace_root.into_std_path_buf();

    // If no workspace members found, fall back to root/src if it exists
    if members.is_empty() {
        let root_src = root.join("src");
        if root_src.exists() {
            members.push(root_src);
        }
    }

    Ok(WorkspaceInfo { root, members })
}

pub fn load_workspace(manifest_path: &AbsPathBuf) -> Result<(RootDatabase, Vfs)> {
    let (host, vfs, _proc_macro) = load_workspace_at(
        manifest_path.parent().unwrap().as_ref(),
        &CargoConfig {
            sysroot: Some(RustLibSource::Discover),
            ..Default::default()
        },
        &LoadCargoConfig {
            load_out_dirs_from_check: true,
            with_proc_macro_server: ProcMacroServerChoice::Sysroot,
            prefill_caches: true,
        },
        &|msg: String| {
            println!("  {}", msg);
        },
    )
    .with_context(|| "Failed to load workspace")?;

    println!("✅ Workspace loaded successfully!\n");

    Ok((host, vfs))
}

pub fn load_cargo_metadata(
    manifest_path: &PathBuf,
) -> Result<HashMap<String, (String, Vec<String>)>> {
    let mut crate_info = HashMap::new();

    for package in MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .context("Failed to load cargo metadata")?
        .packages
    {
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

pub fn apply_file_changes(
    host: &mut RootDatabase,
    vfs: &mut Vfs,
    changed_files: &[PathBuf],
) -> Result<()> {
    for file_path in changed_files {
        vfs.set_file_contents(
            VfsPath::from(
                AbsPathBuf::try_from(
                    file_path
                        .canonicalize()?
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?,
                )
                .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?,
            ),
            Some(
                fs::read_to_string(file_path)
                    .with_context(|| format!("Failed to read file: {:?}", file_path))?
                    .into_bytes(),
            ),
        );
    }

    let vfs_changes = vfs.take_changes();

    if !vfs_changes.is_empty() {
        let mut change = ChangeWithProcMacros::new();

        for (file_id, vfs_change) in vfs_changes {
            match vfs_change.change {
                Change::Create(contents, _) | Change::Modify(contents, _) => {
                    let text = String::from_utf8(contents)
                        .with_context(|| format!("File {:?} is not valid UTF-8", file_id))?;
                    change.change_file(file_id, Some(text));
                },
                Change::Delete => {
                    change.change_file(file_id, None);
                },
            }
        }

        host.apply_change(change);
    }

    Ok(())
}

pub fn file_id_to_path(
    vfs: &Vfs,
    file_id: ra_ap_ide_db::FileId,
    _project_dir: &PathBuf,
) -> Option<PathBuf> {
    Some(PathBuf::from(<ra_ap_paths::AbsPath as AsRef<
        std::path::Path,
    >>::as_ref(
        vfs.file_path(file_id).as_path()?
    )))
}
