# Peer Contact Manager — Phase B Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Workspace Peers" menu item and dedicated dialog for viewing, removing, and pre-authorising sync peers for a workspace, with contact book integration.

**Architecture:** All resolution logic lives in `krillnotes-core` (`Workspace` gains three peer methods; `PeerInfo` struct added to `peer_registry.rs`). Three thin Tauri commands wrap these. Two new React components (`WorkspacePeersDialog`, `AddPeerFromContactsDialog`) follow existing dialog patterns. The existing `create_invite_bundle_cmd` is reused for invite file creation with no changes.

**Spec:** `docs/superpowers/specs/2026-03-11-peer-contact-manager-phase-b-design.md`

**Tech Stack:** Rust (`rusqlite`, `serde`), Tauri v2, React 19, TypeScript, Tailwind v4.

---

## Setup: Create Worktree

- [ ] **Create feature worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/contact-manager-phase-b -b feat/contact-manager-phase-b
```

All subsequent work happens in `.worktrees/feat/contact-manager-phase-b/`. Run git commands from that directory.

---

## Chunk 1: Core — `PeerInfo` struct + `Workspace` peer methods

### Files

- **Modify:** `krillnotes-core/src/core/peer_registry.rs` — add `PeerInfo` struct
- **Modify:** `krillnotes-core/src/core/workspace.rs` — add `list_peers_info`, `add_contact_as_peer`, `remove_peer`
- **Modify:** `krillnotes-core/src/lib.rs` — re-export `PeerInfo`

---

### Task 1: Add `PeerInfo` struct to `peer_registry.rs`

**Files:**
- Modify: `krillnotes-core/src/core/peer_registry.rs`

- [ ] **Step 1: Add `PeerInfo` struct**

Find the existing `SyncPeer` struct in `peer_registry.rs` and add `PeerInfo` immediately after it:

```rust
/// A resolved view of a sync peer, joining sync_peers with the contact book.
/// This is the type returned to callers (Tauri, future frontends).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    /// Raw device ID from sync_peers (PRIMARY KEY). May be `identity:<pubkey>` for
    /// pre-authorised contacts who have never synced yet.
    pub peer_device_id: String,
    /// Ed25519 public key (base64) — the peer's identity.
    pub peer_identity_id: String,
    /// Resolved display name: local_name || declared_name || first 8 chars of key + "…"
    pub display_name: String,
    /// 4-word BIP-39 fingerprint derived from BLAKE3(peer_identity_id).
    pub fingerprint: String,
    /// Trust level string if peer is in the contact book ("Tofu", "CodeVerified",
    /// "Vouched", "VerifiedInPerson"). None if not in contacts.
    pub trust_level: Option<String>,
    /// Contact UUID (as String) if peer is in the contact book. None otherwise.
    pub contact_id: Option<String>,
    /// ISO 8601 timestamp of last .swarm bundle exchange. None if never synced.
    pub last_sync: Option<String>,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p krillnotes-core 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/peer_registry.rs
git commit -m "feat(core): add PeerInfo struct to peer_registry"
```

---

### Task 2: Add failing tests for `Workspace` peer methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (test module at the bottom)

- [ ] **Step 1: Add three failing tests to the existing `#[cfg(test)]` block in `workspace.rs`**

Find the existing test module (search for `#[cfg(test)]`) and add inside it:

```rust
#[test]
fn test_list_peers_info_unknown_peer() {
    // A peer that is NOT in the contact book should have:
    // - display_name = first 8 chars of identity_id + "…"
    // - trust_level = None
    // - contact_id = None
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path().join("ws.db"), "").unwrap();
    ws.add_contact_as_peer("AAAAAAAAAAAAAAAA").unwrap();

    // Contact manager with no contacts
    let cm_dir = tempfile::tempdir().unwrap();
    let key = [0u8; 32];
    let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();

    let peers = ws.list_peers_info(&cm).unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].display_name, "AAAAAAAA…");
    assert!(peers[0].trust_level.is_none());
    assert!(peers[0].contact_id.is_none());
    assert!(!peers[0].fingerprint.is_empty());
}

#[test]
fn test_list_peers_info_known_contact() {
    // A peer whose public key matches a contact should have resolved name + trust level.
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path().join("ws.db"), "").unwrap();
    let pubkey = "BBBBBBBBBBBBBBBB";
    ws.add_contact_as_peer(pubkey).unwrap();

    let cm_dir = tempfile::tempdir().unwrap();
    let key = [1u8; 32];
    let mut cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();
    let contact = cm.create_contact("Bob", pubkey, TrustLevel::CodeVerified).unwrap();

    let peers = ws.list_peers_info(&cm).unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].display_name, "Bob");
    assert_eq!(peers[0].trust_level.as_deref(), Some("CodeVerified"));
    let expected_contact_id = contact.contact_id.to_string();
    assert_eq!(peers[0].contact_id.as_deref(), Some(expected_contact_id.as_str()));
}

#[test]
fn test_add_and_remove_peer() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path().join("ws.db"), "").unwrap();
    let pubkey = "CCCCCCCCCCCCCCCC";

    // Add
    ws.add_contact_as_peer(pubkey).unwrap();
    let cm_dir = tempfile::tempdir().unwrap();
    let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), [0u8; 32]).unwrap();
    let peers = ws.list_peers_info(&cm).unwrap();
    assert_eq!(peers.len(), 1);

    // Remove by placeholder device id
    let placeholder = format!("identity:{}", pubkey);
    ws.remove_peer(&placeholder).unwrap();
    let peers = ws.list_peers_info(&cm).unwrap();
    assert_eq!(peers.len(), 0);
}
```

