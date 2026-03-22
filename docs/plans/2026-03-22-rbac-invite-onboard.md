# RBAC Plan C: Invite-to-Subtree + Onboarding — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add subtree-scoped invites and a post-accept onboarding flow so that workspace owners (and subtree owners) can invite peers to specific subtrees, assign roles on accept, and send a snapshot — completing the RBAC invite lifecycle.

**Architecture:** Wire format changes add `scope_note_id` + `scope_note_title` to `InviteRecord`, `InviteFile`, and `ReceivedResponse`. A new `PermissionPending` status tracks accepted-but-not-yet-onboarded peers. The frontend gets an `OnboardPeerDialog` (role picker + extracted `ChannelPicker`) and context menu integration. The snapshot sent is always the full workspace — RBAC controls what the peer can *do*, not what data they have.

**Tech Stack:** Rust, rusqlite, serde, ed25519-dalek, Tauri v2, React 19, TypeScript, Tailwind v4, i18next

**Spec:** `docs/plans/2026-03-22-rbac-ui-design.md` § 4 (Invite-to-Subtree and Post-Accept Onboarding)

**Depends on:** PR #108 (Plan A — backend permission queries)

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `krillnotes-desktop/src/components/OnboardPeerDialog.tsx` | Post-accept dialog: peer card, scope reminder, role picker, channel picker, Grant & sync / Later / Reject |
| `krillnotes-desktop/src/components/ChannelPicker.tsx` | Extracted channel-type selector + relay/folder sub-UI, reused by OnboardPeerDialog and WorkspacePeersDialog |

### Modified files

| File | Change |
|------|--------|
| `krillnotes-core/src/core/invite.rs` | Add `scope_note_id` + `scope_note_title` to `InviteRecord` and `InviteFile`; update `create_invite()` signature |
| `krillnotes-core/src/core/received_response.rs` | Add `scope_note_id` + `scope_note_title` to `ReceivedResponse`; add `PermissionPending` variant |
| `krillnotes-desktop/src-tauri/src/commands/invites.rs` | Add `scope_note_id` param to `create_invite`; look up note title; propagate scope on `import_invite_response`; update `save_invite_file` InviteFile construction; add `InviteInfo.scope_note_id/title` |
| `krillnotes-desktop/src-tauri/src/commands/sync.rs` | Add `scope_note_id` param to `share_invite_link`; update `create_relay_invite` InviteFile construction |
| `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs` | Propagate scope from invite to response in `poll_receive_workspace`; add `PermissionPending` arm to `update_response_status`; add scope fields + `permissionPending` to `ReceivedResponseInfo` From impl |
| `krillnotes-desktop/src/types.ts` | Add scope fields to `InviteInfo` and `ReceivedResponseInfo`; add `"permissionPending"` status |
| `krillnotes-desktop/src/components/CreateInviteDialog.tsx` | Accept optional `scopeNoteId` + `scopeNoteTitle` props; show scope badge; pass to `create_invite` command |
| `krillnotes-desktop/src/components/InviteManagerDialog.tsx` | Accept optional `initialScope` prop; show scope per invite; wire OnboardPeerDialog for scoped responses |
| `krillnotes-desktop/src/components/PendingResponsesSection.tsx` | Show `permissionPending` state with [Onboard] button; show scope badge |
| `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` | Replace inline channel picker with `ChannelPicker` component |
| `krillnotes-desktop/src/components/ContextMenu.tsx` | Add `effectiveRole` + `onInviteToSubtree` props; render "Invite to this subtree…" for Owner+ |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Wire context menu invite action → open InviteManagerDialog with scope |
| `krillnotes-desktop/src/i18n/locales/en.json` | Add all new i18n keys |

---

### Task 1: Add scope fields to `InviteRecord` and `InviteFile`

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs`

**Critical:** New fields on `InviteFile` MUST use `#[serde(default, skip_serializing_if = "Option::is_none")]`. The signing system (`sign_payload` / `verify_payload`) re-serializes to `serde_json::Value` then canonical JSON. If `None` fields serialize as `null`, old invite files (which lack the field entirely) will produce a different canonical form and signature verification will break.

- [ ] **Step 1: Add scope fields to `InviteRecord`**

Add two fields after `relay_url`:

```rust
// In InviteRecord:
#[serde(default)]
pub scope_note_id: Option<String>,
#[serde(default)]
pub scope_note_title: Option<String>,
```

- [ ] **Step 2: Add scope fields to `InviteFile`**

Add two fields (before `signature`):

```rust
// In InviteFile:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub scope_note_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub scope_note_title: Option<String>,
```

- [ ] **Step 3: Update `InviteManager::create_invite()` signature**

Add `scope_note_id: Option<String>` and `scope_note_title: Option<String>` parameters. Set them on both `InviteRecord` and `InviteFile` during construction.

Find the `InviteRecord { ... }` and `InviteFile { ... }` struct literals inside `create_invite()` (around lines 231-260) and add the new fields.

- [ ] **Step 4: Write tests for scope on invite creation and signing round-trip**

Add to the existing test module in `invite.rs`:

