# Architecture

Forgen is a CLI tool (`cargo forgen`) that runs rust-analyzer at runtime, feeds source files to plugins, and saves the textual replacements they return under `target/.forgen/<mirrored-path>.json`.

Those saved replacements are then applied at compile time by the `forgen` proc-macro crate, using a per-file inner attribute like `#![forgen::file("path/to/file.rs")]`.

Plugins receive a `FileContext` that intentionally does **not** expose rust-analyzer types directly — so that, when dylib loading lands, plugins won't need `ra_ap_*` as a dependency.

The plugin-facing replacement API is still byte-range based while running inside the CLI, because the CLI has access to the original file contents. However, the on-disk JSON format consumed by the proc macro is occurrence-based (`{ index, old_text, new_text }`), since proc-macro `TokenStream`s do not provide a stable, ergonomic byte-range model for reconstructing user edits from source spans alone.

## Workspace

| Member  | Description                                              | Crate name     |
| ------- | -------------------------------------------------------- | -------------- |
| `macro` | Proc-macro crate that applies saved replacements         | `forgen`       |
| `api`   | Internal API + plugin API                                | `forgen-api`   |
| `cli`   | Build tool that analyzes files and writes `.forgen` JSON | `cargo-forgen` |
| `test`  | Test crate for plugins, the CLI, and macro application   |                |

## Building

You need to install `cargo-forgen` in your path to use it in tests or checks. For that, run `cargo install --path ./cli`. For running cargo commands inside the test folder, use the run scripts for each respective platform:

```bash
./run.bash check
./run.bash build
./run.bash test
```

## Roadmap

- [x] Load workspace with rust-analyzer at runtime
- [x] Type inference for local `let` bindings via `Semantics`
- [x] Plugin trait + `FileContext` API (no ra*ap*\* exposure to plugins)
- [x] Replacement format `{ range: { start, end }, text }`
- [x] Hardcoded f64-logger plugin (explicit + inferred `f64` bindings)
- [x] Idempotent insertions (re-running forgen is safe)
- [ ] Watch mode re-applying plugins on file save and refreshing `target/.forgen`
- [ ] Dylib plugin loading (plugins as `.dll` / `.so`)
- [ ] `SyntaxNode`-compatible wrapper enum so plugins work without ra_ap_syntax
- [ ] Plugin registry in `build.rs` / `forgen.toml`
- [ ] `comptime!` macro with live type access (similar to Zig's comptime)
- [ ] Stable ABI for `FileContext` and the plugin-facing byte-range `Replacement`
