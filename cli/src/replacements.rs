use anyhow::{anyhow, bail, Context, Result};
use forgen_api::{FileContext, Replacement};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Replacement format written to `target/.forgen/<mirrored-path>.json`.
///
/// This stays independent from the plugin-facing `forgen_api::Replacement`
/// type on purpose:
/// - plugins still produce byte-range edits, which are ergonomic while running
///   inside the CLI with access to the original source text;
/// - proc macros later consume this serialised form, where locating the edit by
///   "nth occurrence of `old_text`" is more practical than relying on raw byte
///   ranges from `TokenStream` spans.
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone)]
struct RawOp {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct Cluster {
    start: usize,
    end: usize,
    ops: Vec<RawOp>,
}

/// Clear `target/.forgen/`, aggregate replacements from all plugins per file,
/// serialise them into the occurrence-based JSON format, and write them out.
///
/// Returns the total number of saved replacement entries written.
pub fn write_saved_replacements(
    workspace_root: &Path,
    files: &[FileContext],
    by_path: &HashMap<String, Vec<Replacement>>,
) -> Result<usize> {
    let out_root = workspace_root.join("target").join(".forgen");

    if out_root.exists() {
        fs::remove_dir_all(&out_root)
            .with_context(|| format!("Failed to clear {}", out_root.display()))?;
    }
    fs::create_dir_all(&out_root)
        .with_context(|| format!("Failed to create {}", out_root.display()))?;

    let file_map: HashMap<&str, &FileContext> = files.iter().map(|f| (f.path.as_str(), f)).collect();

    let mut paths: Vec<_> = by_path.keys().cloned().collect();
    paths.sort();

    let mut total_saved = 0usize;

    for rel_path in paths {
        let replacements = by_path
            .get(&rel_path)
            .expect("path collected from map keys must exist");

        if replacements.is_empty() {
            continue;
        }

        let file = file_map
            .get(rel_path.as_str())
            .copied()
            .ok_or_else(|| anyhow!("Replacement references unknown file `{rel_path}`"))?;

        let saved = serialise_file_replacements(&file.source, replacements)
            .with_context(|| format!("Failed to serialise replacements for `{}`", file.path))?;

        if saved.is_empty() {
            continue;
        }

        let output_path = mirrored_json_path(&out_root, &rel_path)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(&saved)?;
        fs::write(&output_path, json)
            .with_context(|| format!("Failed to write {}", output_path.display()))?;

        total_saved += saved.len();
    }

    Ok(total_saved)
}

/// Convert byte-range replacements against `source` into occurrence-based
/// replacement records.
pub fn serialise_file_replacements(
    source: &str,
    replacements: &[Replacement],
) -> Result<Vec<SavedReplacement>> {
    let ops = normalise_raw_ops(source, replacements)?;
    if ops.is_empty() {
        return Ok(Vec::new());
    }

    let clusters = cluster_ops(source, &ops)?;
    let mut out = Vec::with_capacity(clusters.len());

    for cluster in clusters {
        let original = source
            .get(cluster.start..cluster.end)
            .ok_or_else(|| anyhow!("Cluster range is not on UTF-8 boundaries"))?
            .to_string();

        let rewritten = apply_ops_to_chunk(source, cluster.start, cluster.end, &cluster.ops)?;
        let index = occurrence_index_at(source, &original, cluster.start)?;

        out.push(SavedReplacement {
            index,
            old_text: original,
            new_text: rewritten,
        });
    }

    Ok(out)
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
            bail!(
                "Replacement #{i} range [{start}..{end}] is out of bounds for file length {len}"
            );
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            bail!(
                "Replacement #{i} range [{start}..{end}] does not lie on UTF-8 boundaries"
            );
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

fn cluster_ops(source: &str, ops: &[RawOp]) -> Result<Vec<Cluster>> {
    if ops.is_empty() {
        return Ok(Vec::new());
    }

    // First pass: cluster by touching / overlapping raw spans.
    let mut raw_clusters: Vec<Cluster> = Vec::new();
    for op in ops {
        match raw_clusters.last_mut() {
            Some(last) if op.start <= last.end => {
                last.end = last.end.max(op.end);
                last.ops.push(op.clone());
            }
            Some(last) if op.start == last.end => {
                last.end = last.end.max(op.end);
                last.ops.push(op.clone());
            }
            _ => raw_clusters.push(Cluster {
                start: op.start,
                end: op.end,
                ops: vec![op.clone()],
            }),
        }
    }

    // Second pass: ensure insertion-only clusters have at least one byte of
    // surrounding original text so `old_text` is non-empty, then merge any
    // clusters that would overlap after that expansion.
    let mut merged: Vec<Cluster> = Vec::new();

    for cluster in raw_clusters {
        let (adj_start, adj_end) = expanded_bounds(source, cluster.start, cluster.end);

        match merged.last_mut() {
            Some(prev) => {
                let (prev_adj_start, prev_adj_end) = expanded_bounds(source, prev.start, prev.end);

                if adj_start <= prev_adj_end && prev_adj_start <= adj_end {
                    prev.start = prev.start.min(cluster.start);
                    prev.end = prev.end.max(cluster.end);
                    prev.ops.extend(cluster.ops);
                } else {
                    merged.push(cluster);
                }
            }
            None => merged.push(cluster),
        }
    }

    let mut out = Vec::with_capacity(merged.len());

    for mut cluster in merged {
        let (start, end) = expanded_bounds(source, cluster.start, cluster.end);
        cluster.start = start;
        cluster.end = end;
        cluster.ops.sort_by_key(|op| (op.start, op.end));
        out.push(cluster);
    }

    Ok(out)
}

fn expanded_bounds(source: &str, start: usize, end: usize) -> (usize, usize) {
    if start != end {
        return (start, end);
    }

    if source.is_empty() {
        return (0, 0);
    }

    if end < source.len() {
        let next = next_char_end(source, end).unwrap_or(end);
        return (start, next.max(start));
    }

    if start > 0 {
        let prev = prev_char_start(source, start).unwrap_or(start);
        return (prev.min(end), end);
    }

    (start, end)
}

fn apply_ops_to_chunk(source: &str, chunk_start: usize, chunk_end: usize, ops: &[RawOp]) -> Result<String> {
    let original = source
        .get(chunk_start..chunk_end)
        .ok_or_else(|| anyhow!("Chunk range [{chunk_start}..{chunk_end}] is invalid UTF-8"))?;

    let mut result = original.to_string();

    // Apply in descending order, and for identical starts apply later entries
    // first to match the old "reverse application" semantics.
    let mut indexed: Vec<(usize, &RawOp)> = ops.iter().enumerate().collect();
    indexed.sort_by(|(ia, a), (ib, b)| {
        b.start
            .cmp(&a.start)
            .then_with(|| b.end.cmp(&a.end))
            .then_with(|| ib.cmp(ia))
    });

    for (_idx, op) in indexed {
        if op.start < chunk_start || op.end > chunk_end {
            bail!(
                "Operation [{}, {}] falls outside chunk [{}, {}]",
                op.start,
                op.end,
                chunk_start,
                chunk_end
            );
        }

        let local_start = op.start - chunk_start;
        let local_end = op.end - chunk_start;

        if local_start > local_end || local_end > result.len() {
            bail!(
                "Local operation [{}, {}] is invalid for chunk of length {}",
                local_start,
                local_end,
                result.len()
            );
        }

        if !result.is_char_boundary(local_start) || !result.is_char_boundary(local_end) {
            bail!(
                "Local operation [{}, {}] is not on UTF-8 boundaries",
                local_start,
                local_end
            );
        }

        result.replace_range(local_start..local_end, &op.text);
    }

    Ok(result)
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

fn next_char_end(source: &str, start: usize) -> Option<usize> {
    let slice = source.get(start..)?;
    let ch = slice.chars().next()?;
    Some(start + ch.len_utf8())
}

fn prev_char_start(source: &str, start: usize) -> Option<usize> {
    if start == 0 {
        return None;
    }

    let mut i = start - 1;
    while i > 0 && !source.is_char_boundary(i) {
        i -= 1;
    }

    if source.is_char_boundary(i) {
        Some(i)
    } else {
        None
    }
}
