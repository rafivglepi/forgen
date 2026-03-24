#![feature(proc_macro_tracked_path)]

extern crate proc_macro;

use proc_macro::{tracked, Group, Span, TokenStream, TokenTree};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use syn::LitStr;

#[derive(Debug, Clone, Deserialize)]
struct SavedReplacement {
    index: usize,
    old_text: String,
    new_text: String,
}

#[derive(Debug, Clone)]
struct ResolvedReplacement {
    start: usize,
    end: usize,
    new_text: String,
}

#[proc_macro_attribute]
pub fn file(attr: TokenStream, input: TokenStream) -> TokenStream {
    match expand_file(attr, input) {
        Ok(ts) => ts,
        Err(msg) => compile_error_ts(&msg),
    }
}

fn expand_file(attr: TokenStream, input: TokenStream) -> Result<TokenStream, String> {
    let declared_rel_path = parse_attr_path(attr)?;

    let invocation_file = first_local_file(&input).ok_or_else(|| {
        "forgen::file could not determine the local source file path from the input tokens"
            .to_string()
    })?;

    let workspace_root = find_workspace_root(&invocation_file).ok_or_else(|| {
        format!(
            "forgen::file could not find a Cargo workspace root for `{}`",
            invocation_file.display()
        )
    })?;

    let expected_file = workspace_root.join(normalize_rel_path(&declared_rel_path));
    let invocation_file_norm = normalize_path(&invocation_file);
    let expected_file_norm = normalize_path(&expected_file);

    if invocation_file_norm != expected_file_norm {
        return Err(format!(
            "forgen::file path mismatch: attribute declared `{}`, but macro is expanding `{}`",
            declared_rel_path, invocation_file_norm
        ));
    }

    let json_path = workspace_root
        .join("target")
        .join(".forgen")
        .join(format!("{}.json", normalize_rel_path(&declared_rel_path)));

    track(&json_path);

    if !json_path.exists() {
        return Ok(input);
    }

    let json = fs::read_to_string(&json_path).map_err(|e| {
        format!(
            "forgen::file failed to read replacement file `{}`: {}",
            json_path.display(),
            e
        )
    })?;

    let saved: Vec<SavedReplacement> = serde_json::from_str(&json).map_err(|e| {
        format!(
            "forgen::file failed to parse replacement file `{}` as JSON: {}",
            json_path.display(),
            e
        )
    })?;

    if saved.is_empty() {
        return Ok(input);
    }

    let original_source = fs::read_to_string(&invocation_file).map_err(|e| {
        format!(
            "forgen::file failed to read source file `{}`: {}",
            invocation_file.display(),
            e
        )
    })?;

    let source_without_attr = remove_forgen_attr_line(&original_source, &declared_rel_path)
        .ok_or_else(|| {
            format!(
                "forgen::file could not remove `#![forgen::file(\"{}\")]` from `{}`",
                declared_rel_path,
                invocation_file.display()
            )
        })?;

    let input_tokens: Vec<TokenTree> = input.clone().into_iter().collect();

    let (original_body, _rewritten_without_attr) =
        choose_body_candidate_pair(&source_without_attr, &input_tokens).ok_or_else(|| {
            "forgen::file could not derive the original source body token stream".to_string()
        })?;

    let resolved = resolve_saved_replacements(&original_source, &saved)?;
    let rewritten_full_source = apply_resolved_replacements(&original_source, &resolved)?;
    let rewritten_without_attr =
        remove_forgen_attr_line(&rewritten_full_source, &declared_rel_path).ok_or_else(|| {
            format!(
                "forgen::file could not remove rewritten `#![forgen::file(\"{}\")]` line",
                declared_rel_path
            )
        })?;

    let rewritten_body =
        choose_rewritten_body_tokens(&source_without_attr, &rewritten_without_attr, &input_tokens)
            .ok_or_else(|| {
                "forgen::file could not derive the rewritten source body token stream".to_string()
            })?;

    let merged =
        merge_body_into_input(&input_tokens, &original_body, &rewritten_body).ok_or_else(|| {
            "forgen::file could not align the source body with the proc-macro input token stream"
                .to_string()
        })?;

    Ok(merged)
}