You will need these imports at the top of the test module (add any that are missing):

```rust
use crate::core::contact::{ContactManager, TrustLevel};
```

- [ ] **Step 2: Run tests — expect compile failure**

```bash
cargo test -p krillnotes-core test_list_peers_info 2>&1 | tail -10
cargo test -p krillnotes-core test_add_and_remove_peer 2>&1 | tail -10
```

Expected: compile errors — `no method named list_peers_info`, `add_contact_as_peer`, `remove_peer` on `Workspace`.

---

### Task 3: Implement `list_peers_info`, `add_contact_as_peer`, `remove_peer` on `Workspace`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

- [ ] **Step 1: Find where `Workspace` stores its DB connection**

Run: `grep -n "conn\|connection\|Connection" krillnotes-core/src/core/workspace.rs | head -20`

Note the field name (likely `conn`, `db`, or `connection`) and type (likely `Arc<Mutex<Connection>>` or `Connection`). Use this field in the implementation below, adjusting lock syntax if needed.

- [ ] **Step 2: Add imports for `PeerInfo` and `PeerRegistry`**

At the top of `workspace.rs`, find the existing `use` block for `contact` or `peer_registry` items and add:

```rust
use crate::core::contact::{generate_fingerprint, TrustLevel};
use crate::core::peer_registry::{PeerInfo, PeerRegistry};
```

`generate_fingerprint` is `pub fn generate_fingerprint(public_key_b64: &str) -> Result<String>` in `contact.rs` — it is fallible (base64 decode can fail). Handle this with a fallback in the implementation below.

- [ ] **Step 3: Add the three methods to the `impl Workspace` block**

Find the `impl Workspace` block and add these methods. Replace `self.conn` with the actual field name and locking pattern from Step 1.

```rust
/// Returns a resolved view of all sync peers for this workspace, joining
/// sync_peers with the given contact manager for name/trust resolution.
/// Sorted by display_name ascending.
pub fn list_peers_info(
    &self,
    contact_manager: &crate::core::contact::ContactManager,
) -> crate::core::error::KrillnotesResult<Vec<PeerInfo>> {
    let conn = self.conn.lock().map_err(|_| crate::core::error::KrillnotesError::LockPoisoned)?;
    let registry = PeerRegistry::new(&conn);
    let peers = registry.list_peers()?;
    let contacts = contact_manager.list_contacts()?;

    let mut result: Vec<PeerInfo> = peers
        .into_iter()
        .map(|peer| {
            let contact = contacts
                .iter()
                .find(|c| c.public_key == peer.peer_identity_id);

            let display_name = contact
                .map(|c| c.local_name.clone().unwrap_or_else(|| c.declared_name.clone()))
                .unwrap_or_else(|| {
                    let key = &peer.peer_identity_id;
                    format!("{}…", &key[..key.len().min(8)])
                });

            // generate_fingerprint is fallible (base64 decode). Fall back to key prefix.
            let fingerprint = generate_fingerprint(&peer.peer_identity_id)
                .unwrap_or_else(|_| format!("{}…", &peer.peer_identity_id[..peer.peer_identity_id.len().min(8)]));

            // Use an explicit match rather than format!("{:?}") to guarantee the strings
            // match what the frontend expects ("Tofu", "CodeVerified", "Vouched", "VerifiedInPerson").
            let trust_level = contact.map(|c| match c.trust_level {
                TrustLevel::Tofu => "Tofu".to_string(),
                TrustLevel::CodeVerified => "CodeVerified".to_string(),
                TrustLevel::Vouched => "Vouched".to_string(),
                TrustLevel::VerifiedInPerson => "VerifiedInPerson".to_string(),
            });

            PeerInfo {
                peer_device_id: peer.peer_device_id,
                peer_identity_id: peer.peer_identity_id,
                display_name,
                fingerprint,
                trust_level,
                contact_id: contact.map(|c| c.contact_id.to_string()),
                last_sync: peer.last_sync,
            }
        })
        .collect();

    result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(result)
}

/// Pre-authorises a contact as a workspace sync peer before any .swarm exchange.
/// Uses `identity:<peer_identity_id>` as a placeholder device ID. This placeholder
/// is replaced with the real device ID when the peer's first .swarm update is imported.
pub fn add_contact_as_peer(
    &self,
    peer_identity_id: &str,
) -> crate::core::error::KrillnotesResult<()> {
    let placeholder_device_id = format!("identity:{}", peer_identity_id);
    let conn = self.conn.lock().map_err(|_| crate::core::error::KrillnotesError::LockPoisoned)?;
    let registry = PeerRegistry::new(&conn);
    registry.add_peer(&placeholder_device_id, peer_identity_id)
}

/// Removes a peer from this workspace's sync peer list by device ID.
/// The device ID may be a real device ID or a placeholder `identity:…` ID.
pub fn remove_peer(
    &self,
    peer_device_id: &str,
) -> crate::core::error::KrillnotesResult<()> {
    let conn = self.conn.lock().map_err(|_| crate::core::error::KrillnotesError::LockPoisoned)?;
    let registry = PeerRegistry::new(&conn);
    registry.remove_peer(peer_device_id)
}
```

