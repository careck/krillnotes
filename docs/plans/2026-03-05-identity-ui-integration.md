# Identity UI Integration — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire the `IdentityManager` foundation into Tauri + React so identities replace per-workspace passwords entirely. Multi-identity support, on-demand unlock, first-launch onboarding.

**Architecture:** 10 new Tauri commands expose `IdentityManager` to the frontend. `AppState` gains `identity_manager` + `unlocked_identities` HashMap; loses `workspace_passwords`. Workspace creation auto-generates DB passwords bound to an identity. Three new React dialogs (IdentityManager, CreateIdentity, UnlockIdentity) replace the old password dialogs.

**Tech Stack:** Rust (krillnotes-core identity module on `swarm` branch), Tauri v2, React 19, Tailwind v4, i18next

**Design doc:** `docs/plans/2026-03-05-identity-ui-integration-design.md`

**Branch:** `feat/identity-ui` off `swarm`

---

### Task 1: Add `workspace_id` Field to Workspace Struct

The `workspace_id` is currently only in the `workspace_meta` DB table. We need it on the struct so `write_info_json` can include it — the workspace manager reads `info.json` to find identity bindings without opening the DB.

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Add field to struct**

Add `workspace_id: String` to the `Workspace` struct (after `workspace_root`):

```rust
pub struct Workspace {
    storage: Storage,
    script_registry: ScriptRegistry,
    operation_log: OperationLog,
    device_id: String,
    current_user_id: i64,
    workspace_root: PathBuf,
    workspace_id: String,  // NEW — UUID from workspace_meta
    attachment_key: Option<[u8; 32]>,
    // ... rest unchanged
}
```

**Step 2: Populate in `create`**

In `Workspace::create`, the `workspace_id` is already generated as a local variable. Store it in the struct:

```rust
// After: let workspace_id = uuid::Uuid::new_v4().to_string();
// In the struct construction at the bottom of create():
workspace_id,
```

**Step 3: Populate in `open`**

In `Workspace::open`, the `workspace_id` is already read/generated. Store it in the struct the same way.

**Step 4: Add public getter**

```rust
/// Returns the unique workspace UUID (stored in workspace_meta).
pub fn workspace_id(&self) -> &str {
    &self.workspace_id
}
```

**Step 5: Update `write_info_json`**

Add `workspace_id` to the JSON:

```rust
let info = serde_json::json!({
    "workspace_id": self.workspace_id,
    "created_at": created_at,
    "note_count": note_count,
    "attachment_count": attachment_count,
});
```

**Step 6: Run tests**