```rust
#[test]
fn create_invite_with_scope() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf());
    let key = test_key();
    let (record, file) = mgr.create_invite(
        "ws-1", "Test WS", Some(7), &key, "Alice",
        None, None, None, None, None, vec![],
        Some("note-42".to_string()),
        Some("My Subtree".to_string()),
    ).unwrap();
    assert_eq!(record.scope_note_id.as_deref(), Some("note-42"));
    assert_eq!(record.scope_note_title.as_deref(), Some("My Subtree"));
    assert_eq!(file.scope_note_id.as_deref(), Some("note-42"));
    assert_eq!(file.scope_note_title.as_deref(), Some("My Subtree"));
}

#[test]
fn create_invite_without_scope_is_backward_compat() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf());
    let key = test_key();
    let (record, file) = mgr.create_invite(
        "ws-1", "Test WS", None, &key, "Alice",
        None, None, None, None, None, vec![],
        None, None,
    ).unwrap();
    assert!(record.scope_note_id.is_none());
    assert!(file.scope_note_id.is_none());
}

#[test]
fn scopeless_invite_signature_still_verifies() {
    // Simulate an old invite file (no scope fields in JSON).
    let key = test_key();
    let pubkey_b64 = STANDARD.encode(key.verifying_key().to_bytes());
    let old_json = serde_json::json!({
        "file_type": "krillnotes-invite-v1",
        "invite_id": "abc",
        "workspace_id": "ws-1",
        "workspace_name": "Test",
        "inviter_public_key": pubkey_b64,
        "inviter_declared_name": "Alice",
        "expires_at": null,
        "signature": ""
    });
    // Sign the old-format JSON
    let sig = sign_payload(&old_json, &key);
    let mut signed = old_json.clone();
    signed["signature"] = serde_json::Value::String(sig.clone());

    // Deserialize into new InviteFile (scope defaults to None)
    let file: InviteFile = serde_json::from_value(signed).unwrap();
    assert!(file.scope_note_id.is_none());

    // Re-serialize and verify — must still pass
    let re_serialized = serde_json::to_value(&file).unwrap();
    verify_payload(&re_serialized, &file.signature, &pubkey_b64).unwrap();
}
```

Note: The `test_key()` helper already exists in the test module. There are ~11 existing `mgr.create_invite(...)` calls in the test module (lines ~495-639) that need two extra `None, None` arguments appended for the new scope parameters. Update ALL existing callers before running tests — the compiler will flag them.

- [ ] **Step 5: Run tests**

Run: `cargo test -p krillnotes-core -- invite`

Expected: All invite tests pass including the 3 new ones.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "feat(core): add scope_note_id + scope_note_title to InviteRecord and InviteFile"
```

---

### Task 2: Add scope fields to `ReceivedResponse` + `PermissionPending` status

**Files:**
- Modify: `krillnotes-core/src/core/received_response.rs`

- [ ] **Step 1: Add `PermissionPending` variant to `ReceivedResponseStatus`**

Insert between `PeerAdded` and `SnapshotSent`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceivedResponseStatus {
    Pending,
    PeerAdded,
    PermissionPending,  // ← NEW: peer accepted, awaiting role grant
    SnapshotSent,
}
```

- [ ] **Step 2: Add scope fields to `ReceivedResponse`**

```rust
// In ReceivedResponse:
#[serde(default)]
pub scope_note_id: Option<String>,
#[serde(default)]
pub scope_note_title: Option<String>,
```

- [ ] **Step 3: Update `ReceivedResponseManager` (if applicable)**

Check the constructor or builder method for `ReceivedResponse` in `received_response.rs`. If there's a `new()` or builder, add `scope_note_id` and `scope_note_title` parameters. If responses are constructed inline at call sites, this step is done — the call sites are updated in Task 4.

- [ ] **Step 4: Write test for PermissionPending serialization round-trip**

```rust
#[test]
fn permission_pending_status_serializes() {
    let status = ReceivedResponseStatus::PermissionPending;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"permissionPending\"");
    let back: ReceivedResponseStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ReceivedResponseStatus::PermissionPending);
}

#[test]
fn received_response_with_scope_round_trips() {
    let resp = ReceivedResponse {
        response_id: uuid::Uuid::new_v4(),
        invite_id: uuid::Uuid::new_v4(),
        workspace_id: "ws-1".into(),
        workspace_name: "Test".into(),
        invitee_public_key: "key".into(),
        invitee_declared_name: "Bob".into(),
        received_at: chrono::Utc::now(),
        status: ReceivedResponseStatus::PermissionPending,
        scope_note_id: Some("note-42".into()),
        scope_note_title: Some("My Subtree".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let back: ReceivedResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.scope_note_id.as_deref(), Some("note-42"));
    assert_eq!(back.status, ReceivedResponseStatus::PermissionPending);
}

#[test]
fn old_response_without_scope_deserializes() {
    // ReceivedResponse uses #[serde(rename_all = "camelCase")] — keys must be camelCase
    let json = r#"{
        "responseId": "00000000-0000-0000-0000-000000000001",
        "inviteId": "00000000-0000-0000-0000-000000000002",
        "workspaceId": "ws-1",
        "workspaceName": "Test",
        "inviteePublicKey": "key",
        "inviteeDeclaredName": "Bob",
        "receivedAt": "2026-03-22T00:00:00Z",
        "status": "pending"
    }"#;
    let resp: ReceivedResponse = serde_json::from_str(json).unwrap();
    assert!(resp.scope_note_id.is_none());
    assert_eq!(resp.status, ReceivedResponseStatus::Pending);
}
```