**If the error type or lock pattern differs** from what's shown, adapt to match the existing patterns in `workspace.rs` (search for another method that locks `self.conn`).

- [ ] **Step 4: Run tests**

```bash
cargo test -p krillnotes-core test_list_peers_info 2>&1 | tail -15
cargo test -p krillnotes-core test_add_and_remove_peer 2>&1 | tail -10
```

Expected: all three tests PASS.

- [ ] **Step 5: Run full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -10
```

Expected: all tests pass, no regressions.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs krillnotes-core/src/core/peer_registry.rs
git commit -m "feat(core): add list_peers_info, add_contact_as_peer, remove_peer to Workspace"
```

---

### Task 4: Re-export `PeerInfo` from crate root

**Files:**
- Modify: `krillnotes-core/src/lib.rs`

- [ ] **Step 1: Add `PeerInfo` to the public re-exports**

Find the existing re-export block in `lib.rs` that re-exports items from `core`. Add `PeerInfo`:

```rust
pub use crate::core::peer_registry::PeerInfo;
```

Place it near the existing `SyncPeer` or other peer-registry re-exports.

- [ ] **Step 2: Verify**

```bash
cargo check -p krillnotes-core 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/lib.rs
git commit -m "feat(core): re-export PeerInfo from crate root"
```

---

## Chunk 2: Tauri Commands + TypeScript Types

### Files

- **Modify:** `krillnotes-desktop/src-tauri/src/lib.rs` — 3 new commands
- **Modify:** `krillnotes-desktop/src/types.ts` — add `PeerInfo` interface

---

### Task 5: Add Tauri commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Understand how window label → identity UUID is resolved**

`AppState` has **no** direct window-label → identity-UUID map. The pattern used throughout `lib.rs` (see `open_workspace_cmd` ~line 563) is:

1. Get the workspace folder path from `state.workspace_paths` by window label.
2. Call `read_info_json_full(&folder)` to extract the workspace UUID from `info.json`.
3. Call `identity_manager.get_workspace_binding(&workspace_uuid)` to get the identity UUID.

**Before implementing, check if `Workspace` already stores its identity UUID internally:**

```bash
grep -n "identity_uuid\|identity_id" krillnotes-core/src/core/workspace.rs | head -10
```

- If `Workspace` has a public `identity_uuid: Uuid` field or getter → use it directly (simplest path).
- If not → use the three-step pattern above, following `open_workspace_cmd` as a reference.

The implementation below uses the three-step pattern. If the Workspace already stores it, replace steps 1–3 with a direct field access.

- [ ] **Step 2: Add `list_workspace_peers` command**

Find where existing workspace commands are defined in `lib.rs` (search for `fn list_contacts`) and add the new commands nearby:

```rust
#[tauri::command]
async fn list_workspace_peers(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<krillnotes_core::PeerInfo>, String> {
    let window_label = window.label().to_string();

    // Resolve identity UUID from workspace binding (see open_workspace_cmd for full pattern).
    let identity_uuid = {
        let paths = state.workspace_paths.lock().map_err(|e| e.to_string())?;
        let folder = paths.get(&window_label).ok_or("Workspace path not found")?.clone();
        drop(paths);
        let (ws_uuid_opt, _, _, _) = read_info_json_full(&folder);
        let workspace_uuid = ws_uuid_opt.ok_or("Workspace UUID missing from info.json")?;
        let mgr = state.identity_manager.lock().map_err(|e| e.to_string())?;
        mgr.get_workspace_binding(&workspace_uuid)
            .map_err(|e| e.to_string())?
            .ok_or("No identity bound to this workspace")?
            .identity_uuid
        // mgr and paths both dropped before acquiring workspaces/contact_managers
    };

    // Acquire workspaces and contact_managers separately (never hold both simultaneously;
    // call list_peers_info with a reference, then drop both before returning).
    let peers = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        let contact_managers = state.contact_managers.lock().map_err(|e| e.to_string())?;
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found — identity must be unlocked")?;
        workspace.list_peers_info(cm).map_err(|e| e.to_string())?
        // both locks dropped here
    };
    Ok(peers)
}
```

- [ ] **Step 3: Add `remove_workspace_peer` command**

