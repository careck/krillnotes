# Plan 1a: Device ID Migration

**Issue:** Child of #171 (Mobile support)
**Branch:** `mobile`
**Spec:** `docs/superpowers/specs/2026-04-29-mobile-support-design.md` § Preliminary: Device ID Migration

## Context

The codebase has three device identification mechanisms:

1. **MAC-based** — `device.rs::get_device_id()` returns `device-<16 hex>` via `mac_address` crate. Called in 21 places across sync/relay commands (invites, swarm, relay_accounts, sync, identity, receive_poll) and export.
2. **Per-identity device UUID** — `identity::ensure_device_uuid(dir)` creates/reads a UUID file in each identity directory. Already persisted, already stable.
3. **Composite** — `{identity_uuid}:{device_uuid}` stored in `workspace_meta.device_id`. Used as the HLC node ID and in `RegisterDevice` operations.

The MAC-based ID (1) caused relay sync bugs due to instability across network interfaces. It also blocks mobile (no MAC access on iOS/Android). The per-identity UUID (2) is already the right approach — we just need to make (1) use the same mechanism and remove the `mac_address` dependency.

## Steps

### Step 1: Create app-level device UUID

**Files:** `krillnotes-core/src/core/device.rs`

Replace `get_device_id()` with two functions:

```rust
/// Read or create the app-level device UUID seed file.
/// File: {data_dir}/device_id (plain text, one UUID).
pub fn get_or_create_seed_device_id(data_dir: &Path) -> Result<String>

/// Full priority chain for resolving device ID for a workspace.
/// 1. workspace_meta has device_id → use it
/// 2. operations table has local device_id → use it, write to workspace_meta + seed file
/// 3. Seed file exists → use it, write to workspace_meta
/// 4. Generate device-{uuid}, write everywhere
pub fn resolve_device_id(conn: &Connection, data_dir: &Path) -> Result<String>
```

`get_or_create_seed_device_id`:
- Read `{data_dir}/device_id` file
- If exists and non-empty → return trimmed contents
- If absent → generate `device-{Uuid::new_v4()}`, write to file, return it

`resolve_device_id`:
- Query `SELECT value FROM workspace_meta WHERE key = 'device_id'`
- If found → return it (already migrated or set during creation)
- If absent → query `SELECT DISTINCT device_id FROM operations LIMIT 1`
- If found → write to `workspace_meta`, also write to seed file, return it
- If absent → call `get_or_create_seed_device_id(data_dir)`, write to `workspace_meta`, return it

Remove all `mac_address` crate usage. The old `get_device_id()` function is deleted.

### Step 2: Update Cargo.toml

**File:** `krillnotes-core/Cargo.toml`

- Remove `mac_address = "1.1"` dependency
- Keep `hostname = "0.3"` (used for human-readable device names, has graceful fallback)

### Step 3: Update call sites in krillnotes-core

**File:** `krillnotes-core/src/core/export.rs`

The only core call site. Currently calls `get_device_id()` during export. Replace with:
- Accept `device_id: &str` as a parameter (passed from the Workspace struct which already holds it)
- Or read from `self.device_id` if export is a Workspace method

### Step 4: Update Workspace::open() and init_core()

**File:** `krillnotes-core/src/core/workspace/mod.rs`

`open()`:
- Add `data_dir: &Path` parameter
- After opening DB, call `resolve_device_id(conn, data_dir)` to get/migrate the device ID
- Store result in `self.device_id`
- The existing composite device_id logic (`{identity_uuid}:{device_uuid}`) remains — `resolve_device_id` returns whatever is in workspace_meta, which is already composite for existing workspaces

`init_core()`:
- Add `data_dir: &Path` parameter
- Use `get_or_create_seed_device_id(data_dir)` when constructing the initial device_id if no identity_dir is provided
- Existing logic for composite `{identity_uuid}:{device_uuid}` format stays as-is

### Step 5: Update 21 call sites in krillnotes-desktop

**Files:** `krillnotes-desktop/src-tauri/src/commands/`
- `invites.rs` (2 calls)
- `swarm.rs` (5 calls)
- `relay_accounts.rs` (3 calls)
- `sync.rs` (5 calls)
- `identity.rs` (1 call)
- `receive_poll.rs` (4 calls)

All these call `get_device_id()` independently. Replace each with:
- Read the device_id from the Workspace instance (already available via AppState) OR
- Call `get_or_create_seed_device_id(&home_dir())` for operations that don't have a workspace context

Pattern: most of these commands already have access to the workspace via `state.workspaces`. Extract `device_id` from the workspace struct instead of computing it fresh each time. This is more correct anyway — the device_id should come from the workspace context, not be independently derived.

For commands that operate before a workspace is open (relay account management, identity operations), use `get_or_create_seed_device_id` with the home_dir path.

### Step 6: Update Workspace::open() callers in lib.rs

**File:** `krillnotes-desktop/src-tauri/src/lib.rs` (and commands/workspace.rs)

All calls to `Workspace::open()` and `Workspace::init_core()` need the new `data_dir` parameter. Pass `&home_dir()` on desktop. On mobile (future), the Tauri app will pass the app sandbox directory.

### Step 7: Migration gate in create_workspace command

**File:** `krillnotes-desktop/src-tauri/src/commands/workspace.rs`

In `create_workspace` command:
- Check if seed file exists at `home_dir()/device_id`
- If not → call `list_workspace_files()` (already exists in this file)
- If workspaces exist → return error: "Please open an existing workspace first to migrate your device identity"
- If no workspaces → proceed (fresh install, generate new UUID)

### Step 8: Update tests

**File:** `krillnotes-core/src/core/device.rs` (test module)

- Remove MAC-based tests
- Add tests for `get_or_create_seed_device_id`: creates file on first call, reads same value on second call
- Add tests for `resolve_device_id`: priority chain (workspace_meta > operations > seed file > generate)
- Use temp directories for test isolation

**Files:** Any existing tests that call `get_device_id()` — update to use new API.

### Step 9: Verify and test

- `cargo test -p krillnotes-core` — all tests pass
- `cd krillnotes-desktop && npx tsc --noEmit` — type check
- `cd krillnotes-desktop && npm run tauri dev` — manual test:
  - Open existing workspace → device_id migrated to seed file
  - Create new workspace → picks up seed file device_id
  - Check relay sync still works with migrated ID

## Commit sequence

1. `refactor: replace MAC-based device ID with persisted UUID` — steps 1-6
2. `feat: add migration gate for device ID on new workspace creation` — step 7
3. `test: add device ID migration tests` — step 8

## Risks

- **Relay identity break:** If the old MAC-based `get_device_id()` value was stored on the relay as the device identifier, switching to a UUID means the relay won't recognize the device. However, the MAC-based ID was already unstable (the problem we're fixing), so relay identity was already unreliable. The composite device_id in workspace_meta (which IS stable) is what matters for workspace sync.
- **Export compatibility:** Exported archives embed a device_id. Old exports have MAC-based IDs. Import should handle both formats (no format validation on the device_id string).
