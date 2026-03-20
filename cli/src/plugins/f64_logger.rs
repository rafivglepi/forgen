use forgen_api::{FileReplacement, Plugin, Replacement, WorkspaceContext};

/// Inserts a `println!` trace line after every `let` binding whose type is
/// `f64`, whether the annotation is written explicitly (`let x: f64 = …`)
/// or inferred (`let x = some_f64_expr`).
///
/// This plugin is a built-in example that ships with `cargo-forgen`. It also
/// serves as a reference implementation for authors writing their own dylib
/// plugins: notice that it only depends on `forgen-api` — no `ra_ap_*` crates.
pub struct F64LoggerPlugin;

impl Plugin for F64LoggerPlugin {
    fn name(&self) -> &str {
        "f64-logger"
    }

    fn run(&self, ctx: &WorkspaceContext) -> Vec<FileReplacement> {
        let mut results = Vec::new();

        for file in &ctx.files {
            let mut replacements = Vec::new();

            for binding in file.bindings_of_type("f64") {
                // Insertion point: right after the closing `;` of the statement.
                let insert_at = binding.range.end;

                // Replicate the indentation of the line that contains the `let`.
                let indent = leading_indent(&file.source, insert_at);

                replacements.push(Replacement::insert(
                    insert_at,
                    format!(
                        "\n{indent}println!(\"{name}: {{}}\", {name});",
                        indent = indent,
                        name = binding.name,
                    ),
                ));
            }

            if !replacements.is_empty() {
                results.push(FileReplacement::new(file.path.clone(), replacements));
            }
        }

        results
    }
}

/// Returns the leading whitespace (spaces and tabs) of the line that contains
/// `offset` (a byte offset into `source`).
fn leading_indent(source: &str, offset: u32) -> String {
    let up_to = (offset as usize).min(source.len());
    let line_start = source[..up_to].rfind('\n').map(|i| i + 1).unwrap_or(0);

    source[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