Note: Adapt field names to match the struct's actual serde attributes (check whether `ReceivedResponse` uses `rename_all = "camelCase"` — if so, use camelCase keys in the JSON literal test).

- [ ] **Step 5: Run tests**

Run: `cargo test -p krillnotes-core -- received_response`

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/received_response.rs
git commit -m "feat(core): add PermissionPending status and scope fields to ReceivedResponse"
```

---

### Task 3: Update Tauri invite commands with scope

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/invites.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs`

- [ ] **Step 1: Add scope fields to `InviteInfo` struct**

In `invites.rs` (line 14-23), add:

```rust
pub struct InviteInfo {
    // ... existing fields ...
    pub scope_note_id: Option<String>,
    pub scope_note_title: Option<String>,
}
```

Also update the mapping from `InviteRecord` → `InviteInfo` (in `list_invites` and `create_invite`) to include the new fields.

- [ ] **Step 2: Add `scope_note_id` parameter to `create_invite` command**

Update the signature at line ~92:

```rust
pub fn create_invite(
    window: tauri::Window,
    state: State<'_, AppState>,
    identity_uuid: String,
    workspace_name: String,
    expires_in_days: Option<u32>,
    save_path: String,
    scope_note_id: Option<String>,  // ← NEW
) -> std::result::Result<InviteInfo, String>
```

When `scope_note_id` is `Some`, look up the note title from the workspace:

```rust
let scope_note_title = if let Some(ref nid) = scope_note_id {
    let workspaces = state.workspaces.lock().unwrap();
    let ws = workspaces.get(window.label())
        .ok_or("No workspace for this window")?;
    Some(ws.get_note(nid).map_err(|e| e.to_string())?.title)
} else {
    None
};
```

Pass `scope_note_id` and `scope_note_title` to `im.create_invite(...)`.

- [ ] **Step 3: Add `scope_note_id` parameter to `share_invite_link` command**

In `sync.rs` (line ~222), add `scope_note_id: Option<String>` parameter. Same title lookup pattern as Step 2. Pass through to `im.create_invite(...)`.

- [ ] **Step 4: Update `save_invite_file` to include scope fields**

In `invites.rs`, `save_invite_file` (line ~163) constructs an `InviteFile` struct literal. Add the scope fields from the invite record:

```rust
scope_note_id: record.scope_note_id.clone(),
scope_note_title: record.scope_note_title.clone(),
```

- [ ] **Step 5: Update `create_relay_invite` to include scope fields**

In `sync.rs`, `create_relay_invite` (line ~350) also constructs an `InviteFile` struct literal. Same fix as Step 4 — add scope fields from the invite record.

- [ ] **Step 6: Propagate scope on `import_invite_response`**

In `invites.rs` `import_invite_response` (line ~286): after looking up the matching `InviteRecord` by `invite_id`, copy `scope_note_id` and `scope_note_title` from the invite record onto the `ReceivedResponse` being created.

Find where `ReceivedResponse` is constructed (or `ReceivedResponse::new()` is called) and add:

```rust
scope_note_id: invite_record.scope_note_id.clone(),
scope_note_title: invite_record.scope_note_title.clone(),
```

If `ReceivedResponse::new()` is used, either add scope parameters to `new()` or set the fields after construction.

- [ ] **Step 7: Check `fetch_relay_invite_response` for scope propagation**

In `sync.rs`, `fetch_relay_invite_response` (line ~678) returns a `PendingPeer`, NOT a `ReceivedResponse`. It does not create a response record. No scope propagation is needed here — the `ReceivedResponse` for relay-fetched responses is created later by `import_invite_response` (Step 6) or `poll_receive_workspace` (Task 4). Verify this is the case and move on.

- [ ] **Step 8: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib`

Expected: Compiles without errors. (The `npm run tauri dev` test comes later with frontend changes.)

- [ ] **Step 9: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/invites.rs krillnotes-desktop/src-tauri/src/commands/sync.rs
git commit -m "feat(desktop): add scope_note_id to invite creation and response import commands"
```

---

### Task 4: Propagate scope in relay polling + update response status handling

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/receive_poll.rs`

- [ ] **Step 1: Update `poll_receive_workspace` to propagate scope**

In `receive_poll.rs`, find where `ReceivedResponse` is constructed inside `poll_receive_workspace` (line ~116). The function downloads accept bundles from the relay and creates response records. After matching the response to an invite (by `invite_id`), copy scope fields:

```rust
scope_note_id: invite_record.scope_note_id.clone(),
scope_note_title: invite_record.scope_note_title.clone(),
```

- [ ] **Step 2: Add `PermissionPending` arm to `update_response_status`**

In `receive_poll.rs`, find the `update_response_status` command (line ~67). It has a match block that maps status strings to `ReceivedResponseStatus` variants. Add:

```rust
"permissionPending" => ReceivedResponseStatus::PermissionPending,
```

Without this, the `OnboardPeerDialog.handleLater()` call will fail because `"permissionPending"` hits the `_ => return Err(...)` arm.

- [ ] **Step 3: Update `ReceivedResponseInfo` struct and From impl**

In `receive_poll.rs`, find the `ReceivedResponseInfo` struct and its `From<ReceivedResponse>` impl (lines ~20-44). Make these changes:

1. Add scope fields to the `ReceivedResponseInfo` struct:
```rust
pub scope_note_id: Option<String>,
pub scope_note_title: Option<String>,
```

2. Add `PermissionPending` arm to the status mapping in the `From` impl:
```rust
ReceivedResponseStatus::PermissionPending => "permissionPending".to_string(),
```

3. Map scope fields in the `From` impl:
```rust
scope_note_id: resp.scope_note_id,
scope_note_title: resp.scope_note_title,
```

- [ ] **Step 4: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib`

