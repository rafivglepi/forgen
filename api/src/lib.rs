//! # forgen-api
//!
//! Plugins are compiled as `cdylib` crates and only need to depend on this
//! crate — **no `ra_ap_*` dependencies required**. All type resolution is
//! performed by the Forgen runtime before plugins are invoked.
//!
//! ## Quick start
//!
//! ```toml
//! # Cargo.toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! forgen-api = { path = "../api" }   # or a crates.io version once published
//! ```
//!
//! ```rust,no_run
//! use forgen_api::{plugin_export, FileReplacement, Plugin, Replacement, WorkspaceContext};
//!
//! #[derive(Default)]
//! pub struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn name(&self) -> &str {
//!         "my-plugin"
//!     }
//!
//!     fn run(&self, ctx: &WorkspaceContext) -> Vec<FileReplacement> {
//!         let mut results = Vec::new();
//!
//!         for file in &ctx.files {
//!             let mut replacements = Vec::new();
//!
//!             for binding in file.bindings_of_type("f64") {
//!                 replacements.push(Replacement::insert(
//!                     binding.range.end,
//!                     format!("\nprintln!(\"{}: {{}}\", {});", binding.name, binding.name),
//!                 ));
//!             }
//!
//!             if !replacements.is_empty() {
//!                 results.push(FileReplacement::new(file.path.clone(), replacements));
//!             }
//!         }
//!
//!         results
//!     }
//! }
//!
//! plugin_export!(MyPlugin, "my-plugin");
//! ```

mod context;
pub mod manifest;
mod plugin;
mod replacement;
pub mod syntax;
pub mod tree;

// Re-export everything so plugin authors only need `use forgen_api::*;`
// (or cherry-pick individual names).

pub use context::{
    EnumDef, FieldDef, FileContext, FnDef, FnParam, ImplDef, LetBinding, StructDef, VariantDef,
    WorkspaceContext,
};
pub use manifest::{Dependency, DependencySource, PackageManifest, WorkspaceManifest};
pub use plugin::Plugin;

/// Compile-time FNV-1a hash of a byte string.
///
/// Used to derive [`FORGEN_ABI_VERSION`] from the crate's Cargo version
/// string so that the constant never needs to be bumped by hand.
const fn fnv1a(s: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < s.len() {
        hash ^= s[i] as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
        i += 1;
    }
    hash
}

/// ABI version of `forgen-api`, automatically derived from the crate's
/// `Cargo.toml` version string via a compile-time FNV-1a hash.
///
/// Both the CLI and the suite dylib embed this constant at compile
/// time.  Before the CLI passes any pointers into the dylib it checks that
/// the two values match; a mismatch means the suite was built against
/// a different release of `forgen-api` and the load is aborted with a clear
/// error message.
///
/// **You never need to bump this by hand.** Incrementing the `[package]
/// version` in `forgen-api/Cargo.toml` changes the hash automatically.
pub const FORGEN_ABI_VERSION: u64 = fnv1a(env!("CARGO_PKG_VERSION").as_bytes());
pub use replacement::{FileReplacement, Replacement, TextRange};
pub use tree::{DirNode, FileRef, FsEntry};

// Re-export serde_json so the `plugin_export!` macro can reference it as
// `::serde_json::…` without requiring plugin crates to add their own
// serde_json dependency.
#[doc(hidden)]
pub use serde_json;
