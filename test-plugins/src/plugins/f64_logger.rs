use forgen_api::{FileReplacement, Plugin, PluginRuntime, Replacement, WorkspaceContext};

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

    fn run(&self, ctx: &WorkspaceContext, runtime: &mut PluginRuntime<'_>) -> Vec<FileReplacement> {
        let mut results = Vec::new();

        for file in &ctx.files {
            let mut replacements = Vec::new();

            for binding in file.bindings_of_type("f64") {
                if already_logged(file, runtime.plugin_id(), &binding.name) {
                    continue;
                }

                replacements.push(Replacement::insert(
                    // Insertion point: right after the closing `;` of the statement.
                    binding.range.end,
                    format!("println!(\"{name}: {{}}\", {name});", name = binding.name,),
                ));
            }

            if !replacements.is_empty() {
                results.push(FileReplacement::new(file.path.clone(), replacements));
            }
        }

        results
    }
}

fn already_logged(file: &forgen_api::FileContext, plugin_id: &str, binding_name: &str) -> bool {
    file.generated_regions_for(plugin_id).any(|region| {
        let start = region.inner_range.start as usize;
        let end = region.inner_range.end as usize;
        file.source()
            .get(start..end)
            .map(|text| text.contains(&format!("\"{binding_name}: {{}}\"")))
            .unwrap_or(false)
    })
}
