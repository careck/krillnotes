# Config & CI Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden project config (bundle ID, action pinning), upgrade `rand` to 0.9, add CI quality gates, and fix zip dependency layering.

**Architecture:** Five independent changes — config tweaks, a small core refactor, a dep upgrade, and a new CI workflow. Each produces a standalone commit.

**Tech Stack:** Rust, GitHub Actions, Tauri v2, Cargo

**Spec:** `docs/superpowers/specs/2026-04-24-config-ci-hardening-design.md`

---

### Task 1: H10 — Change Bundle Identifier

**Files:**
- Modify: `krillnotes-desktop/src-tauri/tauri.conf.json:5`

- [ ] **Step 1: Update the identifier**

In `krillnotes-desktop/src-tauri/tauri.conf.json` line 5, change:

```json
"identifier": "com.careck.krillnotes",
```

to:

```json
"identifier": "com.2pisoftware.krillnotes",
```

- [ ] **Step 2: Verify no other files reference the old identifier**

Run:
```bash
grep -rn "com.careck.krillnotes" --include='*.json' --include='*.toml' --include='*.rs' --include='*.ts' .
```

Expected: no results.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/tauri.conf.json
git commit -m "fix(H10): change bundle identifier to com.2pisoftware.krillnotes"
```

---

### Task 2: M14 — Pin tauri-action Version

**Files:**
- Modify: `.github/workflows/release.yml:53`

- [ ] **Step 1: Pin the action version**

In `.github/workflows/release.yml` line 53, change:

```yaml
uses: tauri-apps/tauri-action@v0
```

to:

```yaml
uses: tauri-apps/tauri-action@action-v0.6.2
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "fix(M14): pin tauri-action to action-v0.6.2"
```

---

### Task 3: Move `peek_swarm_header` to Core + Remove Desktop zip Dep

Core already has `read_header()` in `krillnotes-core/src/core/swarm/header.rs:122-135`. The desktop's `peek_swarm_header` (in `commands/swarm.rs:85-110`) adds invite/response file detection before reading the header. We add that detection to core's function and delete the desktop duplicate.

**Files:**
- Modify: `krillnotes-core/src/core/swarm/header.rs` (enhance `read_header`)
- Modify: `krillnotes-desktop/src-tauri/src/commands/swarm.rs` (delete `peek_swarm_header`, call core)
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml:48` (remove zip dep)
- Test: `krillnotes-core/src/core/swarm/header.rs` (existing tests + new ones)

- [ ] **Step 1: Write failing tests for invite/response detection in core**

Add to the `#[cfg(test)] mod tests` block at the bottom of `krillnotes-core/src/core/swarm/header.rs`:

```rust
#[test]
fn test_read_header_rejects_invite_bundle() {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("invite.json", options).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
    }
    let err = read_header(&buf).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("invite"), "expected invite error, got: {msg}");
}

#[test]
fn test_read_header_rejects_response_bundle() {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("response.json", options).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
    }
    let err = read_header(&buf).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("response"), "expected response error, got: {msg}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo test -p krillnotes-core -- test_read_header_rejects
```

Expected: 2 FAIL (no invite/response detection yet).

- [ ] **Step 3: Add invite/response detection to `read_header`**

In `krillnotes-core/src/core/swarm/header.rs`, replace the `read_header` function (lines 120-135) with:

```rust
/// Read the `SwarmHeader` from a `.swarm` zip archive without decrypting
/// the payload.  Used by receive-only polling to dispatch on bundle mode.
///
/// Returns an error if the archive is an invite or response bundle
/// (these have distinct file structures and should be handled by their
/// own import paths).
pub fn read_header(data: &[u8]) -> Result<SwarmHeader> {
    use std::io::Cursor;
    let cursor = Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| {
        KrillnotesError::Swarm(format!("invalid .swarm zip archive: {e}"))
    })?;

    if zip.by_name("invite.json").is_ok() {
        return Err(KrillnotesError::Swarm(
            "bundle contains invite.json — use the invite import path".into(),
        ));
    }
    if zip.by_name("response.json").is_ok() {
        return Err(KrillnotesError::Swarm(
            "bundle contains response.json — use the response import path".into(),
        ));
    }

    let mut header_file = zip.by_name("header.json").map_err(|e| {
        KrillnotesError::Swarm(format!("missing header.json in .swarm bundle: {e}"))
    })?;
    let header: SwarmHeader = serde_json::from_reader(&mut header_file).map_err(|e| {
        KrillnotesError::Swarm(format!("invalid header.json: {e}"))
    })?;
    Ok(header)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cargo test -p krillnotes-core -- swarm::header
```