Expected: Compiles.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/receive_poll.rs
git commit -m "feat(desktop): propagate invite scope through relay polling and add permissionPending status"
```

---

### Task 5: TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

- [ ] **Step 1: Add scope to `InviteInfo`**

```typescript
export interface InviteInfo {
  // ... existing fields ...
  scopeNoteId: string | null;
  scopeNoteTitle: string | null;
}
```

- [ ] **Step 2: Update `ReceivedResponseInfo`**

Add scope fields and the new status variant:

```typescript
export interface ReceivedResponseInfo {
  // ... existing fields ...
  status: "pending" | "peerAdded" | "permissionPending" | "snapshotSent";
  scopeNoteId: string | null;
  scopeNoteTitle: string | null;
}
```

- [ ] **Step 3: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: Passes (or shows only pre-existing errors, not new ones from these type additions).

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(desktop): add scope and permissionPending to TypeScript types"
```

---

### Task 6: Extract `ChannelPicker` component

**Files:**
- Create: `krillnotes-desktop/src/components/ChannelPicker.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Create `ChannelPicker.tsx`**

Extract the channel selection UI from `WorkspacePeersDialog.tsx` lines 362-431 into a reusable component:

```typescript
import { useTranslation } from 'react-i18next';
import type { RelayAccountInfo } from '../types';

export type ChannelType = 'relay' | 'folder' | 'manual';

interface ChannelPickerProps {
  selectedType: ChannelType;
  onTypeChange: (type: ChannelType) => void;
  relayAccounts: RelayAccountInfo[];
  selectedRelayAccountId?: string;
  onRelayAccountSelect?: (accountId: string) => void;
  currentFolderPath?: string;
  onConfigureFolder?: () => void;
  disabled?: boolean;
}

export function ChannelPicker({
  selectedType,
  onTypeChange,
  relayAccounts,
  selectedRelayAccountId,
  onRelayAccountSelect,
  currentFolderPath,
  onConfigureFolder,
  disabled,
}: ChannelPickerProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <select
          className="text-xs border rounded px-1.5 py-0.5 dark:bg-zinc-800 dark:border-zinc-600"
          value={selectedType}
          onChange={e => onTypeChange(e.target.value as ChannelType)}
          disabled={disabled}
        >
          <option value="relay">{t('workspacePeers.relay', 'Relay')}</option>
          <option value="folder">{t('workspacePeers.folder', 'Folder')}</option>
          <option value="manual">{t('workspacePeers.manual', 'Manual')}</option>
        </select>

        {selectedType === 'folder' && onConfigureFolder && (
          <button
            onClick={onConfigureFolder}
            disabled={disabled}
            className="text-xs text-blue-600 hover:underline disabled:opacity-50"
          >
            {t('workspacePeers.configure', 'Configure')}
          </button>
        )}
      </div>

      {selectedType === 'relay' && (
        relayAccounts.length === 0 ? (
          <p className="text-xs text-zinc-400">
            {t('workspacePeers.noRelayAccounts', 'No relay accounts configured')}
          </p>
        ) : (
          <select
            className="text-xs border rounded px-1.5 py-0.5 dark:bg-zinc-800 dark:border-zinc-600"
            value={selectedRelayAccountId ?? ''}
            onChange={e => onRelayAccountSelect?.(e.target.value)}
            disabled={disabled}
          >
            <option value="">{t('workspacePeers.selectRelay', 'Select relay…')}</option>
            {relayAccounts.map(ra => (
              <option key={ra.relayAccountId} value={ra.relayAccountId}>
                {ra.relayUrl} ({ra.email})
              </option>
            ))}
          </select>
        )
      )}

      {currentFolderPath && selectedType === 'folder' && (
        <span className="text-xs text-zinc-500 truncate">{currentFolderPath}</span>
      )}
    </div>
  );
}
```

Note: The exact JSX should mirror the existing UI from `WorkspacePeersDialog.tsx` lines 362-431 — read that file and match the styling. Verify `RelayAccountInfo` field names against `types.ts` (expected: `relayAccountId`, `relayUrl`, `email`).

- [ ] **Step 2: Refactor `WorkspacePeersDialog.tsx` to use `ChannelPicker`**

Replace the inline channel picker JSX (lines ~362-431) with:

```tsx
<ChannelPicker
  selectedType={(pendingChannelType[peer.peerDeviceId] ?? peer.channelType) as ChannelType}
  onTypeChange={type => handleUpdateChannel(peer, type)}
  relayAccounts={relayAccounts}
  selectedRelayAccountId={pendingRelayAccount[peer.peerDeviceId]}
  onRelayAccountSelect={accountId => handleRelayAccountSelect(peer, accountId)}
  currentFolderPath={peer.channelType === 'folder' ? peer.channelConfig : undefined}
  onConfigureFolder={() => handleConfigureFolder(peer)}
