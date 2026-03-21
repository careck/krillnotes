# RBAC Permission UI — Design Spec

**Date:** 2026-03-22
**Status:** Approved
**Depends on:** PR #107 (RBAC gate wiring), `krillnotes-rbac` crate

## Overview

Add UI for managing RBAC permissions across the application. Users need to see their access level, share subtrees with peers, onboard newly-accepted peers with scoped permissions, and understand the impact of permission changes.

The backend RBAC infrastructure is complete (PermissionGate trait, RbacGate implementation, 5 permission operation variants, authorization wired into 15 mutating methods). This spec covers the frontend surface and identifies required backend changes.

## Design Principles

- **Note-centric**: permissions are managed from the note, not from a peer list. The question is "who can see this subtree?" not "what can this peer access?"
- **Reuse existing components**: extend the Info section collapsible, reuse the channel picker from peers view, embed in existing dialogs rather than creating new panels
- **Effective vs. explicit**: tree dots show resolved (effective) access; share anchors and Info section management operate on explicit grants at the anchor node
- **Opt-in cascade**: demoting a peer shows impact on downstream grants but does not auto-revoke — the user decides

## Delegation Model

- 4 roles: Root Owner > Owner > Writer > Reader
- Root Owner is identified by public key comparison, bypasses all checks
- Any Owner on a subtree can grant roles up to their own level within that subtree
- Grants are anchored at a specific note and inherited by all descendants
- The resolver walks up the tree to find the nearest explicit grant (default-deny if none found)

## Backend Changes Required

This UI spec depends on backend changes that must be implemented before or alongside the frontend work.

### Critical: Refactor cascade_revoke to be opt-in

The current `cascade_revoke` in `gate.rs:114-146` runs automatically inside `apply_permission_op` for every `SetPermission`, `RevokePermission`, and `RemovePeer` operation. This contradicts the spec's core design principle of opt-in cascade.

**Required change:** Split cascade into two operations:
1. A read-only `preview_cascade(conn, user_id, note_id, new_role)` method that computes which downstream grants would become invalid, without modifying anything.
2. Remove the automatic `cascade_revoke` call from `apply_permission_op`. Instead, the UI layer explicitly issues individual `RevokePermission` operations for each downstream grant the user selects in the cascade preview dialog.

### Critical: MoveNote must check destination scope

`resolve_scope` for `MoveNote` in `gate.rs:40` only checks the source `note_id`, not `new_parent_id`. A Writer on subtree A could move a note into subtree B where they have no access. The gate must authorize against both source and destination scopes. The UI should also filter drag-drop targets to only valid destinations.

### Critical: New query methods on Workspace / RbacGate

The UI requires permission query methods that do not yet exist:

| Method | Purpose | Implementation |
|--------|---------|----------------|
| `get_note_permissions(note_id)` | Explicit grants anchored at this node | `SELECT * FROM note_permissions WHERE note_id = ?` joined with peer/contact display names |
| `get_effective_role(note_id, user_id)` | Resolved role + anchor info | Extended `resolve_role` that also returns the anchor `note_id`, `granted_by`, and note title. Must special-case root owner (return `"root_owner"` via pubkey comparison, not resolver) |
| `get_inherited_permissions(note_id)` | Grants inherited from ancestors | Walk parent chain, collecting grants from each ancestor with their anchor node info |
| `get_all_effective_roles(user_id)` | Batch query for tree dots | Single-pass computation of effective role for all visible notes, avoiding O(N × D) individual queries. Required for tree dot performance with 1000+ notes |
| `preview_cascade(note_id, user_id, new_role)` | Impact preview for demotion/revocation | Read-only query: find all grants where `granted_by = user_id` and the new role would not satisfy the `require_at_least(Owner)` check |

### Important: Add scope to invite infrastructure

The invite-to-subtree flow requires adding a `scope_note_id: Option<String>` field to:
- `InviteRecord` in `invite.rs`
- `InviteFile` (wire format)
- `ReceivedResponse` in `received_response.rs`

This is a wire format change and must be backward-compatible (use `#[serde(default)]`).

### Important: Add "pending onboarding" state to ReceivedResponseStatus

The current `ReceivedResponseStatus` enum has: `Pending`, `PeerAdded`, `SnapshotSent`. The new state "accepted — pending onboarding" maps to the gap between `PeerAdded` and `SnapshotSent`. Add a new variant `PermissionPending` that represents "peer accepted, awaiting permission grant before snapshot."

