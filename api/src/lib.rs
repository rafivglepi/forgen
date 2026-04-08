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
//! use forgen_api::{plugin_suite, FileReplacement, Plugin, PluginRuntime, Replacement, SuiteRuntime, WorkspaceContext};
//! use forgen_api::rand::Rng;
//!
//! #[derive(Default)]
//! pub struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn name(&self) -> &str {
//!         "my-plugin"
//!     }
//!
//!     fn run(
//!         &self,
//!         ctx: &WorkspaceContext,
//!         runtime: &mut PluginRuntime<'_>,
//!     ) -> Vec<FileReplacement> {
//!         let mut results = Vec::new();
//!
//!         for file in &ctx.files {
//!             let mut replacements = Vec::new();
//!             let mut rng = runtime.rng_for_file(&file.path);
//!
//!             for binding in file.bindings_of_type("f64") {
//!                 if file.generated_regions_for(runtime.plugin_id()).any(|region| {
//!                     let start = region.inner_range.start as usize;
//!                     let end = region.inner_range.end as usize;
//!                     file.source()
//!                         .get(start..end)
//!                         .map(|text| text.contains(&format!("{}:", binding.name)))
//!                         .unwrap_or(false)
//!                 }) {
//!                     continue;
//!                 }
//!
//!                 let insert_at = binding.range.end;
//!                 let indent = leading_indent(file.source(), insert_at);
//!                 let sample: u8 = rand::Rng::gen_range(&mut rng, 0..=9);
//!
//!                 replacements.push(Replacement::insert(
//!                     insert_at,
//!                     format!(
//!                         "\n{indent}println!(\"{name} [{sample}]: {{}}\", {name});",
//!                         indent = indent,
//!                         name = binding.name,
//!                         sample = sample,
//!                     ),
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
//! fn leading_indent(source: &str, offset: u32) -> String {
//!     let up_to = (offset as usize).min(source.len());
//!     let line_start = source[..up_to].rfind('\n').map(|i| i + 1).unwrap_or(0);
//!
//!     source[line_start..]
//!         .chars()
//!         .take_while(|c| *c == ' ' || *c == '\t')
//!         .collect()
//! }
//!
//! fn run(ctx: &WorkspaceContext, runtime: &mut SuiteRuntime) -> Vec<FileReplacement> {
//!     runtime.run_plugin(&MyPlugin, ctx)
//! }
//!
//! plugin_suite!(run);
//! ```

mod context;
pub mod manifest;
mod plugin;
pub mod query;
mod replacement;
mod runtime;
pub mod syntax;
pub mod tree;

// Re-export everything so plugin authors only need `use forgen_api::*;`
// (or cherry-pick individual names).

pub use context::{
    EnumDef, FieldDef, FileContext, FnDef, FnParam, ImplDef, LazyValue, LetBinding, StructDef,
    VariantDef, WorkspaceContext,
};
pub use manifest::{Dependency, DependencySource, PackageManifest, WorkspaceManifest};
pub use plugin::Plugin;
pub use query::{SemanticHandle, SemanticQuery, SemanticResult};
pub use rand;
pub use runtime::{
    is_valid_plugin_id, parse_generated_regions, GeneratedRegion, PluginRuntime, PluginState,
    SuiteRuntime,
};

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

// Re-export serde_json so the proc-macro helpers can reference it as
// `::serde_json::…` without requiring plugin crates to add their own
// serde_json dependency.
#[doc(hidden)]
pub use serde_json;
