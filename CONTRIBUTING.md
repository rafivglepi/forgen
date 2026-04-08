# Architecture

Forgen is a CLI tool (`cargo forgen`) that runs rust-analyzer at runtime, feeds source files to plugins, re-runs them to a fixed point over an in-memory source snapshot, and saves the converged file replacements under `target/.forgen/<mirrored-path>.json`.

Those saved replacements are then applied at compile time by the `forgen` proc-macro crate, using a per-file inner attribute like `#![forgen::file("path/to/file.rs")]`.

Plugins receive a `WorkspaceContext` / `FileContext` pair that intentionally does **not** expose rust-analyzer types directly, plus a `PluginRuntime` that provides plugin-local state and deterministic per-file RNGs. This keeps plugin crates free of `ra_ap_*` dependencies.

The plugin-facing replacement API is still byte-range based while running inside the CLI, because the CLI has access to the current in-memory file contents. However, the on-disk JSON format consumed by the proc macro is occurrence-based (`{ index, old_text, new_text }`), since proc-macro `TokenStream`s do not provide a stable, ergonomic byte-range model for reconstructing user edits from source spans alone.

## Multi-pass execution

- The CLI builds a workspace snapshot, runs the plugin suite once, applies the returned replacements in memory, rebuilds the workspace snapshot, and repeats until no file changes or the max-pass guard is reached.
- Plugins must be idempotent. For the same input snapshot and plugin state, they must return the same replacements.
- Plugins must not assume they run first or that `file.source()` is raw on-disk text. Later passes see earlier generated output.
- Plugins should return an empty vec when there is nothing new to replace.
- Generated output is wrapped with marker comments (`/*#start:plugin-id:hash*/` ... `/*#end:plugin-id:hash*/`) before it leaves the suite. `FileContext::generated_regions()` exposes those ranges so plugins can recognize prior output without text-scanning their own markers.
- `PluginState` is scoped per plugin and kept only in memory for the current CLI process. Watch mode reuses it across reruns; restarting the command resets it.
- Seeded RNGs are also process-local. `PluginRuntime::rng_for_file()` is deterministic for the current process, plugin id, and file path.

## Persisted output

- After convergence, the CLI writes one whole-file saved replacement for every changed file.
- The saved `new_text` still contains the original `#![forgen::file(...)]` line; the proc macro removes that attribute after applying the replacement.
- Marker comments remain in the saved text. They do not affect macro expansion because Rust comments are discarded during tokenization.

## Workspace

| Member  | Description                                              | Crate name     |
| ------- | -------------------------------------------------------- | -------------- |
| `macro` | Proc-macro crate that applies saved replacements         | `forgen`       |
| `api`   | Internal API + plugin API                                | `forgen-api`   |
| `cli`   | Build tool that analyzes files and writes `.forgen` JSON | `cargo-forgen` |
| `test`  | Test crate for plugins, the CLI, and macro application   |                |
