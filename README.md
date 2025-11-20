# Forgen

Enhanced compile-time macro information for Rust - bringing more power to macros with compiler type information!

## Overview

Forgen is a tool that extracts detailed type information from Rust projects using rust-analyzer's internal APIs. The goal is to provide macros with access to rich compiler information (similar to Zig's comptime) that they normally wouldn't have access to.

## Features

✅ **Complete Type Extraction**

- Load Rust projects and their dependencies
- Extract type information for all language items:
  - Functions (with parameter and return types, plus body analysis)
  - Structs (with field types)
  - Enums (with variant fields and types)
  - Traits (with full method signatures)
  - Type aliases
  - Constants
  - Static variables
  - Modules (recursive analysis)

✅ **Function Body Analysis**

- Local variables (name, type, mutability, unique ID)
- Closures (parameters, return type, unique ID)
- Handles variable shadowing with sequential IDs

✅ **Optimized Output**

- Single minified JSON file: `target/.forgen.json`
- Perfect for `include_str!` in macros
- Compact keys (`id` instead of `hir_id`, `ret` instead of `return_type`)
- Omits empty/inferred values
- Booleans as 0/1

✅ **Cross-Referencing**

- Unique HIR IDs for every item
- Stable across re-analysis
- Enable linking between types across files

## Usage

Analyze a project:

```bash
# From any Rust project directory
cargo-forgen

# Or specify a path to Cargo.toml
cargo-forgen /path/to/Cargo.toml
```

The extracted information is saved to `target/.forgen.json` as a single minified JSON file.

## Output Format

The output is a single JSON file with this structure:

```json
{
  "crates": [
    {
      "name": "my_crate",
      "root_file": "...",
      "edition": "2021",
      "is_local": true
    }
  ],
  "files": [
    {
      "path": "src/main.rs",
      "items": [...]
    }
  ]
}
```

### Example Items

**Function with body:**

```json
{
  "kind": "function",
  "name": "main",
  "id": "Function{id:FunctionId(9781)}",
  "ret": "()",
  "body": {
    "locals": [
      { "name": "counter", "ty": "i32", "id": 0, "mut": 1 },
      { "name": "result", "id": 1, "mut": 0 }
    ],
    "closures": [
      {
        "id": 0,
        "params": [{ "name": "x", "ty": "i32" }],
        "ret": "i32"
      }
    ]
  }
}
```

**Struct:**

```json
{
  "kind": "struct",
  "name": "Point",
  "id": "Struct{id:StructId(1111)}",
  "fields": [
    { "name": "x", "ty": "f64" },
    { "name": "y", "ty": "f64" }
  ]
}
```

**Trait with methods:**

```json
{
  "kind": "trait",
  "name": "Greet",
  "id": "Trait{id:TraitId(409)}",
  "items": [
    {
      "kind": "function",
      "name": "greet",
      "ret": "String"
    }
  ]
}
```

**Enum:**

```json
{
  "kind": "enum",
  "name": "Role",
  "id": "Enum{id:EnumId(173)}",
  "variants": [
    { "name": "Guest" },
    { "name": "Member" },
    { "name": "Admin", "fields": [{ "name": "0", "ty": "Admin" }] }
  ]
}
```

### Key Features of Output

- **Compact keys**: `id`, `ret`, `mut`, `params`
- **Omitted empties**: No `params: []`, `fields: []`, `body: {}`, etc.
- **Omitted inferred**: Types that can't be determined are simply not included
- **Minified**: Single-line JSON for minimal size
- **Booleans as integers**: `"mut": 1` (true) or `"mut": 0` (false)

## Using in Macros

Since the output is in `target/.forgen.json`, you can easily include it in proc macros:

```rust
const FORGEN_DATA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/.forgen.json"
));

// Parse and use the type information in your macro
let info: ForgenOutput = serde_json::from_str(FORGEN_DATA)?;
```

## Technical Details

### How It Works

1. Uses rust-analyzer's `ra_ap_*` crates to parse and analyze Rust code
2. Extracts semantic type information from the HIR (High-level Intermediate Representation)
3. Walks function body AST to extract local variables and closures
4. Assigns sequential IDs to locals/closures (handles shadowing correctly)
5. Outputs minified JSON with only non-empty values

### Local Variable IDs

Each local variable gets a unique sequential ID within its function:

```rust
fn example() {
    let x = 1;      // id: 0
    {
        let x = 2;  // id: 1 (different from outer x)
    }
    let x = 3;      // id: 2 (shadows first x)
}
```

All three `x` variables get different IDs, which is correct because they're genuinely different variables.

### Type Inference

- **Explicit types** are extracted from syntax: `let x: i32 = 5` → `"ty": "i32"`
- **Inferred types** are omitted: `let x = 5` → no `ty` field in output
- Future: Use HIR's body source map for full type inference

## Architecture

```
┌─────────────────────┐
│   Rust Project      │
│   (source code)     │
└──────────┬──────────┘
           │
           │ analyzed by
           ▼
┌─────────────────────┐
│  rust-analyzer      │
│  (ra_ap_* crates)   │
└──────────┬──────────┘
           │
           │ extracts HIR
           ▼
┌─────────────────────┐
│  Type Information   │
│  (semantic data)    │
└──────────┬──────────┘
           │
           │ saved to
           ▼
┌─────────────────────┐
│ target/.forgen.json │
│  (minified JSON)    │
└──────────┬──────────┘
           │
           │ include_str!
           ▼
┌─────────────────────┐
│  Proc Macros        │
│  (with type info!)  │
└─────────────────────┘
```

## Next Steps

- [ ] Improve type inference for local variables using HIR body maps
- [ ] Add support for extracting expression types within function bodies
- [ ] Add scope depth tracking for better shadowing analysis
- [ ] Build helpers to parse and query the data
- [ ] Add cross-crate reference resolution (track imports and their sources)

## Dependencies

- `ra_ap_hir`, `ra_ap_ide`, `ra_ap_load-cargo` - rust-analyzer internals
- `ra_ap_syntax` - for Edition support and AST walking
- `serde`, `serde_json` - JSON serialization
- `anyhow` - error handling

## License

MIT