Run: `cargo test -p krillnotes-core`
Expected: all existing tests pass (workspace_id is an additive change)

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: expose workspace_id on Workspace struct and in info.json"
```

---

### Task 2: Add `rename_identity` to IdentityManager

**Files:**
- Modify: `krillnotes-core/src/core/identity.rs`

**Step 1: Write the test**

```rust
#[test]
fn test_rename_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Old Name", "pass123").unwrap();
    let uuid = file.identity_uuid;

    mgr.rename_identity(&uuid, "New Name").unwrap();

    // Check settings
    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].display_name, "New Name");

    // Check identity file
    let unlocked = mgr.unlock_identity(&uuid, "pass123").unwrap();
    assert_eq!(unlocked.display_name, "New Name");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_rename_identity`
Expected: FAIL — `rename_identity` method does not exist

**Step 3: Implement `rename_identity`**

```rust
/// Renames an identity's display name in both the identity file and the settings registry.
pub fn rename_identity(&self, identity_uuid: &Uuid, new_name: &str) -> Result<()> {
    // Update identity file
    let identity_path = self.config_dir.join("identities").join(format!("{}.json", identity_uuid));
    let content = std::fs::read_to_string(&identity_path)
        .map_err(|_| KrillnotesError::IdentityNotFound(identity_uuid.to_string()))?;
    let mut identity_file: IdentityFile = serde_json::from_str(&content)
        .map_err(|e| KrillnotesError::IdentityCorrupt(e.to_string()))?;
    identity_file.display_name = new_name.to_string();
    let json = serde_json::to_string_pretty(&identity_file)?;
    std::fs::write(&identity_path, json)?;

    // Update settings registry
    let mut settings = self.load_settings()?;
    if let Some(identity_ref) = settings.identities.iter_mut().find(|i| i.uuid == *identity_uuid) {
        identity_ref.display_name = new_name.to_string();
    }
    self.save_settings(&settings)?;

    Ok(())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_rename_identity`
Expected: PASS

**Step 5: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests pass

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/identity.rs
git commit -m "feat: add rename_identity to IdentityManager"
```

---

### Task 3: Update AppState + Initialization

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add imports**

At the top of `lib.rs`, add:

```rust
use krillnotes_core::{IdentityManager, UnlockedIdentity};
use uuid::Uuid;
```

**Step 2: Modify AppState struct**

Replace `workspace_passwords` with identity fields:

```rust
pub struct AppState {
    pub workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub focused_window: Arc<Mutex<Option<String>>>,
    // REMOVED: pub workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>,
    pub identity_manager: Arc<Mutex<IdentityManager>>,
    pub unlocked_identities: Arc<Mutex<HashMap<Uuid, UnlockedIdentity>>>,
    pub paste_menu_items: Arc<Mutex<HashMap<String, (tauri::menu::MenuItem<tauri::Wry>, tauri::menu::MenuItem<tauri::Wry>)>>>,
    pub workspace_menu_items: Arc<Mutex<HashMap<String, Vec<tauri::menu::MenuItem<tauri::Wry>>>>>,
    pub pending_file_open: Arc<Mutex<Option<PathBuf>>>,
}
```

**Step 3: Update AppState initialization in `run()`**

In the `.manage(AppState { ... })` block, replace `workspace_passwords` with:

```rust
identity_manager: Arc::new(Mutex::new(
    IdentityManager::new(settings::config_dir()).expect("Failed to init IdentityManager")
)),
unlocked_identities: Arc::new(Mutex::new(HashMap::new())),
```

**Step 4: Add `config_dir` helper to settings.rs**

In `settings.rs`, add a public function that returns the config directory path (same logic as `settings_file_path` but just the directory):

```rust
/// Returns the config directory for Krillnotes.
/// - macOS / Linux: `~/.config/krillnotes/`
/// - Windows: `%APPDATA%/Krillnotes/`
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("Krillnotes")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".config").join("krillnotes")
    }
}
```

**Step 5: Remove all `workspace_passwords` references**

Remove password caching code from:
- `create_workspace` command (the `settings.cache_workspace_passwords` block)
- `open_workspace` command (the `settings.cache_workspace_passwords` block)
- `get_cached_password` command (delete entirely)
- Remove from `tauri::generate_handler![]`

**Step 6: Compile check**

Run: `cd krillnotes-desktop && cargo check`
Expected: compile errors from modified `create_workspace`/`open_workspace` signatures (fixed in Tasks 5–6), but AppState itself compiles

**Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/src/settings.rs
git commit -m "feat: add identity_manager and unlocked_identities to AppState, remove password caching"
```

---

### Task 4: Identity Tauri Commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Implement identity commands**

Add these commands before the existing workspace commands:

```rust
#[tauri::command]
fn list_identities(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<krillnotes_core::IdentityRef>, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.list_identities().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_identity(
    state: State<'_, AppState>,
    display_name: String,
    passphrase: String,
) -> std::result::Result<krillnotes_core::IdentityRef, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let file = mgr.create_identity(&display_name, &passphrase)
        .map_err(|e| e.to_string())?;
    let uuid = file.identity_uuid;

    // Auto-unlock
    let unlocked = mgr.unlock_identity(&uuid, &passphrase)
        .map_err(|e| e.to_string())?;
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .insert(uuid, unlocked);

    // Return the IdentityRef
    let identities = mgr.list_identities().map_err(|e| e.to_string())?;
    identities.into_iter().find(|i| i.uuid == uuid)
        .ok_or_else(|| "Identity created but not found in registry".to_string())
}

#[tauri::command]
fn unlock_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let unlocked = mgr.unlock_identity(&uuid, &passphrase)
        .map_err(|e| match e {
            KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })?;
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .insert(uuid, unlocked);
    Ok(())
}