/>
```

Adjust prop names to match the actual state variable names and handler functions in `WorkspacePeersDialog`.

- [ ] **Step 3: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: Passes. The existing peers dialog behavior should be unchanged.

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/ChannelPicker.tsx krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "refactor(desktop): extract ChannelPicker from WorkspacePeersDialog"
```

---

### Task 7: Create `OnboardPeerDialog`

**Files:**
- Create: `krillnotes-desktop/src/components/OnboardPeerDialog.tsx`

**Context:** This dialog is shown when the inviter clicks [Onboard] on a scoped invite response. It orchestrates: accept_peer → set_permission → send snapshot → update response status. The channel picker determines how the snapshot is delivered.

- [ ] **Step 1: Create the component**

```typescript
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { ChannelPicker, type ChannelType } from './ChannelPicker';
import type { ReceivedResponseInfo, RelayAccountInfo } from '../types';

interface OnboardPeerDialogProps {
  open: boolean;
  response: ReceivedResponseInfo;
  identityUuid: string;
  onComplete: () => void;  // Called after successful onboarding or Later/Reject
  onClose: () => void;
}

export function OnboardPeerDialog({
  open, response, identityUuid, onComplete, onClose,
}: OnboardPeerDialogProps) {
  const { t } = useTranslation();
  const [role, setRole] = useState<'owner' | 'writer' | 'reader'>('writer');
  const [channelType, setChannelType] = useState<ChannelType>('relay');
  const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
  const [selectedRelayId, setSelectedRelayId] = useState<string>('');
  const [processing, setProcessing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid })
        .then(accounts => {
          setRelayAccounts(accounts);
          if (accounts.length > 0) setSelectedRelayId(accounts[0].relayAccountId);
        })
        .catch(() => setRelayAccounts([]));
    }
  }, [open, identityUuid]);

  if (!open) return null;

  const handleGrantAndSync = async () => {
    setProcessing(true);
    setError(null);
    try {
      // Step 1: Accept peer (add to workspace) — skip if already added
      if (response.status === 'pending') {
        await invoke('accept_peer', {
          identityUuid,
          inviteePublicKey: response.inviteePublicKey,
          declaredName: response.inviteeDeclaredName,
          trustLevel: 'Tofu',
          localName: null,
        });
      }

      // Step 2: Grant permission on the scoped subtree
      if (response.scopeNoteId) {
        await invoke('set_permission', {
          noteId: response.scopeNoteId,
          userId: response.inviteePublicKey,
          role,
        });
      }

      // Step 3: Send snapshot via selected channel
      if (channelType === 'relay') {
        await invoke('send_snapshot_via_relay', {
          identityUuid,
          peerPublicKeys: [response.inviteePublicKey],
        });
      } else if (channelType === 'folder') {
        // Folder mode: user picks save location
        const savePath = await save({
          defaultPath: `snapshot_${response.inviteeDeclaredName}.swarm`,
          filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
        });
        if (!savePath) { setProcessing(false); return; }
        await invoke('create_snapshot_for_peers', {
          identityUuid,
          peerPublicKeys: [response.inviteePublicKey],
          savePath,
        });
      } else {
        // Manual mode: save file dialog
        const savePath = await save({
          defaultPath: `snapshot_${response.inviteeDeclaredName}.swarm`,
          filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
        });
        if (!savePath) { setProcessing(false); return; }
        await invoke('create_snapshot_for_peers', {
          identityUuid,
          peerPublicKeys: [response.inviteePublicKey],
          savePath,
        });
      }

      // Step 4: Update response status
      await invoke('update_response_status', {
        identityUuid,
        responseId: response.responseId,
        status: 'snapshotSent',
      });

      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  const handleLater = async () => {
    setProcessing(true);
    try {
      // Accept peer but don't grant or send snapshot
      if (response.status === 'pending') {
        await invoke('accept_peer', {
          identityUuid,
          inviteePublicKey: response.inviteePublicKey,
          declaredName: response.inviteeDeclaredName,
          trustLevel: 'Tofu',
          localName: null,
        });
      }
      await invoke('update_response_status', {
        identityUuid,
        responseId: response.responseId,
        status: 'permissionPending',
      });
      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  const handleReject = async () => {
    setProcessing(true);
    try {
      await invoke('dismiss_response', {
        identityUuid,
        responseId: response.responseId,
      });
      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">
          {t('onboard.title', 'Onboard Peer')}
        </h2>

        {/* Peer card */}
        <div className="bg-zinc-50 dark:bg-zinc-800 rounded-lg p-3 mb-4">
          <p className="font-medium">{response.inviteeDeclaredName}</p>
          <p className="text-xs text-zinc-500 font-mono truncate">
            {response.inviteePublicKey.slice(0, 16)}…
          </p>
        </div>

        {/* Scope reminder */}
        {response.scopeNoteTitle && (
          <div className="mb-4">
            <label className="block text-sm font-medium text-zinc-500 mb-1">
              {t('onboard.scope', 'Invited to subtree')}
            </label>
            <p className="text-sm bg-zinc-50 dark:bg-zinc-800 rounded px-3 py-1.5">
              {response.scopeNoteTitle}
            </p>
          </div>
        )}

        {/* Role picker */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t('onboard.role', 'Role')}
          </label>
          <select
            className="w-full border rounded px-3 py-2 dark:bg-zinc-800 dark:border-zinc-700"
            value={role}
            onChange={e => setRole(e.target.value as typeof role)}
            disabled={processing}
          >
            <option value="owner">{t('roles.owner', 'Owner — full control of subtree')}</option>
            <option value="writer">{t('roles.writer', 'Writer — create and edit notes')}</option>
            <option value="reader">{t('roles.reader', 'Reader — view only')}</option>
          </select>
        </div>

        {/* Channel picker */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t('onboard.channel', 'Sync channel')}
          </label>
          <ChannelPicker
            selectedType={channelType}
            onTypeChange={setChannelType}
            relayAccounts={relayAccounts}
            selectedRelayAccountId={selectedRelayId}
            onRelayAccountSelect={setSelectedRelayId}
            disabled={processing}
          />
        </div>

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        {/* Actions */}
        <div className="flex justify-between">
          <button
            onClick={handleReject}
            disabled={processing}
            className="px-3 py-2 text-sm text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 rounded disabled:opacity-50"
          >
            {t('onboard.reject', 'Reject')}
          </button>
          <div className="flex gap-2">
            <button
              onClick={handleLater}
              disabled={processing}
              className="px-4 py-2 text-sm rounded border dark:border-zinc-700 disabled:opacity-50"
            >
              {t('onboard.later', 'Later')}
            </button>
            <button
              onClick={handleGrantAndSync}
              disabled={processing}
              className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
            >
              {processing
                ? t('common.saving', 'Saving…')
                : t('onboard.grantAndSync', 'Grant & sync')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
```

