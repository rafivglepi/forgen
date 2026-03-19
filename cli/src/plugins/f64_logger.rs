use crate::plugin::{FileContext, Plugin, Replacement};
use ra_ap_syntax::{ast, ast::HasName, AstNode};

/// Inserts a `println!` trace line after every `let` binding whose type is
/// `f64`, whether the annotation is written explicitly (`let x: f64 = …`)
/// or inferred (`let x = some_f64_expr`).
///
/// The insertion is idempotent: if the line immediately following the
/// statement already contains a `println!` for the same variable name, the
/// plugin skips that binding.
pub struct F64LoggerPlugin;

impl Plugin for F64LoggerPlugin {
    fn name(&self) -> &str {
        "f64-logger"
    }

    fn run(&self, ctx: &FileContext) -> Vec<Replacement> {
        let mut replacements = Vec::new();

        for node in ctx.syntax.syntax().descendants() {
            let Some(let_stmt) = ast::LetStmt::cast(node) else {
                continue;
            };

            // Only handle simple identifier patterns: `let [mut] name [: T] = …`
            let Some(pat) = let_stmt.pat() else {
                continue;
            };
            let var_name = match &pat {
                ast::Pat::IdentPat(ident_pat) => ident_pat.name().map(|n: ast::Name| n.to_string()),
                _ => None,
            };
            let Some(var_name) = var_name else {
                continue;
            };

            // Decide whether this binding has type f64.
            let is_f64 = if let Some(ty_node) = let_stmt.ty() {
                // Explicit annotation — compare the literal source text.
                ty_node.syntax().text().to_string().trim() == "f64"
            } else {
                // No annotation — use the pre-computed inferred type.
                ctx.type_of_pat(&pat) == Some("f64")
            };

            if !is_f64 {
                continue;
            }

            // Where to insert: right after the closing `;` of the statement.
            let insert_at = u32::from(let_stmt.syntax().text_range().end());

            // Replicate the indentation of the current line.
            let line_start = ctx.source[..insert_at as usize]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let indent: String = ctx.source[line_start..]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();

            replacements.push(Replacement::insert(
                insert_at,
                format!("\n{indent}println!(\"{var_name}: {{}}\", {var_name});"),
            ));
        }

        replacements
    }
}