fn parse_attr_path(attr: TokenStream) -> Result<String, String> {
    syn::parse::<LitStr>(attr)
        .map(|lit| lit.value())
        .map_err(|_| {
            "forgen::file expected a single string literal path like `\"src/lib.rs\"`".to_string()
        })
}

fn resolve_saved_replacements(
    source: &str,
    saved: &[SavedReplacement],
) -> Result<Vec<ResolvedReplacement>, String> {
    let mut out = Vec::with_capacity(saved.len());

    for rep in saved {
        let start = find_occurrence_start(source, &rep.old_text, rep.index).ok_or_else(|| {
            format!(
                "forgen::file could not find occurrence {} of `{}` in source",
                rep.index, rep.old_text
            )
        })?;

        let end = start + rep.old_text.len();

        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(format!(
                "forgen::file resolved a replacement to non-character boundaries: [{}..{}]",
                start, end
            ));
        }

        if &source[start..end] != rep.old_text {
            return Err(format!(
                "forgen::file resolved replacement text mismatch at [{}..{}]",
                start, end
            ));
        }

        out.push(ResolvedReplacement {
            start,
            end,
            new_text: rep.new_text.clone(),
        });
    }

    out.sort_by_key(|r| (r.start, r.end));

    for pair in out.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if a.end > b.start {
            return Err(format!(
                "forgen::file found overlapping saved replacements: [{}..{}) overlaps [{}..{})",
                a.start, a.end, b.start, b.end
            ));
        }
    }

    Ok(out)
}

fn apply_resolved_replacements(
    source: &str,
    replacements: &[ResolvedReplacement],
) -> Result<String, String> {
    let mut out = source.to_string();

    for rep in replacements.iter().rev() {
        if rep.start > rep.end || rep.end > out.len() {
            return Err(format!(
                "forgen::file found out-of-bounds replacement range [{}..{})",
                rep.start, rep.end
            ));
        }
        if !out.is_char_boundary(rep.start) || !out.is_char_boundary(rep.end) {
            return Err(format!(
                "forgen::file found non-character-boundary replacement range [{}..{})",
                rep.start, rep.end
            ));
        }

        out.replace_range(rep.start..rep.end, &rep.new_text);
    }

    Ok(out)
}

fn find_occurrence_start(source: &str, needle: &str, target_index: usize) -> Option<usize> {
    if needle.is_empty() {
        let mut seen = 0usize;
        for i in 0..=source.len() {
            if !source.is_char_boundary(i) {
                continue;
            }
            if seen == target_index {
                return Some(i);
            }
            seen += 1;
        }
        return None;
    }

    let mut seen = 0usize;
    for i in 0..source.len() {
        if !source.is_char_boundary(i) {
            continue;
        }
        if source[i..].starts_with(needle) {
            if seen == target_index {
                return Some(i);
            }
            seen += 1;
        }
    }

    None
}

fn remove_forgen_attr_line(source: &str, declared_rel_path: &str) -> Option<String> {
    let normalized = normalize_rel_path(declared_rel_path);

    let candidates = [
        format!("#![forgen::file(\"{}\")]", declared_rel_path),
        format!("#![forgen::file(\"{}\")]", normalized),
    ];

    for candidate in candidates {
        if let Some(s) = remove_line_containing(source, &candidate) {
            return Some(s);
        }
    }

    None
}

