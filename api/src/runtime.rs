use crate::{FileReplacement, Plugin, Replacement, TextRange, WorkspaceContext};
use rand::{rngs::StdRng, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

const START_MARKER_PREFIX: &str = "/*#start:";
const END_MARKER_PREFIX: &str = "/*#end:";
const MARKER_SUFFIX: &str = "*/";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginState {
    values: Map<String, Value>,
}

impl PluginState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn try_get<T>(&self, key: &str) -> serde_json::Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        self.values
            .get(key)
            .cloned()
            .map(serde_json::from_value)
            .transpose()
    }

    pub fn get<T>(&self, key: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.try_get(key).ok().flatten()
    }

    pub fn set_value(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.values.insert(key.into(), value)
    }

    pub fn set<T>(&mut self, key: impl Into<String>, value: T) -> serde_json::Result<()>
    where
        T: Serialize,
    {
        self.values.insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    pub fn remove_value(&mut self, key: &str) -> Option<Value> {
        self.values.remove(key)
    }

    pub fn try_remove<T>(&mut self, key: &str) -> serde_json::Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        self.values
            .remove(key)
            .map(serde_json::from_value)
            .transpose()
    }

    pub fn remove<T>(&mut self, key: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.try_remove(key).ok().flatten()
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteRuntime {
    suite_seed: u64,
    plugin_states: HashMap<String, PluginState>,
}

impl Default for SuiteRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SuiteRuntime {
    pub fn new() -> Self {
        Self::with_seed(rand::random())
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            suite_seed: seed,
            plugin_states: HashMap::new(),
        }
    }

    pub fn seed(&self) -> u64 {
        self.suite_seed
    }

    pub fn plugin_state(&self, plugin_id: &str) -> Option<&PluginState> {
        self.plugin_states.get(plugin_id)
    }

    pub fn plugin_state_mut(&mut self, plugin_id: &str) -> &mut PluginState {
        self.plugin_states.entry(plugin_id.to_owned()).or_default()
    }

    pub fn run_plugin<P>(&mut self, plugin: &P, ctx: &WorkspaceContext) -> Vec<FileReplacement>
    where
        P: Plugin,
    {
        let plugin_id = plugin.name().to_owned();
        if !is_valid_plugin_id(&plugin_id) {
            eprintln!(
                "[forgen] plugin id `{plugin_id}` is invalid; Plugin::name() must contain only ASCII letters, digits, '_' or '-'. Skipping plugin output.",
            );
            return Vec::new();
        }

        let raw = {
            let state = self.plugin_states.entry(plugin_id.clone()).or_default();
            let mut runtime = PluginRuntime {
                plugin_id: &plugin_id,
                suite_seed: self.suite_seed,
                state,
            };
            plugin.run(ctx, &mut runtime)
        };

        wrap_file_replacements(&plugin_id, raw)
    }
}

#[derive(Debug)]
pub struct PluginRuntime<'a> {
    plugin_id: &'a str,
    suite_seed: u64,
    state: &'a mut PluginState,
}

impl<'a> PluginRuntime<'a> {
    pub fn plugin_id(&self) -> &str {
        self.plugin_id
    }

    pub fn state(&mut self) -> &mut PluginState {
        self.state
    }

