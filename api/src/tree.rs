use serde::{Deserialize, Serialize};

/// A directory node in the workspace file tree.
///
/// Every [`WorkspaceContext`] carries a `file_tree` rooted at the workspace
/// root so plugins can make decisions based on folder structure without doing
/// string manipulation on paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirNode {
    /// The directory name (e.g. `"src"`, `"tests"`).
    pub name: String,
    /// Workspace-relative path (forward slashes, no leading `./`).
    /// Empty string for the workspace root.
    pub path: String,
    /// All direct children of this directory.
    pub entries: Vec<FsEntry>,
}

impl DirNode {
    /// Find a direct child directory by name.
    pub fn dir(&self, name: &str) -> Option<&DirNode> {
        self.entries.iter().find_map(|e| {
            if let FsEntry::Dir(d) = e {
                if d.name == name {
                    return Some(d);
                }
            }
            None
        })
    }

    /// Find a direct child file by name.
    pub fn file(&self, name: &str) -> Option<&FileRef> {
        self.entries.iter().find_map(|e| {
            if let FsEntry::File(f) = e {
                if f.name == name {
                    return Some(f);
                }
            }
            None
        })
    }

    /// Recursively collect all file paths in this subtree.
    pub fn all_files(&self) -> Vec<&FileRef> {
        let mut out = Vec::new();
        collect_files(self, &mut out);
        out
    }

    /// Recursively find the first directory whose name matches `name`.
    pub fn find_dir(&self, name: &str) -> Option<&DirNode> {
        if self.name == name {
            return Some(self);
        }
        for entry in &self.entries {
            if let FsEntry::Dir(d) = entry {
                if let Some(found) = d.find_dir(name) {
                    return Some(found);
                }
            }
        }
        None
    }
}

fn collect_files<'a>(dir: &'a DirNode, out: &mut Vec<&'a FileRef>) {
    for entry in &dir.entries {
        match entry {
            FsEntry::File(f) => out.push(f),
            FsEntry::Dir(d) => collect_files(d, out),
        }
    }
}

/// Either a subdirectory or a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsEntry {
    Dir(DirNode),
    File(FileRef),
}

/// A reference to a source file within the tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    /// Just the filename, e.g. `"lib.rs"`.
    pub name: String,
    /// Workspace-relative path (forward slashes). Matches `FileContext::path`.
    pub path: String,
}