```rust
#[tauri::command]
async fn remove_workspace_peer(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    peer_device_id: String,
) -> Result<(), String> {
    let window_label = window.label().to_string();
    let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
    let workspace = workspaces
        .get(&window_label)
        .ok_or("Workspace not found")?;
    workspace.remove_peer(&peer_device_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Add `add_contact_as_peer` command**

```rust
#[tauri::command]
async fn add_contact_as_peer(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    identity_uuid: String,
    contact_id: String,
) -> Result<krillnotes_core::PeerInfo, String> {
    let window_label = window.label().to_string();
    let identity_uuid = identity_uuid
        .parse::<uuid::Uuid>()
        .map_err(|e| e.to_string())?;
    let contact_id = contact_id
        .parse::<uuid::Uuid>()
        .map_err(|e| e.to_string())?;

    // Step 1: Get contact's public key — hold contact_managers only, then drop.
    let peer_identity_id = {
        let contact_managers = state.contact_managers.lock().map_err(|e| e.to_string())?;
        let cm = contact_managers
            .get(&identity_uuid)
            .ok_or("Contact manager not found — identity must be unlocked")?;
        let contact = cm
            .get_contact(contact_id)
            .map_err(|e| e.to_string())?
            .ok_or("Contact not found")?;
        contact.public_key.clone()
        // contact_managers dropped here
    };

    // Step 2: Add to workspace — hold workspaces only, then drop.
    {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        let workspace = workspaces
            .get(&window_label)
            .ok_or("Workspace not found")?;
        workspace
            .add_contact_as_peer(&peer_identity_id)
            .map_err(|e| e.to_string())?;
        // workspaces dropped here
    }

    // Step 3: Build PeerInfo for the caller. Acquire each lock separately.
    // (Never hold workspaces and contact_managers simultaneously — see lock ordering comment
    // in AppState. Instead, collect what we need from contact_managers, drop it, then read
    // workspace peers.)
    let peers = {
        let workspaces = state.workspaces.lock().map_err(|e| e.to_string())?;
        let workspace = workspaces.get(&window_label).ok_or("Workspace not found")?;
        let contact_managers = state.contact_managers.lock().map_err(|e| e.to_string())?;
        let cm = contact_managers.get(&identity_uuid)
            .ok_or("Contact manager not found")?;
        workspace.list_peers_info(cm).map_err(|e| e.to_string())?
    };
    peers
        .into_iter()
        .find(|p| p.peer_identity_id == peer_identity_id)
        .ok_or_else(|| "Peer not found after insert".to_string())
}
```

- [ ] **Step 5: Register the three commands in `generate_handler!`**

Find the `tauri::generate_handler![...]` call and add:

```
list_workspace_peers,
remove_workspace_peer,
add_contact_as_peer,
```

- [ ] **Step 6: Build to verify**

```bash
cd krillnotes-desktop && cargo build -p krillnotes-desktop 2>&1 | grep -E "^error"
```

Expected: no errors. Fix any type mismatches against the actual `AppState` field names.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(tauri): add list_workspace_peers, remove_workspace_peer, add_contact_as_peer commands"
```

---

### Task 6: Add `PeerInfo` TypeScript interface

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Add `PeerInfo` interface**

Find the `ContactInfo` interface in `types.ts` and add `PeerInfo` immediately after it:

```typescript
export interface PeerInfo {
  peerDeviceId: string;
  peerIdentityId: string;
  displayName: string;
  fingerprint: string;
  trustLevel?: string;    // undefined if peer is not in the contact book
  contactId?: string;     // UUID string, undefined if not in contacts
  lastSync?: string;      // ISO 8601, undefined if never synced
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(frontend): add PeerInfo TypeScript interface"
```

---

## Chunk 3: Menu Wiring

### Files

- **Modify:** `krillnotes-desktop/src-tauri/src/menu.rs` — new menu item
- **Modify:** `krillnotes-desktop/src/i18n/locales/en.json` (and 6 other locale files) — new key
- **Modify:** `krillnotes-desktop/src/App.tsx` — menu event handler + dialog state

---

### Task 7: Add "Workspace Peers" menu item

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

- [ ] **Step 1: Understand the `menu.rs` locale pattern**

`menu.rs` does NOT use a typed struct for strings. `locales.rs` exposes `menu_strings(lang) -> serde_json::Value` and `menu.rs` accesses it via the helper `s(strings, "key", "fallback")`. Confirm this:

```bash
grep -n "^fn s\|s(strings" krillnotes-desktop/src-tauri/src/menu.rs | head -5
grep -n "workspace_properties" krillnotes-desktop/src-tauri/src/menu.rs | head -5
```

The pattern for the existing `workspace_properties` item will look like:
```rust
let workspace_properties_item = MenuItemBuilder::with_id("workspace_properties",
    s(strings, "workspaceProperties", "Workspace Properties…"))
    .enabled(false)
    .build(app)?;
```

- [ ] **Step 2: Add the new menu item using the same pattern**

