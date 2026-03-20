use crate::syntax::raw::RawNode;
use crate::TextRange;
use crate::{manifest::WorkspaceManifest, tree::DirNode};
use serde::{Deserialize, Serialize};

/// The entire workspace handed to every plugin in one shot.
///
/// Rather than calling plugins once per file, Forgen builds a complete picture
/// of the workspace and passes it here. Plugins are therefore free to
/// cross-reference files (e.g. "insert a log line in `a.rs` only if `b.rs`
/// defines a certain type") without any extra plumbing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// Absolute path to the workspace root on disk.
    /// Forward slashes are used on all platforms.
    pub workspace_root: String,

    /// Every Rust source file reachable from a local crate in the workspace.
    pub files: Vec<FileContext>,

    /// Cargo metadata for the whole workspace.
    pub manifest: WorkspaceManifest,

    /// The source file tree rooted at the workspace root.
    pub file_tree: DirNode,
}

impl WorkspaceContext {
    /// Find a file by its workspace-relative path (forward slashes).
    pub fn file(&self, path: &str) -> Option<&FileContext> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Iterate over files whose path starts with `prefix`.
    pub fn files_in<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a FileContext> + 'a {
        self.files
            .iter()
            .filter(move |f| f.path.starts_with(prefix))
    }
}

/// Pre-computed information about a single Rust source file.
///
/// All type information is resolved by the Forgen runtime using rust-analyzer
/// before plugins are invoked. Plugins never need to depend on `ra_ap_*`
/// crates — they work exclusively with the data in this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    /// Path relative to the workspace root (forward slashes, no leading `./`).
    pub path: String,

    /// Raw UTF-8 source text exactly as it exists on disk.
    pub source: String,

    /// The full CST of this file, with all trivia (whitespace, comments).
    pub tree: RawNode,

    /// Every `let` binding in the file, across all scopes (flattened).
    /// This includes bindings inside function bodies, closures, blocks, etc.
    pub let_bindings: Vec<LetBinding>,

    /// All function and method definitions in the file (both top-level
    /// functions and methods inside `impl` blocks are included).
    pub functions: Vec<FnDef>,

    /// Struct definitions.
    pub structs: Vec<StructDef>,

    /// Enum definitions.
    pub enums: Vec<EnumDef>,

    /// `impl` blocks (both inherent impls and trait impls).
    /// Note: the methods in each `ImplDef` also appear in `functions` for
    /// convenient flat iteration.
    pub impls: Vec<ImplDef>,
}

impl FileContext {
    /// Find a `let` binding by variable name.
    pub fn binding(&self, name: &str) -> Option<&LetBinding> {
        self.let_bindings.iter().find(|b| b.name == name)
    }

    /// Iterate over all bindings whose effective type equals `ty`.
    pub fn bindings_of_type<'a>(
        &'a self,
        ty: &'a str,
    ) -> impl Iterator<Item = &'a LetBinding> + 'a {
        self.let_bindings.iter().filter(move |b| b.ty() == Some(ty))
    }

    /// Find a function definition by name.
    pub fn function(&self, name: &str) -> Option<&FnDef> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// Find a struct definition by name.
    pub fn struct_def(&self, name: &str) -> Option<&StructDef> {
        self.structs.iter().find(|s| s.name == name)
    }

    /// Find an enum definition by name.
    pub fn enum_def(&self, name: &str) -> Option<&EnumDef> {
        self.enums.iter().find(|e| e.name == name)
    }
}

// ---------------------------------------------------------------------------
// Let bindings
// ---------------------------------------------------------------------------

/// A single `let` (or `let mut`) binding extracted from a source file.
///
/// Only simple identifier patterns are captured (`let [mut] name [: T] = …`).
/// Destructuring patterns are skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    /// The bound variable name (e.g. `x` from `let x: f64 = 1.0`).
    pub name: String,

    /// The explicit type annotation as written in source, if present.
    /// Example: `"f64"` from `let x: f64 = …`.
    pub explicit_type: Option<String>,

    /// The type inferred by the compiler, only present when there is no
    /// explicit annotation. Populated by the Forgen runtime via rust-analyzer.
    /// Example: `"f64"` from `let x = 1.0_f64`.
    pub inferred_type: Option<String>,

    /// Byte range of the entire `let … ;` statement (including the semicolon).
    pub range: TextRange,

    /// Whether the binding was declared `let mut`.
    pub is_mut: bool,
}

impl LetBinding {
    /// Returns the effective type of this binding.
    ///
    /// Prefers the explicit annotation; falls back to the inferred type.
    /// Returns `None` only when neither is available (e.g. inference failed).
    pub fn ty(&self) -> Option<&str> {
        self.explicit_type
            .as_deref()
            .or(self.inferred_type.as_deref())
    }

