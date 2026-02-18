# Rust Guidelines Compliance Design

**Date:** 2026-02-19
**Status:** Approved

## Problem

A full audit of the Krillnotes Rust codebase against the MS Rust Guidelines revealed violations across three tiers:

1. **Tier 1 — Safety/Correctness:** `unsafe impl Send + Sync` on `Workspace` to paper over `rhai::Engine` being `!Send`; `.unwrap()` on `fields_json` deserialization that panics on corrupt DB; magic value `86400`; `#[allow]` instead of `#[expect]`.
2. **Tier 2 — Documentation:** Every source file is missing `//!` module docs, `///` item docs, `# Errors` sections, and `#[doc(inline)]` on re-exports.
3. **Tier 3 — App-level:** `mimalloc` not set as global allocator; dead `greet` Tauri command.

## Approach: Safety first, then documentation

Fix structural issues before documenting so the docs accurately describe correct code.

## Section 1 — Structural fix: `rhai/sync` + remove `unsafe impl`

**Root cause:** `rhai::Engine` uses `Rc<_>` internally, making it `!Send`. `SchemaRegistry` holds an `Engine` as a long-lived field (required for future scripted views, commands, and action hooks), so it is also `!Send`. The current workaround is:

```rust
unsafe impl Send for Workspace {}
unsafe impl Sync for Workspace {}
```

This is a violation of M-UNSAFE — no `// SAFETY:` justification exists and the invariant cannot actually be upheld safely.

**Fix:** Enable the `rhai/sync` feature flag, which replaces all internal `Rc<_>` with `Arc<_>` and `RefCell<_>` with `Mutex<_>`, making `Engine`, `Dynamic`, and all Rhai types `Send + Sync`. With that in place, `SchemaRegistry: Send + Sync` naturally, `Workspace: Send + Sync` naturally, and the two `unsafe impl` lines are deleted.

No API changes. No structural rework. The engine remains a long-lived field, ready for future scripting phases.

**Changes:**
- `krillnotes-core/Cargo.toml`: add `features = ["sync"]` to the `rhai` dependency
- `krillnotes-core/src/core/workspace.rs`: delete `unsafe impl Send for Workspace` and `unsafe impl Sync for Workspace`

## Section 2 — Mechanical pass: documentation, magic values, dead code

All changes in this section are purely additive — no behavioral differences.

### Documentation

Applied uniformly across all `.rs` files:

- `//!` module-level doc comment at the top of each file (one sentence describing purpose)
- `///` doc comment on every public type, field, enum variant, and function
- `# Errors` section on every `fn` that returns `Result`
- `#[doc(inline)]` on all `pub use` re-exports in `lib.rs` and `core/mod.rs`
- Crate-level `//!` in `lib.rs` per C-CRATE-DOC

### Magic values

- `operation_log.rs`: extract `86400` → `const SECONDS_PER_DAY: i64 = 86_400;` (M-DOCUMENTED-MAGIC)
- `storage.rs`: add inline comment explaining the `3` in the table-count validation

### Dead code and lint hygiene

- Replace any `#[allow(dead_code)]` with `#[expect(dead_code, reason = "...")]` (M-LINT-OVERRIDE-EXPECT)
- Remove the dead `greet` Tauri command from `krillnotes-desktop/src-tauri/src/lib.rs`

### App-level allocator

- Add `mimalloc` as the global allocator in `krillnotes-desktop/src-tauri/src/main.rs` (M-MIMALLOC-APPS)
- Add `mimalloc` dependency to `krillnotes-desktop/src-tauri/Cargo.toml`

## Files touched

| File | Changes |
|------|---------|
| `krillnotes-core/Cargo.toml` | Add `rhai/sync` feature |
| `krillnotes-core/src/core/workspace.rs` | Remove `unsafe impl`, add docs |
| `krillnotes-core/src/core/scripting.rs` | Add docs, add `Debug` derive to `SchemaRegistry` |
| `krillnotes-core/src/core/storage.rs` | Add docs, comment on magic `3` |
| `krillnotes-core/src/core/operation_log.rs` | Add docs, extract `SECONDS_PER_DAY` const |
| `krillnotes-core/src/core/operation.rs` | Add docs |
| `krillnotes-core/src/core/note.rs` | Add docs |
| `krillnotes-core/src/core/error.rs` | Add docs |
| `krillnotes-core/src/core/device.rs` | Add `//!` header, `# Errors` section |
| `krillnotes-core/src/core/mod.rs` | Add `//!`, `#[doc(inline)]` on re-exports |
| `krillnotes-core/src/lib.rs` | Add crate doc, `#[doc(inline)]` on re-exports |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Remove `greet` command, add docs |
| `krillnotes-desktop/src-tauri/src/menu.rs` | Add docs |
| `krillnotes-desktop/src-tauri/src/main.rs` | Add `mimalloc` global allocator |
| `krillnotes-desktop/src-tauri/Cargo.toml` | Add `mimalloc` dependency |

## Success criteria

- `cargo build` succeeds with no warnings on both crates
- `cargo test` passes on both crates
- No `unsafe impl` blocks remain in the codebase
- No `.unwrap()` on deserialized data in hot paths
- Every public item has a `///` doc comment
- `cargo doc --no-deps` produces no warnings