In `menu.rs`, directly after the `workspace_properties` item definition, add:

```rust
let workspace_peers_item = MenuItemBuilder::with_id("workspace_peers",
    s(strings, "workspacePeers", "Workspace Peers"))
    .enabled(false)
    .build(app)?;
```

Then add it to the Edit menu `SubmenuBuilder` adjacent to `workspace_properties`:

```rust
.item(&workspace_properties_item)
.item(&workspace_peers_item)
```

(Use the actual variable names from the file — they may differ from `workspace_properties_item`.)

- [ ] **Step 3: Register the menu event in `MENU_MESSAGES`**

`lib.rs` contains a static `MENU_MESSAGES` lookup table that maps item IDs to event payload strings. Find it:

```bash
grep -n "MENU_MESSAGES\|workspace_properties" krillnotes-desktop/src-tauri/src/lib.rs | head -10
```

Add a new entry for the workspace peers item, following the same pattern as `workspace_properties`:

```rust
("workspace_peers", "Edit > Workspace Peers clicked"),
```

**This step is critical.** Without it, clicking the menu item emits no event and the dialog never opens — no compile error, just silent failure.

- [ ] **Step 4: Build to verify**

```bash
cd krillnotes-desktop && cargo build -p krillnotes-desktop 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(menu): add Workspace Peers menu item and MENU_MESSAGES entry"
```

---

### Task 8: Add locale strings for all 7 languages

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: `krillnotes-desktop/src/i18n/locales/de.json`
- Modify: `krillnotes-desktop/src/i18n/locales/es.json`
- Modify: `krillnotes-desktop/src/i18n/locales/fr.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ja.json`
- Modify: `krillnotes-desktop/src/i18n/locales/ko.json`
- Modify: `krillnotes-desktop/src/i18n/locales/zh.json`

- [ ] **Step 1: Add `workspacePeers` key to each locale file**

Find the `"workspaceProperties"` key in each file (they share the same JSON structure) and add `"workspacePeers"` next to it.

**en.json:** `"workspacePeers": "Workspace Peers"`
**de.json:** `"workspacePeers": "Arbeitsbereich-Peers"`
**es.json:** `"workspacePeers": "Pares del Espacio de Trabajo"`
**fr.json:** `"workspacePeers": "Pairs de l'espace de travail"`
**ja.json:** `"workspacePeers": "ワークスペースのピア"`
**ko.json:** `"workspacePeers": "워크스페이스 피어"`
**zh.json:** `"workspacePeers": "工作区对等节点"`

Also add the following UI-string keys used by the React components (add them in the `"ui"` or appropriate section, following the existing key naming convention):

**en.json additions:**
```json
"workspacePeers": "Workspace Peers",
"addFromContacts": "Add from contacts",
"createInviteFile": "Create invite file",
"notInContacts": "not in contacts",
"neverSynced": "never",
"removePeer": "Remove peer",
"addToContacts": "Add to contacts",
"confirmRemovePeer": "Remove this peer from the workspace?",
"noPeers": "No peers yet. Add a contact or create an invite file to get started."
```

Apply equivalent translations to all other locale files (machine translation is acceptable for these UI strings).

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/
git commit -m "feat(i18n): add Workspace Peers locale strings"
```

---

### Task 9: Wire menu event and dialog state in `App.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Add dialog state**

In `App.tsx`, find where the other dialog state variables are declared (e.g. `showContactBook`, `showOperationsLog`). Add:

```typescript
const [showWorkspacePeers, setShowWorkspacePeers] = useState(false);
```

- [ ] **Step 2: Determine how `identityUuid` is passed to dialogs in `App.tsx`**

The identity UUID must be passed as a prop to `WorkspacePeersDialog` so it can call `list_contacts` for the "Add from contacts" picker. The existing pattern in `App.tsx` is to pass it as a prop (e.g. `SwarmInviteDialog` receives `unlockedIdentityUuid`). Find the prop name:

```bash
grep -n "identityUuid\|unlockedIdentity\|identity_uuid" krillnotes-desktop/src/App.tsx | head -10
```

Note the state variable name that holds the currently active identity UUID. You will pass it as a prop to `WorkspacePeersDialog` in Step 4.

- [ ] **Step 3: Add menu handler**

Find the `handlers` object in `createMenuHandlers` (or equivalent function). Add:

```typescript
'Edit > Workspace Peers clicked': () => {
  setShowWorkspacePeers(true);
},
```

The key `'Edit > Workspace Peers clicked'` matches the string registered in `MENU_MESSAGES` in Task 7 Step 3. These must be identical.

- [ ] **Step 4: Render the dialog with identity UUID prop**

Find where other dialogs are conditionally rendered in the JSX. Add — passing the identity UUID from Step 2:

```tsx
{showWorkspacePeers && (
  <WorkspacePeersDialog
    identityUuid={unlockedIdentityUuid /* use actual state variable name from Step 2 */}
    onClose={() => setShowWorkspacePeers(false)}
  />
)}
```

