# Folder Sync Addressing Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix folder sync so multiple devices sharing a flat folder only process bundles addressed to them, and channel config updates surface errors when the peer row is not found.

**Architecture:** Two independent fixes in two files. (1) `FolderChannel` filename format changes from sender-prefixed to recipient-prefixed so each device filters its inbox by filename without decrypting. (2) `PeerRegistry::update_channel_config` returns an error on 0 rows affected instead of silently succeeding.

**Tech Stack:** Rust, `rusqlite`, `tempfile` (tests), `chrono`, `uuid`

**Spec:** `docs/superpowers/specs/2026-03-15-folder-sync-addressing-design.md`

---

## Chunk 1: Fix `update_channel_config` silent no-op

### Task 1: Add failing test for 0-row update

**Files:**
- Modify: `krillnotes-core/src/core/peer_registry.rs` (test block, ~line 470)

- [ ] **Step 1: Add failing test**

In the `#[cfg(test)]` block at the bottom of `peer_registry.rs`, add after the existing `test_update_channel_config` test:

```rust
#[test]
fn test_update_channel_config_unknown_peer_returns_error() {
    let (_f, s) = open_db();
    let reg = PeerRegistry::new(s.connection());
    // No peer was added — update should fail, not silently succeed
    let result = reg.update_channel_config(
        "nonexistent-device",
        "folder",
        r#"{"path":"/tmp"}"#,
    );
    assert!(result.is_err(), "expected Err when device_id not found");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_update_channel_config_unknown_peer_returns_error 2>&1 | tail -20
```

Expected: `FAILED` — the current implementation returns `Ok(())` even for 0 rows.

### Task 2: Implement the fix

**Files:**
- Modify: `krillnotes-core/src/core/peer_registry.rs` (~line 344)

- [ ] **Step 3: Capture row count and return error on 0**

Find `update_channel_config` (around line 344). Replace:

```rust
self.conn.execute(
    "UPDATE sync_peers SET channel_type = ?1, channel_params = ?2 WHERE peer_device_id = ?3",
    rusqlite::params![channel_type, channel_params, peer_device_id],
)?;
Ok(())
```

With:

```rust
let rows = self.conn.execute(
    "UPDATE sync_peers SET channel_type = ?1, channel_params = ?2 WHERE peer_device_id = ?3",
    rusqlite::params![channel_type, channel_params, peer_device_id],
)?;
if rows == 0 {
    return Err(KrillnotesError::Swarm(format!(
        "peer not found: {peer_device_id}"
    )));
}
Ok(())
```

- [ ] **Step 4: Run both channel config tests**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_update_channel_config 2>&1 | tail -20
```

Expected: both `test_update_channel_config` and `test_update_channel_config_unknown_peer_returns_error` pass.

- [ ] **Step 5: Run full core test suite**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-core/src/core/peer_registry.rs
git commit -m "fix: update_channel_config returns error when peer not found"
```

---

## Chunk 2: Recipient-prefixed filenames and inbox filter

### Task 3: Update send test to assert new filename format

**Files:**
- Modify: `krillnotes-core/src/core/sync/folder.rs` (test block, `test_folder_channel_send_creates_file`)

- [ ] **Step 1: Update the send test**

Find `test_folder_channel_send_creates_file` (around line 178). Replace the assertions after `channel.send_bundle(...)` with:

```rust
let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
    .filter_map(|e| e.ok())
    .filter(|e| e.path().extension().map_or(false, |ext| ext == "swarm"))
    .collect();
assert_eq!(files.len(), 1);

// New format: {RECIPIENT_identity_short}_{14-digit-ts}_{8-char-uuid}.swarm
let filename = files[0]
    .path()
    .file_name()
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
let stem = filename.strip_suffix(".swarm").unwrap();
// Use splitn(3) so the uuid part is kept whole even if it contains '_'
let parts: Vec<&str> = stem.splitn(3, '_').collect();
assert_eq!(parts.len(), 3, "filename must have exactly 3 '_'-separated segments");
// First segment = recipient_short = "peer-id".chars().take(8).collect() = "peer-id" (7 chars)
assert_eq!(parts[0], "peer-id", "first segment must be recipient identity short");
// Second segment = 14-digit timestamp
assert_eq!(parts[1].len(), 14, "second segment must be 14-digit timestamp");
assert!(
    parts[1].chars().all(|c| c.is_ascii_digit()),
    "second segment must be all digits"
);
// Third segment = 8-char uuid short
assert_eq!(parts[2].len(), 8, "third segment must be 8-char uuid");
```

