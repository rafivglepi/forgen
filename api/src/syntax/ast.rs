use super::kind::SyntaxKind;
use super::raw::RawNode;
#[allow(unused_imports)]
use super::raw::RawToken;
use crate::TextRange;
use serde::{Deserialize, Serialize};

// ============================================================================
// Item
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::Item`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Item {
    Const(RawNode),
    Enum(RawNode),
    ExternBlock(RawNode),
    ExternCrate(RawNode),
    Fn(RawNode),
    Impl(RawNode),
    MacroCall(RawNode),
    MacroDef(RawNode),
    MacroRules(RawNode),
    Module(RawNode),
    Static(RawNode),
    Struct(RawNode),
    Trait(RawNode),
    TraitAlias(RawNode),
    TypeAlias(RawNode),
    Union(RawNode),
    Use(RawNode),
}

impl Item {
    /// Attempts to cast a [`RawNode`] into the appropriate `Item` variant based
    /// on its [`SyntaxKind`].  Returns `None` when the kind is not an item kind.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::CONST => Some(Item::Const(node)),
            SyntaxKind::ENUM => Some(Item::Enum(node)),
            SyntaxKind::EXTERN_BLOCK => Some(Item::ExternBlock(node)),
            SyntaxKind::EXTERN_CRATE => Some(Item::ExternCrate(node)),
            SyntaxKind::FN => Some(Item::Fn(node)),
            SyntaxKind::IMPL => Some(Item::Impl(node)),
            SyntaxKind::MACRO_CALL => Some(Item::MacroCall(node)),
            SyntaxKind::MACRO_DEF => Some(Item::MacroDef(node)),
            SyntaxKind::MACRO_RULES => Some(Item::MacroRules(node)),
            SyntaxKind::MODULE => Some(Item::Module(node)),
            SyntaxKind::STATIC => Some(Item::Static(node)),
            SyntaxKind::STRUCT => Some(Item::Struct(node)),
            SyntaxKind::TRAIT => Some(Item::Trait(node)),
            SyntaxKind::TRAIT_ALIAS => Some(Item::TraitAlias(node)),
            SyntaxKind::TYPE_ALIAS => Some(Item::TypeAlias(node)),
            SyntaxKind::UNION => Some(Item::Union(node)),
            SyntaxKind::USE => Some(Item::Use(node)),
            _ => None,
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    pub fn raw(&self) -> &RawNode {
        match self {
            Item::Const(n)
            | Item::Enum(n)
            | Item::ExternBlock(n)
            | Item::ExternCrate(n)
            | Item::Fn(n)
            | Item::Impl(n)
            | Item::MacroCall(n)
            | Item::MacroDef(n)
            | Item::MacroRules(n)
            | Item::Module(n)
            | Item::Static(n)
            | Item::Struct(n)
            | Item::Trait(n)
            | Item::TraitAlias(n)
            | Item::TypeAlias(n)
            | Item::Union(n)
            | Item::Use(n) => n,
        }
    }

    /// Returns the byte-offset range of this item in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this item by concatenating all leaf tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// Expr
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::Expr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    Array(RawNode),
    Asm(RawNode),
    Await(RawNode),
    Become(RawNode),
    BinOp(RawNode),
    Block(RawNode),
    Break(RawNode),
    Call(RawNode),
    Cast(RawNode),
    Closure(RawNode),
    Continue(RawNode),
    Field(RawNode),
    For(RawNode),
    FormatArgs(RawNode),
    If(RawNode),
    Index(RawNode),
    Let(RawNode),
    Literal(RawNode),
    Loop(RawNode),
    MacroCall(RawNode),
    Match(RawNode),
    MethodCall(RawNode),
    OffsetOf(RawNode),
    Paren(RawNode),
    Path(RawNode),
    Prefix(RawNode),
    Range(RawNode),
    Record(RawNode),
    Ref(RawNode),
    Return(RawNode),
    Try(RawNode),
    Tuple(RawNode),
    Underscore(RawNode),
    While(RawNode),
    Yeet(RawNode),
    Yield(RawNode),
}

impl Expr {
    /// Attempts to cast a [`RawNode`] into the appropriate `Expr` variant based
    /// on its [`SyntaxKind`].  Returns `None` when the kind is not an expression kind.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::ARRAY_EXPR => Some(Expr::Array(node)),
            SyntaxKind::ASM_EXPR => Some(Expr::Asm(node)),
            SyntaxKind::AWAIT_EXPR => Some(Expr::Await(node)),
            SyntaxKind::BECOME_EXPR => Some(Expr::Become(node)),
            SyntaxKind::BIN_EXPR => Some(Expr::BinOp(node)),
            SyntaxKind::BLOCK_EXPR => Some(Expr::Block(node)),
            SyntaxKind::BREAK_EXPR => Some(Expr::Break(node)),
            SyntaxKind::CALL_EXPR => Some(Expr::Call(node)),
            SyntaxKind::CAST_EXPR => Some(Expr::Cast(node)),
            SyntaxKind::CLOSURE_EXPR => Some(Expr::Closure(node)),
            SyntaxKind::CONTINUE_EXPR => Some(Expr::Continue(node)),
            SyntaxKind::FIELD_EXPR => Some(Expr::Field(node)),
            SyntaxKind::FOR_EXPR => Some(Expr::For(node)),
            SyntaxKind::FORMAT_ARGS_EXPR => Some(Expr::FormatArgs(node)),
            SyntaxKind::IF_EXPR => Some(Expr::If(node)),
            SyntaxKind::INDEX_EXPR => Some(Expr::Index(node)),
            SyntaxKind::LET_EXPR => Some(Expr::Let(node)),
            SyntaxKind::LITERAL => Some(Expr::Literal(node)),
            SyntaxKind::LOOP_EXPR => Some(Expr::Loop(node)),
            SyntaxKind::MACRO_EXPR => Some(Expr::MacroCall(node)),
            SyntaxKind::MATCH_EXPR => Some(Expr::Match(node)),
            SyntaxKind::METHOD_CALL_EXPR => Some(Expr::MethodCall(node)),
            SyntaxKind::OFFSET_OF_EXPR => Some(Expr::OffsetOf(node)),
            SyntaxKind::PAREN_EXPR => Some(Expr::Paren(node)),
            SyntaxKind::PATH_EXPR => Some(Expr::Path(node)),
            SyntaxKind::PREFIX_EXPR => Some(Expr::Prefix(node)),
            SyntaxKind::RANGE_EXPR => Some(Expr::Range(node)),
            SyntaxKind::RECORD_EXPR => Some(Expr::Record(node)),
            SyntaxKind::REF_EXPR => Some(Expr::Ref(node)),
            SyntaxKind::RETURN_EXPR => Some(Expr::Return(node)),
            SyntaxKind::TRY_EXPR => Some(Expr::Try(node)),
            SyntaxKind::TUPLE_EXPR => Some(Expr::Tuple(node)),
            SyntaxKind::UNDERSCORE_EXPR => Some(Expr::Underscore(node)),
            SyntaxKind::WHILE_EXPR => Some(Expr::While(node)),
            SyntaxKind::YEET_EXPR => Some(Expr::Yeet(node)),
            SyntaxKind::YIELD_EXPR => Some(Expr::Yield(node)),
            _ => None,
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    pub fn raw(&self) -> &RawNode {
        match self {
            Expr::Array(n)
            | Expr::Asm(n)
            | Expr::Await(n)
            | Expr::Become(n)
            | Expr::BinOp(n)
            | Expr::Block(n)
            | Expr::Break(n)
            | Expr::Call(n)
            | Expr::Cast(n)
            | Expr::Closure(n)
            | Expr::Continue(n)
            | Expr::Field(n)
            | Expr::For(n)
            | Expr::FormatArgs(n)
            | Expr::If(n)
            | Expr::Index(n)
            | Expr::Let(n)
            | Expr::Literal(n)
            | Expr::Loop(n)
            | Expr::MacroCall(n)
            | Expr::Match(n)
            | Expr::MethodCall(n)
            | Expr::OffsetOf(n)
            | Expr::Paren(n)
            | Expr::Path(n)
            | Expr::Prefix(n)
            | Expr::Range(n)
            | Expr::Record(n)
            | Expr::Ref(n)
            | Expr::Return(n)
            | Expr::Try(n)
            | Expr::Tuple(n)
            | Expr::Underscore(n)
            | Expr::While(n)
            | Expr::Yeet(n)
            | Expr::Yield(n) => n,
        }
    }

    /// Returns the byte-offset range of this expression in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this expression by concatenating all leaf tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// Pat
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::Pat`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pat {
    Box(RawNode),
    ConstBlock(RawNode),
    Ident(RawNode),
    Literal(RawNode),
    Macro(RawNode),
    Or(RawNode),
    Paren(RawNode),
    Path(RawNode),
    Range(RawNode),
    Record(RawNode),
    Ref(RawNode),
    Rest(RawNode),
    Slice(RawNode),
    Tuple(RawNode),
    TupleStruct(RawNode),
    Wildcard(RawNode),
}