- [ ] **Step 5: Add import**

```typescript
import WorkspacePeersDialog from './components/WorkspacePeersDialog';
```

- [ ] **Step 6: Skip TypeScript check until component exists**

Do not run `tsc --noEmit` here — the import will fail until Task 11 creates the component file. Continue to Task 10 (`AddContactDialog` prop).

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat(frontend): wire Workspace Peers menu event and dialog state"
```

---

## Chunk 4: React Components

### Files

- **Create:** `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`
- **Create:** `krillnotes-desktop/src/components/AddPeerFromContactsDialog.tsx`

---

### Task 10: Add `prefillPublicKey` prop to `AddContactDialog`

**Files:**
- Modify: `krillnotes-desktop/src/components/AddContactDialog.tsx`

`WorkspacePeersDialog` needs to open `AddContactDialog` pre-filled with an unknown peer's public key. The current `AddContactDialog` does not accept this prop — add it now as a prerequisite.

- [ ] **Step 1: Add `prefillPublicKey` to `AddContactDialog`**

In `AddContactDialog.tsx`, find the props interface (it currently has `identityUuid`, `onSaved`, `onClose`). Add the optional prop:

```typescript
interface AddContactDialogProps {
  identityUuid: string;
  prefillPublicKey?: string;   // ← add this
  onSaved: (contact: ContactInfo) => void;
  onClose: () => void;
}
```

Then find where the `publicKey` state is initialised (likely `useState('')`) and change it to use the prop:

```typescript
const [publicKey, setPublicKey] = useState(prefillPublicKey ?? '');
```

If the component uses `useEffect` to compute the fingerprint from `publicKey`, the pre-fill will automatically trigger the fingerprint preview.

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors (other than the missing `WorkspacePeersDialog` import from Task 9 Step 5).

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/AddContactDialog.tsx
git commit -m "feat(frontend): add prefillPublicKey prop to AddContactDialog"
```

---

### Task 11: Create `WorkspacePeersDialog`

**Files:**
- Create: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

Before implementing, read these files for patterns to follow:
- `krillnotes-desktop/src/components/ContactBookDialog.tsx` — dialog structure, trust badge, search pattern
- `krillnotes-desktop/src/components/EditContactDialog.tsx` — inline delete confirmation pattern

- [ ] **Step 1: Create `WorkspacePeersDialog.tsx`**

`identityUuid` is passed as a prop from `App.tsx` (same pattern as other dialogs — no Tauri command needed to retrieve it).

