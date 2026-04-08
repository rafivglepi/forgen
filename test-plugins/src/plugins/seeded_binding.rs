use forgen_api::rand::Rng;
use forgen_api::{FileReplacement, Plugin, PluginRuntime, Replacement, WorkspaceContext};

pub struct SeededBindingPlugin;

impl Plugin for SeededBindingPlugin {
    fn name(&self) -> &str {
        "seeded-binding"
    }

    fn run(&self, ctx: &WorkspaceContext, runtime: &mut PluginRuntime<'_>) -> Vec<FileReplacement> {
        let mut results = Vec::new();

        for file in &ctx.files {
            if file.path != "test/src/main.rs" {
                continue;
            }

            if file
                .generated_regions_for(runtime.plugin_id())
                .next()
                .is_some()
            {
                continue;
            }

            let Some(anchor) = file.binding("counter") else {
                continue;
            };

            let mut rng = runtime.rng_for_file(&file.path);
            let sample: u8 = rng.gen_range(10..=99);

            results.push(FileReplacement::new(
                file.path.clone(),
                vec![Replacement::insert(
                    anchor.range.end,
                    format!(
                        "let seeded_runtime_value: f64 = {sample} as f64;",
                        sample = sample,
                    ),
                )],
            ));
        }

        results
    }
}
