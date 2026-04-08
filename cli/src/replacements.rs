use anyhow::{anyhow, bail, Context, Result};
use forgen_api::Replacement;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Replacement format written to `target/.forgen/<mirrored-path>.json`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SavedReplacement {
    /// The nth occurrence of `old_text` in the original source file.
    ///
    /// This is zero-based and counts *overlapping* matches.
    pub index: usize,
    /// The original text to replace.
    pub old_text: String,
    /// The replacement text.
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub replacements: Vec<SavedReplacement>,
}

#[derive(Debug, Clone)]
pub struct FileModel {
    original: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Original { start: usize, end: usize },
    Inserted(String),
}

#[derive(Debug, Clone)]
struct RawOp {
    start: usize,
    end: usize,
    text: String,
}

pub fn build_file_models(original_sources: &HashMap<String, String>) -> HashMap<String, FileModel> {
    original_sources
        .iter()
        .map(|(path, source)| (path.clone(), FileModel::new(source.clone())))
        .collect()
}

pub fn clear_saved_replacements(workspace_root: &Path) -> Result<()> {
    let out_root = workspace_root.join("target").join(".forgen");

    if out_root.exists() {
        fs::remove_dir_all(&out_root)
            .with_context(|| format!("Failed to clear {}", out_root.display()))?;
    }

    fs::create_dir_all(&out_root)
        .with_context(|| format!("Failed to create {}", out_root.display()))?;

    Ok(())
}

pub fn replace_saved_replacements(
    workspace_root: &Path,
    changed_files: &[ChangedFile],
) -> Result<usize> {
    clear_saved_replacements(workspace_root)?;
    write_final_file_replacements(workspace_root, changed_files)
}

/// Clear `target/.forgen/`, write occurrence-based JSON replacements, and
/// return the total number of saved replacement entries written.
pub fn write_final_file_replacements(
    workspace_root: &Path,
    changed_files: &[ChangedFile],
) -> Result<usize> {
    let out_root = workspace_root.join("target").join(".forgen");
    fs::create_dir_all(&out_root)
        .with_context(|| format!("Failed to create {}", out_root.display()))?;

    let mut total_saved = 0usize;

    for changed in changed_files {
        if changed.replacements.is_empty() {
            continue;
        }

        let output_path = mirrored_json_path(&out_root, &changed.path)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(&changed.replacements)?;
        fs::write(&output_path, json)
            .with_context(|| format!("Failed to write {}", output_path.display()))?;

        total_saved += changed.replacements.len();
    }

    Ok(total_saved)
}