```tsx
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { PeerInfo } from '../types';
import AddPeerFromContactsDialog from './AddPeerFromContactsDialog';
import AddContactDialog from './AddContactDialog';

interface Props {
  identityUuid: string;   // passed from App.tsx — the workspace owner's identity UUID
  onClose: () => void;
}

// Maps trust level strings to badge CSS classes (reuse ContactBookDialog palette)
const TRUST_BADGE: Record<string, string> = {
  Tofu: 'bg-gray-500 text-white',
  CodeVerified: 'bg-blue-600 text-white',
  Vouched: 'bg-purple-600 text-white',
  VerifiedInPerson: 'bg-green-600 text-white',
};

export default function WorkspacePeersDialog({ identityUuid, onClose }: Props) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // peer_device_id of the peer pending removal confirmation (null = none)
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [showAddFromContacts, setShowAddFromContacts] = useState(false);
  // When set, open AddContactDialog pre-filled with this peer's public key
  const [addContactForPeer, setAddContactForPeer] = useState<PeerInfo | null>(null);

  const loadPeers = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<PeerInfo[]>('list_workspace_peers');
      setPeers(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPeers();
  }, [loadPeers]);

  const handleRemove = async (peer: PeerInfo) => {
    if (confirmRemoveId !== peer.peerDeviceId) {
      setConfirmRemoveId(peer.peerDeviceId);
      return;
    }
    try {
      await invoke('remove_workspace_peer', { peerDeviceId: peer.peerDeviceId });
      setConfirmRemoveId(null);
      await loadPeers();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleCreateInvite = async () => {
    // Reuse the existing invite creation logic from App.tsx.
    // BEFORE implementing this, find the 'File > Invite Peer clicked' handler in App.tsx
    // and copy its create_invite_bundle_cmd invocation verbatim here.
    // The parameters (workspaceId, workspaceName, sourceDeviceId, offeredRole, etc.)
    // come from app state already resolved in that handler.
    // This is an explicit implementation step — do not leave it as empty strings.
    try {
      // Copy from App.tsx 'File > Invite Peer clicked' handler
    } catch (e) {
      setError(String(e));
    }
  };

  const formatLastSync = (lastSync?: string) => {
    if (!lastSync) return t('neverSynced');
    const d = new Date(lastSync);
    const diff = Date.now() - d.getTime();
    const minutes = Math.floor(diff / 60000);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return d.toLocaleDateString();
  };

  return (
    <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/50">
      <div className="bg-background border border-border rounded-lg shadow-xl w-[520px] max-h-[600px] flex flex-col">

        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border">
          <div>
            <h2 className="text-lg font-semibold">{t('workspacePeers')}</h2>
            <p className="text-sm text-muted-foreground">
              {peers.length} {peers.length === 1 ? 'peer' : 'peers'}
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground text-xl leading-none"
          >
            ✕
          </button>
        </div>

        {/* Peer list */}
        <div className="flex-1 overflow-y-auto p-4 space-y-2">
          {loading && (
            <p className="text-sm text-muted-foreground text-center py-8">Loading…</p>
          )}
          {!loading && peers.length === 0 && (
            <p className="text-sm text-muted-foreground text-center py-8">
              {t('noPeers')}
            </p>
          )}
          {error && (
            <p className="text-sm text-red-500 p-2 rounded bg-red-50">{error}</p>
          )}
          {peers.map((peer) => (
            <div
              key={peer.peerDeviceId}
              className="flex items-center justify-between p-3 rounded-md border border-border bg-muted/30"
            >
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-sm truncate">{peer.displayName}</span>
                  {peer.trustLevel && (
                    <span className={`text-xs px-1.5 py-0.5 rounded font-medium ${TRUST_BADGE[peer.trustLevel] ?? 'bg-gray-400 text-white'}`}>
                      {peer.trustLevel}
                    </span>
                  )}
                  {!peer.trustLevel && (
                    <span className="text-xs text-muted-foreground italic">
                      {t('notInContacts')}
                    </span>
                  )}
                </div>
                <div className="text-xs text-muted-foreground font-mono mt-0.5">
                  {peer.fingerprint}
                </div>
                <div className="text-xs text-muted-foreground mt-0.5">
                  {formatLastSync(peer.lastSync)}
                </div>
              </div>

              <div className="flex items-center gap-1 ml-2 shrink-0">
                {/* Add to contacts — only for peers not in contact book */}
                {!peer.contactId && (
                  <button
                    title={t('addToContacts')}
                    onClick={() => setAddContactForPeer(peer)}
                    className="p-1.5 rounded hover:bg-muted text-blue-500 text-sm"
                  >
                    ＋
                  </button>
                )}

                {/* Remove / confirm remove */}
                {confirmRemoveId === peer.peerDeviceId ? (
                  <div className="flex items-center gap-1">
                    <span className="text-xs text-red-500">{t('confirmRemovePeer')}</span>
                    <button
                      onClick={() => handleRemove(peer)}
                      className="text-xs px-2 py-1 bg-red-500 text-white rounded hover:bg-red-600"
                    >
                      Remove
                    </button>
                    <button
                      onClick={() => setConfirmRemoveId(null)}
                      className="text-xs px-2 py-1 rounded hover:bg-muted"
                    >
                      Cancel
                    </button>
                  </div>
                ) : (
                  <button
                    title={t('removePeer')}
                    onClick={() => handleRemove(peer)}
                    className="p-1.5 rounded hover:bg-muted text-muted-foreground hover:text-red-500 text-sm"
                  >
                    🗑
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="flex items-center gap-2 p-4 border-t border-border">
          <button
            onClick={() => setShowAddFromContacts(true)}
            className="px-3 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
          >
            ＋ {t('addFromContacts')}
          </button>
          <button
            onClick={handleCreateInvite}
            className="px-3 py-2 text-sm rounded-md border border-border hover:bg-muted"
          >
            📨 {t('createInviteFile')}
          </button>
        </div>
      </div>

      {/* Sub-dialogs */}
      {showAddFromContacts && (
        <AddPeerFromContactsDialog
          identityUuid={identityUuid}
          currentPeers={peers}
          onAdded={async () => {
            setShowAddFromContacts(false);
            await loadPeers();
          }}
          onClose={() => setShowAddFromContacts(false)}
        />
      )}

      {addContactForPeer && (
        <AddContactDialog
          identityUuid={identityUuid}
          prefillPublicKey={addContactForPeer.peerIdentityId}
          onSaved={() => {
            setAddContactForPeer(null);
            loadPeers();
          }}
          onClose={() => setAddContactForPeer(null)}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -30
```

Fix any type errors. Expected: no errors once `AddPeerFromContactsDialog` is created in Task 12.

- [ ] **Step 3: Commit** (after Task 12 passes TypeScript)

Hold commit until after Task 12.

---

### Task 12: Create `AddPeerFromContactsDialog`

**Files:**
- Create: `krillnotes-desktop/src/components/AddPeerFromContactsDialog.tsx`

- [ ] **Step 1: Create the component**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ContactInfo, PeerInfo } from '../types';

interface Props {
  identityUuid: string;
  currentPeers: PeerInfo[];
  onAdded: () => void;
  onClose: () => void;
}