impl Pat {
    /// Attempts to cast a [`RawNode`] into the appropriate `Pat` variant based
    /// on its [`SyntaxKind`].  Returns `None` when the kind is not a pattern kind.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::BOX_PAT => Some(Pat::Box(node)),
            SyntaxKind::CONST_BLOCK_PAT => Some(Pat::ConstBlock(node)),
            SyntaxKind::IDENT_PAT => Some(Pat::Ident(node)),
            SyntaxKind::LITERAL_PAT => Some(Pat::Literal(node)),
            SyntaxKind::MACRO_PAT => Some(Pat::Macro(node)),
            SyntaxKind::OR_PAT => Some(Pat::Or(node)),
            SyntaxKind::PAREN_PAT => Some(Pat::Paren(node)),
            SyntaxKind::PATH_PAT => Some(Pat::Path(node)),
            SyntaxKind::RANGE_PAT => Some(Pat::Range(node)),
            SyntaxKind::RECORD_PAT => Some(Pat::Record(node)),
            SyntaxKind::REF_PAT => Some(Pat::Ref(node)),
            SyntaxKind::REST_PAT => Some(Pat::Rest(node)),
            SyntaxKind::SLICE_PAT => Some(Pat::Slice(node)),
            SyntaxKind::TUPLE_PAT => Some(Pat::Tuple(node)),
            SyntaxKind::TUPLE_STRUCT_PAT => Some(Pat::TupleStruct(node)),
            SyntaxKind::WILDCARD_PAT => Some(Pat::Wildcard(node)),
            _ => None,
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    pub fn raw(&self) -> &RawNode {
        match self {
            Pat::Box(n)
            | Pat::ConstBlock(n)
            | Pat::Ident(n)
            | Pat::Literal(n)
            | Pat::Macro(n)
            | Pat::Or(n)
            | Pat::Paren(n)
            | Pat::Path(n)
            | Pat::Range(n)
            | Pat::Record(n)
            | Pat::Ref(n)
            | Pat::Rest(n)
            | Pat::Slice(n)
            | Pat::Tuple(n)
            | Pat::TupleStruct(n)
            | Pat::Wildcard(n) => n,
        }
    }