fn remove_line_containing(source: &str, needle: &str) -> Option<String> {
    let idx = source.find(needle)?;
    let line_start = source[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = source[idx..]
        .find('\n')
        .map(|off| idx + off + 1)
        .unwrap_or(source.len());

    let mut out = String::with_capacity(source.len().saturating_sub(line_end - line_start));
    out.push_str(&source[..line_start]);
    out.push_str(&source[line_end..]);
    Some(out)
}

fn choose_body_candidate_pair(
    source_without_attr: &str,
    input_tokens: &[TokenTree],
) -> Option<(Vec<TokenTree>, String)> {
    let mut current = source_without_attr.to_string();
    let mut candidates = Vec::new();

    loop {
        if let Ok(ts) = TokenStream::from_str(&current) {
            candidates.push((ts.into_iter().collect::<Vec<_>>(), current.clone()));
        }

        match remove_first_inner_attr_line(&current) {
            Some(next) => current = next,
            None => break,
        }
    }

    candidates.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));

    for (candidate, source) in candidates {
        if find_fuzzy_subsequence_range(input_tokens, &candidate).is_some() {
            return Some((candidate, source));
        }
    }

    None
}

fn choose_rewritten_body_tokens(
    original_source_without_attr: &str,
    rewritten_without_attr: &str,
    input_tokens: &[TokenTree],
) -> Option<Vec<TokenTree>> {
    let mut original_current = original_source_without_attr.to_string();
    let mut rewritten_current = rewritten_without_attr.to_string();
    let mut candidates = Vec::new();

    loop {
        if let Ok(ts) = TokenStream::from_str(&original_current) {
            candidates.push((
                ts.into_iter().collect::<Vec<_>>(),
                rewritten_current.clone(),
            ));
        }

        match (
            remove_first_inner_attr_line(&original_current),
            remove_first_inner_attr_line(&rewritten_current),
        ) {
            (Some(next_original), Some(next_rewritten)) => {
                original_current = next_original;
                rewritten_current = next_rewritten;
            }
            _ => break,
        }
    }

    candidates.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));

    for (original_candidate, rewritten_source) in candidates {
        if find_fuzzy_subsequence_range(input_tokens, &original_candidate).is_some() {
            if let Ok(ts) = TokenStream::from_str(&rewritten_source) {
                return Some(ts.into_iter().collect());
            }
        }
    }

    None
}

fn merge_body_into_input(
    input_tokens: &[TokenTree],
    original_body: &[TokenTree],
    rewritten_body: &[TokenTree],
) -> Option<TokenStream> {
    let (start, end) = find_fuzzy_subsequence_range(input_tokens, original_body)?;
    Some(merge_body_region(
        &input_tokens[..start],
        &input_tokens[start..end],
        rewritten_body,
        &input_tokens[end..],
    ))
}

