# UnixSecs Newtype Wrapper — Design Spec

**Date:** 2026-04-24
**Motivation:** Issue #157 — sync path stored HLC milliseconds in seconds columns. Fix was trivial but the bug class is preventable at compile time.

## Approach

**Chosen: Approach A (wrap seconds only)**

Create `UnixSecs(i64)` newtype so that assigning raw `i64` or `u64` (milliseconds) to a seconds-expecting field is a compile error. The milliseconds side stays as `u64` inside `HlcTimestamp` — future Approach B can wrap that too.

### Approaches Considered

| Approach | Scope | Trade-off |
|----------|-------|-----------|
| **A: `UnixSecs` only** ✓ | ~20 call sites | Catches the #157 bug class with minimal churn |
| B: `UnixSecs` + `WallMillis` | ~40+ call sites | Full type safety, larger blast radius |
| C: Runtime wrapper | ~20 call sites | No compile-time guarantee |

## Type Definition

```rust
// krillnotes-core/src/core/timestamp.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixSecs(i64);
```

### Constructors
- `UnixSecs::now()` — wraps `chrono::Utc::now().timestamp()`
- `UnixSecs::from_secs(i64)` — explicit construction from known-seconds value
- `UnixSecs::ZERO` — const for default/placeholder use

### Accessors
- `UnixSecs::as_i64(&self) -> i64` — explicit unwrap when raw value needed (SQL params, arithmetic)

### Trait Implementations
- `Serialize` / `Deserialize` — transparent (JSON number, same as before)
- `rusqlite::types::ToSql` — delegates to inner `i64`
- `rusqlite::types::FromSql` — reads `i64` from SQLite INTEGER
- `Display` — delegates to inner `i64`

### No `From<i64>` implementation
Deliberately omit `From<i64>` to force callers to use named constructors. This is the key safety property — you can't accidentally assign milliseconds.

## HLC Conversion

```rust
// krillnotes-core/src/core/hlc.rs
impl HlcTimestamp {
    pub fn to_unix_secs(&self) -> UnixSecs {
        UnixSecs::from_secs((self.wall_ms / 1000) as i64)
    }
}
```

## Structs Updated

| Struct | Fields | File |
|--------|--------|------|
| `Note` | `created_at`, `modified_at` | `note.rs` |
| `UserScript` | `created_at`, `modified_at` | `user_script.rs` |
| `AttachmentMeta` | `created_at` | `attachments.rs` (check if applicable) |

## Call Site Patterns

| Old pattern | New pattern |
|-------------|------------|
| `chrono::Utc::now().timestamp()` | `UnixSecs::now()` |
| `(ts.wall_ms / 1000) as i64` | `ts.to_unix_secs()` |
| `note.created_at` (in SQL params) | `note.created_at` (ToSql handles it) |
| `row.get::<_, i64>(col)?` | `row.get::<_, UnixSecs>(col)?` or `UnixSecs::from_secs(row.get(col)?)` |

## Serde Compatibility

`#[serde(transparent)]` ensures JSON serialization is unchanged — `created_at` still appears as a plain number in Rust→TypeScript IPC. No frontend changes needed.

## Testing

- Existing test `test_apply_incoming_create_note_stores_seconds_timestamp` continues to pass
- Existing test `test_m6_set_note_checked_stores_seconds_timestamp` continues to pass
- All 608 existing tests pass without modification (beyond type changes)
- The compiler itself now prevents the original bug — no new runtime test needed
