# Forgen

Developer tool for compile-time codegen in Rust, filling the blanks between macros and build.

Forgen writes plugin-generated replacements to `target/.forgen/<mirrored-path>.json`, and a proc macro applies them during compilation.

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
- While coding, run `cargo forgen --watch` to keep `target/.forgen/` up to date.

## Development

For more detailed information, see [CONTRIBUTING.md](CONTRIBUTING.md).
