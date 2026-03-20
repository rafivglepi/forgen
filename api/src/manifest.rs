use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metadata about the whole Cargo workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    /// All packages that are members of the workspace.
    pub members: Vec<PackageManifest>,
    /// The absolute path to the workspace root on disk (forward slashes).
    pub workspace_root: String,
    /// The absolute path to the target directory (forward slashes).
    pub target_directory: String,
    /// The raw `[workspace.metadata]` table, if present.
    pub metadata: serde_json::Value,
}

impl WorkspaceManifest {
    /// Find a package by name.
    pub fn package(&self, name: &str) -> Option<&PackageManifest> {
        self.members.iter().find(|p| p.name == name)
    }

    /// Read a typed value from `[workspace.metadata.forgen.<key>]`.
    /// Returns `None` if the key is absent or cannot be deserialized to `T`.
    pub fn forgen_metadata<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let val = self.metadata.get("forgen")?.get(key)?;
        serde_json::from_value(val.clone()).ok()
    }
}

/// Metadata about a single Cargo package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    /// Normal (non-dev, non-build) dependencies.
    pub dependencies: Vec<Dependency>,
    /// Dev dependencies.
    pub dev_dependencies: Vec<Dependency>,
    /// Build dependencies.
    pub build_dependencies: Vec<Dependency>,
    /// Feature definitions (feature name → list of enabled features/deps).
    pub features: HashMap<String, Vec<String>>,
    /// Raw `[package.metadata]` table, if present.
    pub metadata: serde_json::Value,
}

impl PackageManifest {
    /// Returns `true` if this package declares the given feature.
    pub fn has_feature(&self, name: &str) -> bool {
        self.features.contains_key(name)
    }

    /// Find a dependency by name (searches normal deps only).
    pub fn dependency(&self, name: &str) -> Option<&Dependency> {
        self.dependencies
            .iter()
            .find(|d| d.name == name || d.rename.as_deref() == Some(name))
    }
}

/// A single Cargo dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// The crate name as in crates.io / the registry.
    pub name: String,
    /// The `rename` field if `package = "…"` was used.
    pub rename: Option<String>,
    /// The version requirement string (e.g. `"1.0"`, `">=2, <3"`).
    pub req: String,
    /// Features explicitly enabled for this dependency.
    pub features: Vec<String>,
    /// Whether this dependency is optional.
    pub optional: bool,
    /// Whether default features are enabled.
    pub default_features: bool,
    /// Where this dependency comes from.
    pub source: DependencySource,
}

/// Where a dependency is sourced from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DependencySource {
    /// A crates.io or registry dependency.
    Registry,
    /// A path dependency.
    Path { path: String },
    /// A git dependency.
    Git { url: String, rev: Option<String> },
    /// Unknown source.
    Unknown,
}