#[tauri::command]
fn lock_identity(
    app: AppHandle,
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Find and close all workspace windows belonging to this identity
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bound_workspaces = mgr.get_workspaces_for_identity(&uuid)
        .map_err(|e| e.to_string())?;
    let bound_workspace_ids: HashSet<String> = bound_workspaces.iter()
        .map(|(ws_uuid, _)| ws_uuid.clone())
        .collect();

    // Match workspace_ids against open workspaces via info.json workspace_id
    let paths = state.workspace_paths.lock().expect("Mutex poisoned");
    let labels_to_close: Vec<String> = paths.iter()
        .filter(|(_, path)| {
            let (ws_id, _, _, _) = read_info_json_full(path);
            ws_id.map(|id| bound_workspace_ids.contains(&id)).unwrap_or(false)
        })
        .map(|(label, _)| label.clone())
        .collect();
    drop(paths);

    for label in &labels_to_close {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.close();
        }
    }

    // Wipe identity from memory
    state.unlocked_identities.lock().expect("Mutex poisoned").remove(&uuid);
    Ok(())
}

#[tauri::command]
fn delete_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Must be locked first
    let is_unlocked = state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid);
    if is_unlocked {
        return Err("Lock the identity before deleting it".to_string());
    }

    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.delete_identity(&uuid).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.rename_identity(&uuid, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn change_identity_passphrase(
    state: State<'_, AppState>,
    identity_uuid: String,
    old_passphrase: String,
    new_passphrase: String,
) -> std::result::Result<(), String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.change_passphrase(&uuid, &old_passphrase, &new_passphrase)
        .map_err(|e| match e {
            KrillnotesError::IdentityWrongPassphrase => "WRONG_PASSPHRASE".to_string(),
            other => other.to_string(),
        })
}

#[tauri::command]
fn get_unlocked_identities(
    state: State<'_, AppState>,
) -> Vec<String> {
    state.unlocked_identities.lock().expect("Mutex poisoned")
        .keys()
        .map(|uuid| uuid.to_string())
        .collect()
}