Expected: all header tests PASS (existing + 2 new).

- [ ] **Step 5: Replace desktop `peek_swarm_header` with core call**

In `krillnotes-desktop/src-tauri/src/commands/swarm.rs`:

1. Delete the entire `peek_swarm_header` function (lines 85-110).

2. Replace line 120 (`let header = peek_swarm_header(&data)?;`) with:

```rust
    let header = krillnotes_core::core::swarm::header::read_header(&data)
        .map_err(|e| e.to_string())?;
```

- [ ] **Step 6: Remove zip dep from desktop Cargo.toml**

In `krillnotes-desktop/src-tauri/Cargo.toml`, delete line 48:

```toml
zip = { version = "8", default-features = false }
```

- [ ] **Step 7: Verify desktop compiles**

Run:
```bash
cargo check -p krillnotes-desktop
```

Expected: compiles with no errors. If there are other zip usages we missed, `cargo check` will catch them.

- [ ] **Step 8: Run full test suite**

Run:
```bash
cargo test -p krillnotes-core
```

Expected: all tests PASS.

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/swarm/header.rs \
        krillnotes-desktop/src-tauri/src/commands/swarm.rs \
        krillnotes-desktop/src-tauri/Cargo.toml
git commit -m "refactor: move swarm bundle classification to core, remove desktop zip dep"
```

---

### Task 4: M15 — Upgrade rand 0.8 → 0.9

The crypto stack (`ed25519-dalek`, `x25519-dalek`, `aes-gcm`, `crypto_box`) currently depends on `rand_core 0.6`. `rand 0.9` uses `rand_core 0.9`. This may cause trait incompatibilities. The first step is an audit.

**Files:**
- Modify: `krillnotes-core/Cargo.toml:38`
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml:44`
- Modify (if needed): All `.rs` files using `rand::thread_rng()` — see list below

**Known usages of `thread_rng()` (must change to `rand::rng()` in 0.9):**
- `krillnotes-core/src/core/swarm/crypto.rs:121,130`
- `krillnotes-core/src/core/swarm/invite.rs:90`
- `krillnotes-core/src/core/attachment.rs:69-70`
- `krillnotes-desktop/src-tauri/src/commands/workspace.rs:432,712,1159`

**Usages of `rand::rngs::OsRng` (likely unchanged but verify):**
- `krillnotes-core/src/core/identity.rs:282,288,305,481,500,795` (imported as `aead::OsRng`)
- `krillnotes-core/src/core/swarm/crypto.rs:60,197`
- `krillnotes-core/src/core/contact.rs:147`
- `krillnotes-core/src/core/sync/relay/relay_account.rs:125`
- `krillnotes-core/src/core/sync/relay/auth.rs:57`
- `krillnotes-desktop/src-tauri/src/commands/swarm.rs:428`
- Many test files (using `SigningKey::generate(&mut OsRng)`)

- [ ] **Step 1: Audit crypto crate compatibility with rand 0.9**

Run:
```bash
cargo tree -p krillnotes-core -i rand_core 2>/dev/null | head -5
```

Check if `rand_core` version is 0.6.x. Then attempt the upgrade:

```bash
cd /tmp && cargo init rand-audit && cd rand-audit
cargo add rand@0.9 ed25519-dalek@2 --features ed25519-dalek/rand_core
cargo check 2>&1
cd -
rm -rf /tmp/rand-audit
```

If `ed25519-dalek` and `rand 0.9` can coexist (Cargo resolves both `rand_core 0.6` and `0.9`), proceed. The key test: does `SigningKey::generate(&mut rand::rngs::OsRng)` still compile when `rand = "0.9"`?

If it does NOT compile: the `OsRng` from `rand 0.9` implements `rand_core 0.9` traits, but `ed25519-dalek` expects `rand_core 0.6` traits. In that case, crypto code must use `rand_core 0.6::OsRng` directly (from `aead::OsRng` or `rand_core = "0.6"` as an explicit dep). Only non-crypto code gets `rand 0.9`.

**Fallback if incompatible:** Keep `rand = "0.8"` in `krillnotes-core/Cargo.toml`. Only upgrade `rand` in the desktop crate if its usages don't interact with crypto crates. Document the blocker in the commit message and the issue.

- [ ] **Step 2: Bump rand version in both Cargo.toml files**

In `krillnotes-core/Cargo.toml` line 38, change:
```toml
rand = "0.8",
```
to:
```toml
rand = "0.9",
```

