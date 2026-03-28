# Invite Workflow Redesign — Design Spec

**Issue:** #113 — Improving the invitation and onboarding workflow
**Date:** 2026-03-27

## Problem

The current invite/onboard flow has several UX issues:
1. Role is not set at invite time — deferred to onboarding, which is confusing
2. Channel choice (relay vs file) comes too late — inviter must save a file first, then separately upload to relay
3. Workspace-level invites are still exposed — "Create Invite" and "Share Invite Link" buttons create unscoped invites, which we no longer want
4. "Accept Invite" is buried in the File menu — should be in the Identity dialog so the identity binding is clear
5. Invitee sees no role, subtree, or workspace metadata when reviewing an invite
6. Invitee doesn't know the inviter's relay server — can't make an informed response channel choice
7. No inline relay signup — if the invitee isn't on the inviter's relay, they're stuck

## Approach: Rewrite the Dialog Chain

Replace the existing multi-dialog flow with purpose-built components for the streamlined workflow. This is a rewrite, not an incremental modification.

### Component Mapping (old → new)

| Remove | Replace with |
|---|---|
| `CreateInviteDialog.tsx` | `InviteWorkflow.tsx` (single dialog, 2 steps) |
| `ImportInviteDialog.tsx` | `AcceptInviteWorkflow.tsx` (single dialog, 3 steps) |
| `AcceptPeerDialog.tsx` | Absorbed into simplified `OnboardPeerDialog.tsx` |
| `SwarmInviteDialog.tsx` | Delete (legacy, workspace-level only) |

## Data Model Changes

### InviteFile (wire format in .swarm files)

Add one new field:

```
offered_role: String    // "owner" | "writer" | "reader"
```

- Set by inviter at creation time
- Displayed to invitee in the review screen
- Auto-applied during onboarding via `set_permission`
- Included in the Ed25519 signature (part of canonical JSON), so it cannot be tampered with

### InviteRecord (on-disk JSON)

Add matching field:

```
offered_role: String
```

### ReceivedResponseInfo

Add channel tracking and denormalized invite data:

```
response_channel: String           // "relay" | "file"
relay_account_id: Option<String>   // which relay account received it (if relay)
offered_role: String               // denormalized from InviteRecord for OnboardPeerDialog display
```

Set at import time — `import_invite_response` (file) or `fetch_relay_invite_response` (relay). `response_channel` and `relay_account_id` are persisted so `OnboardPeerDialog` knows which snapshot delivery path to take. `offered_role` is copied from the linked `InviteRecord` so the onboard dialog can display and auto-apply it without a separate lookup.

### No relay URL field in InviteFile

The inviter's relay server is NOT stored in the invite data. If the invite was sent via relay, the invitee already knows the server from the URL they fetched it from (e.g., `https://relay.example.com/invite/abc123`). The frontend derives the relay server from the fetch URL and passes it as UI state.

### No database schema changes

Invites are stored as JSON files, not in SQLite.

## InviteWorkflow (Bob's side — replaces CreateInviteDialog)

### Trigger

Right-click note → "Invite to subtree" (unchanged). This is the **only** way to create invites.

### Single-step form

- **Subtree** — read-only display of the scoped note title (passed as prop)
- **Role picker** — dropdown: Owner / Writer / Reader. Default: Writer
- **Expiry picker** — dropdown: No expiry / 7 days / 30 days / Custom (same as today)
- **Channel toggle** — two cards: Relay / File
  - Relay card only enabled if Bob has relay account(s) (checked via `has_relay_credentials`)
  - If no relay account, the relay card is disabled with a hint
- **Relay account dropdown** — shown when Relay is selected. Lists Bob's accounts (email @ server). Pre-selected if only one account.

### On submit (relay)

1. Call `create_relay_invite` (new command — creates invite + uploads in one step, no .swarm file saved to disk)
2. Show success state: link copied to clipboard, summary of role/subtree/expiry
3. "Copy Again" and "Done" buttons

### On submit (file)

1. OS save dialog opens
2. Call `create_invite` (existing command, now with `offered_role` parameter)
3. Show success state: file saved confirmation, summary
4. "Done" button

### New Tauri command

`create_relay_invite` — atomic invite creation + relay upload:
- Parameters: `identity_uuid`, `workspace_name`, `expires_in_days`, `scope_note_id`, `offered_role`, `relay_account_id`
- Returns: relay URL string
- No `.swarm` file created on disk

### Modified Tauri command

`create_invite` — add `offered_role: String` parameter.

## AcceptInviteWorkflow (Alice's side — replaces ImportInviteDialog)

