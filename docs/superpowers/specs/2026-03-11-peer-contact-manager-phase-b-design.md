# Peer Contact Manager — Phase B Design Spec
*Date: 2026-03-11*

---

## Overview

Phase B adds a **Workspace Peers** dialog — a dedicated UI for viewing and managing the sync peer list for a workspace. It surfaces the existing `sync_peers` table with resolved contact names, fingerprints, and trust levels, and provides actions to remove peers, pre-authorise known contacts as peers, and create invite files for unknown contacts.

This spec follows Phase A (encrypted contact book UI, PR #91).

---

## Scope

**In scope:**
- "Workspace Peers" app menu item → `WorkspacePeersDialog`
- View peers: resolved display name, fingerprint, trust level, last sync
- Remove peer from workspace
- Add known contact as peer (pre-authorise by identity key)
- Create invite file (surfaces existing `.swarm` invite mechanism)
- "Add to contacts" shortcut for peers not yet in the contact book

**Out of scope:**
- Snapshot file creation for known contacts (Phase C)
- Peer device ID resolution from signed operations
- Live sync status / connection state (not applicable — sync is file-based)

---

## Architecture Principle

All resolution and mutation logic lives in `krillnotes-core`. Tauri commands are thin wrappers. This ensures future frontends (mobile, web, headless) share the same logic without duplication.

---

## Core Data Layer (`krillnotes-core`)

### New `PeerInfo` struct — `peer_registry.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub peer_device_id: String,
    pub peer_identity_id: String,    // Ed25519 public key
    pub display_name: String,        // local_name || declared_name || 8-char key prefix
    pub fingerprint: String,         // 4 BIP-39 words (BLAKE3)
    pub trust_level: Option<String>, // None if peer not in contact book
    pub contact_id: Option<String>,  // Some(uuid) if in contact book
    pub last_sync: Option<String>,   // ISO 8601, None if never synced
}
```

### New methods on `Workspace` — `workspace.rs`

`Workspace` is the primary API. All peer logic is added here, keeping Tauri commands thin.

| Method | Signature | Notes |
|---|---|---|
| `list_peers_info` | `(&self, cm: &ContactManager) -> Result<Vec<PeerInfo>>` | Joins `sync_peers` + contact book; calls `generate_fingerprint` per row; sorts by display name |
| `add_contact_as_peer` | `(&self, peer_identity_id: &str) -> Result<()>` | Inserts into `sync_peers`; uses `identity:<peer_identity_id>` as placeholder `peer_device_id` until real device ID arrives on first sync |
| `remove_peer` | `(&self, peer_device_id: &str) -> Result<()>` | Delegates to `PeerRegistry::remove_peer` |

**Placeholder device ID convention:** When pre-authorising a contact before any sync has occurred, the device ID is unknown. Use `identity:<peer_identity_id>` as a placeholder primary key. When the peer's first `.swarm` update is imported, upsert with the real device ID and remove the placeholder row.

---

## Tauri Commands (`src-tauri/src/lib.rs`)

Three new commands — all thin wrappers, no business logic:

| Command | Parameters | Returns |
|---|---|---|
| `list_workspace_peers` | `window: Window, state: State<AppState>` | `Result<Vec<PeerInfo>, String>` |
| `remove_workspace_peer` | `window, state, peer_device_id: String` | `Result<(), String>` |
| `add_contact_as_peer` | `window, state, identity_uuid: String, contact_id: String` | `Result<PeerInfo, String>` |

`add_contact_as_peer`: looks up contact by `contact_id` from the identity's `ContactManager` to retrieve `peer_identity_id`, calls `Workspace::add_contact_as_peer`, then returns a fresh `PeerInfo` for the new row.

The existing invite command (`.swarm` invite file creation) is reused as-is — no new command needed.

---

## TypeScript Types (`types.ts`)

```typescript
export interface PeerInfo {
  peerDeviceId: string;
  peerIdentityId: string;
  displayName: string;
  fingerprint: string;
  trustLevel?: string;
  contactId?: string;
  lastSync?: string;       // ISO 8601
}
```

---

## React Components

### `WorkspacePeersDialog.tsx`

Props: `onClose: () => void`

Identity UUID and workspace context come from the window — already available via existing app state pattern.

**Layout (footer-action pattern, matching existing dialogs):**

```
┌─────────────────────────────────────────────┐
│ Workspace Peers                         [✕] │
│ My Workspace · 3 peers                      │
├─────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────┐ │
│ │ Alice                              [🗑] │ │
│ │ apple-banana-cloud-delta               │ │
│ │ Code Verified · 5 minutes ago          │ │
│ └─────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────┐ │
│ │ a1b2c3d4…                     [＋] [🗑] │ │
│ │ echo-foxtrot-green-hotel               │ │
│ │ not in contacts · never                │ │
│ └─────────────────────────────────────────┘ │
├─────────────────────────────────────────────┤
│ [Add from contacts]    [Create invite file] │
└─────────────────────────────────────────────┘
```

**Per-row details:**
- Display name: `localName || declaredName || peerIdentityId.slice(0, 8) + '…'`
- Fingerprint: 4 words, muted monospace (reuse style from `ContactBookDialog`)
- Trust level badge: reuse colour coding from `ContactBookDialog` (grey/blue/purple/green); omitted if `trustLevel` is null
- Last sync: human-readable relative time or "never"
- **Remove:** trash icon, inline confirmation (same pattern as `EditContactDialog` delete)
- **Add to contacts** (unknown peers only): `＋` icon → opens `AddContactDialog` pre-filled with `peerIdentityId` as public key

**Footer:**
- "Add from contacts" → opens `AddPeerFromContactsDialog`
- "Create invite file" → invokes existing invite command

### `AddPeerFromContactsDialog.tsx`

A filtered contact picker for the current workspace's identity.

- Fetches `list_contacts` and `list_workspace_peers` in parallel
- Filters out contacts whose `publicKey` already appears in `peerIdentityId` of any peer
- Renders filtered list with name + fingerprint
- Single-click to select, Save calls `add_contact_as_peer`
- Cancel closes without action

### Menu wiring

- `menu.rs`: new item `"workspace_peers"` in the Workspace menu, alongside existing workspace settings item
- `locales/*.json`: new key `workspace_peers` in all 7 languages (English: "Workspace Peers")
- `App.tsx`: listen for `workspace_peers` menu event, toggle `WorkspacePeersDialog` visibility — same pattern as all other menu-driven dialogs

---

## Key Decisions Log

| Decision | Choice | Rationale |
|---|---|---|
| Where peers UI lives | Dedicated dialog via menu item | Matches user expectation; feature may grow complex |
| Resolution location | Server-side in `krillnotes-core` | Multi-frontend architecture; keeps Tauri thin |
| Layout pattern | Footer actions, icon buttons per row | Matches existing dialog design (ContactBookDialog, etc.) |
| Placeholder device ID | `identity:<peer_identity_id>` prefix | Allows pre-authorisation before first sync; upserted on real sync |
| Invite file creation | Reuse existing command | No new format needed; surface from dialog only |