    /// Returns `true` if this binding's effective type matches `ty`.
    pub fn has_type(&self, ty: &str) -> bool {
        self.ty() == Some(ty)
    }
}

// ---------------------------------------------------------------------------
// Functions / methods
// ---------------------------------------------------------------------------

/// A function or method definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnDef {
    /// The function name.
    pub name: String,

    /// Parameters (excludes the `self` receiver, if any — see `has_self`).
    pub params: Vec<FnParam>,

    /// Whether this function/method has a `self` / `&self` / `&mut self`
    /// receiver as its first parameter.
    pub has_self: bool,

    /// Return type as written in source, or `None` for `-> ()` / no annotation.
    pub return_type: Option<String>,

    /// Byte range of the entire function (signature + body).
    pub range: TextRange,

    /// Whether the function is declared `pub` (any form of `pub`).
    pub is_pub: bool,

    /// Whether the function is `async`.
    pub is_async: bool,
}

/// A single parameter of a function or method (excluding `self` receivers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnParam {
    /// Parameter name as written in source, or `_` for unnamed parameters.
    pub name: String,

    /// Type as written in source.
    pub ty: Option<String>,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// A struct definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructDef {
    /// The struct name.
    pub name: String,

    /// Named fields. Empty for unit structs and tuple structs (see `tuple_fields`).
    pub fields: Vec<FieldDef>,

    /// For tuple structs: the positional fields in declaration order.
    /// The `name` of each entry is its zero-based index as a string ("0", "1", …).
    pub tuple_fields: Vec<FieldDef>,

    /// Byte range of the entire struct definition.
    pub range: TextRange,

    /// Whether the struct is declared `pub` (any form of `pub`).
    pub is_pub: bool,
}

impl StructDef {
    /// Returns `true` if this is a tuple struct (e.g. `struct Foo(i32, f64)`).
    pub fn is_tuple(&self) -> bool {
        !self.tuple_fields.is_empty()
    }

    /// Returns `true` if this is a unit struct (e.g. `struct Foo;`).
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty() && self.tuple_fields.is_empty()
    }
}

/// A named field inside a struct or enum variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    /// Field name (or positional index string for tuple fields).
    pub name: String,

    /// Type as written in source.
    pub ty: String,

    /// Whether the field is declared `pub` (any form of `pub`).
    pub is_pub: bool,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// An enum definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDef {
    /// The enum name.
    pub name: String,

    /// Variants in declaration order.
    pub variants: Vec<VariantDef>,

    /// Byte range of the entire enum definition.
    pub range: TextRange,

    /// Whether the enum is declared `pub` (any form of `pub`).
    pub is_pub: bool,
}

/// A single variant of an enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantDef {
    /// Variant name.
    pub name: String,

    /// Named fields for struct-like variants (`Variant { x: f64, y: f64 }`).
    pub fields: Vec<FieldDef>,

    /// Positional fields for tuple-like variants (`Variant(i32, f64)`).
    /// The `name` of each entry is its zero-based index as a string.
    pub tuple_fields: Vec<FieldDef>,
}

impl VariantDef {
    /// Returns `true` if this is a unit variant (no fields).
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty() && self.tuple_fields.is_empty()
    }

    /// Returns `true` if this is a struct-like variant.
    pub fn is_struct(&self) -> bool {
        !self.fields.is_empty()
    }

    /// Returns `true` if this is a tuple-like variant.
    pub fn is_tuple(&self) -> bool {
        !self.tuple_fields.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Impl blocks
// ---------------------------------------------------------------------------

/// An `impl` block (either inherent or trait implementation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplDef {
    /// The type being implemented for, as written in source (e.g. `"MyStruct"`
    /// or `"MyStruct<T>"`).
    pub self_ty: String,

    /// The trait being implemented, if this is a trait impl
    /// (e.g. `"Display"` from `impl Display for MyStruct`).
    pub trait_: Option<String>,

    /// Methods defined in this impl block.
    /// These same `FnDef` values also appear in `FileContext::functions`.
    pub methods: Vec<FnDef>,

    /// Byte range of the entire `impl` block (including the opening brace).
    pub range: TextRange,
}

impl ImplDef {
    /// Returns `true` if this is a trait implementation.
    pub fn is_trait_impl(&self) -> bool {
        self.trait_.is_some()
    }

    /// Find a method by name.
    pub fn method(&self, name: &str) -> Option<&FnDef> {
        self.methods.iter().find(|m| m.name == name)
    }
}