pub fn collect_changed_files(file_models: &HashMap<String, FileModel>) -> Result<Vec<ChangedFile>> {
    let mut out = Vec::new();

    for (path, model) in file_models {
        let replacements = serialise_file_model(model)
            .with_context(|| format!("Failed to serialise saved replacements for `{path}`"))?;

        if !replacements.is_empty() {
            out.push(ChangedFile {
                path: path.clone(),
                replacements,
            });
        }
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn apply_replacements_to_source(source: &str, replacements: &[Replacement]) -> Result<String> {
    let ops = normalise_raw_ops(source, replacements)?;
    apply_raw_ops(source, &ops)
}

pub fn changed_paths_from_replacements(
    sources_by_path: &HashMap<String, String>,
    replacements_by_path: &HashMap<String, Vec<Replacement>>,
) -> Result<HashSet<String>> {
    let mut changed = HashSet::new();

    for (path, replacements) in replacements_by_path {
        let source = sources_by_path
            .get(path)
            .ok_or_else(|| anyhow!("Replacement references unknown file `{path}`"))?;

        let rewritten = apply_replacements_to_source(source, replacements)
            .with_context(|| format!("Failed to apply replacements for `{path}`"))?;

        if rewritten != *source {
            changed.insert(path.clone());
        }
    }

    Ok(changed)
}

pub fn apply_replacements_to_file_model(
    model: &mut FileModel,
    current_source: &str,
    replacements: &[Replacement],
) -> Result<String> {
    let rendered = model.rendered();
    if rendered != current_source {
        bail!("File model is out of sync with the current source snapshot");
    }

    let ops = normalise_raw_ops(current_source, replacements)?;
    for op in ops.into_iter().rev() {
        model.replace_range(op.start, op.end, op.text)?;
    }

    Ok(model.rendered())
}

impl FileModel {
    fn new(original: String) -> Self {
        let segments = split_original_segments(&original);

        Self { original, segments }
    }

    fn rendered(&self) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Original { start, end } => out.push_str(&self.original[*start..*end]),
                Segment::Inserted(text) => out.push_str(text),
            }
        }
        out
    }

    fn is_changed(&self) -> bool {
        match self.segments.as_slice() {
            [] => !self.original.is_empty(),
            [Segment::Original { start, end }] => *start != 0 || *end != self.original.len(),
            _ => true,
        }
    }

    fn replace_range(&mut self, start: usize, end: usize, text: String) -> Result<()> {
        let start_idx = self.split_at_offset(start)?;
        let end_idx = self.split_at_offset(end)?;

        let replacement_segments = if text.is_empty() {
            Vec::new()
        } else {
            vec![Segment::Inserted(text)]
        };

        self.segments
            .splice(start_idx..end_idx, replacement_segments);
        self.normalise_segments();
        Ok(())
    }

    fn split_at_offset(&mut self, offset: usize) -> Result<usize> {
        let total_len: usize = self.segments.iter().map(Segment::len).sum();
        if offset > total_len {
            bail!("Offset {offset} is out of bounds for file model length {total_len}");
        }

        let mut cursor = 0usize;
        for index in 0..self.segments.len() {
            let segment_len = self.segments[index].len();
            let next = cursor + segment_len;

            if offset == cursor {
                return Ok(index);
            }
            if offset == next {
                return Ok(index + 1);
            }
            if offset < next {
                let split_at = offset - cursor;
                let replacement = self.segments[index].clone().split(split_at)?;
                self.segments.splice(index..=index, replacement);
                return Ok(index + 1);
            }

            cursor = next;
        }

        Ok(self.segments.len())
    }

    fn normalise_segments(&mut self) {
        let mut merged: Vec<Segment> = Vec::with_capacity(self.segments.len());

        for segment in self.segments.drain(..) {
            if segment.is_empty() {
                continue;
            }

            match (merged.last_mut(), segment) {
                (Some(Segment::Inserted(existing)), Segment::Inserted(next)) => {
                    existing.push_str(&next);
                }
                (_, next) => merged.push(next),
            }
        }

        self.segments = merged;
    }
}

impl Segment {
    fn len(&self) -> usize {
        match self {
            Segment::Original { start, end } => end - start,
            Segment::Inserted(text) => text.len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Segment::Original { start, end } => start == end,
            Segment::Inserted(text) => text.is_empty(),
        }
    }

    fn split(self, split_at: usize) -> Result<[Segment; 2]> {
        match self {
            Segment::Original { start, end } => Ok([
                Segment::Original {
                    start,
                    end: start + split_at,
                },
                Segment::Original {
                    start: start + split_at,
                    end,
                },
            ]),
            Segment::Inserted(text) => {
                if !text.is_char_boundary(split_at) {
                    bail!("Inserted segment split is not on a UTF-8 boundary");
                }

                Ok([
                    Segment::Inserted(text[..split_at].to_owned()),
                    Segment::Inserted(text[split_at..].to_owned()),
                ])
            }
        }
    }
}

fn serialise_file_model(model: &FileModel) -> Result<Vec<SavedReplacement>> {
    if !model.is_changed() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut pending_start: Option<usize> = None;
    let mut pending_end = 0usize;
    let mut pending_new_text = String::new();

    for (segment_index, segment) in model.segments.iter().enumerate() {
        match segment {
            Segment::Inserted(text) => {
                if pending_start.is_none() {
                    pending_start = Some(cursor);
                    pending_end = cursor;
                }
                pending_new_text.push_str(text);
            }
            Segment::Original { start, end } => {
                if *start < cursor {
                    bail!("Original segments are out of order in the file model");
                }

                if *start > cursor {
                    if pending_start.is_none() {
                        pending_start = Some(cursor);
                    }
                    pending_end = *start;
                }

                if let Some(change_start) = pending_start.take() {
                    out.push(make_saved_replacement_for_change(
                        model,
                        change_start,
                        pending_end,
                        std::mem::take(&mut pending_new_text),
                        segment_index,
                    )?);
                }

                cursor = *end;
            }
        }
    }

    if cursor < model.original.len() {
        if pending_start.is_none() {
            pending_start = Some(cursor);
        }
        pending_end = model.original.len();
    }

    if let Some(change_start) = pending_start {
        out.push(make_saved_replacement_for_change(
            model,
            change_start,
            pending_end,
            pending_new_text,
            model.segments.len(),
        )?);
    }

    Ok(out)
}

