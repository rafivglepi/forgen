use crate::query::SemanticHandle;
use crate::syntax::raw::RawNode;
use crate::TextRange;
use crate::{manifest::WorkspaceManifest, tree::DirNode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

type LazyInit<T> = dyn Fn() -> T + Send + Sync + 'static;

/// A small runtime-only lazy cell used by the Forgen CLI to defer expensive
/// per-file and per-workspace computations until plugins actually request
/// them.
///
/// This intentionally lives in `forgen-api` so both the CLI and plugin dylibs
/// see the exact same type layout.
pub struct LazyValue<T> {
    value: OnceLock<T>,
    init: Arc<LazyInit<T>>,
}

impl<T> LazyValue<T> {
    /// Create a new lazy value from an initializer closure.
    pub fn new<F>(init: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self {
            value: OnceLock::new(),
            init: Arc::new(init),
        }
    }

    /// Create a lazy value that is already initialized.
    pub fn from_value(value: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        let cell: OnceLock<T> = OnceLock::new();
        let _ = cell.set(value);
        Self {
            value: cell,
            init: Arc::new(|| panic!("LazyValue initializer was called for an eager value")),
        }
    }

    /// Get the value, initializing it on first access.
    pub fn get(&self) -> &T {
        self.value.get_or_init(|| (self.init)())
    }

    /// Returns `true` if this value has already been initialized.
    pub fn is_initialized(&self) -> bool {
        self.value.get().is_some()
    }
}

impl<T: Clone + Send + Sync + 'static> Clone for LazyValue<T> {
    fn clone(&self) -> Self {
        // If already initialized, reuse the value; otherwise share the init fn.
        match self.value.get() {
            Some(v) => LazyValue::from_value(v.clone()),
            None => LazyValue {
                value: OnceLock::new(),
                init: Arc::clone(&self.init),
            },
        }
    }
}

impl Default for LazyValue<Option<String>> {
    fn default() -> Self {
        LazyValue::from_value(None)
    }
}

impl<T> Deref for LazyValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T: fmt::Debug> fmt::Debug for LazyValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value.get() {
            Some(value) => f.debug_tuple("LazyValue").field(value).finish(),
            None => f.write_str("LazyValue(<uninitialized>)"),
        }
    }
}

/// The entire workspace handed to every plugin in one shot.
///
/// Rather than calling plugins once per file, Forgen builds a complete picture
/// of the workspace and passes it here. Plugins are therefore free to
/// cross-reference files (e.g. "insert a log line in `a.rs` only if `b.rs`
/// defines a certain type") without any extra plumbing.
///
/// Expensive workspace-wide data may be computed lazily.
#[derive(Debug)]
pub struct WorkspaceContext {
    /// Absolute path to the workspace root on disk.
    /// Forward slashes are used on all platforms.
    pub workspace_root: String,

    /// Every Rust source file reachable from a local crate in the workspace.
    ///
    /// The list itself is eager, but the expensive per-file contents behind
    /// each [`FileContext`] are lazy.
    pub files: Vec<FileContext>,

    /// Cargo metadata for the whole workspace.
    pub manifest: WorkspaceManifest,

    file_tree: LazyValue<DirNode>,

    /// Oracle for semantic (RA-backed) queries across the workspace.
    /// `None` when running without a live rust-analyzer context (tests).
    semantics: Option<SemanticHandle>,
}

impl WorkspaceContext {
    /// Create a new workspace context.
    pub fn new(
        workspace_root: String,
        files: Vec<FileContext>,
        manifest: WorkspaceManifest,
        file_tree: LazyValue<DirNode>,
        semantics: Option<SemanticHandle>,
    ) -> Self {
        Self {
            workspace_root,
            files,
            manifest,
            file_tree,
            semantics,
        }
    }

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

    /// The source file tree rooted at the workspace root.
    pub fn file_tree(&self) -> &DirNode {
        self.file_tree.get()
    }