### Trigger

Identity dialog → "Accept Invite" button (only shown for unlocked identities). Identity UUID is passed as a prop — no identity picker step needed.

### Step 1 — Import

- Identity shown as read-only label ("Accepting as: Alice's Identity")
- Text input to paste a relay invite URL
- "or" divider
- "Load .swarm file" button → OS file picker

### Step 2 — Review

**Invite details (primary section):**
- Invited by: name + fingerprint (truncated, copyable)
- Role: badge showing `offered_role` (e.g., green "Writer" pill)
- Subtree: note title (if scoped)
- Expiry: formatted date or "No expiry"
- Relay server: shown only if invite was fetched via relay URL (derived from the URL, not from invite data)

**Workspace info (collapsible section):**
- Description, author name, org, homepage URL, license, language, tags
- All from existing `InviteFile` fields — just displaying what's already there

Buttons: "Decline" and "Next"

### Step 3 — Respond

**Channel picker:**
- If invite came via relay: relay option is pre-selected, shows the relay server URL
  - If Alice has an account on that server: shows account info with checkmark
  - If Alice does NOT have an account: inline signup form (email + password). Calls existing `register_relay_account` command. On success, proceeds with response.
  - Other relay accounts listed but inviter's server is highlighted/recommended
- File option always available as fallback

**On "Accept & Send":**
- Relay: sends response via relay, shows success
- File: OS save dialog, saves response `.swarm`

### No new Tauri commands needed

`register_relay_account` already exists for inline signup. `respond_to_invite` and `send_response_via_relay` handle the response sending.

## OnboardPeerDialog (simplified)

### What it shows

- **Peer card:** declared name + truncated public key (unchanged)
- **Scope reminder:** subtree name (unchanged)
- **Role display:** read-only badge showing `offered_role` from the invite — NOT a dropdown
- **Channel display:** read-only — "via relay (relay.example.com)" or "via file". Determined by how the response arrived (from `response_channel` on `ReceivedResponseInfo`). No `ChannelPicker`.

### Buttons

- **Reject** — rejects the peer
- **Grant & Sync** — auto-applies `set_permission` with the invite's `offered_role` on the scoped note, then sends snapshot:
  - If `response_channel == "relay"`: sends snapshot via the relay account that received the response
  - If `response_channel == "file"`: OS folder picker opens, snapshot saved there

### What gets removed

- Role picker dropdown (role is binding from invite)
- `ChannelPicker` component (channel is determined by response)
- "Later" button (no reason to defer with role pre-set)

### What gets absorbed

- `AcceptPeerDialog` functionality — since all invites are now scoped (no workspace-level invites), the separate `AcceptPeerDialog` path is no longer needed. Its fingerprint verification is dropped from the onboard flow for reduced friction.

## InviteManagerDialog (stripped down)

### What stays

- Invite list view — each invite shows: subtree name, offered role (new), expiry, use count, revoked status, relay URL if uploaded
- Per-invite actions: "Copy Link" (if relay), "Revoke", "Delete" (if revoked)
- "Purge Revoked" bulk action
- "Import Response" button + relay URL response fetch

### What gets removed

- "+ Create Invite" button
- "Share Invite Link" button
- "Upload to Relay" per-invite button
- `initialScope` prop

### Access

From `WorkspacePeersDialog` → "Manage Invites" button (unchanged). No longer opened by the context menu — the context menu triggers `InviteWorkflow` directly.

## Identity Dialog Changes

### Add "Accept Invite" button

Added to the selected identity's action bar (alongside Rename, Passphrase, Export, etc.). Only shown when identity is unlocked. Opens `AcceptInviteWorkflow` with `identityUuid` pre-set.

### AcceptedInvitesSection enhancement

Each accepted invite now also shows the offered role as a badge.

### Menu removal

- `menu.rs`: Remove "Accept Invite" from File menu
- `App.tsx`: Remove the menu event handler for accept-invite

## Cleanup Summary

### Components to delete

- `CreateInviteDialog.tsx`
- `ImportInviteDialog.tsx`
- `AcceptPeerDialog.tsx`
- `SwarmInviteDialog.tsx`

### Button removals

- `InviteManagerDialog`: "+ Create Invite", "Share Invite Link", "Upload to Relay" per-invite
- `WorkspacePeersDialog`: "Share Invite Link" footer button

### Locale changes (all 7 languages)

- Remove strings for deleted components and menu items
- Add strings for `InviteWorkflow`, `AcceptInviteWorkflow`, and updated `OnboardPeerDialog`
