#![allow(non_camel_case_types, clippy::upper_case_acronyms)]

use serde::{Deserialize, Serialize};

/// A mirror of `ra_ap_syntax::SyntaxKind` that is stable across rust-analyzer
/// versions and carries no dependency on any `ra_ap_*` crate.
///
/// Every variant here corresponds 1-to-1 with the upstream enum so that the
/// Forgen runtime can convert between the two without loss of information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SyntaxKind {
    // -----------------------------------------------------------------------
    // Punctuation / operators
    // -----------------------------------------------------------------------
    DOLLAR,
    SEMICOLON,
    COMMA,
    L_PAREN,
    R_PAREN,
    L_CURLY,
    R_CURLY,
    L_BRACK,
    R_BRACK,
    L_ANGLE,
    R_ANGLE,
    AT,
    POUND,
    TILDE,
    QUESTION,
    AMP,
    PIPE,
    PLUS,
    STAR,
    SLASH,
    CARET,
    PERCENT,
    UNDERSCORE,
    DOT,
    DOT2,
    DOT3,
    DOT2EQ,
    COLON,
    COLON2,
    EQ,
    EQ2,
    FAT_ARROW,
    BANG,
    NEQ,
    MINUS,
    THIN_ARROW,
    LTEQ,
    GTEQ,
    PLUSEQ,
    MINUSEQ,
    PIPEEQ,
    AMPEQ,
    CARETEQ,
    SLASHEQ,
    STAREQ,
    PERCENTEQ,
    AMP2,
    PIPE2,
    SHL,
    SHR,
    SHLEQ,
    SHREQ,

    // -----------------------------------------------------------------------
    // Strict keywords
    // -----------------------------------------------------------------------
    SELF_TYPE_KW,
    ABSTRACT_KW,
    AS_KW,
    BECOME_KW,
    BOX_KW,
    BREAK_KW,
    CONST_KW,
    CONTINUE_KW,
    CRATE_KW,
    DO_KW,
    ELSE_KW,
    ENUM_KW,
    EXTERN_KW,
    FALSE_KW,
    FINAL_KW,
    FN_KW,
    FOR_KW,
    IF_KW,
    IMPL_KW,
    IN_KW,
    LET_KW,
    LOOP_KW,
    MACRO_KW,
    MATCH_KW,
    MOD_KW,
    MOVE_KW,
    MUT_KW,
    OVERRIDE_KW,
    PRIV_KW,
    PUB_KW,
    REF_KW,
    RETURN_KW,
    SELF_KW,
    STATIC_KW,
    STRUCT_KW,
    SUPER_KW,
    TRAIT_KW,
    TRUE_KW,
    TYPE_KW,
    TYPEOF_KW,
    UNSAFE_KW,
    UNSIZED_KW,
    USE_KW,
    VIRTUAL_KW,
    WHERE_KW,
    WHILE_KW,
    YIELD_KW,

    // -----------------------------------------------------------------------
    // Contextual / weak keywords
    // -----------------------------------------------------------------------
    ASM_KW,
    ASYNC_KW,
    ATT_SYNTAX_KW,
    AUTO_KW,
    BUILTIN_KW,
    CLOBBER_ABI_KW,
    DEFAULT_KW,
    DYN_KW,
    FORMAT_ARGS_KW,
    GEN_KW,
    GLOBAL_ASM_KW,
    LABEL_KW,
    MACRO_RULES_KW,
    NAKED_ASM_KW,
    OFFSET_OF_KW,
    OPTIONS_KW,
    PRESERVES_FLAGS_KW,
    PURE_KW,
    RAW_KW,
    READONLY_KW,
    SAFE_KW,
    SYM_KW,
    TRY_KW,
    UNION_KW,
    YEET_KW,

    // -----------------------------------------------------------------------
    // Literals
    // -----------------------------------------------------------------------
    BYTE,
    BYTE_STRING,
    CHAR,
    C_STRING,
    FLOAT_NUMBER,
    INT_NUMBER,
    STRING,

    // -----------------------------------------------------------------------
    // Trivia and special tokens
    // -----------------------------------------------------------------------
    COMMENT,
    ERROR,
    FRONTMATTER,
    IDENT,
    LIFETIME_IDENT,
    NEWLINE,
    SHEBANG,
    WHITESPACE,
    TOMBSTONE,

    // -----------------------------------------------------------------------
    // Composite node kinds (AST nodes)
    // -----------------------------------------------------------------------
    ABI,
    ARG_LIST,
    ARRAY_EXPR,
    ARRAY_TYPE,
    ASM_CLOBBER_ABI,
    ASM_CONST,
    ASM_DIR_SPEC,
    ASM_EXPR,
    ASM_LABEL,
    ASM_OPERAND_EXPR,
    ASM_OPERAND_NAMED,
    ASM_OPTION,
    ASM_OPTIONS,
    ASM_REG_OPERAND,
    ASM_REG_SPEC,
    ASM_SYM,
    ASSOC_ITEM_LIST,
    ASSOC_TYPE_ARG,
    ATTR,
    AWAIT_EXPR,
    BECOME_EXPR,
    BIN_EXPR,
    BLOCK_EXPR,
    BOX_PAT,
    BREAK_EXPR,
    CALL_EXPR,
    CAST_EXPR,
    CLOSURE_EXPR,
    CONST,
    CONST_ARG,
    CONST_BLOCK_PAT,
    CONST_PARAM,
    CONTINUE_EXPR,
    DYN_TRAIT_TYPE,
    ENUM,
    EXPR_STMT,
    EXTERN_BLOCK,
    EXTERN_CRATE,
    EXTERN_ITEM_LIST,
    FIELD_EXPR,
    FN,
    FN_PTR_TYPE,
    FOR_BINDER,
    FOR_EXPR,
    FOR_TYPE,
    FORMAT_ARGS_ARG,
    FORMAT_ARGS_ARG_NAME,
    FORMAT_ARGS_EXPR,
    GENERIC_ARG_LIST,
    GENERIC_PARAM_LIST,
    IDENT_PAT,
    IF_EXPR,
    IMPL,
    IMPL_TRAIT_TYPE,
    INDEX_EXPR,
    INFER_TYPE,
    ITEM_LIST,
    LABEL,
    LET_ELSE,
    LET_EXPR,
    LET_STMT,
    LIFETIME,
    LIFETIME_ARG,
    LIFETIME_PARAM,
    LITERAL,
    LITERAL_PAT,
    LOOP_EXPR,
    MACRO_CALL,
    MACRO_DEF,
    MACRO_EXPR,
    MACRO_ITEMS,
    MACRO_PAT,
    MACRO_RULES,
    MACRO_STMTS,
    MACRO_TYPE,
    MATCH_ARM,
    MATCH_ARM_LIST,
    MATCH_EXPR,
    MATCH_GUARD,
    META,
    METHOD_CALL_EXPR,
    MODULE,
    NAME,
    NAME_REF,
    NEVER_TYPE,
    OFFSET_OF_EXPR,
    OR_PAT,
    PARAM,
    PARAM_LIST,
    PAREN_EXPR,
    PAREN_PAT,
    PAREN_TYPE,
    PARENTHESIZED_ARG_LIST,
    PATH,
    PATH_EXPR,
    PATH_PAT,
    PATH_SEGMENT,
    PATH_TYPE,
    PREFIX_EXPR,
    PTR_TYPE,
    RANGE_EXPR,
    RANGE_PAT,
    RECORD_EXPR,
    RECORD_EXPR_FIELD,
    RECORD_EXPR_FIELD_LIST,
    RECORD_FIELD,
    RECORD_FIELD_LIST,
    RECORD_PAT,
    RECORD_PAT_FIELD,
    RECORD_PAT_FIELD_LIST,
    REF_EXPR,
    REF_PAT,
    REF_TYPE,
    RENAME,
    REST_PAT,
    RET_TYPE,
    RETURN_EXPR,
    RETURN_TYPE_SYNTAX,
    SELF_PARAM,
    SLICE_PAT,
    SLICE_TYPE,
    SOURCE_FILE,
    STATIC,
    STMT_LIST,
    STRUCT,
    TOKEN_TREE,
    TRAIT,
    TRAIT_ALIAS,
    TRY_BLOCK_MODIFIER,
    TRY_EXPR,
    TUPLE_EXPR,
    TUPLE_FIELD,
    TUPLE_FIELD_LIST,
    TUPLE_PAT,
    TUPLE_STRUCT_PAT,
    TUPLE_TYPE,
    TYPE_ALIAS,
    TYPE_ANCHOR,
    TYPE_ARG,
    TYPE_BOUND,
    TYPE_BOUND_LIST,
    TYPE_PARAM,
    UNDERSCORE_EXPR,
    UNION,
    USE,
    USE_BOUND_GENERIC_ARGS,
    USE_TREE,
    USE_TREE_LIST,
    VARIANT,
    VARIANT_LIST,
    VISIBILITY,
    WHERE_CLAUSE,
    WHERE_PRED,
    WHILE_EXPR,
    WILDCARD_PAT,
    YEET_EXPR,
    YIELD_EXPR,
}