In `krillnotes-desktop/src-tauri/Cargo.toml` line 44, change:
```toml
rand = "0.8"
```
to:
```toml
rand = "0.9"
```

- [ ] **Step 3: Run cargo check to find compile errors**

```bash
cargo check --workspace 2>&1
```

Collect all errors. The expected breaking changes in rand 0.9:
- `thread_rng()` removed → use `rand::rng()`
- `gen()` renamed → `random()` (not used in this codebase)
- `OsRng` path may change
- Trait bounds may shift

- [ ] **Step 4: Fix `thread_rng()` → `rand::rng()` in core**

In each file, replace `rand::thread_rng()` with `rand::rng()`:

**`krillnotes-core/src/core/swarm/crypto.rs`** — lines 121, 130:
```rust
// Line 121: was rand::thread_rng().fill_bytes(&mut aes_key);
rand::rng().fill_bytes(&mut aes_key);

// Line 130: was EphemeralSecret::random_from_rng(rand::thread_rng());
EphemeralSecret::random_from_rng(rand::rng());
```

**`krillnotes-core/src/core/swarm/invite.rs`** — line 90:
```rust
// was rand::thread_rng().fill_bytes(&mut token_bytes);
rand::rng().fill_bytes(&mut token_bytes);
```

**`krillnotes-core/src/core/attachment.rs`** — lines 69-70:
```rust
// was rand::thread_rng().fill_bytes(&mut nonce_bytes);
// was rand::thread_rng().fill_bytes(&mut file_salt);
rand::rng().fill_bytes(&mut nonce_bytes);
rand::rng().fill_bytes(&mut file_salt);
```

- [ ] **Step 5: Fix `thread_rng()` → `rand::rng()` in desktop**

**`krillnotes-desktop/src-tauri/src/commands/workspace.rs`** — lines 432, 712, 1159:

Each occurrence of:
```rust
rand::thread_rng().fill_bytes(&mut bytes);
```
becomes:
```rust
rand::rng().fill_bytes(&mut bytes);
```

- [ ] **Step 6: Fix any remaining compile errors from Step 3**

Address any `OsRng` import path changes or trait bound issues found in the cargo check output. If `rand::rngs::OsRng` no longer satisfies crypto crate trait bounds, use the `OsRng` re-exported from `aead` or add `rand_core = "0.6"` as an explicit dependency for crypto-specific code.

- [ ] **Step 7: Run the full test suite**

```bash
cargo test -p krillnotes-core
```

Expected: all tests PASS. Pay special attention to:
- Identity tests (key generation, encryption)
- Swarm crypto tests (envelope encryption/decryption)
- Attachment tests (file encryption)
- Signature tests

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "deps(M15): upgrade rand 0.8 → 0.9

Migrate thread_rng() → rng() across core and desktop crates.
OsRng and RngCore usage unchanged."
```

---

### Task 5: H9 — Add CI Workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the CI workflow file**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  pull_request:
  push:
    branches: [master]

jobs:
  check:
    runs-on: ubuntu-22.04

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: "22"
          cache: "npm"
          cache-dependency-path: krillnotes-desktop/package-lock.json

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache Rust dependencies
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: ". -> target"

      - name: Install Linux system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libappindicator3-dev \
            librsvg2-dev \
            patchelf \
            xdg-utils

      - name: Check formatting
        run: cargo fmt --check

      - name: Clippy
        run: cargo clippy --workspace -- -D warnings

      - name: Run core tests
        run: cargo test -p krillnotes-core

      - name: Install frontend dependencies
        working-directory: krillnotes-desktop
        run: npm ci

      - name: TypeScript type check
        working-directory: krillnotes-desktop
        run: npx tsc --noEmit

  audit:
    runs-on: ubuntu-22.04
    continue-on-error: true

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-audit
        run: cargo install cargo-audit --locked

      - name: Run cargo audit
        run: cargo audit
```

- [ ] **Step 2: Validate YAML syntax**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" 2>&1 || echo "YAML parse error"
```

Expected: no output (valid YAML). If python3/yaml not available:
```bash
npx yaml-lint .github/workflows/ci.yml 2>/dev/null || echo "install: npm install -g yaml-lint"
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(H9): add PR quality gates — fmt, clippy, test, tsc, audit"
```

---

### Post-Implementation

After all 5 tasks are committed:

- [ ] **Push branch and verify CI passes on the PR itself** (the new CI workflow will run on its own PR)
- [ ] **Verify `cargo test -p krillnotes-core` passes locally**
- [ ] **Verify `cargo check -p krillnotes-desktop` passes locally**
- [ ] **Create PR targeting `master`**
