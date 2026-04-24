# Config & CI Hardening — Design Spec

**Issue:** #144
**Date:** 2026-04-24
**Parent spec:** `2026-04-22-pre-1.0-audit-remediation-design.md` (Batch 7)

## Scope

### In scope (5 items)

| ID | Summary |
|----|---------|
| H9 | Add CI workflow (GitHub Actions) |
| H10 | Change bundle identifier to `com.2pisoftware.krillnotes` |
| M14 | Pin `tauri-action` to `action-v0.6.2` |
| M15 | Upgrade `rand` 0.8 → 0.9 |
| — | Remove bare `zip` dep from desktop Cargo.toml |

### Dropped from original issue

| ID | Reason |
|----|--------|
| C3 (CSP) | Local desktop app with external embed needs (YouTube). Maintenance cost outweighs security benefit for non-public app. |
| M16 (reqwest blocking gate) | `reqwest` already gated behind `relay` feature. No mobile target exists — YAGNI. Trivial refactor when needed. |

## H9 — CI Workflow

**File:** `.github/workflows/ci.yml`

**Triggers:**
- `pull_request` (all branches)
- `push` to `master`

**Runner:** `ubuntu-22.04` only. Release workflow already covers cross-platform builds at tag time.

**Job 1: `check`** (required, blocks merge)
1. `actions/checkout@v4`
2. `actions/setup-node@v4` (node 22, npm cache from `krillnotes-desktop/package-lock.json`)
3. `dtolnay/rust-toolchain@stable`
4. `Swatinem/rust-cache@v2` (workspace: `. -> target`)
5. System deps: same `apt-get` block as release workflow (webkit, appindicator, rsvg, patchelf, xdg-utils)
6. `cargo fmt --check`
7. `cargo clippy -- -D warnings`
8. `cargo test -p krillnotes-core`
9. `cd krillnotes-desktop && npm ci && npx tsc --noEmit`

**Job 2: `audit`** (advisory, non-blocking)
1. `actions/checkout@v4`
2. `dtolnay/rust-toolchain@stable`
3. `cargo install cargo-audit` (or use `rustsec/audit-check` action)
4. `cargo audit`
5. `continue-on-error: true`

## H10 — Bundle Identifier

**File:** `krillnotes-desktop/src-tauri/tauri.conf.json` line 5

Change: `"com.careck.krillnotes"` → `"com.2pisoftware.krillnotes"`

No other files reference this identifier.

## M14 — Pin tauri-action

**File:** `.github/workflows/release.yml` line 53

Change: `tauri-apps/tauri-action@v0` → `tauri-apps/tauri-action@action-v0.6.2`

## M15 — Upgrade rand 0.8 → 0.9

**File:** `krillnotes-core/Cargo.toml` line 38

Change: `rand = "0.8"` → `rand = "0.9"`

### Known API migrations (rand 0.8 → 0.9)
- `thread_rng()` → `rand::rng()`
- `OsRng` import path may change
- `Rng` trait: `gen()` → `random()`
- `gen_range()` unchanged
- `CryptoRng` trait reorganization

### Verification
- `cargo test -p krillnotes-core` — all existing tests must pass
- Manual review of identity/crypto code paths that use `rand`

## Zip Cleanup

The sole zip usage in desktop is `peek_swarm_header()` in `commands/swarm.rs:86-110`. This function reads a `.swarm` zip bundle, validates its type (invite vs response vs snapshot), and deserializes a `SwarmHeader` — all core types and logic.

**Fix:** Move `peek_swarm_header` into `krillnotes-core` (e.g., `core::swarm::header` or a new `core::swarm::peek` module). The desktop command (`open_swarm_file_cmd`) then calls core's function instead.

After the move:
- Remove `zip = { version = "8", default-features = false }` from `krillnotes-desktop/src-tauri/Cargo.toml` line 48
- Desktop no longer depends on `zip` directly — it gets bundle reading through core's API

## Testing

- **CI workflow:** Push branch, verify all checks pass on the PR itself
- **Bundle ID:** Verify in `tauri.conf.json` (build-time only, no runtime test needed)
- **rand upgrade:** `cargo test -p krillnotes-core` covers crypto/identity paths
- **zip removal:** `cargo check -p krillnotes-desktop` confirms no direct usage
