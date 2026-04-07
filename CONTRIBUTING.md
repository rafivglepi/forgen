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