Note: `make_test_peer` uses `identity_id = "peer-id"`. `"peer-id".chars().take(8)` = `"peer-id"` (7 chars — `take(8)` on a 7-char string returns all 7), so `parts[0]` will be `"peer-id"`.

- [ ] **Step 2: Run to verify it fails**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_folder_channel_send_creates_file 2>&1 | tail -20
```

Expected: `FAILED` — current filename has 4 segments with sender prefix.

### Task 4: Add inbox-filter tests (all expected to fail)

**Files:**
- Modify: `krillnotes-core/src/core/sync/folder.rs` (test block)

- [ ] **Step 3: Rename and rewrite the own-files filter test**

Find `test_folder_channel_receive_filters_own_bundles` and replace it entirely:

```rust
#[test]
fn test_folder_channel_inbox_prefix_filtering() {
    let dir = tempfile::tempdir().unwrap();
    // identity_short = first 8 chars of "my-identity" = "my-ident"
    let channel = FolderChannel::new(
        "my-identity".to_string(),
        "my-device".to_string(),
    );

    // File addressed TO this device (new format) — must be returned
    let inbox_file = dir.path().join("my-ident_20260315120000_abcdef01.swarm");
    std::fs::write(&inbox_file, b"for me").unwrap();

    // File addressed to a different device — must be skipped
    let other_file = dir.path().join("other-ide_20260315120000_abcdef02.swarm");
    std::fs::write(&other_file, b"not for me").unwrap();

    let bundles = channel.receive_bundles_from_dir(dir.path()).unwrap();
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].data, b"for me");
}
```

- [ ] **Step 4: Add test for old-format files**

Add after the previous test:

```rust
#[test]
fn test_folder_channel_ignores_old_format_files() {
    let dir = tempfile::tempdir().unwrap();
    // identity_short = "my-ident"
    let channel = FolderChannel::new(
        "my-identity".to_string(),
        "my-device".to_string(),
    );

    // Old-format file: starts with MY identity short but second segment is NOT 14 digits
    // (it's "my-devic", an 8-char device short)
    let old_file = dir.path().join("my-ident_my-devic_20260315_abcdef.swarm");
    std::fs::write(&old_file, b"old format").unwrap();

    // New-format file addressed to this device — must be returned
    let new_file = dir.path().join("my-ident_20260315120000_abcdef01.swarm");
    std::fs::write(&new_file, b"new format").unwrap();

    let bundles = channel.receive_bundles_from_dir(dir.path()).unwrap();
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].data, b"new format");
}
```

- [ ] **Step 5: Run new tests to verify they fail**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_folder_channel 2>&1 | tail -30
```

Expected: `test_folder_channel_inbox_prefix_filtering` and `test_folder_channel_ignores_old_format_files` FAIL; others may also fail. All failures are expected at this point.

### Task 5: Implement recipient-prefixed `send_bundle`

**Files:**
- Modify: `krillnotes-core/src/core/sync/folder.rs` (struct definition and `send_bundle`)

- [ ] **Step 6: Rename `device_short` to `_device_short`**

In the struct definition (around line 13-21), rename the field:

```rust
pub struct FolderChannel {
    identity_short: String,
    _device_short: String,   // retained for future use; not in filenames
    folder_paths: std::sync::Mutex<Vec<String>>,
}
```

In `FolderChannel::new` (around line 24-30):

