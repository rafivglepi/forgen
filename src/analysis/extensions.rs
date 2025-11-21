use std::collections::HashSet;
use std::path::PathBuf;

pub fn extract_type_names(type_str: &str, referenced_types: &mut HashSet<String>) {
    for word in type_str.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty() && word.chars().next().unwrap().is_uppercase() {
            referenced_types.insert(word.to_string());
        }
    }
}

/// Normalize file path for external vs local crates
pub fn normalize_file_path(
    file_path: &PathBuf,
    project_dir: &PathBuf,
    is_external: bool,
) -> String {
    let file_path: &std::path::Path = file_path.as_path();
    let project_dir: &std::path::Path = project_dir.as_path();

    if !is_external {
        return file_path
            .strip_prefix(project_dir)
            .unwrap_or(file_path)
            .to_string_lossy()
            .into_owned();
    }

    let comps: Vec<_> = file_path.components().collect();

    if let Some(src_index) = comps.iter().position(|c| c.as_os_str() == "src") {
        if src_index > 0 {
            return comps[src_index - 1..]
                .iter()
                .collect::<PathBuf>()
                .to_string_lossy()
                .into_owned();
        }
    }

    file_path.to_string_lossy().into_owned()
}
