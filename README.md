# Forgen

Enhanced compile-time macro information for Rust - bringing more power to macros with compiler type information!

## Overview

Forgen is a tool that extracts detailed type information from Rust projects using rust-analyzer's internal APIs. The goal is to provide macros with access to rich compiler information (similar to Zig's comptime) that they normally wouldn't have access to.

## Current Status

✅ **Phase 1: Type Extraction** - Currently working!

The analyzer can now successfully:
- Load Rust projects and their dependencies
- Extract type information for all language items:
  - Functions (with parameter and return types)
  - Structs (with field types)
  - Enums (with variant fields and types)
  - Traits (with associated items)
  - Type aliases
  - Constants
  - Static variables
  - Modules (recursive analysis)

## How It Works

1. Uses rust-analyzer's `ra_ap_*` crates to parse and analyze Rust code
2. Extracts semantic type information from the HIR (High-level Intermediate Representation)
3. Currently logs all type information to console for verification

## Usage

Analyze the test project:
```bash
cargo run --release
```

Analyze a specific project:
```bash
cargo run --release path/to/Cargo.toml
```

## Example Output

```
📦 Crate: forgen_test

📁 Module: <root>
  📌 Const: PI: f64
  📌 Const: MAX_USERS: u32
  📝 Type alias: UserId = u64
  📝 Type alias: Score = f32
  
  📦 Struct: User
    • id: u64
    • name: String
    • score: f32
    
  🔀 Enum: Role
    • Guest
    • Member
    • Admin
      - 0: Admin
      
  🎭 Trait: Greet
    • fn greet
    
  🔧 Function: add
    • param 0: i32
    • param 1: i32
    → returns: i32
```

## Next Steps

- [ ] Save extracted type information to `/target/forgen` directory
- [ ] Create a serialization format for the type data (JSON/bincode)
- [ ] Build a proc-macro helper that can read this data at compile time
- [ ] Create a comptime-like API for macros to query type information

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
│  /target/forgen/    │
│  (serialized data)  │
└──────────┬──────────┘
           │
           │ imported by
           ▼
┌─────────────────────┐
│  Proc Macros        │
│  (with type info!)  │
└─────────────────────┘
```

## Dependencies

- `ra_ap_hir`, `ra_ap_ide`, `ra_ap_load-cargo` - rust-analyzer internals
- `ra_ap_syntax` - for Edition support
- `anyhow` - error handling

## License

MIT