#[tauri::command]
fn is_identity_unlocked(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> bool {
    Uuid::parse_str(&identity_uuid)
        .map(|uuid| state.unlocked_identities.lock().expect("Mutex poisoned").contains_key(&uuid))
        .unwrap_or(false)
}

#[tauri::command]
fn get_workspaces_for_identity(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> std::result::Result<Vec<WorkspaceBindingInfo>, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let bindings = mgr.get_workspaces_for_identity(&uuid)
        .map_err(|e| e.to_string())?;
    Ok(bindings.into_iter().map(|(ws_uuid, binding)| WorkspaceBindingInfo {
        workspace_uuid: ws_uuid,
        db_path: binding.db_path,
    }).collect())
}
```

**Step 2: Add `WorkspaceBindingInfo` struct**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBindingInfo {
    pub workspace_uuid: String,
    pub db_path: String,
}
```

**Step 3: Add `read_info_json_full` helper**

Update the existing `read_info_json` to also return workspace_id, or add a new helper:

```rust
fn read_info_json_full(workspace_dir: &Path) -> (Option<String>, Option<i64>, Option<usize>, Option<usize>) {
    let path = workspace_dir.join("info.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (None, None, None, None),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };
    let workspace_id = v["workspace_id"].as_str().map(|s| s.to_string());
    let created_at = v["created_at"].as_i64();
    let note_count = v["note_count"].as_u64().map(|n| n as usize);
    let attachment_count = v["attachment_count"].as_u64().map(|n| n as usize);
    (workspace_id, created_at, note_count, attachment_count)
}
```

Update `read_info_json` to call `read_info_json_full` and discard workspace_id.

**Step 4: Register in `generate_handler!`**

Add all new commands to the handler list:

```rust
list_identities,
create_identity,
unlock_identity,
lock_identity,
delete_identity,
rename_identity,
change_identity_passphrase,
get_unlocked_identities,
is_identity_unlocked,
get_workspaces_for_identity,
```

**Step 5: Compile check**

Run: `cd krillnotes-desktop && cargo check`

**Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add 10 identity Tauri commands"
```

---

### Task 5: Modify `create_workspace` for Identity Flow

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Change signature**

Replace `password: String` with `identity_uuid: String`:

```rust
#[tauri::command]
async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    identity_uuid: String,
) -> std::result::Result<WorkspaceInfo, String> {
```

**Step 2: Generate random password + bind to identity**

Replace the body after `std::fs::create_dir_all(&folder)`:

```rust
let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

// Generate random DB password
let password: String = {
    let mut bytes = [0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
};

let db_path = folder.join("notes.db");
let workspace = Workspace::create(&db_path, &password)
    .map_err(|e| format!("Failed to create: {e}"))?;

// Read the workspace_id from the newly created workspace
let workspace_uuid = workspace.workspace_id().to_string();

// Bind workspace to identity (encrypt DB password with identity seed)
{
    let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
    let unlocked = identities.get(&uuid)
        .ok_or_else(|| "Identity is not unlocked".to_string())?;
    let seed = unlocked.signing_key.to_bytes();
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.bind_workspace(
        &uuid,
        &workspace_uuid,
        &db_path.display().to_string(),
        &password,
        &seed,
    ).map_err(|e| format!("Failed to bind workspace to identity: {e}"))?;
}

// Wipe plaintext password (it's now encrypted in identity settings)
drop(password);
```

Rest of the function (window creation, store_workspace, etc.) remains unchanged.

**Step 3: Compile check**

Run: `cd krillnotes-desktop && cargo check`

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: create_workspace generates random password and binds to identity"
```

---

### Task 6: Modify `open_workspace` for Identity Flow

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Change signature**

Remove `password` param:

```rust
#[tauri::command]
async fn open_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<WorkspaceInfo, String> {
```

**Step 2: Look up identity binding and decrypt password**

Replace the body after `let db_path = folder.join("notes.db")`:

```rust
// Read workspace_id from info.json
let (ws_uuid_opt, _, _, _) = read_info_json_full(&folder);
let workspace_uuid = ws_uuid_opt
    .ok_or_else(|| "IDENTITY_REQUIRED".to_string())?;

// Look up which identity this workspace is bound to
let (identity_uuid, db_password) = {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let settings = mgr.load_settings().map_err(|e| e.to_string())?;
    let binding = settings.workspaces.get(&workspace_uuid)
        .ok_or_else(|| "IDENTITY_REQUIRED".to_string())?;
    let identity_uuid = binding.identity_uuid;

    // Check if identity is unlocked
    let identities = state.unlocked_identities.lock().expect("Mutex poisoned");
    let unlocked = identities.get(&identity_uuid)
        .ok_or_else(|| format!("IDENTITY_LOCKED:{}", identity_uuid))?;
    let seed = unlocked.signing_key.to_bytes();

    let password = mgr.decrypt_db_password(&workspace_uuid, &seed)
        .map_err(|e| format!("Failed to decrypt DB password: {e}"))?;
    (identity_uuid, password)
};

let workspace = Workspace::open(&db_path, &db_password)
    .map_err(|e| format!("Failed to open: {e}"))?;
```

The `IDENTITY_LOCKED:<uuid>` error string lets the frontend know which identity to prompt for. Rest of the function unchanged.

**Step 3: Compile check**

Run: `cd krillnotes-desktop && cargo check`

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: open_workspace decrypts DB password from identity"
```

---

### Task 7: Update `WorkspaceEntry` to Include Identity Info

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add identity fields to `WorkspaceEntry` struct**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    path: String,
    is_open: bool,
    last_modified: i64,
    size_bytes: u64,
    created_at: Option<i64>,
    note_count: Option<usize>,
    attachment_count: Option<usize>,
    // NEW
    workspace_uuid: Option<String>,
    identity_uuid: Option<String>,
    identity_name: Option<String>,
}
```

**Step 2: Update `list_workspace_files` to populate identity info**

After reading `info.json`, look up the identity binding:

```rust
let (workspace_id, created_at, note_count, attachment_count) = read_info_json_full(&folder);

// Look up identity binding for this workspace
let (identity_uuid, identity_name) = workspace_id.as_ref()
    .and_then(|ws_id| {
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        let settings = mgr.load_settings().ok()?;
        let binding = settings.workspaces.get(ws_id)?;
        let identity_ref = settings.identities.iter()
            .find(|i| i.uuid == binding.identity_uuid)?;
        Some((Some(identity_ref.uuid.to_string()), Some(identity_ref.display_name.clone())))
    })
    .unwrap_or((None, None));
```

**Step 3: Compile check and commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: include identity info in WorkspaceEntry"
```

---

### Task 8: Update `duplicate_workspace` for Identity Flow

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Change signature**

Replace source/new passwords with identity:

```rust
#[tauri::command]
fn duplicate_workspace(
    state: State<'_, AppState>,
    source_path: String,
    identity_uuid: String,
    new_name: String,
) -> std::result::Result<(), String> {
```

**Step 2: Update body**

- Decrypt source DB password from identity (same as open_workspace pattern)
- Generate new random password for the duplicate
- Export with source password, import with new password
- Bind new workspace to identity

**Step 3: Compile check and commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: duplicate_workspace uses identity for password management"
```

---

### Task 9: Add "Manage Identities" Menu Item

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (menu event handler)

**Step 1: Add menu item to File menu**

In `build_file_menu`, add after `open_item`:

```rust
let identities_item = MenuItemBuilder::with_id(
    "file_identities",
    s(strings, "manageIdentities", "Manage Identities…"),
).build(app)?;
```

Add to the submenu builder items list.

**Step 2: Handle menu event**

In `handle_menu_event` in `lib.rs`, add:

```rust
"file_identities" => {
    emit_to_focused_or_main(&app, &state, "menu-event", "File > Manage Identities clicked");
}
```

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add Manage Identities menu item"
```

---

### Task 10: Remove Password Caching Code

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/settings.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (any remaining references)

**Step 1: Remove `cache_workspace_passwords` from `AppSettings`**

Remove the field, its default function, and its serde attribute from `settings.rs`. Update tests.

**Step 2: Full compile check**

Run: `cd krillnotes-desktop && cargo check`
Expected: compiles clean — no more references to `workspace_passwords` or `cache_workspace_passwords`

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/settings.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "chore: remove password caching (replaced by identity system)"
```

---

### Task 11: TypeScript Types for Identity

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Add identity types**

```typescript
export interface IdentityRef {
  uuid: string;
  displayName: string;
  file: string;
  lastUsed: string;  // ISO 8601
}

export interface WorkspaceBindingInfo {
  workspaceUuid: string;
  dbPath: string;
}
```

**Step 2: Update `WorkspaceEntry`**

Add to the existing interface:

```typescript
export interface WorkspaceEntry {
  // ... existing fields ...
  workspaceUuid: string | null;
  identityUuid: string | null;
  identityName: string | null;
}
```

**Step 3: Update `AppSettings`**

Remove `cacheWorkspacePasswords` from `AppSettings` interface.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add TypeScript types for identity"
```

---

### Task 12: i18n Strings

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json` (and all other locale files)

**Step 1: Add identity section to `en.json`**

```json
"identity": {
  "manageTitle": "Identities",
  "create": "Create Identity",
  "unlock": "Unlock",
  "lock": "Lock",
  "delete": "Delete",
  "rename": "Rename",
  "changePassphrase": "Change Passphrase",
  "displayName": "Display Name",
  "passphrase": "Passphrase",
  "confirmPassphrase": "Confirm Passphrase",
  "currentPassphrase": "Current Passphrase",
  "newPassphrase": "New Passphrase",
  "wrongPassphrase": "Wrong passphrase. Please try again.",
  "passphraseMismatch": "Passphrases do not match.",
  "passphraseRequired": "Passphrase is required.",
  "nameRequired": "Display name is required.",
  "unlocked": "Unlocked",
  "locked": "Locked",
  "enterPassphrase": "Enter passphrase for {{name}}",
  "createFirst": "Create Your First Identity",
  "createFirstDescription": "An identity protects your workspaces with a single passphrase.",
  "deleteConfirm": "Are you sure you want to delete \"{{name}}\"?",
  "deleteHasBound": "This identity has bound workspaces. Unbind them first.",
  "mustLockFirst": "Lock the identity before deleting it.",
  "boundWorkspaces": "{{count}} workspace(s)",
  "noIdentities": "No identities yet",
  "filterByIdentity": "Filter by identity",
  "allIdentities": "All identities",
  "passphraseChanged": "Passphrase changed successfully."
}
```

**Step 2: Add menu string**

```json
"menu": {
  "manageIdentities": "Manage Identities…",
  // ... existing entries ...
}
```

**Step 3: Copy keys to other locale files with English values as placeholders**

Copy the `identity` section to `de.json`, `fr.json`, `es.json`, `ja.json`, `ko.json`, `zh.json` with the same English text (to be translated later).

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/
git commit -m "feat: add i18n strings for identity management"
```

---

### Task 13: CreateIdentityDialog Component

**Files:**
- Create: `krillnotes-desktop/src/components/CreateIdentityDialog.tsx`

**Step 1: Create the component**

A dialog with:
- Display name input
- Passphrase input
- Confirm passphrase input
- Error display
- Create button
- Cancel button (hidden in first-launch mode)

Props:

```typescript
interface CreateIdentityDialogProps {
  isOpen: boolean;
  isFirstLaunch?: boolean;  // hides cancel, shows welcome text
  onCreated: (identity: IdentityRef) => void;
  onCancel: () => void;
}
```

The dialog calls `invoke('create_identity', { displayName, passphrase })` on submit. Validates passphrase === confirm before sending.

Follow the same dialog patterns used in `NewWorkspaceDialog.tsx` (overlay, rounded card, Tailwind classes).

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/CreateIdentityDialog.tsx
git commit -m "feat: add CreateIdentityDialog component"
```

---

### Task 14: UnlockIdentityDialog Component

**Files:**
- Create: `krillnotes-desktop/src/components/UnlockIdentityDialog.tsx`

**Step 1: Create the component**

A focused dialog that prompts for a specific identity's passphrase:
- Shows identity display name
- Passphrase input with autoFocus
- Error display (wrong passphrase)
- Unlock button
- Cancel button

Props:

```typescript
interface UnlockIdentityDialogProps {
  isOpen: boolean;
  identityUuid: string;
  identityName: string;
  onUnlocked: () => void;
  onCancel: () => void;
}
```

Calls `invoke('unlock_identity', { identityUuid, passphrase })`. On `WRONG_PASSPHRASE` error, shows inline error and clears input.

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/UnlockIdentityDialog.tsx
git commit -m "feat: add UnlockIdentityDialog component"
```

---

### Task 15: IdentityManagerDialog Component

**Files:**
- Create: `krillnotes-desktop/src/components/IdentityManagerDialog.tsx`

**Step 1: Create the component**

The identity picker + CRUD manager. Shows:
- List of all identities with lock/unlock status icons
- [+] button to create new identity (opens CreateIdentityDialog)
- Per-identity ⋮ menu with: Unlock/Lock, Rename, Change Passphrase, Delete
- Close button

Props:

```typescript
interface IdentityManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
}
```

Internal state:
- `identities: IdentityRef[]` — from `invoke('list_identities')`
- `unlockedIds: Set<string>` — from `invoke('get_unlocked_identities')`
- `showCreate: boolean`
- `renaming: string | null` — identity UUID being renamed
- `changingPassphrase: string | null`
- `unlocking: string | null` — identity UUID being unlocked

Actions call the corresponding Tauri commands and refresh the list.

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/IdentityManagerDialog.tsx
git commit -m "feat: add IdentityManagerDialog component"
```

---

### Task 16: App.tsx Startup + First-Launch Flow

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Add first-launch identity check**

In the startup `useEffect`, add a check:

```typescript
useEffect(() => {
  const win = getCurrentWebviewWindow();
  if (win.label !== 'main') return;

  invoke<IdentityRef[]>('list_identities').then(identities => {
    if (identities.length === 0) {
      setShowCreateFirstIdentity(true);
    }
  });
}, []);
```

**Step 2: Add state variables**

```typescript
const [showCreateFirstIdentity, setShowCreateFirstIdentity] = useState(false);
const [showIdentityManager, setShowIdentityManager] = useState(false);
```

**Step 3: Add menu event handler**

In `createMenuHandlers`, add:

```typescript
'File > Manage Identities clicked': () => {
  setShowIdentityManager(true);
},
```

**Step 4: Render dialogs**

```tsx
<CreateIdentityDialog
  isOpen={showCreateFirstIdentity}
  isFirstLaunch={true}
  onCreated={() => setShowCreateFirstIdentity(false)}
  onCancel={() => setShowCreateFirstIdentity(false)}
/>
<IdentityManagerDialog
  isOpen={showIdentityManager}
  onClose={() => setShowIdentityManager(false)}
/>
```

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat: add first-launch identity creation and Manage Identities menu handler"
```

---

### Task 17: NewWorkspaceDialog — Remove Password, Add Identity Selector

**Files:**
- Modify: `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx`

**Step 1: Remove password step**

Remove the `step` state, `SetPasswordDialog` import, and the password step entirely.

**Step 2: Add identity selector**

On submit (after name validation), the dialog needs to know which identity to bind to. Add a dropdown of unlocked identities:

```typescript
const [identities, setIdentities] = useState<IdentityRef[]>([]);
const [unlockedIds, setUnlockedIds] = useState<string[]>([]);
const [selectedIdentity, setSelectedIdentity] = useState<string>('');

useEffect(() => {
  if (!isOpen) return;
  Promise.all([
    invoke<IdentityRef[]>('list_identities'),
    invoke<string[]>('get_unlocked_identities'),
  ]).then(([ids, unlocked]) => {
    setIdentities(ids);
    setUnlockedIds(unlocked);
    // Default to first unlocked identity
    if (unlocked.length > 0 && !selectedIdentity) {
      setSelectedIdentity(unlocked[0]);
    }
  });
}, [isOpen]);
```

**Step 3: Update create call**

```typescript
await invoke<WorkspaceInfo>('create_workspace', {
  path,
  identityUuid: selectedIdentity,
});
```

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/NewWorkspaceDialog.tsx
git commit -m "feat: NewWorkspaceDialog uses identity selector instead of password"
```

---

### Task 18: WorkspaceManagerDialog — Identity Integration

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx`

**Step 1: Add identity filter dropdown**

At the top of the dialog, add a filter dropdown:

```typescript
const [identityFilter, setIdentityFilter] = useState<string>('all');
const [identities, setIdentities] = useState<IdentityRef[]>([]);
const [unlockedIds, setUnlockedIds] = useState<string[]>([]);
```

Filter `sortedEntries` by `identityFilter` before rendering:

```typescript
const filteredEntries = identityFilter === 'all'
  ? sortedEntries
  : sortedEntries.filter(e => e.identityUuid === identityFilter);
```

**Step 2: Show identity name per workspace entry**

Add identity display name and lock icon to each entry row:

```tsx
{entry.identityName && (
  <span className="text-xs text-muted-foreground">
    {unlockedIds.includes(entry.identityUuid!) ? '🔓' : '🔒'} {entry.identityName}
  </span>
)}
```

**Step 3: Replace password flow with identity unlock**

Replace `handleOpen` to use the new identity-based flow:

```typescript
const handleOpen = async (target: WorkspaceEntry = selected!) => {
  if (!target || target.isOpen) return;
  setOpening(true);
  setError('');

  try {
    await invoke('open_workspace', { path: target.path });
    onClose();
  } catch (err) {
    const errStr = String(err);
    if (errStr.startsWith('IDENTITY_LOCKED:')) {
      // Identity needs unlocking
      const identityUuid = errStr.split(':')[1];
      const identity = identities.find(i => i.uuid === identityUuid);
      setUnlockTarget({
        uuid: identityUuid,
        name: identity?.displayName ?? 'Unknown',
        workspacePath: target.path,
      });
    } else if (errStr === 'IDENTITY_REQUIRED') {
      setError('This workspace is not bound to any identity.');
    } else if (errStr !== 'focused_existing') {
      setError(errStr);
    }
    setOpening(false);
  }
};
```

**Step 4: Add UnlockIdentityDialog integration**

When `unlockTarget` is set, show `UnlockIdentityDialog`. On success, retry `open_workspace`.

**Step 5: Remove EnterPasswordDialog references**

Remove all references to `EnterPasswordDialog`, `pendingOpen`, `handlePasswordConfirm`, `passwordError`, `get_cached_password`.

**Step 6: Update duplicate flow**

Update duplicate to pass `identityUuid` instead of passwords.

**Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx
git commit -m "feat: WorkspaceManagerDialog with identity filter and unlock flow"
```

---

### Task 19: Remove Old Password Components + Settings Cleanup

**Files:**
- Delete: `krillnotes-desktop/src/components/EnterPasswordDialog.tsx`
- Delete: `krillnotes-desktop/src/components/SetPasswordDialog.tsx`
- Modify: `krillnotes-desktop/src/components/SettingsDialog.tsx`

**Step 1: Delete password dialog files**

```bash
rm krillnotes-desktop/src/components/EnterPasswordDialog.tsx
rm krillnotes-desktop/src/components/SetPasswordDialog.tsx
```

**Step 2: Remove `cacheWorkspacePasswords` toggle from SettingsDialog**

In `SettingsDialog.tsx`, remove the `cachePasswords` state, the toggle UI, and the `cacheWorkspacePasswords` field from the save payload.

**Step 3: Remove any remaining imports**

Search for and remove any remaining imports of the deleted components across the codebase.

**Step 4: TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

**Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove password dialogs and cache_workspace_passwords setting"
```

---

### Task 20: Full Integration Test

**Step 1: Build and run**

```bash
cd krillnotes-desktop && npm update && npm run tauri dev
```

**Step 2: Manual test checklist**

- [ ] First launch: CreateIdentityDialog appears, cannot be dismissed
- [ ] Create identity: name + passphrase → identity created and auto-unlocked
- [ ] Create workspace: name + identity selector → workspace opens, no password prompt
- [ ] Close workspace window, reopen from workspace manager → opens silently (identity still unlocked)
- [ ] Identity manager: shows identity with 🔓 icon
- [ ] Lock identity from manager → workspace windows close
- [ ] Open workspace after locking → UnlockIdentityDialog appears
- [ ] Enter passphrase → workspace opens
- [ ] Create second identity → both appear in manager
- [ ] Create workspace bound to second identity → opens fine
- [ ] Both identities' workspaces open simultaneously
- [ ] Rename identity → name updates in list
- [ ] Change passphrase → old fails, new works
- [ ] Workspace manager filter → filters by identity
- [ ] Duplicate workspace → works with identity

**Step 3: Run Rust tests**

```bash
cargo test -p krillnotes-core
```

**Step 4: Run TypeScript check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

**Step 5: Final commit if any fixes needed**

---

### Task 21: Export/Import Compatibility

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

The existing export/import commands need the DB password. Since the user no longer types it, the export command must decrypt it from the identity.

**Step 1: Update `export_workspace_to_file`**

The existing command has access to the open `Workspace` and its password needs to come from the identity binding. Since the workspace is already open, we can look up the binding and decrypt the password the same way `open_workspace` does.

Alternatively, store the DB password alongside the workspace in a `workspace_passwords: HashMap<String, String>` that maps window label → decrypted password (populated during open, wiped on close). This avoids re-deriving on every export.

**Step 2: Update import flow**

Import creates a new workspace — same flow as `create_workspace`: generate random password, bind to identity.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: export/import uses identity for password management"
```
