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
mod plugin;
mod replacement;

// Re-export everything so plugin authors only need `use forgen_api::*;`
// (or cherry-pick individual names).

pub use context::{
    EnumDef, FieldDef, FileContext, FnDef, FnParam, ImplDef, LetBinding, StructDef, VariantDef,
    WorkspaceContext,
};
pub use plugin::Plugin;
pub use replacement::{FileReplacement, Replacement, TextRange};

// Re-export serde_json so the `plugin_export!` macro can reference it as
// `::serde_json::…` without requiring plugin crates to add their own
// serde_json dependency.
#[doc(hidden)]
pub use serde_json;