Note: Verify that `accept_peer`, `set_permission`, `update_response_status`, `dismiss_response`, `send_snapshot_via_relay`, `create_snapshot_for_peers`, and `list_relay_accounts` command names and parameter names match the actual Tauri command signatures. Read the relevant command files to confirm exact parameter names (especially casing — Tauri uses camelCase params from the TS side).

- [ ] **Step 2: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/OnboardPeerDialog.tsx
git commit -m "feat(desktop): add OnboardPeerDialog for post-accept role + channel assignment"
```

---

### Task 8: Update `PendingResponsesSection` for scoped invites

**Files:**
- Modify: `krillnotes-desktop/src/components/PendingResponsesSection.tsx`

- [ ] **Step 1: Add `onOnboardPeer` callback prop**

```typescript
interface Props {
  identityUuid: string;
  workspaceId?: string;
  onAcceptResponse: (resp: ReceivedResponseInfo) => void;
  onSendSnapshot: (resp: ReceivedResponseInfo) => void;
  onOnboardPeer: (resp: ReceivedResponseInfo) => void;  // ← NEW
}
```

- [ ] **Step 2: Handle `permissionPending` status display**

Add a new status case in the rendering logic (after `peerAdded`, before `snapshotSent`):

```tsx
{resp.status === 'permissionPending' && (
  <div>
    <span className="text-xs px-2 py-0.5 rounded bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-300">
      {t('pendingResponses.awaitingOnboard', 'Awaiting onboarding')}
    </span>
    <button
      onClick={() => onOnboardPeer(resp)}
      className="ml-2 text-xs px-2 py-1 rounded bg-blue-600 text-white"
    >
      {t('pendingResponses.onboard', 'Onboard')}
    </button>
  </div>
)}
```

- [ ] **Step 3: Show scope badge on responses that have a scope**

For all response statuses, add a scope indicator if `resp.scopeNoteId` is set:

```tsx
{resp.scopeNoteTitle && (
  <span className="text-xs text-zinc-400 ml-1">
    → {resp.scopeNoteTitle}
  </span>
)}
```

- [ ] **Step 4: Route scoped `pending` responses to onboard flow instead of direct accept**

For responses with `resp.scopeNoteId`, change the "Accept and send snapshot" button to "Onboard":

```tsx
{resp.status === 'pending' && (
  resp.scopeNoteId ? (
    <button onClick={() => onOnboardPeer(resp)} className="...">
      {t('pendingResponses.onboard', 'Onboard')}
    </button>
  ) : (
    <button onClick={() => onAcceptResponse(resp)} className="...">
      {t('pendingResponses.acceptAndSend', 'Accept and send snapshot')}
    </button>
  )
)}
```

- [ ] **Step 5: Type-check and verify**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/PendingResponsesSection.tsx
git commit -m "feat(desktop): add permissionPending state and onboard routing to PendingResponsesSection"
```

---

### Task 9: Update `InviteManagerDialog` to wire onboarding

**Files:**
- Modify: `krillnotes-desktop/src/components/InviteManagerDialog.tsx`

- [ ] **Step 1: Add `initialScope` prop and onboard state**

```typescript
interface Props {
  identityUuid: string;
  workspaceName: string;
  onClose: () => void;
  initialScope?: { noteId: string; noteTitle: string } | null;  // ← NEW
}
```

Add state for the onboard dialog:

```typescript
const [onboardResponse, setOnboardResponse] = useState<ReceivedResponseInfo | null>(null);
```

- [ ] **Step 2: Auto-open CreateInviteDialog when `initialScope` is set**

On mount / when `initialScope` changes, if set, auto-open the create dialog:

```typescript
useEffect(() => {
  if (initialScope) {
    setShowCreate(true);
  }
}, [initialScope]);
```

- [ ] **Step 3: Pass scope to `CreateInviteDialog`**

Update the CreateInviteDialog rendering to pass scope:

```tsx
{showCreate && (
  <CreateInviteDialog
    identityUuid={identityUuid}
    workspaceName={workspaceName}
    scopeNoteId={initialScope?.noteId}
    scopeNoteTitle={initialScope?.noteTitle}
    onCreated={invite => { load(); setShowCreate(false); }}
    onClose={() => setShowCreate(false)}
  />
)}
```

- [ ] **Step 4: Show scope badge per invite in the invite list**

In the invite list rendering, add scope display:

```tsx
{invite.scopeNoteId && (
  <span className="text-xs text-zinc-400">
    → {invite.scopeNoteTitle ?? invite.scopeNoteId}
  </span>
)}
```

- [ ] **Step 5: Wire `onOnboardPeer` from PendingResponsesSection**

Pass the handler to PendingResponsesSection:

```tsx
<PendingResponsesSection
  identityUuid={identityUuid}
  workspaceId={...}
  onAcceptResponse={handleAcceptResponse}
  onSendSnapshot={handleSendSnapshot}
  onOnboardPeer={resp => setOnboardResponse(resp)}
/>
```

Render OnboardPeerDialog:

```tsx
{onboardResponse && (
  <OnboardPeerDialog
    open={true}
    response={onboardResponse}
    identityUuid={identityUuid}
    onComplete={() => { setOnboardResponse(null); load(); }}
    onClose={() => setOnboardResponse(null)}
  />
)}
```

- [ ] **Step 6: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/InviteManagerDialog.tsx
git commit -m "feat(desktop): wire OnboardPeerDialog and scope display into InviteManagerDialog"
```

---

### Task 10: Update `CreateInviteDialog` with scope

**Files:**
- Modify: `krillnotes-desktop/src/components/CreateInviteDialog.tsx`

- [ ] **Step 1: Add scope props**

```typescript
interface Props {
  identityUuid: string;
  workspaceName: string;
  scopeNoteId?: string;       // ← NEW
  scopeNoteTitle?: string;    // ← NEW
  onCreated: (invite: InviteInfo) => void;
  onClose: () => void;
}
```

- [ ] **Step 2: Display scope badge in the configure step**

Add after the description text (line ~193), before the expiry selector:

```tsx
{scopeNoteId && (
  <div className="mb-4">
    <label className="block text-sm font-medium mb-1">
      {t('invite.scope', 'Subtree scope')}
    </label>
    <p className="text-sm bg-zinc-50 dark:bg-zinc-800 rounded px-3 py-1.5">
      {scopeNoteTitle ?? scopeNoteId}
    </p>
  </div>
)}
```

- [ ] **Step 3: Pass scope to `create_invite` command**

Update the `handleCreate` invoke call (line ~53):

```typescript
const invite = await invoke<InviteInfo>('create_invite', {
  identityUuid,
  workspaceName,
  expiresInDays: effectiveDays ?? undefined,
  savePath,
  scopeNoteId: scopeNoteId ?? null,  // ← NEW
});
```

- [ ] **Step 4: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/CreateInviteDialog.tsx
git commit -m "feat(desktop): add scope display and passthrough to CreateInviteDialog"
```

---

### Task 11: Add "Invite to this subtree…" to context menu

**Files:**
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

- [ ] **Step 1: Add new props to ContextMenu**

```typescript
// Add to the existing props interface:
effectiveRole?: string | null;       // "owner" | "writer" | "reader" | "root_owner" | "none" | null
onInviteToSubtree?: (noteId: string) => void;
```

- [ ] **Step 2: Add "Invite to this subtree…" menu entry**

Add after tree actions, before the Delete divider (around line 122):

```tsx
{noteId && effectiveRole && (effectiveRole === 'owner' || effectiveRole === 'root_owner') && onInviteToSubtree && (
  <>
    <div className="border-t dark:border-zinc-700 my-1" />
    <button
      onClick={() => { onInviteToSubtree(noteId); onClose(); }}
      className="w-full text-left px-3 py-1.5 text-sm hover:bg-zinc-100 dark:hover:bg-zinc-700"
    >
      {t('contextMenu.inviteToSubtree', 'Invite to this subtree…')}
    </button>
  </>
)}
```

- [ ] **Step 3: Wire in WorkspaceView**

In `WorkspaceView.tsx`, find where `<ContextMenu>` is rendered. Add:

1. State for the invite scope:
```typescript
const [inviteScope, setInviteScope] = useState<{ noteId: string; noteTitle: string } | null>(null);
```

2. Pass `effectiveRole` and handler to ContextMenu:
```tsx
<ContextMenu
  // ... existing props ...
  effectiveRole={/* fetch from get_effective_role for the right-clicked note */}
  onInviteToSubtree={(noteId) => {
    const note = /* find note by id from the tree data */;
    setInviteScope({ noteId, noteTitle: note?.title ?? noteId });
  }}
/>
```

3. Open InviteManagerDialog with scope when `inviteScope` is set:
```tsx
{inviteScope && (
  <InviteManagerDialog
    identityUuid={identityUuid}
    workspaceName={workspaceName}
    initialScope={inviteScope}
    onClose={() => setInviteScope(null)}
  />
)}
```

Note: The `effectiveRole` for the right-clicked note needs to be resolved. If `WorkspaceView` already has an effective roles map (from `get_all_effective_roles`), use it. If not, fetch it lazily when the context menu opens. Check how the tree data and context menu coordinate in `WorkspaceView` — the `effectiveRole` may need to be looked up by `noteId` from a roles map.