impl SyntaxKind {
    /// Returns `true` if this kind is trivia (whitespace, newlines, or comments).
    /// Trivia tokens are attached to the tree but carry no semantic meaning.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
        )
    }

    /// Returns `true` if this kind is any keyword — both strict (reserved) and
    /// contextual / weak keywords.
    pub fn is_keyword(self) -> bool {
        matches!(
            self,
            // Strict keywords
            SyntaxKind::SELF_TYPE_KW
                | SyntaxKind::ABSTRACT_KW
                | SyntaxKind::AS_KW
                | SyntaxKind::BECOME_KW
                | SyntaxKind::BOX_KW
                | SyntaxKind::BREAK_KW
                | SyntaxKind::CONST_KW
                | SyntaxKind::CONTINUE_KW
                | SyntaxKind::CRATE_KW
                | SyntaxKind::DO_KW
                | SyntaxKind::ELSE_KW
                | SyntaxKind::ENUM_KW
                | SyntaxKind::EXTERN_KW
                | SyntaxKind::FALSE_KW
                | SyntaxKind::FINAL_KW
                | SyntaxKind::FN_KW
                | SyntaxKind::FOR_KW
                | SyntaxKind::IF_KW
                | SyntaxKind::IMPL_KW
                | SyntaxKind::IN_KW
                | SyntaxKind::LET_KW
                | SyntaxKind::LOOP_KW
                | SyntaxKind::MACRO_KW
                | SyntaxKind::MATCH_KW
                | SyntaxKind::MOD_KW
                | SyntaxKind::MOVE_KW
                | SyntaxKind::MUT_KW
                | SyntaxKind::OVERRIDE_KW
                | SyntaxKind::PRIV_KW
                | SyntaxKind::PUB_KW
                | SyntaxKind::REF_KW
                | SyntaxKind::RETURN_KW
                | SyntaxKind::SELF_KW
                | SyntaxKind::STATIC_KW
                | SyntaxKind::STRUCT_KW
                | SyntaxKind::SUPER_KW
                | SyntaxKind::TRAIT_KW
                | SyntaxKind::TRUE_KW
                | SyntaxKind::TYPE_KW
                | SyntaxKind::TYPEOF_KW
                | SyntaxKind::UNSAFE_KW
                | SyntaxKind::UNSIZED_KW
                | SyntaxKind::USE_KW
                | SyntaxKind::VIRTUAL_KW
                | SyntaxKind::WHERE_KW
                | SyntaxKind::WHILE_KW
                | SyntaxKind::YIELD_KW
                // Contextual / weak keywords
                | SyntaxKind::ASM_KW
                | SyntaxKind::ASYNC_KW
                | SyntaxKind::ATT_SYNTAX_KW
                | SyntaxKind::AUTO_KW
                | SyntaxKind::BUILTIN_KW
                | SyntaxKind::CLOBBER_ABI_KW
                | SyntaxKind::DEFAULT_KW
                | SyntaxKind::DYN_KW
                | SyntaxKind::FORMAT_ARGS_KW
                | SyntaxKind::GEN_KW
                | SyntaxKind::GLOBAL_ASM_KW
                | SyntaxKind::LABEL_KW
                | SyntaxKind::MACRO_RULES_KW
                | SyntaxKind::NAKED_ASM_KW
                | SyntaxKind::OFFSET_OF_KW
                | SyntaxKind::OPTIONS_KW
                | SyntaxKind::PRESERVES_FLAGS_KW
                | SyntaxKind::PURE_KW
                | SyntaxKind::RAW_KW
                | SyntaxKind::READONLY_KW
                | SyntaxKind::SAFE_KW
                | SyntaxKind::SYM_KW
                | SyntaxKind::TRY_KW
                | SyntaxKind::UNION_KW
                | SyntaxKind::YEET_KW
        )
    }

    /// Returns `true` if this kind is a punctuation token or operator.
    pub fn is_punct(self) -> bool {
        matches!(
            self,
            SyntaxKind::DOLLAR
                | SyntaxKind::SEMICOLON
                | SyntaxKind::COMMA
                | SyntaxKind::L_PAREN
                | SyntaxKind::R_PAREN
                | SyntaxKind::L_CURLY
                | SyntaxKind::R_CURLY
                | SyntaxKind::L_BRACK
                | SyntaxKind::R_BRACK
                | SyntaxKind::L_ANGLE
                | SyntaxKind::R_ANGLE
                | SyntaxKind::AT
                | SyntaxKind::POUND
                | SyntaxKind::TILDE
                | SyntaxKind::QUESTION
                | SyntaxKind::AMP
                | SyntaxKind::PIPE
                | SyntaxKind::PLUS
                | SyntaxKind::STAR
                | SyntaxKind::SLASH
                | SyntaxKind::CARET
                | SyntaxKind::PERCENT
                | SyntaxKind::UNDERSCORE
                | SyntaxKind::DOT
                | SyntaxKind::DOT2
                | SyntaxKind::DOT3
                | SyntaxKind::DOT2EQ
                | SyntaxKind::COLON
                | SyntaxKind::COLON2
                | SyntaxKind::EQ
                | SyntaxKind::EQ2
                | SyntaxKind::FAT_ARROW
                | SyntaxKind::BANG
                | SyntaxKind::NEQ
                | SyntaxKind::MINUS
                | SyntaxKind::THIN_ARROW
                | SyntaxKind::LTEQ
                | SyntaxKind::GTEQ
                | SyntaxKind::PLUSEQ
                | SyntaxKind::MINUSEQ
                | SyntaxKind::PIPEEQ
                | SyntaxKind::AMPEQ
                | SyntaxKind::CARETEQ
                | SyntaxKind::SLASHEQ
                | SyntaxKind::STAREQ
                | SyntaxKind::PERCENTEQ
                | SyntaxKind::AMP2
                | SyntaxKind::PIPE2
                | SyntaxKind::SHL
                | SyntaxKind::SHR
                | SyntaxKind::SHLEQ
                | SyntaxKind::SHREQ
        )
    }

    /// Returns `true` if this kind is a literal token.
    pub fn is_literal(self) -> bool {
        matches!(
            self,
            SyntaxKind::BYTE
                | SyntaxKind::BYTE_STRING
                | SyntaxKind::CHAR
                | SyntaxKind::C_STRING
                | SyntaxKind::FLOAT_NUMBER
                | SyntaxKind::INT_NUMBER
                | SyntaxKind::STRING
        )
    }

    /// Returns `true` if this kind is a leaf token (not a composite syntax node).
    ///
    /// This covers trivia, keywords, punctuation, literals, identifiers, and
    /// other non-composite tokens such as `SHEBANG`, `ERROR`, `FRONTMATTER`,
    /// and `TOMBSTONE`.
    pub fn is_token(self) -> bool {
        self.is_trivia()
            || self.is_keyword()
            || self.is_punct()
            || self.is_literal()
            || matches!(
                self,
                SyntaxKind::IDENT
                    | SyntaxKind::LIFETIME_IDENT
                    | SyntaxKind::SHEBANG
                    | SyntaxKind::FRONTMATTER
                    | SyntaxKind::COMMENT
                    | SyntaxKind::ERROR
                    | SyntaxKind::TOMBSTONE
            )
    }

    /// Returns `true` if this kind is a composite syntax node (i.e. an inner
    /// tree node, not a leaf token).
    ///
    /// This is the exact complement of [`SyntaxKind::is_token`].
    pub fn is_node(self) -> bool {
        !self.is_token()
    }
}