### Important: Note deletion must clean up anchored grants

When a note with `SetPermission` grants anchored to it is deleted, the `note_permissions` rows become orphaned (no FK cascade in `schema.sql`). The `delete_note_recursive` and `delete_note_promote` methods must clean up `note_permissions` rows for deleted notes. The UI should show a warning if deleting a note that has permission grants anchored to it: "This note has permissions shared with N peers. Deleting it will revoke their access to this subtree."

## Section 1: Tree Indicators

### Role Dots

A small colored dot before each note title in the tree, showing the current user's **effective role** on that note:

- Green `●` = Owner
- Orange `●` = Writer
- Yellow `●` = Reader
- No dot = no access (node not visible, or ghost ancestor)

Dots cascade down from the grant anchor. If you're granted Writer on `/Projects`, every note under `/Projects` shows an orange dot. A closer grant overrides: if you're also granted Reader on `/Projects/Docs`, that subtree switches to yellow.

**Root Owner** sees all green dots everywhere — they have full access by virtue of being root owner.

### Share Anchor Icon

A small "shared" icon (e.g., people/share icon `👥`) appears **only on the exact node** where a `SetPermission` grant is anchored. It does NOT appear on children.

Visible to any user with Owner role on that subtree — they're the ones who can manage sharing.

### Ghost Ancestors

When a peer has access to a subtree (e.g., Writer on `/Projects/API`), the tree shows the ancestor path (`/ → Projects → API`) with parent nodes greyed out and non-interactive:

- No dots on ghost nodes
- No context menu actions
- No editing capability
- Just structural breadcrumbs for orientation

The root owner controls the tree structure, so if ancestor titles are sensitive, they can reorganize (e.g., make the shared subtree a root-level note).

## Section 2: Info Section Extension

Permission information is added to the **existing collapsible Info section** in the detail panel (right side). No new panel is created.

### New rows below the existing ID field

#### "Your role" row (always shown)

Shows the current user's effective role as a colored tag with dot:

```
Your role    ● Writer
```

If the role is inherited from a parent node, shows the source:

```
Your role    ● Writer
             Inherited from Backend API · granted by Root Owner
```

"Backend API" is a clickable link that navigates to the anchor node in the tree.

For root owner: `● Owner (Root)`

#### "Shared with" section (only for Owner+ on this node)

**At the anchor node** (grants anchored here):

Header: "Shared with — anchored here"

Lists each peer with an explicit grant at this node:
- Colored dot + peer name
- Role badge (clickable → dropdown to change role: Owner/Writer/Reader)
- ✕ button to revoke

Changing role or revoking triggers the cascade preview (Section 5) if the peer has granted downstream access.

**At a child node** (grants inherited from parent):

Header: "Access from parent grants"

Lists peers with inherited access, each row dimmed:
- Colored dot + peer name
- Role badge (read-only, not clickable)
- "via **Anchor Node**" — clickable link to navigate to where the grant lives

No ✕ button, no role change. To modify, navigate to the anchor node.

**Mixed case**: a single note may have both anchored grants AND inherited grants from different ancestors. Show both groups with their respective headers. Only anchored grants get management controls.

#### "+ Share this subtree..." button

Shown to Owner+ on any node. Opens the Share Dialog (Section 3). Creates a new grant anchored at this node.

## Section 3: Share Dialog

Opened from:
1. "+ Share this subtree..." button in the Info section
2. Right-click context menu → "Share subtree..." (only shown to Owner+ on that node)

### Dialog layout

- **Scope** (read-only): pre-filled with the note path. To share a different subtree, navigate there first.
- **Peer picker**: search bar + scrollable list (max ~5 visible rows, rest accessible by scrolling). Shows only connected peers without an existing explicit grant at this node. Search filters by peer name and fingerprint. Shows peer count: "7 of 14 peers · scroll or search".
- **Role dropdown**: Owner / Writer / Reader. Descriptions inline (e.g., "Writer — can create and edit notes"). Options capped at the granting user's own role level.
- **Actions**: Cancel / Share

### On confirm

Emits a `SetPermission` operation. Grant appears immediately in the Info section's "Shared with" list.

### Peer filtering

The "7 of 14 peers" display requires combining `list_workspace_peers` (existing) with `get_note_permissions` (new) to filter out peers who already have an explicit grant at this node. This filtering is done client-side to avoid a dedicated Tauri command.

### Context menu integration