    /// Returns `true` if the file tree has already been computed.
    pub fn is_file_tree_initialized(&self) -> bool {
        self.file_tree.is_initialized()
    }

    /// Oracle for semantic (RA-backed) queries on the whole workspace.
    /// `None` when running without a live rust-analyzer context (tests).
    pub fn semantics(&self) -> Option<&SemanticHandle> {
        self.semantics.as_ref()
    }
}

/// Information about a single Rust source file.
///
/// The file identity (`path`) is always cheap and eager. Expensive derived data
/// such as the CST, symbol lists, and inferred `let` binding types may be
/// computed lazily by the Forgen runtime the first time a plugin requests them.
#[derive(Debug)]
pub struct FileContext {
    /// Path relative to the workspace root (forward slashes, no leading `./`).
    pub path: String,

    source: LazyValue<String>,
    tree: LazyValue<RawNode>,
    /// Syntax pass only — each binding carries its own lazy `inferred_type`.
    let_bindings: LazyValue<Vec<LetBinding>>,
    functions: LazyValue<Vec<FnDef>>,
    structs: LazyValue<Vec<StructDef>>,
    enums: LazyValue<Vec<EnumDef>>,
    impls: LazyValue<Vec<ImplDef>>,

    /// Oracle for semantic (RA-backed) queries on this file.
    /// `None` when running without a live rust-analyzer context (tests).
    semantics: Option<SemanticHandle>,
}

impl FileContext {
    /// Create a new file context with lazy field initializers.
    pub fn new(
        path: String,
        source: LazyValue<String>,
        tree: LazyValue<RawNode>,
        let_bindings: LazyValue<Vec<LetBinding>>,
        functions: LazyValue<Vec<FnDef>>,
        structs: LazyValue<Vec<StructDef>>,
        enums: LazyValue<Vec<EnumDef>>,
        impls: LazyValue<Vec<ImplDef>>,
        semantics: Option<SemanticHandle>,
    ) -> Self {
        Self {
            path,
            source,
            tree,
            let_bindings,
            functions,
            structs,
            enums,
            impls,
            semantics,
        }
    }

    /// Raw UTF-8 source text exactly as it exists on disk.
    pub fn source(&self) -> &str {
        self.source.get().as_str()
    }

    /// The full CST of this file, with all trivia (whitespace, comments).
    pub fn tree(&self) -> &RawNode {
        self.tree.get()
    }

    /// Every `let` binding in the file, across all scopes (flattened).
    /// This includes bindings inside function bodies, closures, blocks, etc.
    pub fn let_bindings(&self) -> &[LetBinding] {
        self.let_bindings.get().as_slice()
    }

    /// All function and method definitions in the file.
    pub fn functions(&self) -> &[FnDef] {
        self.functions.get().as_slice()
    }

    /// Struct definitions.
    pub fn structs(&self) -> &[StructDef] {
        self.structs.get().as_slice()
    }

    /// Enum definitions.
    pub fn enums(&self) -> &[EnumDef] {
        self.enums.get().as_slice()
    }

    /// `impl` blocks (both inherent impls and trait impls).
    pub fn impls(&self) -> &[ImplDef] {
        self.impls.get().as_slice()
    }

    /// Returns `true` if the file source has already been loaded.
    pub fn is_source_initialized(&self) -> bool {
        self.source.is_initialized()
    }

    /// Returns `true` if the CST has already been built.
    pub fn is_tree_initialized(&self) -> bool {
        self.tree.is_initialized()
    }

    /// Returns `true` if `let` bindings have already been extracted.
    pub fn is_let_bindings_initialized(&self) -> bool {
        self.let_bindings.is_initialized()
    }

    /// Returns `true` if function definitions have already been extracted.
    pub fn is_functions_initialized(&self) -> bool {
        self.functions.is_initialized()
    }