    pub fn rng_for_file(&self, file_path: &str) -> StdRng {
        StdRng::seed_from_u64(derive_file_seed(self.suite_seed, self.plugin_id, file_path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedRegion {
    pub plugin_id: String,
    pub hash: String,
    pub full_range: TextRange,
    pub inner_range: TextRange,
    pub start_marker_range: TextRange,
    pub end_marker_range: TextRange,
}

pub fn is_valid_plugin_id(plugin_id: &str) -> bool {
    !plugin_id.is_empty()
        && plugin_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn parse_generated_regions(source: &str) -> Vec<GeneratedRegion> {
    let mut stack: Vec<ParsedMarker> = Vec::new();
    let mut regions = Vec::new();
    let mut index = 0usize;

    while index < source.len() {
        let Some(marker) = parse_marker_at(source, index) else {
            index += 1;
            continue;
        };

        let marker_end = marker.end;

        match marker.kind {
            MarkerKind::Start => stack.push(marker),
            MarkerKind::End => {
                if let Some(open_index) = stack.iter().rposition(|open| {
                    open.plugin_id == marker.plugin_id && open.hash == marker.hash
                }) {
                    let open = stack.remove(open_index);
                    regions.push(GeneratedRegion {
                        plugin_id: open.plugin_id,
                        hash: open.hash,
                        full_range: TextRange::new(open.start as u32, marker.end as u32),
                        inner_range: TextRange::new(open.end as u32, marker.start as u32),
                        start_marker_range: TextRange::new(open.start as u32, open.end as u32),
                        end_marker_range: TextRange::new(marker.start as u32, marker.end as u32),
                    });
                }
            }
        }

        index = marker_end;
    }

    regions.sort_by_key(|region| (region.full_range.start, region.full_range.end));
    regions
}

fn wrap_file_replacements(
    plugin_id: &str,
    file_replacements: Vec<FileReplacement>,
) -> Vec<FileReplacement> {
    file_replacements
        .into_iter()
        .map(|mut file_replacement| {
            file_replacement.replacements = file_replacement
                .replacements
                .into_iter()
                .map(|replacement| wrap_replacement(plugin_id, replacement))
                .collect();
            file_replacement
        })
        .collect()
}

fn wrap_replacement(plugin_id: &str, mut replacement: Replacement) -> Replacement {
    if replacement.text.is_empty() {
        return replacement;
    }

    replacement.text = wrap_generated_text(plugin_id, &replacement.text);
    replacement
}

fn wrap_generated_text(plugin_id: &str, text: &str) -> String {
    let hash = marker_hash(plugin_id, text);
    format!(
        "{START_MARKER_PREFIX}{plugin_id}:{hash}{MARKER_SUFFIX}{text}{END_MARKER_PREFIX}{plugin_id}:{hash}{MARKER_SUFFIX}"
    )
}

fn marker_hash(plugin_id: &str, text: &str) -> String {
    let mut bytes = Vec::with_capacity(plugin_id.len() + text.len() + 1);
    bytes.extend_from_slice(plugin_id.as_bytes());
    bytes.push(0xff);
    bytes.extend_from_slice(text.as_bytes());
    format!("{:016x}", fnv1a64(&bytes))
}

fn derive_file_seed(suite_seed: u64, plugin_id: &str, file_path: &str) -> u64 {
    let mut bytes = Vec::with_capacity(plugin_id.len() + file_path.len() + 17);
    bytes.extend_from_slice(&suite_seed.to_le_bytes());
    bytes.push(0xfe);
    bytes.extend_from_slice(plugin_id.as_bytes());
    bytes.push(0xff);
    bytes.extend_from_slice(file_path.as_bytes());
    fnv1a64(&bytes)
}

const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut index = 0usize;

    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
        index += 1;
    }

    hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Start,
    End,
}

#[derive(Debug, Clone)]
struct ParsedMarker {
    kind: MarkerKind,
    plugin_id: String,
    hash: String,
    start: usize,
    end: usize,
}

fn parse_marker_at(source: &str, start: usize) -> Option<ParsedMarker> {
    let rest = source.get(start..)?;
    let (kind, prefix) = if rest.starts_with(START_MARKER_PREFIX) {
        (MarkerKind::Start, START_MARKER_PREFIX)
    } else if rest.starts_with(END_MARKER_PREFIX) {
        (MarkerKind::End, END_MARKER_PREFIX)
    } else {
        return None;
    };

    let marker_end = rest.find(MARKER_SUFFIX)?;
    let payload = rest.get(prefix.len()..marker_end)?;
    let (plugin_id, hash) = payload.rsplit_once(':')?;

    if plugin_id.is_empty() || hash.is_empty() {
        return None;
    }

    Some(ParsedMarker {
        kind,
        plugin_id: plugin_id.to_owned(),
        hash: hash.to_owned(),
        start,
        end: start + marker_end + MARKER_SUFFIX.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirNode, LazyValue, WorkspaceManifest};

    struct CountingPlugin;

    impl Plugin for CountingPlugin {
        fn name(&self) -> &str {
            "counting-plugin"
        }

        fn run(
            &self,
            _ctx: &WorkspaceContext,
            runtime: &mut PluginRuntime<'_>,
        ) -> Vec<FileReplacement> {
            let next = runtime.state().get::<u32>("count").unwrap_or(0) + 1;
            runtime.state().set("count", next).unwrap();
            Vec::new()
        }
    }

    fn empty_workspace() -> WorkspaceContext {
        WorkspaceContext::new(
            "/tmp/workspace".to_owned(),
            Vec::new(),
            WorkspaceManifest {
                members: Vec::new(),
                workspace_root: "/tmp/workspace".to_owned(),
                target_directory: "/tmp/workspace/target".to_owned(),
                metadata: Value::Null,
            },
            LazyValue::from_value(DirNode {
                name: String::new(),
                path: String::new(),
                entries: Vec::new(),
            }),
            None,
        )
    }

    #[test]
    fn plugin_state_round_trips_typed_values() {
        let mut state = PluginState::new();
        state.set("flag", true).unwrap();
        state.set("count", 3u32).unwrap();

        assert_eq!(state.get::<bool>("flag"), Some(true));
        assert_eq!(state.try_get::<u32>("count").unwrap(), Some(3));
        assert_eq!(state.remove::<bool>("flag"), Some(true));
        assert!(!state.contains("flag"));
    }

    #[test]
    fn file_seed_is_stable_per_plugin_and_file() {
        let seed = 1234u64;
        let a = derive_file_seed(seed, "demo", "src/lib.rs");
        let b = derive_file_seed(seed, "demo", "src/lib.rs");
        let c = derive_file_seed(seed, "demo", "src/main.rs");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn generated_region_parser_handles_nested_markers_and_block_comments() {
        let inner = wrap_generated_text(
            "inner",
            "\n    /* nested comment */\n    let y: f64 = 2.0;\n",
        );
        let outer = wrap_generated_text("outer", &format!("\n    let x: f64 = 1.0;\n{inner}\n"));
        let source = format!("fn main() {{{outer}}}");

        let regions = parse_generated_regions(&source);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].plugin_id, "outer");
        assert_eq!(regions[1].plugin_id, "inner");
        assert!(regions[0].full_range.start < regions[1].full_range.start);
        assert!(regions[0].full_range.end > regions[1].full_range.end);
    }

    #[test]
    fn wrapped_text_uses_stable_marker_hashes() {
        let first = wrap_generated_text("demo", "println!(\"hi\");");
        let second = wrap_generated_text("demo", "println!(\"hi\");");
        let third = wrap_generated_text("demo", "println!(\"bye\");");

        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn suite_runtime_retains_plugin_state_across_invocations() {
        let mut runtime = SuiteRuntime::with_seed(7);
        let ctx = empty_workspace();

        assert!(runtime.run_plugin(&CountingPlugin, &ctx).is_empty());
        assert!(runtime.run_plugin(&CountingPlugin, &ctx).is_empty());

        let state = runtime.plugin_state("counting-plugin").unwrap();
        assert_eq!(state.get::<u32>("count"), Some(2));
    }
}