fn make_saved_replacement_for_change(
    model: &FileModel,
    start: usize,
    end: usize,
    new_text: String,
    next_segment_index: usize,
) -> Result<SavedReplacement> {
    if start < end {
        return make_saved_replacement(&model.original, start, end, new_text);
    }

    make_anchored_insertion_replacement(model, start, new_text, next_segment_index)
}

fn make_anchored_insertion_replacement(
    model: &FileModel,
    position: usize,
    inserted_text: String,
    next_segment_index: usize,
) -> Result<SavedReplacement> {
    if let Some((start, end)) = find_nonempty_original_before(model, next_segment_index) {
        let old_text = model.original[start..end].to_owned();
        let index = occurrence_index_at(&model.original, &old_text, start)?;
        return Ok(SavedReplacement {
            index,
            old_text: old_text.clone(),
            new_text: format!("{old_text}{inserted_text}"),
        });
    }

    if let Some((start, end)) = find_nonempty_original_after(model, next_segment_index) {
        let old_text = model.original[start..end].to_owned();
        let index = occurrence_index_at(&model.original, &old_text, start)?;
        return Ok(SavedReplacement {
            index,
            old_text: old_text.clone(),
            new_text: format!("{inserted_text}{old_text}"),
        });
    }

    bail!(
        "Could not find a non-empty original anchor for insertion at byte {}",
        position
    )
}

fn find_nonempty_original_before(
    model: &FileModel,
    next_segment_index: usize,
) -> Option<(usize, usize)> {
    model.segments[..next_segment_index]
        .iter()
        .rev()
        .find_map(|segment| original_anchor_range(&model.original, segment))
}

fn find_nonempty_original_after(
    model: &FileModel,
    next_segment_index: usize,
) -> Option<(usize, usize)> {
    model.segments[next_segment_index..]
        .iter()
        .find_map(|segment| original_anchor_range(&model.original, segment))
}

fn original_anchor_range(source: &str, segment: &Segment) -> Option<(usize, usize)> {
    let Segment::Original { start, end } = segment else {
        return None;
    };

    let text = source.get(*start..*end)?;
    if text.trim().is_empty() {
        return None;
    }

    Some((*start, *end))
}

fn split_original_segments(original: &str) -> Vec<Segment> {
    if original.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut start = 0usize;

    for (index, ch) in original.char_indices() {
        if ch == '\n' {
            segments.push(Segment::Original {
                start,
                end: index + ch.len_utf8(),
            });
            start = index + ch.len_utf8();
        }
    }

    if start < original.len() {
        segments.push(Segment::Original {
            start,
            end: original.len(),
        });
    }

    segments
}

fn make_saved_replacement(
    original: &str,
    start: usize,
    end: usize,
    new_text: String,
) -> Result<SavedReplacement> {
    let old_text = original
        .get(start..end)
        .ok_or_else(|| anyhow!("Saved replacement range is not on UTF-8 boundaries"))?
        .to_owned();
    let index = occurrence_index_at(original, &old_text, start)?;

    Ok(SavedReplacement {
        index,
        old_text,
        new_text,
    })
}

fn mirrored_json_path(out_root: &Path, rel_path: &str) -> Result<PathBuf> {
    let rel = Path::new(rel_path);
    let file_name = rel
        .file_name()
        .ok_or_else(|| anyhow!("Invalid relative file path `{rel_path}`"))?;

    let mut json_name = file_name.to_os_string();
    json_name.push(".json");

    let dir = rel.parent().unwrap_or_else(|| Path::new(""));
    Ok(out_root.join(dir).join(json_name))
}