- [ ] **Step 4: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/ContextMenu.tsx krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(desktop): add 'Invite to this subtree...' context menu entry for Owner+"
```

---

### Task 12: Add i18n strings

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`

- [ ] **Step 1: Add all new translation keys**

Add to the English locale file:

```json
{
  "onboard": {
    "title": "Onboard Peer",
    "scope": "Invited to subtree",
    "role": "Role",
    "channel": "Sync channel",
    "grantAndSync": "Grant & sync",
    "later": "Later",
    "reject": "Reject"
  },
  "roles": {
    "owner": "Owner — full control of subtree",
    "writer": "Writer — create and edit notes",
    "reader": "Reader — view only"
  },
  "pendingResponses": {
    "awaitingOnboard": "Awaiting onboarding",
    "onboard": "Onboard"
  },
  "invite": {
    "scope": "Subtree scope"
  },
  "contextMenu": {
    "inviteToSubtree": "Invite to this subtree…"
  }
}
```

Merge these into the existing JSON structure (don't overwrite existing keys in the same sections). Check for existing `roles` or `invite` sections and merge rather than replace.

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/en.json
git commit -m "feat(i18n): add English strings for invite-to-subtree and onboarding"
```

---

### Task 13: Integration test — full round-trip

**Files:**
- Modify: `krillnotes-core/src/core/invite.rs` (test section)

- [ ] **Step 1: Write integration test for scoped invite → response → scope propagation**

```rust
#[test]
fn scoped_invite_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut mgr = InviteManager::new(dir.path().to_path_buf());
    let inviter_key = test_key();

    // Create scoped invite
    let (record, file) = mgr.create_invite(
        "ws-1", "Test WS", None, &inviter_key, "Alice",
        None, None, None, None, None, vec![],
        Some("note-42".to_string()),
        Some("Backend API".to_string()),
    ).unwrap();

    assert_eq!(record.scope_note_id.as_deref(), Some("note-42"));

    // Serialize and verify the invite file
    let file_json = serde_json::to_string(&file).unwrap();
    let parsed: InviteFile = serde_json::from_str(&file_json).unwrap();
    assert_eq!(parsed.scope_note_id.as_deref(), Some("note-42"));
    assert_eq!(parsed.scope_note_title.as_deref(), Some("Backend API"));

    // Verify signature still valid
    let payload = serde_json::to_value(&parsed).unwrap();
    let pubkey_b64 = STANDARD.encode(inviter_key.verifying_key().to_bytes());
    verify_payload(&payload, &parsed.signature, &pubkey_b64).unwrap();
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test -p krillnotes-core`

Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/invite.rs
git commit -m "test(core): add scoped invite round-trip integration test"
```

---

### Task 14: Manual smoke test

- [ ] **Step 1: Start dev server**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`

- [ ] **Step 2: Test scoped invite creation**

1. Open a workspace with the RBAC feature enabled
2. Right-click a note in the tree → verify "Invite to this subtree…" appears (only if you are Owner/Root Owner)
3. Click it → CreateInviteDialog opens with scope badge showing the note title
4. Create the invite → verify it appears in InviteManager with scope badge

- [ ] **Step 3: Test onboarding flow (if you have a second identity)**

1. Import the scoped invite on another identity
2. Send a response
3. On the inviter side, verify the response shows "Onboard" button (not "Accept and send snapshot")
4. Click Onboard → verify OnboardPeerDialog opens with correct scope, role picker, and channel picker
5. Select a role and channel → click "Grant & sync"
6. Verify the peer gets added, permission is set, and snapshot is sent

- [ ] **Step 4: Test "Later" and "Reject"**

1. For a pending scoped response, click Onboard → Later → verify status becomes "Awaiting onboarding"
2. Clicking Onboard again on the same response should reopen the dialog
3. For another response, click Onboard → Reject → verify response is removed

---

## Implementation Notes

### Signing backward compatibility

The `sign_payload` / `verify_payload` system in `invite.rs` works by:
1. Serializing the struct to `serde_json::Value`
2. Removing `"signature"` key
3. Sorting all keys via `sort_json` (BTreeMap)
4. Signing/verifying the canonical JSON string

For InviteFile, new `Option` fields MUST use `#[serde(skip_serializing_if = "Option::is_none")]` so that `None` values are omitted from the JSON Value. This ensures old invite files (which lack scope fields) produce identical canonical JSON when round-tripped through deserialization + re-serialization.

### Status lifecycle

```
                    ┌─────────── unscoped invite ───────────┐
                    │                                        │
    Pending ──→ PeerAdded ──→ SnapshotSent                  │
                    │                                        │
                    └── scoped invite ──┐                    │
                                        ▼                    │
    Pending ──→ OnboardPeerDialog ──→ Grant & sync ──→ SnapshotSent
                    │
                    └──→ Later ──→ PermissionPending ──→ [Onboard again] ──→ SnapshotSent
                    │
                    └──→ Reject ──→ (removed)
```

### Full workspace snapshots

Snapshots always contain the full workspace (all notes, scripts, attachments). The RBAC gate controls what the peer can *do* with the data, not what data they physically hold. This ensures that if a peer's role is later upgraded, the data is already available locally without requiring a new snapshot.
