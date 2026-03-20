use crate::{syntax::SyntaxKind, TextRange};
use serde::{Deserialize, Serialize};

// ============================================================================
// RawNode
// ============================================================================

/// A node in the serialised Concrete Syntax Tree.
///
/// Mirrors a `rowan::SyntaxNode<RustLanguage>` but is fully owned, serialisable,
/// and free of any dependency on `ra_ap_syntax`.  Every [`FileContext`] carries
/// a `RawNode` rooted at `SyntaxKind::SOURCE_FILE` that contains the **complete**
/// CST — including whitespace, newlines, and comment tokens — so plugins can see
/// everything in the source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNode {
    pub kind: SyntaxKind,
    pub range: TextRange,
    pub children: Vec<Child>,
}

impl RawNode {
    /// Concatenates the text of all token descendants (leaf nodes).
    pub fn text(&self) -> String {
        let mut buf = String::new();
        for child in &self.children {
            match child {
                Child::Token(t) => buf.push_str(&t.text),
                Child::Node(n) => buf.push_str(&n.text()),
            }
        }
        buf
    }

    /// Iterates only the direct `Child::Node` children of this node.
    pub fn child_nodes(&self) -> impl Iterator<Item = &RawNode> {
        self.children.iter().filter_map(|c| {
            if let Child::Node(n) = c {
                Some(n)
            } else {
                None
            }
        })
    }

    /// Iterates only the direct `Child::Token` children of this node.
    pub fn child_tokens(&self) -> impl Iterator<Item = &RawToken> {
        self.children.iter().filter_map(|c| {
            if let Child::Token(t) = c {
                Some(t)
            } else {
                None
            }
        })
    }

    /// Yields `self` and all descendant nodes recursively (pre-order, depth-first).
    ///
    /// The result is collected into a `Vec` first so that the returned iterator
    /// does not require any lifetime-juggling with recursive closures.
    pub fn descendants(&self) -> impl Iterator<Item = &RawNode> {
        fn collect<'a>(node: &'a RawNode, acc: &mut Vec<&'a RawNode>) {
            acc.push(node);
            for child in node.child_nodes() {
                collect(child, acc);
            }
        }

        let mut result: Vec<&RawNode> = Vec::new();
        collect(self, &mut result);
        result.into_iter()
    }

    /// Returns the first leaf token reachable from this node (depth-first).
    pub fn first_token(&self) -> Option<&RawToken> {
        for child in &self.children {
            match child {
                Child::Token(t) => return Some(t),
                Child::Node(n) => {
                    if let Some(t) = n.first_token() {
                        return Some(t);
                    }
                }
            }
        }
        None
    }

    /// Returns the first *direct* child node whose kind matches `kind`.
    pub fn find_first_child(&self, kind: SyntaxKind) -> Option<&RawNode> {
        self.child_nodes().find(|n| n.kind == kind)
    }

    /// Returns an iterator over all *direct* child nodes whose kind matches `kind`.
    pub fn find_children(&self, kind: SyntaxKind) -> impl Iterator<Item = &RawNode> {
        self.children.iter().filter_map(move |c| {
            if let Child::Node(n) = c {
                if n.kind == kind {
                    return Some(n);
                }
            }
            None
        })
    }

    /// Returns the first *direct* child token whose kind matches `kind`.
    pub fn find_token(&self, kind: SyntaxKind) -> Option<&RawToken> {
        self.child_tokens().find(|t| t.kind == kind)
    }

    /// Returns `true` if this node's kind is trivia (whitespace, newline, or comment).
    pub fn is_trivia(&self) -> bool {
        self.kind.is_trivia()
    }
}

// ============================================================================
// Child
// ============================================================================

/// Either a node or a token — the children of a [`RawNode`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Child {
    Node(RawNode),
    Token(RawToken),
}

// ============================================================================
// RawToken
// ============================================================================

/// A leaf token in the CST (identifier, keyword, punctuation, literal, trivia…).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawToken {
    pub kind: SyntaxKind,
    pub text: String,
    pub range: TextRange,
}

impl RawToken {
    /// Returns `true` if this token is trivia (whitespace, newline, or comment).
    pub fn is_trivia(&self) -> bool {
        self.kind.is_trivia()
    }

    /// Returns `true` if this token is a comment (including doc comments).
    pub fn is_comment(&self) -> bool {
        self.kind == SyntaxKind::COMMENT
    }

    /// Returns `true` if this token is whitespace or a newline.
    pub fn is_whitespace(&self) -> bool {
        matches!(self.kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
    }

    /// Classifies the comment token, returning `None` when the token is not a
    /// `COMMENT`.
    ///
    /// The detection order matters: longer prefixes are checked before shorter
    /// ones so that `//!` is not shadowed by `//`, etc.
    ///
    /// | Prefix | Shape | Placement |
    /// |--------|-------|-----------|
    /// | `//!`  | Line  | Inner     |
    /// | `///`  | Line  | Outer     |
    /// | `/*!`  | Block | Inner     |
    /// | `/**`  | Block | Outer     |
    /// | `//`   | Line  | —         |
    /// | `/*`   | Block | —         |
    pub fn comment_kind(&self) -> Option<CommentKind> {
        if self.kind != SyntaxKind::COMMENT {
            return None;
        }

        let t = self.text.as_str();

        // Check the longer/more-specific prefixes first.
        if t.starts_with("//!") {
            Some(CommentKind {
                shape: CommentShape::Line,
                doc: Some(CommentPlacement::Inner),
            })
        } else if t.starts_with("///") {
            Some(CommentKind {
                shape: CommentShape::Line,
                doc: Some(CommentPlacement::Outer),
            })
        } else if t.starts_with("/*!") {
            Some(CommentKind {
                shape: CommentShape::Block,
                doc: Some(CommentPlacement::Inner),
            })
        } else if t.starts_with("/**") {
            Some(CommentKind {
                shape: CommentShape::Block,
                doc: Some(CommentPlacement::Outer),
            })
        } else if t.starts_with("//") {
            Some(CommentKind {
                shape: CommentShape::Line,
                doc: None,
            })
        } else if t.starts_with("/*") {
            Some(CommentKind {
                shape: CommentShape::Block,
                doc: None,
            })
        } else {
            // Malformed comment token — return a best-effort Line/None.
            None
        }
    }
}

// ============================================================================
// Comment classification helpers
// ============================================================================

/// Whether a comment is written as a line comment (`//`) or a block comment
/// (`/* … */`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommentShape {
    Line,
    Block,
}

/// Whether a doc comment is *inner* (`//!` / `/*!`) or *outer* (`///` / `/**`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommentPlacement {
    /// Inner doc comment — documents the enclosing item (`//!` or `/*!`).
    Inner,
    /// Outer doc comment — documents the following item (`///` or `/**`).
    Outer,
}

/// The fully-classified kind of a comment token.
///
/// `doc` is `None` for plain (non-doc) comments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentKind {
    pub shape: CommentShape,
    pub doc: Option<CommentPlacement>,
}