    /// Returns the byte-offset range of this pattern in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this pattern by concatenating all leaf tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// TypeRef
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::Type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeRef {
    Array(RawNode),
    DynTrait(RawNode),
    FnPtr(RawNode),
    For(RawNode),
    ImplTrait(RawNode),
    Infer(RawNode),
    Macro(RawNode),
    Never(RawNode),
    Paren(RawNode),
    Path(RawNode),
    Ptr(RawNode),
    Ref(RawNode),
    Slice(RawNode),
    Tuple(RawNode),
}

impl TypeRef {
    /// Attempts to cast a [`RawNode`] into the appropriate `TypeRef` variant
    /// based on its [`SyntaxKind`].  Returns `None` when the kind is not a type
    /// kind.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::ARRAY_TYPE => Some(TypeRef::Array(node)),
            SyntaxKind::DYN_TRAIT_TYPE => Some(TypeRef::DynTrait(node)),
            SyntaxKind::FN_PTR_TYPE => Some(TypeRef::FnPtr(node)),
            SyntaxKind::FOR_TYPE => Some(TypeRef::For(node)),
            SyntaxKind::IMPL_TRAIT_TYPE => Some(TypeRef::ImplTrait(node)),
            SyntaxKind::INFER_TYPE => Some(TypeRef::Infer(node)),
            SyntaxKind::MACRO_TYPE => Some(TypeRef::Macro(node)),
            SyntaxKind::NEVER_TYPE => Some(TypeRef::Never(node)),
            SyntaxKind::PAREN_TYPE => Some(TypeRef::Paren(node)),
            SyntaxKind::PATH_TYPE => Some(TypeRef::Path(node)),
            SyntaxKind::PTR_TYPE => Some(TypeRef::Ptr(node)),
            SyntaxKind::REF_TYPE => Some(TypeRef::Ref(node)),
            SyntaxKind::SLICE_TYPE => Some(TypeRef::Slice(node)),
            SyntaxKind::TUPLE_TYPE => Some(TypeRef::Tuple(node)),
            _ => None,
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    pub fn raw(&self) -> &RawNode {
        match self {
            TypeRef::Array(n)
            | TypeRef::DynTrait(n)
            | TypeRef::FnPtr(n)
            | TypeRef::For(n)
            | TypeRef::ImplTrait(n)
            | TypeRef::Infer(n)
            | TypeRef::Macro(n)
            | TypeRef::Never(n)
            | TypeRef::Paren(n)
            | TypeRef::Path(n)
            | TypeRef::Ptr(n)
            | TypeRef::Ref(n)
            | TypeRef::Slice(n)
            | TypeRef::Tuple(n) => n,
        }
    }