```rust
pub fn new(identity_id: String, device_id: String) -> Self {
    Self {
        identity_short: identity_id.chars().take(8).collect(),
        _device_short: device_id.chars().take(8).collect(),
        folder_paths: std::sync::Mutex::new(vec![]),
    }
}
```

- [ ] **Step 7: Rewrite `send_bundle` filename generation**

In `send_bundle` (around line 107-111), replace:

```rust
let timestamp = Utc::now().format("%Y%m%d%H%M%S");
let uuid_short = &Uuid::new_v4().to_string()[..8];
let filename = format!("{}_{}_{}_{}.swarm",
    self.identity_short, self.device_short, timestamp, uuid_short
);
```

With:

```rust
let recipient_short: String = peer.peer_identity_id.chars().take(8).collect();
if recipient_short.is_empty() {
    return Err(KrillnotesError::Swarm(
        "folder channel peer has no identity key".to_string(),
    ));
}
let timestamp = Utc::now().format("%Y%m%d%H%M%S");
let uuid_short = &Uuid::new_v4().to_string()[..8];
let filename = format!("{}_{}_{}.swarm", recipient_short, timestamp, uuid_short);
```

- [ ] **Step 8: Run send test to verify it passes**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_folder_channel_send_creates_file 2>&1 | tail -20
```

Expected: PASS.

### Task 6: Implement inbox-prefix filter in `receive_bundles_from_dir`

**Files:**
- Modify: `krillnotes-core/src/core/sync/folder.rs` (`receive_bundles_from_dir`, ~line 48-94)

- [ ] **Step 9: Replace sender-exclusion filter with inbox-prefix + format-check filter**

Two separate deletions are required in `receive_bundles_from_dir`:

**Deletion 1** — Remove the `own_prefix` binding before the loop (around line 55 of the source file, just before the `let mut bundles` line):

```rust
// DELETE this entire line:
let own_prefix = format!("{}_{}", self.identity_short, self.device_short);
```

**Deletion 2 + Replacement** — Inside the loop, find the `if filename.starts_with(&own_prefix)` block (around lines 72-74) and replace it with the inbox-prefix filter and format-check guard:

```rust
// DELETE:
//   if filename.starts_with(&own_prefix) {
//       continue;
//   }

// REPLACE WITH:
// Inbox filter: only process files addressed to this device.
let inbox_prefix = format!("{}_", self.identity_short);
if !filename.starts_with(&inbox_prefix) {
    continue;
}

// Format guard: new-format files have a 14-digit timestamp as the second segment.
// Old-format files (sender_device_ts_uuid) have an 8-char device short there — skip them.
let rest = &filename[inbox_prefix.len()..];
let next_segment = rest.split('_').next().unwrap_or("");
if next_segment.len() != 14 || !next_segment.chars().all(|c| c.is_ascii_digit()) {
    log::debug!(
        target: "krillnotes::sync::folder",
        "skipping old-format or misaddressed file: {}",
        filename
    );
    continue;
}
```

After both edits, no reference to `own_prefix` or `self._device_short` should remain in `receive_bundles_from_dir`.

- [ ] **Step 10: Run all folder channel tests**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core test_folder_channel 2>&1 | tail -30
```

Expected: all `test_folder_channel_*` tests pass.

- [ ] **Step 11: Run full core test suite**

```bash
cd /Users/careck/Source/Krillnotes
cargo test -p krillnotes-core 2>&1 | tail -30
```

Expected: all tests pass, zero warnings for `_device_short` (the leading underscore suppresses dead-code lint).

- [ ] **Step 12: Commit**

```bash
cd /Users/careck/Source/Krillnotes
git add krillnotes-core/src/core/sync/folder.rs
git commit -m "fix: recipient-prefixed filenames and inbox filter for folder sync"
```

---

## Final: PR

- [ ] **Push and open PR**

```bash
cd /Users/careck/Source/Krillnotes
git push -u github-https HEAD
```

Then open a PR against `master` describing both fixes.
