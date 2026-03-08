# Swarm UI Design — .swarm File Picker & Invite Flows

**Date:** 2026-03-08
**Status:** Approved
**Builds on:** WP-A (`feat/swarm-wp-a`) — peer model + bundle codec

---

## Goal

Expose the six swarm sync activities to the user via native menu items, modal dialogs, and OS file association for `.swarm` files.

---

## Six User Activities

| # | Activity | Bundle involved |
|---|----------|----------------|
| a | Invite unknown contact to workspace | Creates `invite.swarm` |
| b | Invite known contact to workspace | Creates `invite.swarm` |
| c | Accept invite from unknown contact | Creates `accept.swarm` |
| d | Accept invite from known contact | Creates `accept.swarm` |
| e | Process accepted invite, send initial snapshot | Creates `snapshot.swarm` |
| f | Receive initial snapshot, create local workspace | Reads `snapshot.swarm` |

---

## Entry Points

### Native Menu

Two items added to the existing **Workspace** menu:

```
Workspace
  ├── New workspace
  ├── Open workspace
  ├── ─────────────────────
  ├── Invite peer…          ← new
  ├── Open .swarm file…     ← new
  └── …
```

### OS File Association

`.swarm` extension registered in `tauri.conf.json`. Double-clicking a `.swarm` file emits `file-opened` with the path. The existing stub in `handle_file_opened`:

```rust
// future: Some("swarm") => handle_swarm_open(app, state, path),
```

…is filled in to call `handle_swarm_open`, which stores the path in `AppState` and emits an event to the frontend — same pattern as `.krillnotes` file opening.

---

## Dispatch Logic (Rust)

### `open_swarm_file(path) -> SwarmFileInfo`

Reads `header.json` from the zip, validates the bundle signature, and returns a typed enum to the frontend:

```rust
#[serde(tag = "mode", rename_all = "camelCase")]
enum SwarmFileInfo {
    Invite {
        workspace_name: String,
        offered_role: String,
        offered_scope: Option<String>,
        inviter_display_name: String,
        inviter_fingerprint: String,
        pairing_token: String,
    },
    Accept {
        workspace_name: String,
        declared_name: String,
        acceptor_fingerprint: String,
        acceptor_public_key: String,
        pairing_token: String,
    },
    Snapshot {
        workspace_name: String,
        sender_display_name: String,
        sender_fingerprint: String,
        as_of_operation_id: String,
    },
    Delta { /* stub — WP-C */ },
}
```

The frontend keeps the file path in state for follow-up commands.

### Follow-up Commands

| Command | Input | Output |
|---------|-------|--------|
| `create_accept_bundle_cmd(invite_path, declared_name, identity_uuid, passphrase)` | Path to `invite.swarm` | Bytes written to user-chosen path |
| `create_snapshot_bundle_cmd(window, accept_path, identity_uuid, passphrase)` | Path to `accept.swarm` | Bytes written to user-chosen path |
| `create_workspace_from_snapshot_cmd(snapshot_path, workspace_name, identity_uuid, passphrase)` | Path to `snapshot.swarm` | New workspace label, opens in new window |

---

## React Components

### `SwarmInviteDialog.tsx` — "Invite peer…"

Triggered by the **Invite peer…** menu item.

**Fields:**
- Contact selector: dropdown of existing contacts **or** manual entry (display name + Ed25519 public key)
- Role: Owner / Writer / Reader (select)
- Scope: "Whole workspace" only for now (subtree scope deferred to WP-B)

**Flow:**
1. Identity unlock gate (inline `UnlockIdentityDialog` if locked)
2. Fill form → "Create invite file…" → `tauri-plugin-dialog` save-file picker (`.swarm` filter)
3. Calls `create_invite_bundle_cmd` → writes file
4. Shows success: "Invite saved. Send this file to [name]."
5. New TOFU contact created if manual entry was used

---

### `SwarmOpenDialog.tsx` — "Open .swarm file…" + file association

Triggered by:
- **Open .swarm file…** menu item → opens file picker first, then displays dialog
- OS double-click → `file-opened` event → dialog opens directly with the path

Calls `open_swarm_file(path)` on mount, then renders based on mode:

#### Mode: `invite`
| Element | Detail |
|---------|--------|
| Heading | "Workspace invitation" |
| Info | Workspace name, inviter name, fingerprint (4-word BIP-39), offered role |
| Action | "Accept — save reply…" → save-file picker → calls `create_accept_bundle_cmd` |
| Post-action | "Reply saved. Send this file back to [inviter name]." New TOFU contact created if unknown. |

#### Mode: `accept`
| Element | Detail |
|---------|--------|
| Heading | "[Name] has accepted your invitation" |
| Info | Acceptor name, fingerprint to verify, workspace name |
| Verify prompt | "Confirm their fingerprint matches before sending" |
| Action | "Send snapshot…" → save-file picker → calls `create_snapshot_bundle_cmd` |
| Post-action | "Snapshot saved. Send this file to [name]." Peer added to workspace's `sync_peers` table and contacts. |

#### Mode: `snapshot`
| Element | Detail |
|---------|--------|
| Heading | "Workspace snapshot from [sender]" |
| Info | Sender name + fingerprint |
| Workspace name | Editable text field, defaults to `workspaceName` from bundle header |
| Action | "Create workspace" → calls `create_workspace_from_snapshot_cmd` → opens new workspace window |

#### Mode: `delta`
Stub: "Delta sync is not yet supported in this version."

---

## Identity Unlock Gate

Any action requiring the signing key checks if the identity is already unlocked in `AppState`. If not, the dialog renders the unlock form inline (same pattern as `UnlockIdentityDialog`) before proceeding. If the user cancels, the flow aborts silently.

---

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| Corrupt / invalid `.swarm` file | Inline error: "This file could not be read — it may be corrupt or from an incompatible version." |
| Signature verification failure | Same error — never proceed with unverified bundle |
| Pairing token mismatch (accept) | Warning: "This accept doesn't match a known invite. Only proceed if you trust the sender." (WP-B will enforce strictly) |
| Workspace name collision on snapshot import | Auto-append ` (2)`, ` (3)` etc. |
| No identity configured | "You need to create an identity before using sync features." + link to identity manager |

---

## Workspace Creation (from snapshot)

- **No location picker** — workspace directory is settings-managed
- Identity must be unlocked (or passphrase entered)
- Workspace name defaults to `workspaceName` from snapshot header, user can edit
- New workspace is a fresh encrypted SQLite file populated from snapshot content
- Opens in a new Tauri window on success

---

## Out of Scope (deferred)

- Subtree scope in invite (WP-B)
- Delta sync UI (WP-C)
- Peer list / sync status management
- QR code for key exchange
