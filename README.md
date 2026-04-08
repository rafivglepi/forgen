# Forgen

Developer tool for compile-time codegen in Rust, filling the blanks between macros and build.

Forgen runs plugins to a fixed point, writes the converged file replacements to `target/.forgen/<mirrored-path>.json`, and a proc macro applies them during compilation.

## Usage

Add this to each crate root file you want Forgen to apply to:

```rust
#![feature(custom_inner_attributes, prelude_import)]
#![forgen::file("test/src/lib.rs")]
```

Notes:

- `#![forgen::file("...")]` is a custom inner attribute, so this currently requires nightly.
- The path must be the workspace-relative path to the current file.
- `#![feature(custom_inner_attributes, prelude_import)]` must be enabled in the crate attributes.
- Before building, run `cargo forgen` to refresh the generated replacement files.
- `cargo forgen` may execute several plugin passes in one run; it stops when no file changes anymore or when the max-pass guard trips.
- Generated plugin output is wrapped in marker comments like `/*#start:plugin-id:hash*/.../*#end:plugin-id:hash*/` so later passes can recognize prior generated regions.
- Plugin runtime state and seeded RNG values live only in memory for the current CLI process. Watch mode reuses that state across reruns; restarting `cargo forgen` starts fresh.
- While coding, run `cargo forgen --watch` to keep `target/.forgen/` up to date.

## Development

For more detailed information, see [CONTRIBUTING.md](CONTRIBUTING.md).