    /// Returns the byte-offset range of this type reference in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this type reference by concatenating all
    /// leaf tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// Stmt
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::Stmt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Stmt {
    Expr(RawNode),
    Item(Item),
    Let(RawNode),
}

impl Stmt {
    /// Attempts to cast a [`RawNode`] into the appropriate `Stmt` variant.
    ///
    /// `LET_STMT` and `EXPR_STMT` are matched directly; all remaining kinds are
    /// forwarded to [`Item::cast`] so that any top-level item kind is wrapped as
    /// `Stmt::Item`.  Returns `None` for unrecognised kinds.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::LET_STMT => Some(Stmt::Let(node)),
            SyntaxKind::EXPR_STMT => Some(Stmt::Expr(node)),
            _ => Item::cast(node).map(Stmt::Item),
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    ///
    /// For the `Item` variant the inner [`Item`]'s `raw()` method is delegated
    /// to, so the same unwrapping behaviour is preserved.
    pub fn raw(&self) -> &RawNode {
        match self {
            Stmt::Expr(n) | Stmt::Let(n) => n,
            Stmt::Item(item) => item.raw(),
        }
    }

    /// Returns the byte-offset range of this statement in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this statement by concatenating all leaf
    /// tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// AssocItem
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::AssocItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssocItem {
    Const(RawNode),
    Fn(RawNode),
    MacroCall(RawNode),
    TypeAlias(RawNode),
}

impl AssocItem {
    /// Attempts to cast a [`RawNode`] into the appropriate `AssocItem` variant
    /// based on its [`SyntaxKind`].  Returns `None` for unrecognised kinds.
    pub fn cast(node: RawNode) -> Option<Self> {
        match node.kind {
            SyntaxKind::CONST => Some(AssocItem::Const(node)),
            SyntaxKind::FN => Some(AssocItem::Fn(node)),
            SyntaxKind::MACRO_CALL => Some(AssocItem::MacroCall(node)),
            SyntaxKind::TYPE_ALIAS => Some(AssocItem::TypeAlias(node)),
            _ => None,
        }
    }

    /// Returns a reference to the underlying [`RawNode`].
    pub fn raw(&self) -> &RawNode {
        match self {
            AssocItem::Const(n)
            | AssocItem::Fn(n)
            | AssocItem::MacroCall(n)
            | AssocItem::TypeAlias(n) => n,
        }
    }

    /// Returns the byte-offset range of this associated item in the source file.
    pub fn range(&self) -> TextRange {
        self.raw().range.clone()
    }

    /// Returns the full source text of this associated item by concatenating all
    /// leaf tokens.
    pub fn text(&self) -> String {
        self.raw().text()
    }
}

// ============================================================================
// GenericParam
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::GenericParam`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GenericParam {
    Const(RawNode),
    Lifetime(RawNode),
    Type(RawNode),
}

// ============================================================================
// GenericArg
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::GenericArg`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GenericArg {
    AssocType(RawNode),
    Const(RawNode),
    Lifetime(RawNode),
    Type(RawNode),
}

// ============================================================================
// FieldList
// ============================================================================

/// Mirrors `ra_ap_syntax::ast::FieldList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldList {
    Record(RawNode),
    Tuple(RawNode),
}