    /// Returns `true` if struct definitions have already been extracted.
    pub fn is_structs_initialized(&self) -> bool {
        self.structs.is_initialized()
    }

    /// Returns `true` if enum definitions have already been extracted.
    pub fn is_enums_initialized(&self) -> bool {
        self.enums.is_initialized()
    }

    /// Returns `true` if impl blocks have already been extracted.
    pub fn is_impls_initialized(&self) -> bool {
        self.impls.is_initialized()
    }

    /// Find a `let` binding by variable name.
    pub fn binding(&self, name: &str) -> Option<&LetBinding> {
        self.let_bindings().iter().find(|b| b.name == name)
    }

    /// Iterate over all bindings whose effective type equals `ty`.
    pub fn bindings_of_type<'a>(
        &'a self,
        ty: &'a str,
    ) -> impl Iterator<Item = &'a LetBinding> + 'a {
        self.let_bindings()
            .iter()
            .filter(move |b| b.ty() == Some(ty))
    }

    /// Find a function definition by name.
    pub fn function(&self, name: &str) -> Option<&FnDef> {
        self.functions().iter().find(|f| f.name == name)
    }

    /// Find a struct definition by name.
    pub fn struct_def(&self, name: &str) -> Option<&StructDef> {
        self.structs().iter().find(|s| s.name == name)
    }

    /// Find an enum definition by name.
    pub fn enum_def(&self, name: &str) -> Option<&EnumDef> {
        self.enums().iter().find(|e| e.name == name)
    }

    /// Oracle for semantic (RA-backed) queries on this file.
    /// `None` when running without a live rust-analyzer context (tests).
    pub fn semantics(&self) -> Option<&SemanticHandle> {
        self.semantics.as_ref()
    }

    /// Shortcut: infer the type of the expression at `range` in this file.
    pub fn infer_type_at(&self, range: TextRange) -> Option<String> {
        self.semantics()?.infer_type_at(&self.path, range)
    }

    /// Shortcut: let bindings whose pattern falls inside `scope`.
    pub fn let_bindings_in(&self, scope: TextRange) -> Vec<LetBinding> {
        self.semantics()
            .map(|s| s.let_bindings_in(&self.path, scope))
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Let bindings
// ---------------------------------------------------------------------------

/// A single `let` (or `let mut`) binding extracted from a source file.
///
/// Only simple identifier patterns are captured (`let [mut] name [: T] = …`).
/// Destructuring patterns are skipped.
///
/// **Serde note:** `inferred_type` is skipped during serialization.
/// Call `.ty()` before serializing if you need the inferred type in output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    /// The bound variable name (e.g. `x` from `let x: f64 = 1.0`).
    pub name: String,

    /// The explicit type annotation as written in source, if present.
    /// Example: `"f64"` from `let x: f64 = …`.
    pub explicit_type: Option<String>,

    /// Byte range of the entire `let … ;` statement (including the semicolon).
    pub range: TextRange,

    /// Byte range of the initializer expression (the RHS of `=`).
    /// `None` for `let x: T;` declarations with no initializer.
    pub initializer_range: Option<TextRange>,

    /// Whether the binding was declared `let mut`.
    pub is_mut: bool,

    /// Type inferred by rust-analyzer. Only populated on demand via `.ty()`.
    /// The closure captures the `SemanticHandle` and fires `infer_type_at`.
    ///
    /// Skipped during serialization — call `.ty()` first to materialize it.
    #[serde(skip, default)]
    pub inferred_type: LazyValue<Option<String>>,
}

impl LetBinding {
    /// Returns the effective type of this binding.
    ///
    /// Prefers the explicit annotation (instant, no RA).
    /// Falls back to lazy RA type inference for the initializer expression.
    /// Returns `None` only when neither is available (e.g. inference failed,
    /// or the binding has no initializer).
    pub fn ty(&self) -> Option<&str> {
        self.explicit_type
            .as_deref()
            .or_else(|| self.inferred_type.get().as_deref())
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