fn remove_first_inner_attr_line(source: &str) -> Option<String> {
    let trimmed = source.trim_start_matches(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n');
    let leading_ws_len = source.len() - trimmed.len();

    if !trimmed.starts_with("#![") {
        return None;
    }

    let after_hash = leading_ws_len;
    let line_end = source[after_hash..]
        .find('\n')
        .map(|off| after_hash + off + 1)
        .unwrap_or(source.len());

    let mut out = String::with_capacity(source.len().saturating_sub(line_end - after_hash));
    out.push_str(&source[..after_hash]);
    out.push_str(&source[line_end..]);
    Some(out)
}

fn merge_body_region(
    prefix: &[TokenTree],
    old_body: &[TokenTree],
    new_body: &[TokenTree],
    suffix: &[TokenTree],
) -> TokenStream {
    let mut out = TokenStream::new();

    for token in prefix {
        out.extend([token.clone()]);
    }

    out.extend(merge_token_vecs(old_body, new_body));

    for token in suffix {
        out.extend([token.clone()]);
    }

    out
}

fn merge_streams(old_stream: TokenStream, new_stream: TokenStream) -> TokenStream {
    let old: Vec<TokenTree> = old_stream.into_iter().collect();
    let new: Vec<TokenTree> = new_stream.into_iter().collect();
    merge_token_vecs(&old, &new)
}

fn merge_token_vecs(old: &[TokenTree], new: &[TokenTree]) -> TokenStream {
    let lcs = lcs_pairs(old, new);
    let mut out = TokenStream::new();

    let mut old_i = 0usize;
    let mut new_i = 0usize;

    for (old_match, new_match) in lcs {
        out.extend(merge_changed_region(
            &old[old_i..old_match],
            &new[new_i..new_match],
        ));

        let merged = merge_single_token(&old[old_match], &new[new_match]);
        out.extend([merged]);

        old_i = old_match + 1;
        new_i = new_match + 1;
    }

    out.extend(merge_changed_region(&old[old_i..], &new[new_i..]));
    out
}

fn merge_changed_region(old: &[TokenTree], new: &[TokenTree]) -> TokenStream {
    if new.is_empty() {
        return TokenStream::new();
    }

    if old.is_empty() {
        let span = Span::call_site();
        return assign_span_stream(new.iter().cloned().collect(), span);
    }

    let anchor = old[0].span();
    assign_span_stream(new.iter().cloned().collect(), anchor)
}

fn merge_single_token(old: &TokenTree, new: &TokenTree) -> TokenTree {
    match (old, new) {
        (TokenTree::Group(old_group), TokenTree::Group(new_group))
            if old_group.delimiter() == new_group.delimiter() =>
        {
            let merged_inner = merge_streams(old_group.stream(), new_group.stream());
            let mut group = Group::new(new_group.delimiter(), merged_inner);
            group.set_span(old_group.span());
            TokenTree::Group(group)
        }
        _ => old.clone(),
    }
}

fn lcs_pairs(old: &[TokenTree], new: &[TokenTree]) -> Vec<(usize, usize)> {
    let m = old.len();
    let n = new.len();

    if m == 0 || n == 0 {
        return Vec::new();
    }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in (0..m).rev() {
        for j in (0..n).rev() {
            if tokens_equal_for_anchor(&old[i], &new[j]) {
                dp[i][j] = dp[i + 1][j + 1] + 1;
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut out = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;

    while i < m && j < n {
        if tokens_equal_for_anchor(&old[i], &new[j]) {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    out
}

fn tokens_equal_for_anchor(a: &TokenTree, b: &TokenTree) -> bool {
    match (a, b) {
        (TokenTree::Ident(a), TokenTree::Ident(b)) => a.to_string() == b.to_string(),
        (TokenTree::Punct(a), TokenTree::Punct(b)) => {
            a.as_char() == b.as_char() && a.spacing() == b.spacing()
        }
        (TokenTree::Literal(a), TokenTree::Literal(b)) => a.to_string() == b.to_string(),
        (TokenTree::Group(a), TokenTree::Group(b)) => a.delimiter() == b.delimiter(),
        _ => false,
    }
}

fn token_trees_equal(a: &TokenTree, b: &TokenTree) -> bool {
    match (a, b) {
        (TokenTree::Ident(a), TokenTree::Ident(b)) => a.to_string() == b.to_string(),
        (TokenTree::Punct(a), TokenTree::Punct(b)) => {
            a.as_char() == b.as_char() && a.spacing() == b.spacing()
        }
        (TokenTree::Literal(a), TokenTree::Literal(b)) => a.to_string() == b.to_string(),
        (TokenTree::Group(a), TokenTree::Group(b)) => {
            a.delimiter() == b.delimiter()
                && token_vecs_equal(
                    &a.stream().into_iter().collect::<Vec<_>>(),
                    &b.stream().into_iter().collect::<Vec<_>>(),
                )
        }
        _ => false,
    }
}

fn token_vecs_equal(a: &[TokenTree], b: &[TokenTree]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| token_trees_equal(a, b))
}

fn find_subsequence_range(haystack: &[TokenTree], needle: &[TokenTree]) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return Some((0, 0));
    }

    if needle.len() > haystack.len() {
        return None;
    }

    for start in 0..=haystack.len() - needle.len() {
        let end = start + needle.len();
        if token_vecs_equal(&haystack[start..end], needle) {
            return Some((start, end));
        }
    }

    None
}

fn find_fuzzy_subsequence_range(
    haystack: &[TokenTree],
    needle: &[TokenTree],
) -> Option<(usize, usize)> {
    if let Some(range) = find_subsequence_range(haystack, needle) {
        return Some(range);
    }

    let normalized_needle = strip_optional_trailing_commas(needle);

    if normalized_needle.is_empty() {
        return Some((0, 0));
    }

    for start in 0..haystack.len() {
        for end in (start + 1)..=haystack.len() {
            let normalized_hay = strip_optional_trailing_commas(&haystack[start..end]);
            if token_vecs_equal(&normalized_hay, &normalized_needle) {
                return Some((start, end));
            }
        }
    }

    None
}

fn strip_optional_trailing_commas(tokens: &[TokenTree]) -> Vec<TokenTree> {
    let mut out = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                let normalized_inner = strip_optional_trailing_commas(&inner);
                let trimmed_inner = trim_trailing_comma(&normalized_inner);

                let mut new_group =
                    Group::new(group.delimiter(), trimmed_inner.into_iter().collect());
                new_group.set_span(group.span());
                out.push(TokenTree::Group(new_group));
            }
            other => out.push(other.clone()),
        }
    }

    trim_trailing_comma(&out)
}