Add "Share subtree..." to the existing context menu in `ContextMenu.tsx`. Only visible when the current user has Owner role on the right-clicked node. `ContextMenu` will need a new prop (e.g., `effectiveRole: EffectiveRole | null`) to conditionally render permission-related entries.

### Role-aware UI disabling

Readers can only view notes — they cannot create, edit, or delete. The UI should disable or hide irrelevant actions based on the effective role:

- **Reader**: hide "Add Child", "Add Sibling", "Edit", "Delete" in context menu. Disable edit mode in InfoPanel. Hide "Share subtree..." (not Owner).
- **Writer**: show create/edit actions. Hide "Share subtree..." (not Owner). Disable delete/move on notes not authored by them (backend enforces this, but UI should prevent the attempt).
- **Owner**: full context menu. Show "Share subtree..." and "Invite to this subtree...".

## Section 4: Invite-to-Subtree and Post-Accept Onboarding

### Key insight: scope is set at invite creation

Invites are always created **from a specific note** (via context menu or Info section). The invite carries the subtree scope as metadata. This means:

- The invite creation flow is scope-aware from the start
- The post-accept dialog does NOT need a tree picker — scope is already known
- Multiple people can accept the same unspecified invite; each triggers their own onboarding

### Invite creation flow (revised)

Context menu on a note → "Invite to this subtree..." opens the invite creation dialog:

1. **Scope** (read-only): the note you right-clicked
2. **Recipient**: known contact (from identity contact manager) OR unspecified (for relay links / shared invite files)
3. Standard invite creation (generates `.swarm` invite file or relay link)

The Invite Manager shows the subtree scope per invite in the invite list.

### Post-accept onboarding flow

When a peer accepts an invite, they appear in the Invite Manager under a new state: **"Accepted — pending onboarding"**.

The Invite Manager lifecycle becomes:

```
Active invites (waiting for acceptance)
  ↓ peer accepts
Accepted — pending onboarding  ← NEW state
  ↓ click [Onboard]
Completed (onboarded, syncing)
```

The inviter opens the Invite Manager, sees pending acceptances, clicks **[Onboard]** to open the grant dialog.

### Grant dialog (post-accept)

Simplified dialog — no tree picker needed:

1. **Peer card**: name, key fingerprint, trust level badge (identity just learned from acceptance)
2. **Scope reminder** (read-only): "Invited to: /Projects/Backend API" (set at invite creation)
3. **Role dropdown**: Owner / Writer / Reader (capped at inviter's role)
4. **Channel picker**: reused component from `WorkspacePeersDialog` — dropdown morphs between:
   - **Relay**: relay account selector (lists relays from your identity)
   - **Folder**: path display + Configure/Browse button
   - **Manual**: no sub-UI; "Grant & sync" triggers a save-file dialog for the `.swarm` snapshot
5. **Actions**: Reject / Later / Grant & sync

**"Later"** closes the dialog. The peer stays in "pending onboarding" state. No snapshot is sent.

**"Grant & sync"** emits `SetPermission` + sends a snapshot scoped to the granted subtree via the selected channel.

**"Reject"** removes the peer entirely.

### Snapshot scoping

The snapshot sent after onboarding MUST be scoped to only include notes the peer can access based on their granted subtree. This is a backend concern but is critical for security — no data should flow before permissions are assigned.

## Section 5: Cascade Preview

When demoting or revoking a peer's access, and that peer has granted access to others downstream, the system shows a preview dialog instead of automatically cascading.

### Trigger

- Clicking ✕ (revoke) on a peer in the Info section
- Changing a peer's role to a lower level via the role dropdown

The backend computes "what would be affected" as a read-only query before any changes are applied.

### Dialog

```
Demoting Alice: Owner → Reader
on /Projects/Backend API

Alice previously granted access to others.
Since only Owners can grant roles, these grants
would no longer be valid:

☑ Bob — Writer on /Projects/API
  (Alice is no longer Owner — cannot grant any role)
☑ Carol — Reader on /Projects/API
  (Alice is no longer Owner — cannot grant any role)

[Cancel]    [Demote Alice only]    [Demote & revoke selected]
```

### Behaviours

- **Pre-checked**: grants that would be invalid under the new role. Since only Owners can grant, demoting from Owner to anything below means ALL downstream grants are invalid.
- **Unchecked by user**: the user can uncheck any grant to keep it despite being technically invalid ("stamped grant" override). The grant persists as-is.
- **"Demote Alice only"**: applies only the role change, leaves all downstream grants intact.
- **"Demote & revoke selected"**: applies the role change AND issues `RevokePermission` for each checked grant.

### No cascade on simple revocation

If the revoked peer has NOT granted access to anyone else, the cascade preview is skipped — the revocation happens directly with a simple confirmation.

## New Tauri Commands Required

| Command | Purpose |
|---------|---------|
| `set_permission(note_id, user_id, role)` | Grant a role on a subtree |
| `revoke_permission(note_id, user_id)` | Revoke a grant at a specific node |
| `get_note_permissions(note_id)` | Get explicit grants anchored at this node |
| `get_effective_role(note_id)` | Get current user's resolved role for a note |
| `get_inherited_permissions(note_id)` | Get grants inherited from ancestors (with anchor node info) |
| `preview_cascade(note_id, user_id, new_role)` | Compute what downstream grants would be invalidated |
| `list_pending_acceptances()` | List accepted invites awaiting onboarding |
| `onboard_peer(invite_id, role, channel_config)` | Grant permission + send scoped snapshot |
| `reject_accepted_peer(invite_id)` | Reject an accepted peer |

## New TypeScript Types Required

```typescript
interface PermissionGrant {
  noteId: string | null;     // null for workspace-level grants (e.g., TransferRootOwnership)
  userId: string;            // base64 Ed25519 public key
  role: "owner" | "writer" | "reader";
  grantedBy: string;         // base64 Ed25519 public key
  displayName: string;       // resolved by Tauri command from peer registry / contact book
  grantedByName: string;     // resolved display name of granter
}

// Note: "root_owner" is not a backend Role enum value — it is synthesized by the
// Tauri command when actor == owner_pubkey (bypasses resolver entirely).
interface EffectiveRole {
  role: "owner" | "writer" | "reader" | "root_owner";
  inheritedFrom: string | null;  // note_id of anchor, null if anchored here or root owner
  inheritedFromTitle: string | null;
  grantedBy: string | null;
  grantedByName: string | null;
}

interface CascadeImpact {
  affectedGrants: PermissionGrant[];
  reason: string;  // e.g., "no longer Owner — cannot grant any role"
}

interface PendingAcceptance {
  inviteId: string;
  peerName: string;
  peerPublicKey: string;
  peerFingerprint: string;
  trustLevel: TrustLevel;
  scopeNoteId: string;
  scopeNotePath: string;     // e.g., "/ Projects / Backend API"
  acceptedAt: string;        // ISO 8601
}
```

### Display name resolution

`PermissionGrant.displayName` and `grantedByName` are resolved server-side by the Tauri command. The `note_permissions` table stores only public keys (`user_id`, `granted_by`). The command joins against the peer registry and contact book to resolve display names. If no name is found, falls back to the first 8 characters of the base64 key.

## Components to Create or Modify

### New components

| Component | Purpose |
|-----------|---------|
| `PermissionDot.tsx` | Colored dot indicator for tree nodes |
| `ShareAnchorIcon.tsx` | Shared icon on nodes with explicit grants |
| `ShareDialog.tsx` | Peer picker + role selector for granting access |
| `CascadePreviewDialog.tsx` | Shows impact of demotion/revocation |
| `OnboardPeerDialog.tsx` | Post-accept grant dialog (role + channel) |
| `ChannelPicker.tsx` | Extracted from WorkspacePeersDialog for reuse |

### Modified components

| Component | Changes |
|-----------|---------|
| `TreeNode.tsx` | Add PermissionDot + ShareAnchorIcon, ghost ancestor styling |
| `InfoPanel.tsx` (Info section) | Add "Your role", "Shared with" / "Access from parent grants", "+ Share" button |
| `ContextMenu.tsx` | Add "Share subtree..." and "Invite to this subtree..." entries (Owner+ only). New `effectiveRole` prop for conditional rendering. Role-aware disabling of create/edit/delete actions for Readers/Writers. |
| `WorkspaceView.tsx` | Wire new dialogs, fetch permission state |
| `InviteManagerDialog.tsx` | Add "Accepted — pending onboarding" state, [Onboard] / [Reject] buttons |
| `WorkspacePeersDialog.tsx` | Extract channel picker into shared component |

## Out of Scope

- Undo/redo authorization for permission operations
- Attachment-level permissions
- Workspace-level metadata mutation permissions
- Permission audit log UI (operation log already captures permission ops)
- Bulk permission operations (grant to multiple peers at once)
