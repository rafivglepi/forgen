# Forgen

Developer tool for compile-time codegen in Rust, filling the blanks between macros and build.

Forgen is a global macros system, where you import plugins—whether linters, syntax extensions, or type-aware macros—, and they apply to your codebase with a small snippet at the top of your files:

```rust
#![forgen]
use forgen::forgen
```

## Usage

At the start of every file you want to use Forgen in, import the forgen::forgen macro and use it globally with `#![forgen]`
Before building, run `cargo forgen` to refresh the Forgen metadata, and while coding, run `cargo forgen --watch` to use hot reloading

## Development

For more detailed information, see [CONTRIBUTING.md](CONTRIBUTING.md).