fn trim_trailing_comma(tokens: &[TokenTree]) -> Vec<TokenTree> {
    let mut out = tokens.to_vec();

    if matches!(out.last(), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
        out.pop();
    }

    out
}

fn assign_span_stream(stream: TokenStream, span: Span) -> TokenStream {
    stream
        .into_iter()
        .map(|tt| assign_span_tree(tt, span))
        .collect::<TokenStream>()
}

fn assign_span_tree(tree: TokenTree, span: Span) -> TokenTree {
    match tree {
        TokenTree::Group(group) => {
            let inner = assign_span_stream(group.stream(), span);
            let mut new_group = Group::new(group.delimiter(), inner);
            new_group.set_span(span);
            TokenTree::Group(new_group)
        }
        TokenTree::Ident(mut ident) => {
            ident.set_span(span);
            TokenTree::Ident(ident)
        }
        TokenTree::Punct(mut punct) => {
            punct.set_span(span);
            TokenTree::Punct(punct)
        }
        TokenTree::Literal(mut lit) => {
            lit.set_span(span);
            TokenTree::Literal(lit)
        }
    }
}

fn first_local_file(stream: &TokenStream) -> Option<PathBuf> {
    for tt in stream.clone() {
        if let Some(path) = local_file_of_tree(&tt) {
            return Some(path);
        }
    }
    None
}

fn local_file_of_tree(tree: &TokenTree) -> Option<PathBuf> {
    match tree {
        TokenTree::Group(group) => local_file_of_group(group),
        _ => tree.span().local_file(),
    }
}

fn local_file_of_group(group: &Group) -> Option<PathBuf> {
    group
        .span_open()
        .local_file()
        .or_else(|| group.span().local_file())
}

fn find_workspace_root(start_file: &Path) -> Option<PathBuf> {
    let mut dir = start_file.parent()?.to_path_buf();

    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).ok()?;
            if content.contains("[workspace]") {
                return Some(dir);
            }
        }

        if !dir.pop() {
            return None;
        }
    }
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn track(path: &Path) {
    let s = path.to_string_lossy();
    tracked::path(&*s);
}

fn compile_error_ts(msg: &str) -> TokenStream {
    let escaped = string_literal(msg);
    TokenStream::from_str(&format!("::std::compile_error!({escaped});"))
        .unwrap_or_else(|_| TokenStream::new())
}

fn string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