fn apply_raw_ops(source: &str, ops: &[RawOp]) -> Result<String> {
    let mut result = source.to_string();
    let mut indexed: Vec<(usize, &RawOp)> = ops.iter().enumerate().collect();
    indexed.sort_by(|(ia, a), (ib, b)| {
        b.start
            .cmp(&a.start)
            .then_with(|| b.end.cmp(&a.end))
            .then_with(|| ib.cmp(ia))
    });

    for (_idx, op) in indexed {
        if op.start > op.end || op.end > result.len() {
            bail!(
                "Operation [{}, {}] is out of bounds for source length {}",
                op.start,
                op.end,
                result.len()
            );
        }

        if !result.is_char_boundary(op.start) || !result.is_char_boundary(op.end) {
            bail!(
                "Operation [{}, {}] is not on UTF-8 boundaries",
                op.start,
                op.end
            );
        }

        result.replace_range(op.start..op.end, &op.text);
    }

    Ok(result)
}

fn normalise_raw_ops(source: &str, replacements: &[Replacement]) -> Result<Vec<RawOp>> {
    let len = source.len();
    let mut ops = Vec::with_capacity(replacements.len());

    for (i, rep) in replacements.iter().enumerate() {
        let start = rep.range.start as usize;
        let end = rep.range.end as usize;

        if start > end {
            bail!("Replacement #{i} has start > end: [{start}..{end}]");
        }
        if end > len {
            bail!("Replacement #{i} range [{start}..{end}] is out of bounds for file length {len}");
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            bail!("Replacement #{i} range [{start}..{end}] does not lie on UTF-8 boundaries");
        }

        ops.push(RawOp {
            start,
            end,
            text: rep.text.clone(),
        });
    }

    ops.sort_by_key(|op| (op.start, op.end));
    Ok(ops)
}