export default function AddPeerFromContactsDialog({
  identityUuid,
  currentPeers,
  onAdded,
  onClose,
}: Props) {
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [selected, setSelected] = useState<ContactInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!identityUuid) return;
    setLoading(true);
    invoke<ContactInfo[]>('list_contacts', { identityUuid })
      .then((all) => {
        // Filter out contacts already in the peer list
        const peerKeys = new Set(currentPeers.map((p) => p.peerIdentityId));
        setContacts(all.filter((c) => !peerKeys.has(c.publicKey)));
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [identityUuid, currentPeers]);

  const handleSave = async () => {
    if (!selected) return;
    setSaving(true);
    setError(null);
    try {
      await invoke('add_contact_as_peer', {
        identityUuid,
        contactId: selected.contactId,
      });
      onAdded();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  const displayName = (c: ContactInfo) =>
    c.localName ?? c.declaredName;

  return (
    <div className="fixed inset-0 z-70 flex items-center justify-center bg-black/50">
      <div className="bg-background border border-border rounded-lg shadow-xl w-[400px] max-h-[480px] flex flex-col">

        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-base font-semibold">Add contact as peer</h2>
          <button onClick={onClose} className="text-muted-foreground hover:text-foreground">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-1.5">
          {loading && (
            <p className="text-sm text-muted-foreground text-center py-6">Loading contacts…</p>
          )}
          {!loading && contacts.length === 0 && (
            <p className="text-sm text-muted-foreground text-center py-6">
              All contacts are already peers, or you have no contacts.
            </p>
          )}
          {error && (
            <p className="text-sm text-red-500">{error}</p>
          )}
          {contacts.map((c) => (
            <button
              key={c.contactId}
              onClick={() => setSelected(selected?.contactId === c.contactId ? null : c)}
              className={`w-full text-left p-3 rounded-md border transition-colors ${
                selected?.contactId === c.contactId
                  ? 'border-primary bg-primary/10'
                  : 'border-border hover:bg-muted/50'
              }`}
            >
              <div className="text-sm font-medium">{displayName(c)}</div>
              <div className="text-xs text-muted-foreground font-mono mt-0.5">{c.fingerprint}</div>
            </button>
          ))}
        </div>

        <div className="flex items-center justify-end gap-2 p-4 border-t border-border">
          <button
            onClick={onClose}
            className="px-3 py-2 text-sm rounded-md border border-border hover:bg-muted"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!selected || saving}
            className="px-3 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50"
          >
            {saving ? 'Adding…' : 'Add as peer'}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit both components**

```bash
git add krillnotes-desktop/src/components/WorkspacePeersDialog.tsx \
        krillnotes-desktop/src/components/AddPeerFromContactsDialog.tsx
git commit -m "feat(frontend): add WorkspacePeersDialog and AddPeerFromContactsDialog"
```

---

### Task 13: Smoke test

- [ ] **Step 1: Run dev server**

```bash
cd krillnotes-desktop && npm run tauri dev
```

- [ ] **Step 2: Manual verification checklist**

- [ ] "Workspace Peers" appears in the Edit menu when a workspace is open
- [ ] Clicking it opens `WorkspacePeersDialog`
- [ ] Existing peers show with resolved name, fingerprint, last sync
- [ ] Peers in contact book show trust level badge; unknown peers show "not in contacts" + ＋ button
- [ ] Clicking trash icon shows inline confirmation; confirming removes the peer; list refreshes
- [ ] "Add from contacts" opens picker; contacts already in peer list are absent; selecting one and saving adds it
- [ ] Newly added contact peer appears with placeholder device ID (resolved name from contact book)
- [ ] "Create invite file" opens file-save dialog and creates the `.swarm` file (uses existing mechanism)
- [ ] Clicking ＋ on unknown peer opens `AddContactDialog` pre-filled with the peer's public key
- [ ] Closing the dialog and reopening shows current state

- [ ] **Step 3: Commit any fixups**

```bash
git add -p
git commit -m "fix: workspace peers smoke test fixups"
```

---

## Summary of Files Changed

| File | Change |
|---|---|
| `krillnotes-core/src/core/peer_registry.rs` | Add `PeerInfo` struct |
| `krillnotes-core/src/core/workspace.rs` | Add `list_peers_info`, `add_contact_as_peer`, `remove_peer` + tests |
| `krillnotes-core/src/lib.rs` | Re-export `PeerInfo` |
| `krillnotes-desktop/src-tauri/src/lib.rs` | 3 new Tauri commands + new `MENU_MESSAGES` entry for workspace_peers |
| `krillnotes-desktop/src-tauri/src/menu.rs` | New "Workspace Peers" menu item (using `s(strings, …)` pattern) |
| `krillnotes-desktop/src/types.ts` | Add `PeerInfo` interface |
| `krillnotes-desktop/src/i18n/locales/*.json` (7 files) | New locale keys |
| `krillnotes-desktop/src/App.tsx` | Menu handler + dialog state + identityUuid prop |
| `krillnotes-desktop/src/components/AddContactDialog.tsx` | Add `prefillPublicKey?: string` prop |
| `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` | New component |
| `krillnotes-desktop/src/components/AddPeerFromContactsDialog.tsx` | New component |
