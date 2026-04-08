use serde::{Deserialize, Serialize};

/// A byte-offset range within a source file.
///
/// Both `start` and `end` are byte offsets (not character indices).
/// The range is half-open: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Returns `true` if this is a zero-width range (i.e. a cursor position).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
}

/// A text replacement (or insertion, or deletion) at a byte-offset range in a
/// source file.
///
/// | Scenario    | Condition                          |
/// |-------------|------------------------------------|
/// | Insertion   | `range.start == range.end`         |
/// | Replacement | `range.start != range.end && !text.is_empty()` |
/// | Deletion    | `text.is_empty()`                  |
///
/// The Forgen runner applies replacements in **reverse offset order** so that
/// earlier offsets are not invalidated by later edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replacement {
    pub range: TextRange,
    pub text: String,
}

impl Replacement {
    /// Creates a zero-width insertion at `offset` — nothing is removed, `text`
    /// is inserted at that position.
    pub fn insert(offset: u32, text: String) -> Self {
        Self {
            range: TextRange {
                start: offset,
                end: offset,
            },
            text,
        }
    }

    /// Replaces the bytes in `[start, end)` with `text`.
    pub fn replace(start: u32, end: u32, text: String) -> Self {
        Self {
            range: TextRange { start, end },
            text,
        }
    }

    /// Deletes the bytes in `[start, end)` without inserting anything.
    pub fn delete(start: u32, end: u32) -> Self {
        Self {
            range: TextRange { start, end },
            text: String::new(),
        }
    }
}

/// A set of replacements targeting a single source file.
///
/// `path` is relative to the workspace root and always uses forward slashes,
/// regardless of the host platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReplacement {
    /// Workspace-relative path to the file (forward slashes, no leading `./`).
    pub path: String,
    /// Replacements to apply. The runner will sort these by offset before
    /// writing the output JSON, so plugins may return them in any order.
    pub replacements: Vec<Replacement>,
}

impl FileReplacement {
    pub fn new(path: impl Into<String>, replacements: Vec<Replacement>) -> Self {
        Self {
            path: path.into(),
            replacements,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }
}