fn occurrence_index_at(source: &str, needle: &str, byte_start: usize) -> Result<usize> {
    if !source.is_char_boundary(byte_start) {
        bail!("Occurrence start {byte_start} is not on a UTF-8 boundary");
    }

    if needle.is_empty() {
        let mut count = 0usize;
        let mut i = 0usize;
        loop {
            if source.is_char_boundary(i) {
                if i == byte_start {
                    return Ok(count);
                }
                count += 1;
            }

            if i == source.len() {
                break;
            }
            i += 1;
        }

        bail!("Could not resolve empty-string occurrence at byte {byte_start}");
    }

    let mut count = 0usize;
    let mut i = 0usize;

    while i + needle.len() <= source.len() {
        if source.is_char_boundary(i) && source[i..].starts_with(needle) {
            if i == byte_start {
                return Ok(count);
            }
            count += 1;
        }
        i += 1;
    }

    bail!(
        "Could not resolve occurrence for text starting at byte {}: {:?}",
        byte_start,
        needle
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use forgen_api::TextRange;

    #[test]
    fn apply_replacements_uses_reverse_offset_order() {
        let source = "let x = 1;";
        let replacements = vec![
            Replacement::insert(10, " // end".to_owned()),
            Replacement::replace(4, 5, "value".to_owned()),
        ];

        let rewritten = apply_replacements_to_source(source, &replacements).unwrap();
        assert_eq!(rewritten, "let value = 1; // end");
    }

    #[test]
    fn changed_paths_detects_only_real_changes() {
        let mut sources = HashMap::new();
        sources.insert("src/lib.rs".to_owned(), "fn main() {}".to_owned());

        let mut replacements = HashMap::new();
        replacements.insert(
            "src/lib.rs".to_owned(),
            vec![Replacement {
                range: TextRange::new(3, 7),
                text: "main".to_owned(),
            }],
        );

        let changed = changed_paths_from_replacements(&sources, &replacements).unwrap();
        assert!(changed.is_empty());
    }

    #[test]
    fn serialises_plain_insertion_without_replacing_the_whole_file() {
        let source = "fn main() {\n    let x = 1;\n}\n";
        let mut models = HashMap::new();
        models.insert("src/lib.rs".to_owned(), FileModel::new(source.to_owned()));

        let offset = source.find("\n}").unwrap();
        let rewritten = apply_replacements_to_file_model(
            models.get_mut("src/lib.rs").unwrap(),
            source,
            &[Replacement::insert(
                offset as u32,
                "\n    println!(\"x\");".to_owned(),
            )],
        )
        .unwrap();

        assert_eq!(
            rewritten,
            "fn main() {\n    let x = 1;\n    println!(\"x\");\n}\n"
        );

        let changed = collect_changed_files(&models).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].replacements.len(), 1);
        assert_eq!(changed[0].replacements[0].old_text, "    let x = 1;");
        assert_eq!(
            changed[0].replacements[0].new_text,
            "    let x = 1;\n    println!(\"x\");"
        );
    }

    #[test]
    fn serialises_nested_generated_insertions_as_one_original_boundary_edit() {
        let source = "fn main() {\n    let counter = 0;\n}\n";
        let mut model = FileModel::new(source.to_owned());

        let seeded = "/*#start:seeded-binding:seed*/\n    let seeded_runtime_value: f64 = 11 as f64;/*#end:seeded-binding:seed*/";
        let counter_insert_at = source.find(";\n").unwrap() + 1;
        let pass1 = apply_replacements_to_file_model(
            &mut model,
            source,
            &[Replacement::insert(
                counter_insert_at as u32,
                seeded.to_owned(),
            )],
        )
        .unwrap();

        let logger = "/*#start:f64-logger:log*/\n    println!(\"seeded_runtime_value: {}\", seeded_runtime_value);/*#end:f64-logger:log*/";
        let logger_insert_at = pass1.find(";/*#end:seeded-binding:seed*/").unwrap() + 1;
        apply_replacements_to_file_model(
            &mut model,
            &pass1,
            &[Replacement::insert(
                logger_insert_at as u32,
                logger.to_owned(),
            )],
        )
        .unwrap();

        let mut models = HashMap::new();
        models.insert("src/main.rs".to_owned(), model);

        let changed = collect_changed_files(&models).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].replacements.len(), 1);
        assert_eq!(changed[0].replacements[0].old_text, "    let counter = 0;");
        assert_eq!(
            changed[0].replacements[0].new_text,
            format!(
                "    let counter = 0;{prefix}{logger}{suffix}",
                prefix =
                    "/*#start:seeded-binding:seed*/\n    let seeded_runtime_value: f64 = 11 as f64;",
                logger = logger,
                suffix = "/*#end:seeded-binding:seed*/",
            )
        );
    }

    #[test]
    fn serialises_statement_append_with_statement_anchor() {
        let source =
            "pub fn distance(p1: &Point, p2: &Point) -> f64 {\n    let dx = p2.x - p1.x;\n}\n";
        let mut model = FileModel::new(source.to_owned());
        let insert_at = source.find(";\n").unwrap() + 1;

        apply_replacements_to_file_model(
            &mut model,
            source,
            &[Replacement::insert(
                insert_at as u32,
                "/*#start:f64-logger:h*/\n    println!(\"dx: {}\", dx);/*#end:f64-logger:h*/"
                    .to_owned(),
            )],
        )
        .unwrap();

        let mut models = HashMap::new();
        models.insert("src/lib.rs".to_owned(), model);

        let changed = collect_changed_files(&models).unwrap();
        assert_eq!(changed[0].replacements.len(), 1);
        assert_eq!(changed[0].replacements[0].index, 0);
        assert_eq!(
            changed[0].replacements[0].old_text,
            "    let dx = p2.x - p1.x;"
        );
        assert_eq!(
            changed[0].replacements[0].new_text,
            "    let dx = p2.x - p1.x;/*#start:f64-logger:h*/\n    println!(\"dx: {}\", dx);/*#end:f64-logger:h*/"
        );
    }

    #[test]
    fn clears_saved_replacement_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let out = root.join("target/.forgen/test/src");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("lib.rs.json"), "stale").unwrap();

        clear_saved_replacements(&root).unwrap();

        assert!(root.join("target/.forgen").exists());
        assert!(!root.join("target/.forgen/test/src/lib.rs.json").exists());
    }
}
