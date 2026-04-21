# Changelog

All notable changes to Krillnotes will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Sync on close** — When closing a workspace with unsynchronized changes, the app prompts to sync with relay/folder peers before closing. A new "Sync on Close" setting in Settings → General offers three modes: Always sync, Ask before closing (default), Never sync. Includes a spinner overlay during sync and error recovery if sync fails (PR #135).
- **Note checkbox support** — Schemas can set `show_checkbox: true` to render an interactive checkbox in the tree view. Checked notes display with a strikethrough title. The `is_checked` state is a first-class field on the `Note` struct (like `title`), tracked by a dedicated `SetChecked` CRDT operation with full sync, export/import, and undo support. Rhai scripts can read `note.is_checked` in views/hooks and write via `set_checked(note_id, checked)` in `on_save` hooks (PR #134).
- **Built-in TodoItem schema** — A new system script (`TodoItem`) with `show_checkbox: true` and `is_leaf: true`, ideal for checklists and task lists.

### Fixed
- **Pending sync false positives** — `has_pending_ops_for_any_peer()` now excludes operations authored by the peer and operations received from the peer (echo prevention), matching the filters used by `generate_delta()`. Previously, ops received from a peer were counted as "pending", causing the sync indicator and sync-on-close dialog to trigger when there was nothing to send (PR #135).
- **Stale view tab on note switch** — Switching between note types with different view names no longer produces `render_view` errors. The view rendering effect now uses a ref-based guard to prevent firing with stale state from the previous note.

### Changed
- **Script category renamed "presentation" → "library"** — The internal DB/Rust category value now matches the frontend UI label. Includes a DB migration to update existing rows automatically (PR #130).

## [0.9.2] — 2026-03-31

### Fixed
- **Cross-peer relay routing** — PR #124 introduced per-device derived signing keys for relay registration, but peer-to-peer bundle routing still used identity public keys. Invite acceptances and onboarding snapshots were silently dropped. Fixed by registering the identity public key as an additional device key on the relay (PR #126).
- **Sync engine consuming Accept bundles** — The sync engine acknowledged Accept-mode bundles before the invite poller could process them, causing invite acceptances to disappear.
- **Snapshot relay routing** — `send_snapshot_via_relay` now populates `sender_device_id` and `recipient_device_ids` for proper relay routing.
- **Identity key appearing in My Devices list** — The identity public key registered for routing was showing as a phantom device in the "Send to My Device" dialog.
- **Synthetic invite save on imported identities** — Defensive directory creation prevents "No such file or directory" errors on freshly-imported identities.
- **ARM Linux release build** — Added missing `xdg-utils` dependency for AppImage bundling on `ubuntu-22.04-arm` runners.

## [0.9.1] — 2026-03-29

### Added
- **Multi-device sync** — Run the same identity on multiple machines with full relay routing between them (PR #124):
  - **Composite device IDs** — Device IDs are now `{identity_uuid}:{device_uuid}`, so two machines running the same identity have distinct addresses. A stable `device_uuid` is persisted per machine in the identity directory.
  - **`RegisterDevice` operation** — A new CRDT operation is emitted once per (workspace, device) pair on open, recording device UUID, human-readable device name (hostname), and identity public key. All peers eventually learn about each other's devices.
  - **Relay device routing** — `BundleHeader` now carries `sender_device_id` and `recipient_device_ids` so the relay can route bundles to specific devices. `list_bundles()` accepts a device ID filter parameter.
  - **My Devices UI** — Workspace Peers dialog groups same-identity peers into a "My Devices" section with simplified display (name, channel, sync status — no fingerprint or contact badge). Self-peers are excluded from the Share dialog since they already share identical access.
  - **Send to My Device** — Footer button in Workspace Peers with two paths: *Via Relay* (device picker → `send_self_snapshot_via_relay`) and *Export File* (snapshot addressed to own identity key). Enables bootstrapping a new device without going through the invite flow.
  - **Self-snapshot relay support** — `send_self_snapshot_via_relay` and `list_devices_on_relay` Tauri commands for discovering and syncing with your own devices on the relay.
- Sharing indicator visibility setting (`Off` / `Auto` / `On`) in Settings → Appearance — `Auto` (default) hides permission dots and shared-subtree icon when the workspace has no peers, keeping the tree clean for solo users (#111, PR #120)
- **Invite workflow redesign** — Streamlined the invite/onboard flow so role and channel are chosen upfront (#113, PR #123):
  - **InviteWorkflow** — New single-step dialog (right-click → "Invite to subtree") with role picker (Owner/Writer/Reader), expiry, and channel toggle (relay with account picker, or file). Replaces `CreateInviteDialog`.
  - **AcceptInviteWorkflow** — New 3-step wizard (import → review → respond) with invite details, role badge, collapsible workspace metadata, and inline relay signup. Also handles file-drop `.swarm` opens. Replaces `ImportInviteDialog`.
  - **Simplified OnboardPeerDialog** — Role and channel are now read-only (set at invite time), auto-applied on "Grant & Sync". Removed role picker, channel picker, and "Later" button.
  - **Role at invite time** — `offered_role` is signed into the invite wire format (Ed25519), carried through to accepted invites and received responses, and auto-applied during onboarding.
  - **Channel tracking** — `response_channel` and `relay_account_id` on received responses determine snapshot delivery routing. Relay channel is auto-configured on both inviter and invitee peers after onboarding.
  - **Accept Invite moved to Identity dialog** — Button shown for unlocked identities; removed from File menu.
  - **InviteManagerDialog stripped to list-only** — No more create/share/upload buttons; role badges shown on each invite row.
  - **Remove button on accepted invites** — Clean up old accepted invite entries from the Identity dialog.
  - **Blake3 fingerprint words** in OnboardPeerDialog (e.g., "ghost-heavy-deliver-inject") instead of truncated public keys.
- **TextNote content view** — TextNote schema now registers a markdown-style content view tab via `register_view()`, giving TextNote a dedicated reading pane alongside the fields tab.

### Changed
- **Scripts reorganised** — Only `TextNote` ships as a bundled system script. All other schemas (contacts, project, recipe, product) and templates (book-collection, photo-note, zettelkasten) moved to `example-scripts/` so new workspaces start clean. Task schema merged into project with parent/child constraints.

### Fixed
- Relay account dropdown in workspace peer list now shows the stored relay server on load instead of appearing empty (#114, PR #119)
- ShareDialog now uses CSS variables to respect the active app theme setting (PR #124)
- Relay delta sync: cleared `recipient_device_ids` in `send_bundle()` to fix bundles being invisible to recipients, and added prefix fallback in `find_by_url` so relay accounts are matched correctly when stored as full invite URLs (PR #124)

## [0.9.0] — 2026-03-23

> **Feature-complete release candidate.** This release adds role-based access control (RBAC) with subtree-level permissions, background sync polling, invite-to-subtree scoping, and one-click relay invite sharing. Every workspace mutation is now authorized, signed, and syncable across devices. Cross-platform testing on Windows and Linux is in progress ahead of v1.0.

### Added

#### Role-based access control (RBAC)
- **Subtree-level permissions** — Workspace owners can grant peers granular access to subtrees with five roles: **owner**, **admin**, **editor**, **reader**, and **none**. Permissions cascade from parent to child — the nearest explicit grant wins. Root owner always has full access. (PRs #106–#110)
- **`krillnotes-rbac` crate** — A new optional crate implementing the `PermissionGate` trait. Pluggable authorization layer; all note and script mutations go through `authorize()`. Ships with `RbacGate` (production) and `AllowAllGate` (tests/RBAC-disabled).
- **Tree-walk permission resolver** — Effective roles are computed by walking from the note to the root, inheriting the nearest explicit grant. Tested with full lifecycle integration tests.
- **Permission management UI** — Role dots on tree nodes show effective access level (colour-coded). Share anchor icons (🔗) mark nodes with explicit grants. Ghost ancestor styling for nodes visible only as path context. (PR #110)
- **ShareDialog** — Peer picker + role selector for granting subtree access. Accessible from the Info panel "Shared with" section and the context menu. (PR #110)
- **CascadePreviewDialog** — Impact preview before demotion or revocation, showing affected peers and notes with opt-in checkboxes. (PR #110)
- **Role-aware action disabling** — Context menu, Info panel edit/delete controls, and toolbar actions are disabled based on the user's effective role. Non-owners cannot add siblings at root level. (PR #110)
- **"Shared with" section in Info panel** — Shows who has access to the selected note, their roles, and grant sources. Grants refresh after permission mutations. (PR #110)
- **`RemovePeer` and `TransferRootOwnership` operation variants** — CRDT operations for peer lifecycle management.
- **Protocol version in SwarmHeader** — `.swarm` bundles carry a protocol version; mismatched bundles are rejected with a clear error. Protocol is embedded inside the encrypted payload for tamper-proof enforcement.

#### Sync improvements
- **Background sync polling** — Relay and folder channels automatically poll for incoming operations and snapshots. Polling covers all unlocked identities and refreshes conditions periodically. (PR #105)
- **Send snapshot via relay** — Workspace owners can send snapshots to peers via relay, not just via file. (PR #105)
- **Create workspace from accepted invite** — After accepting a peer's invite and receiving their snapshot, a "Create Workspace" button creates the workspace directly from the accepted invites section. (PR #105)
- **Invite-to-subtree** — Invites can scope access to a specific subtree. `OnboardPeerDialog` wired into Workspace Peers dialog. Permission ops are included in snapshots for peer onboarding. (PR #109)

#### Relay invite sharing
- **One-click relay invite sharing** — "Share Invite Link" button in Workspace Peers and Invite Manager creates an invite, uploads it to the relay, and copies the URL to clipboard. No `.swarm` file exchange needed. (PR #104)
- **File → Accept Invite** — Top-level menu action for invitees to accept an invite by pasting a relay URL or opening a `.swarm` file, without needing a workspace open first.
- **Full relay invite round-trip** — Inviter shares link → invitee fetches and responds via relay → inviter imports response from link. Both sides exchange URLs instead of files.
- **Relay account fallback** — If no relay account is configured, the registration dialog opens automatically and continues the action on success.

### Changed
- **RBAC is feature-gated** — `krillnotes-rbac` is an optional dependency; `permission_gate` is non-optional on `Workspace` (uses `AllowAllGate` when RBAC is disabled).
- `ImportInviteDialog` works standalone (identity selector when no workspace context)
- `InviteManagerDialog` shows relay URLs on invites with "Copy Link" and "Upload to Relay" actions
- `has_relay_credentials` accepts optional `identityUuid` parameter for standalone use

### Fixed
- **Read-access filtering on note queries** — Non-owners only see notes they have permission to access.
- **Permission grant cleanup** — Grants are removed when their anchor note is deleted.
- **MoveNote destination scope check** — Moving a note to a subtree the user doesn't have write access to is blocked.
- **Sync never blocked by RBAC** — Replication proceeds unconditionally; RBAC controls visibility only, preventing split-brain in CRDT sync.
- **Peer watermark reset on `set_permission`** — Ensures a full resend so the peer receives all newly-visible operations.
- **`granted_by` / `revoked_by` populated** — `SetPermission` and `RevokePermission` ops now correctly record the acting user.
- WKWebView clipboard `NotAllowedError` — falls back to showing URL in a selectable text field
- `InviteManagerDialog` uses CSS variable theme system (fixes unreadable dark mode)
- Various dialog z-index and theming fixes across invite and onboarding flows

## [0.4.1] — 2026-03-15

### Changed
- **Relay accounts moved to Identity Manager** — Relay server credentials are now managed per-identity (like contacts) instead of per-peer. A new "Relays" button in Identity Manager opens a relay account book for registering, viewing, and deleting relay accounts. Workspace peer configuration uses a simple dropdown picker instead of the old Configure Relay dialog. (PR #103)
- **Automatic relay session renewal** — Relay account passwords are stored (encrypted) so sessions are automatically refreshed on identity unlock — no more re-entering credentials when sessions expire.
- **Old relay credentials auto-migrated** — Existing relay credentials are automatically migrated to the new per-identity format on first unlock.

### Fixed
- **Folder sync addressing** — Recipient-prefixed filenames and inbox filtering so bundles in a shared folder are only picked up by the intended peer, with base64 slash sanitization for safe path handling
- **Watermark feedback loop** — ACK now tracks the last bundle op (not just applied ops), eliminating infinite full-resend loops in multi-device topologies
- **Poll order** — Inbound-first processing prevents false ACK-behind resets from one-cycle timing lag
- **0-op bundle suppression** — No bundles sent when idle; no ACK ping-pong between peers
- **Echo prevention** — `received_from_peer` tracking prevents hub nodes from echoing forwarded ops back to the original sender
- **HLC-ordered delta application** — Delta ops from all channels are collected and sorted by HLC timestamp before applying, preventing watermark issues when bundles arrive out of order across channels

## [0.4.0] — 2026-03-15

> **Sync is here.** This release adds multi-device workspace sync — via a relay server, a shared folder, or manual `.swarm` file exchange. It also introduces an encrypted contact book, a peer invite workflow, workspace snapshots, and owner-only script enforcement.

### Added

#### Sync engine
- **Three sync channels** — Sync with peers via **relay** (HTTP relay server with mailbox routing), **folder** (shared local/network folder), or **manual** (export/import `.swarm` delta files by hand). Each peer can use a different channel; switch at any time from the Workspace Peers dialog.
- **Relay sync** — Register an account on a relay server, bind your device key, and exchange encrypted delta bundles over HTTP. Session tokens are persisted locally (AES-256-GCM encrypted); expired sessions prompt re-login. Configure Relay dialog with register/login tabs. Relay credentials stored per-identity under `~/.config/krillnotes/identities/<uuid>/relay/`.
- **Folder sync** — Point a peer at a shared directory (local disk, NAS, Dropbox, etc.) and Krillnotes writes `.swarm` delta files into it. The peer's next poll picks up the file, applies it, and deletes the consumed bundle.
- **Manual delta export** — "Create delta Swarm" in the Edit menu opens `CreateDeltaDialog` listing accepted peers with their last-sync operation ID. One `.swarm` file is generated per selected peer, encrypted for that peer's public key. The recipient opens the file to apply the delta.
- **Sync Now button** — One-click sync from the Workspace Peers dialog triggers a full send-and-receive cycle across all configured channels.
- **Force Resync** — Per-peer "↺" button in the Workspace Peers dialog resets the watermark so the next sync re-sends all operations from the last snapshot baseline.
- **Delivery-confirmed watermarks** — `last_sent_op` only advances when the transport confirms the bundle was routed, preventing silent data loss when a relay skips unknown or unverified devices.
- **ACK-based watermark self-correction** — Each outbound delta carries `ack_operation_id` (the last op received from that peer). When a peer sees that the remote's ACK is behind its own `last_sent_op`, it rewinds its watermark automatically — peers self-heal from missed deltas without manual intervention.
- **Sync event streaming** — `SyncEvent` enum (`DeltaSent`, `BundleApplied`, `AuthExpired`, `SyncError`, `IngestError`, `SendSkipped`) published to the frontend for real-time status updates.
- **Peer sync status tracking** — Each peer tracks sync state (`idle`, `syncing`, `error`, `auth_expired`, `not_delivered`) with detail and error messages, displayed as status badges in the Workspace Peers dialog.

#### Peer management and invites
- **Per-identity encrypted contact book** — Contacts are stored per identity under `~/.config/krillnotes/identities/<uuid>/contacts/` as AES-256-GCM encrypted blobs. Encryption key derived via HKDF-SHA256 from the identity seed; only in memory while unlocked. Full CRUD via six Tauri commands. UI: `ContactBookDialog` with search, trust-level badges, `AddContactDialog` (live fingerprint preview, in-person verification gate), and `EditContactDialog` (local name, notes, delete). Accessible via "Contacts (n)" button in Identity Manager.
- **Workspace Peers dialog** — New "Workspace Peers" item in the Edit menu lists all sync peers with resolved display name, 4-word BIP-39 fingerprint, trust-level badge, channel type, sync status, and last-sync time. Actions: remove peer (inline confirmation), add contact as peer, switch channel, configure relay, force resync, and create invite.
- **Multi-use signed invite flow** — `InviteManager` with `InviteRecord`, `InviteFile`, `InviteResponseFile` structs. Ed25519 signing/verification with canonical JSON. Create/list/revoke invites; full invite→response round-trip. Seven Tauri commands. Four React dialogs (`CreateInviteDialog`, `AcceptPeerDialog`, `ImportInviteDialog`, `InviteManagerDialog`). Localised in all 7 languages.
- **Workspace snapshot exchange** — A workspace owner can send a full snapshot to a new peer via a `.swarm` file. `WorkspaceSnapshot` struct with `to_snapshot_json` / `import_snapshot_json` for complete workspace serialisation. Snapshot baseline sets both watermarks so bidirectional delta sync works immediately.
- **`.swarm` file association** — OS registers `.swarm` files with Krillnotes; double-click opens the correct dialog (invite, snapshot, or delta).
- **File > Invite Peer and Open .swarm File menu items**
- **Show and copy public key and fingerprint in Identity Manager** — for sharing with peers.
- Auto-prompt to unlock required identity when opening an invite or snapshot file.

#### Owner-only script enforcement
- **Owner-only scripts** — `owner_pubkey` stored in `workspace_meta` at creation. All six script mutation methods (`create`, `update`, `delete`, `toggle`, `reorder`, `reorder_all`) return `NotOwner` error for non-owners. Non-owner script ops are skipped during sync ingest (logged but not applied). `.swarm` bundle headers embed and validate `owner_pubkey`. UI disables script mutation controls (save, delete, new, toggle, drag-reorder, editor) for non-owners with an info banner. "Owner" badge shown in Workspace Peers dialog.

#### UI improvements
- **Hover indicator caret on tree nodes** — A subtle `›` is shown on the right of tree node rows when the note type has an `on_hover` hook or `showOnHover` fields defined.
- **Identity/contact name in note Info panel** — Created and Modified timestamps show the author's display name inline (local identity first, then contact address book, then 8-char fingerprint for unknown keys).
- **`resolve_identity_name` Tauri command** — Resolves a public key to a display name; used by both the info panel and the operations log.
- **`is_leaf` schema option** — When `is_leaf: true` is set on a schema, notes of that type cannot have children. Blocked in core (`create_note`, `move_note`, `deep_copy_note`) and observed in the UI ("Add Child" and "Paste as Child" are greyed out; drag-drop onto leaf notes is blocked).

#### Swarm protocol internals
- **SwarmHeader codec and bundle-level signatures** — All `.swarm` file payloads are signed with Ed25519 and verified on open.
- **Hybrid encryption for `.swarm` payloads** — X25519 key exchange + AES-256-GCM payload encryption.
- **`ack_operation_id` field in SwarmHeader** — Threaded through delta codec for watermark self-correction.
- **`owner_pubkey` field in SwarmHeader** — Embedded in all bundle types; validated on receive.
- **`SetPermission`, `RevokePermission`, `JoinWorkspace` operation variants** — CRDT operations for future RBAC sync.
- **`peer_registry` table** — Tracks known peers and their sync state per workspace (device ID, identity ID, channel type, channel params, watermarks, sync status).
- **Structured logging** — `log` crate macros throughout sync engine, relay client, folder channel, and Tauri commands (replaces `eprintln`).

### Fixed
- **Rhai engine reloaded after sync ingest and snapshot import** — Applying script operations via delta sync or snapshot import now reloads the Rhai engine so new/updated schemas take effect immediately without restarting the app.
- **Relay recipient device key encoding** — `RelayChannel` now converts peer device keys to hex (matching the relay server's format) instead of sending internal identity placeholders, fixing silent bundle drops.
- **Relay mailbox registration on poll** — `receive_bundles` now calls `ensure_mailbox()` so the relay routes incoming bundles to this account (was silently dropping them).
- **Tokio runtime drop panic** — `poll_sync` restructured to run the sync engine inside `spawn_blocking`, releasing all `MutexGuard`s before spawning, preventing a panic when reqwest's internal Tokio runtime is dropped on an async thread.
- **Poisoned mutex on window close** — Destroyed window handler uses `unwrap_or_else(|e| e.into_inner())` instead of `expect()`, preventing a secondary panic from a poisoned mutex during cleanup.
- **`generate_delta` with no watermark** — Force Resync clears `last_sent_op` to `None`. Previously `generate_delta` rejected this with "snapshot must precede delta"; now it returns all ops and the recipient's `INSERT OR IGNORE` handles duplicates.
- **Workspace Properties dialog crash** — `meta.tags` is now guarded with `?? []` before calling `.join()` (was `TypeError: undefined is not an object`).
- **Script category preserved on export/import** — `ScriptManifestEntry` now includes the `category` field so schema vs. presentation classification survives a `.krillnotes` archive round-trip. Previously all scripts were imported as `"presentation"` (PR #89).
- Library script functions are now visible to schema scripts and their hooks — library source is prepended when compiling schema scripts.
- `register_view` and `register_menu` no longer produce duplicate tabs/entries when a library script is loaded alongside multiple schema scripts.
- Snapshot import no longer seeds a default root note, preserving the imported workspace structure.
- Identity file path resolved relative to `config_dir` in `get_identity_public_key`.
- `source_display_name` correctly populated in invite bundles.
- Unlocked identity UUID refreshes when Identity Manager closes or Swarm dialog opens.
- Schema script pre-validation now sets the loading category so library functions are available during validation.
- Hover tooltip no longer appears for notes whose type has no `on_hover` hook and no `showOnHover` fields.
- Operations log now checks the contact address book when resolving author names, in addition to local identities.
- Note Info panel metadata uses the same `dl/dt/dd` grid layout as the fields view and is hidden on custom view tabs.

### Changed
- **Breaking (Rhai scripts):** `note.node_type` renamed to `note.schema` in all Rhai script contexts.
  Update any user scripts that reference `note.node_type` → `note.schema`.
- `Note` JSON key changed from `nodeType` to `schema` in workspace exports.
  Old `.krillnotes` archives with `nodeType` are still importable (backward compat preserved via serde alias).
- **Breaking (Rhai scripts):** Schema constraint keys renamed — `allowed_parent_types` → `allowed_parent_schemas`,
  `allowed_children_types` → `allowed_children_schemas`. Update any schema definitions that use the old keys.
- **Breaking (Rhai scripts):** `note_link` field option `target_type` renamed to `target_schema`.
  Update any schema definitions that use `target_type` on a `note_link` field.
- **Identity storage refactored** — `identities/<uuid>.json` moved to `identities/<uuid>/identity.json`; per-workspace `binding.json` replaces `identity_settings.json.workspaces`. Auto-migrates on first launch.
- **Codebase refactored** — `krillnotes-core` large files split into focused modules; `lib.rs` Tauri commands split into `commands/` directory with per-domain modules; frontend hooks extracted from large components (PRs #97–#99).

## [0.3.0] — 2026-03-07

> **Breaking changes:** This release introduces an identity-based authentication system (workspaces from v0.2.x must be exported and re-imported), a new scripting API (`save_note` replaces `update_note`, `register_view`/`register_hover`/`register_menu` replace inline hooks, schema versioning is now required), and HLC-based operation timestamps that update the database schema. Additionally, the project is now licensed under MPL-2.0 (previously MIT).

### Added
- **Operation detail panel** — Clicking any row in the Operations Log now opens a side panel showing all fields stored for that operation. The dialog expands from 700 px to 1080 px; clicking the selected row or the ✕ button closes the panel. Author-key fields (`created_by`, `modified_by`, etc.) display the resolved identity display name below the raw public-key hash.
- **Identity system** — A cryptographic identity (an Ed25519 keypair protected by an Argon2id-derived passphrase) now manages workspace access. Each workspace is bound to an identity; the workspace's randomly-generated database password is stored encrypted under the identity key. You unlock your identity once per session with your passphrase, and all bound workspaces open without any additional password prompts.
- **Identity Manager** — A new Identity Manager dialog (accessible from Settings) lets you create, rename, unlock, lock, and delete identities. Each identity shows its UUID and the list of workspaces bound to it.
- **`.swarmid` export/import** — Identities can be exported as a portable `.swarmid` file (encrypted JSON containing your key material). Import a `.swarmid` file on another device to access the same workspaces. On import, an existing identity with the same UUID can be overwritten while preserving all workspace bindings.
- **Workspace Manager** — Replaces the minimal Open Workspace dialog with a full manager. The list shows each workspace's name, last-modified date, and size on disk, sortable by name or modified date. Selecting a workspace reveals an info panel with created date, note count, attachment count, and size — all read from an unencrypted `info.json` sidecar so no password is required just to view metadata. Per-workspace actions: **Open** (requires the bound identity to be unlocked; also triggered by double-clicking a row), **Duplicate** (uses the export→import pipeline; prompts for new name), **Delete** (irreversible red confirmation banner; blocked if the workspace is currently open), and **New** (opens the New Workspace dialog and binds the new workspace to your unlocked identity).
- **Random workspace passwords** — New workspaces no longer ask for a user-visible password. A cryptographically random 32-byte base64 key is generated at creation time, used as the SQLCipher database password, and immediately encrypted under the bound identity. Users never see or type a workspace password.
- **HLC timestamps on operations** — Every mutation is now timestamped with a Hybrid Logical Clock (`wall_ms`, `counter`, `node_id`) instead of a plain Unix integer. HLC timestamps provide causal ordering guarantees even when clocks skew across devices, which is a prerequisite for CRDT merge.
- **Ed25519-signed operations** — Each mutation carries an Ed25519 signature produced by the unlocked identity's signing key. Operations can be verified against the author's public key, laying the foundation for trustless multi-device sync.
- **`UpdateNote` and `SetTags` operation variants** — Title changes now emit a dedicated `UpdateNote` operation (separate from field-level `UpdateField`) to enable last-write-wins conflict resolution on note titles. Tag assignments now emit `SetTags` and are recorded in the operations log for the first time.
- **Author display in Operations Log** — Each row in the Operations Log now shows a short author identifier (first 8 characters of the base64-encoded public key), resolved to the identity's display name when the identity is loaded.
- **Gated operations model (`SaveTransaction`)** — Replaces direct-mutation `on_save` hooks with a transactional API. Scripts now use `set_field()`, `set_title()`, `reject()`, and `commit()` to express mutations declaratively. A 7-step save pipeline (`save_note_with_pipeline`) runs visibility → validate → required → update, ensuring hooks cannot leave a note in an inconsistent state.
- **Field groups** — Schemas can define `field_groups` in `schema()` to visually organise related fields under collapsible sections. Each group supports an optional `visible` closure that dynamically shows or hides the section based on the current field values (e.g. show "Completion details" only when status is "done").
- **Field-level `validate` closures** — Individual field definitions accept a `validate: |v| ...` closure that returns an error string or `()`. Validation runs on-blur in the frontend (inline error under the field) and as a hard gate inside `set_field()` during saves.
- **Note-level `reject()`** — `on_save` hooks can call `reject("message")` to abort a save with a structured error. The frontend displays rejected messages in a note-level error banner above the fields.
- **Script categories** — Scripts are now divided into two categories: **Schema** (`.schema.rhai`) and **Library/Presentation** (`.rhai`). Schema scripts define note types via `schema()`. Presentation scripts define views, hover renderers, and context-menu actions via `register_view()`, `register_hover()`, and `register_menu()`. Calling `schema()` from a presentation script raises a hard error.
- **Two-phase script loading** — On workspace open, presentation scripts load first (Phase A), then schema scripts (Phase B), then deferred view/hover/menu bindings are resolved (Phase C). Library helper functions defined in `.rhai` files are available when schema `on_save` hooks execute.
- **`register_view(type, label, closure)` / `register_view(type, label, options, closure)`** — Registers a named view tab for a note type from a presentation script. Replaces the `on_view` key inside `schema()`. Closures have access to all query functions and display helpers. `display_first: true` pushes the tab to the leftmost position.
- **`register_hover(type, closure)`** — Registers a hover tooltip renderer for a note type from a presentation script. Replaces the `on_hover` key inside `schema()`. Last registration wins.
- **`register_menu(label, types, closure)`** — Registers a context-menu action for one or more note types from a presentation script. Replaces `add_tree_action()`. Closures use the SaveTransaction API for mutations.
- **Tabbed view mode** — When a schema has registered views, the note detail panel shows a tab bar. Custom view tabs appear in registration order; `display_first: true` tabs are leftmost; the Fields tab is always present and always rightmost. No tab bar is shown for types with no registered views.
- **Script Manager category badges and creation flow** — Each script in the manager shows a coloured badge: blue **Schema** or amber **Library**. The "New Script" dialog includes a category selector with starter templates for each category. Scripts with unresolved bindings show a warning icon.
- **Schema versioning** — `schema()` now requires a `version: N` key (integer ≥ 1). All built-in schemas and templates ship at version 1. Registering a schema at a version lower than the currently registered version is a hard error at load time.
- **Data migration closures** — Schemas can declare a `migrate` map keyed by target version number. Each closure receives a note map (`title`, `fields`) and mutates it in place. Migration closures run automatically on workspace open for any notes whose `schema_version` is below the current schema version.
- **Batch migration on load** — After scripts load (Phase D), Krillnotes queries stale notes and runs migration closures in a single transaction per schema type. Multi-version jumps chain closures in order (e.g. a note at v1 against a v3 schema runs the v2 closure then the v3 closure). Any migration error rolls back the entire batch for that schema type; other types continue independently.
- **`schema_version` on notes** — Each note carries a `schema_version` integer stamped with the schema's current version at create time and updated after successful save.
- **`UpdateSchema` operation** — A new operation variant logged once per schema type after a successful batch migration, recording `schema_name`, `from_version`, `to_version`, and `notes_migrated`.
- **Migration toast notification** — After a batch migration, a transient toast appears: *"Contact schema updated — 12 notes migrated to version 3"*. Auto-dismisses after a few seconds.

### Changed
- **License: MIT → MPL-2.0** — Krillnotes is now published under the [Mozilla Public License 2.0](https://mozilla.org/MPL/2.0/). Existing integrations that relied on the MIT license should review the MPL-2.0 terms (file-level copyleft; compatible with GPL).
- **Workspace opening requires an unlocked identity** — `EnterPasswordDialog` and `SetPasswordDialog` are removed. Opening a workspace now requires unlocking the bound identity first. If no identity is unlocked, the Workspace Manager prompts you to unlock one before opening.
- **Note positions changed from integer to float** — `notes.position` in the database is now a `REAL` (f64) column. This enables future fractional mid-point insertion for CRDT reordering without rewriting sibling positions. Existing positions are migrated automatically.
- **Operations table schema updated** — The `timestamp` column is replaced by three HLC columns (`timestamp_wall_ms`, `timestamp_counter`, `timestamp_node_id`). A new `hlc_state` table persists the HLC clock state across sessions. Existing workspaces are migrated automatically on first open.
- **`HashMap` → `BTreeMap` for note fields** — `Note.fields`, `CreateNote.fields`, and related action types now use `BTreeMap` to guarantee deterministic serialization order. This is required for reproducible Ed25519 signatures across processes.
- **`on_save` hook API** — All `on_save` hooks (system scripts and templates) have been migrated from direct note mutation to the new `SaveTransaction` gated model. The `on_add_child` hook is also migrated, with both parent and child pre-seeded into the transaction.
- **`save_note` replaces `update_note` IPC** — The frontend now calls `save_note` instead of `update_note`, which runs the full save pipeline including validation and hooks. The old `update_note` command is removed.
- **`on_view`, `on_hover`, and `add_tree_action` removed** — These APIs no longer exist. All system scripts and templates have been migrated to the new split-file format (`.schema.rhai` + `.rhai`) using `register_view`, `register_hover`, and `register_menu`.
- **`category` column on `user_scripts`** — A `category TEXT NOT NULL DEFAULT 'presentation'` column is added to the `user_scripts` table. Existing user scripts default to `"presentation"`.
- **Version guard on schema registration** — Re-registering an existing schema with a lower version number raises a hard error at load time. Same version allows hooks and fields to be updated freely; higher version triggers Phase D migration.
- **`schema_version` column in `notes` table** — DDL updated to include `schema_version INTEGER NOT NULL DEFAULT 1`. Existing notes default to version 1.

### Fixed
- **Serde camelCase on `SaveResult::ValidationErrors`** — Added explicit `#[serde(rename)]` attributes for `fieldErrors`, `noteErrors`, `previewTitle`, and `previewFields` fields. Enum-level `rename_all` only renames variant tags, not struct variant fields.
- **`evaluate_group_visibility` and `validate_field` invoke parameters** — Fixed frontend invoke calls to pass `schemaName` instead of `noteId`, matching the Tauri command signatures.

---

## [0.2.6] — 2026-03-04

### Added
- **Undo / Redo** — Cmd+Z undoes the most recent note-tree action; Cmd+Shift+Z redoes it. Toolbar buttons are also available. Supported operations: note create, title and field edits, delete (with full subtree restored), move / reorder, and script create / update / delete. Tree hook side-effects (e.g. auto-entering a title immediately after creating a note) are collapsed into a single undo step so one Cmd+Z reverses the whole action. The history limit is configurable in Settings (default 50, max 500) and stored per workspace in `workspace_meta`.
- **Separate script editor undo** — The CodeMirror editor in the Script Manager maintains its own independent undo history. Cmd+Z inside the editor undoes text changes within the editor only and does not affect the note-tree undo stack.
- **Attachment Restore** — Deleting an attachment now shows a "Recently deleted" strip below the attachment list with a per-item Restore button. Deleted attachments can be recovered for the duration of the app session, including after navigating away from the note and returning.

### Changed
- **Operations log always active** — The operations log is now populated for every workspace, regardless of sync settings. Previously it was gated behind sync being enabled (v0.2.5 change); it must be unconditionally active because undo/redo is recorded as first-class `RetractOperation` entries in the same log.

---

## [0.2.5] — 2026-03-02

### Added
- **File attachments** — Any note can have files attached to it. Attachments are encrypted at rest alongside the workspace database using ChaCha20-Poly1305. A drag-and-drop attachment panel in the InfoPanel lets you attach, preview (images show a thumbnail), open, and delete files. Attachments are included in workspace export/import archives and re-encrypted on import. A configurable max attachment size guard is enforced at attach time.
- **`file` field type** — Schema fields can now be typed `file`, storing a reference to a single attached file. In view mode, images render as a thumbnail; other files show a paperclip icon and filename. In edit mode a file picker opens filtered by `allowed_types` MIME types. Replacing a file atomically attaches the new one before deleting the old.
- **`display_image(source, width, alt)` Rhai helper** — Embeds an attached image directly in `on_view` or `on_hover` hook output. `source` is either `"field:fieldName"` (reads the UUID from a `file` field) or `"attach:filename"` (finds by filename). Images are base64-encoded server-side so the frontend renders them without any asynchronous hydration step.
- **`display_download_link(source, label)` Rhai helper** — Renders a clickable download link for an attachment in `on_view` output. Clicking the link decrypts the file on demand and triggers a browser download.
- **`{{image: …}}` markdown syntax** — Textarea fields rendered as markdown now support inline image blocks: `{{image: field:cover, width: 400, alt: My caption}}` or `{{image: attach:photo.png}}`. Images are resolved and embedded server-side during rendering.
- **`get_attachments(note_id)` query function** — Returns attachment metadata for any note. Available in `on_view`, `on_hover`, and `add_tree_action` closures.
- **`stars(value)` / `stars(value, max)` display helpers** — Renders a numeric rating as filled (★) and empty (☆) star characters in `on_view` hook output. Defaults to 5 stars; pass a second argument to use a different scale. Returns `"—"` for a zero or negative value, matching the default field view.
- **Internationalisation (i18n)** — 7 language packs ship out of the box: English, German, French, Spanish, Japanese, Korean, and Simplified Chinese. The active language is chosen from a new dropdown in Settings and takes effect live without restarting the app. Dates and numbers are formatted using the locale's conventions (via `Intl.DateTimeFormat` / `Intl.NumberFormat`).
- **Native menu i18n** — The Tauri native application menu (File, Edit, Tools, View, Help) is also translated. All 20 menu-item labels are read from the same locale JSON files as the React frontend. Changing the language in Settings rebuilds and reapplies all open window menus immediately — no restart required. Locale data is embedded at compile time by `build.rs`, so there is zero runtime I/O overhead.
- **Hover tooltip on tree nodes** — Hovering a tree node for 600ms shows a compact speech-bubble tooltip to the right of the tree panel, without needing to navigate to the note. Two render paths are supported: mark any field with `show_on_hover: true` for an instant inline preview (no IPC), or define an `on_hover` hook in `schema()` to return fully custom HTML via the Rhai scripting engine. The tooltip is a React portal, position-clamped to the viewport, with a left-pointing spike that tracks the hovered row. It dismisses immediately on mouse-leave, click, or drag start.
- **`on_hover` hook** — A new optional hook inside `schema()` blocks. Like `on_view`, it receives a note map and has access to all query functions (`get_children`, `get_notes_for_tag`, etc.) and display helpers (`field`, `stack`, `markdown`, …). The return value is rendered as HTML in the tooltip.
- **`show_on_hover` field flag** — Fields defined with `show_on_hover: true` are surfaced in the hover tooltip without any scripting. Useful for quick previews of a single key field (e.g. a body or description).
- **Zettelkasten template updated** — The bundled `zettelkasten.rhai` now demos both hover paths: Zettel notes show the body field on hover; Kasten folders show a live child-count badge via `on_hover`.
- **Appearance tab in Settings** — Appearance settings (language, light/dark mode, and theme pickers) have been moved from the General tab into their own dedicated Appearance tab. The Settings dialog now has three tabs: General, Appearance, and Sync.
- **Sync tab in Settings** — A locked Sync placeholder tab has been added to the Settings dialog in preparation for the upcoming sync feature.

### Fixed
- **Editor scroll in dialogs** — The CodeMirror script editor inside the Manage Themes and Script Manager dialogs now scrolls correctly. The fix uses a definite `height` instead of `max-height` on the dialog container and adds `will-change: transform` to anchor macOS overlay scrollbars to the correct compositing layer.
- **Cmd+X and Cmd+A in text fields** — Cut and Select All keyboard shortcuts now work correctly on macOS. Previously these were no-ops because the native menu bar was missing `PredefinedMenuItem::cut` and `select_all` entries.
- **Sync settings not translated** — The General and Sync tab labels, and the Sync placeholder text, were displayed in English regardless of the selected language. All six non-English language packs (de, fr, es, ja, ko, zh) now include correct translations for these strings.

### Changed
- **Operations log gated behind sync** — The operations log is no longer populated unless sync is enabled. Since sync is not yet implemented, the log is always empty and the Operations Log menu item is permanently greyed out until sync ships.

---

## [0.2.4] — 2026-02-27

### Added
- **Theme support** — Choose between Light, Dark, and System (follows OS preference) modes from Settings. The active theme applies to all open workspace windows simultaneously; changing the theme in one window instantly updates every other open window.
- **Manage Themes dialog** — Browse, preview, create, edit, and delete custom `.krilltheme` files from a dedicated dialog in Settings. Built-in Light and Dark themes are always available as a baseline.
- **Import theme from file** — A new "Import from file…" button in the Manage Themes dialog lets you load a `.krilltheme` file from disk directly into the editor. If a theme with the same name already exists, a warning banner appears and the Save button becomes "Replace", with a confirmation dialog before overwriting.
- **Import script from file** — A matching "Import from file…" button in the Script Manager loads a `.rhai` file from disk into the script editor. Conflict detection is by `@name` front-matter; same replace-with-confirm flow applies.
- **Split Add Note** — The "Add Note" button is now split into three distinct actions — **Add Child**, **Add Sibling**, and **Add Root Note** — eliminating the type-selection dialog when only one target position makes sense.

### Fixed
- **Theme settings are now application-wide** — Theme mode (light/dark/system) is stored in the shared `settings.json` and applies to all workspaces. Previously, opening a new workspace window could show the wrong theme because the Settings dialog was clobbering the theme fields on every save.
- **Settings save no longer resets theme** — `update_settings` now accepts a partial patch and merges it onto the current settings on disk, so callers that only update workspace directory or password-caching cannot inadvertently reset unrelated fields to their defaults.
- **Workspace menu items disabled until a workspace is open** — File › Export Workspace and other workspace-specific menu items are now greyed out on the initial launch screen and only enabled once a workspace window is open.
- **`window.confirm()` replaced with async dialog** — Native `window.confirm()` is non-blocking in Tauri's WKWebView on macOS (always returns `true` immediately). All confirmation dialogs now use `await confirm()` from `@tauri-apps/plugin-dialog`, fixing silent data-loss on destructive actions.
- **`.krillnotes` file format** — Export archives now use the `.krillnotes` extension. The underlying format is unchanged (standard zip); only the file extension and dialog filters have changed.
- **Importing older archives** — Archives exported before the tags feature (v0.2.3) no longer fail to import. The missing `tags` field on notes now defaults to an empty list instead of causing a deserialisation error.

### Changed
- **App renamed to Krillnotes** — The application bundle, window title, and bundle identifier are now `Krillnotes` / `com.careck.krillnotes` (previously `krillnotes-desktop` / `com.careck.krillnotes-desktop`).

---

## [0.2.3] — 2026-02-26

### Added
- **`note_link` field type** — A new field type that stores a reference to another note by its ID. In edit mode an inline search dropdown lets you find and link a note by title or any text field; an optional `target_type` restricts the picker to notes of a specific schema type. In view mode (default and `on_view` hooks) the linked note's title is rendered as a clickable navigation link. If the linked note is deleted, the field is automatically set to null in all source notes.
- **`get_notes_with_link(note_id)` query function** — Returns all notes that have any `note_link` field pointing to the given note ID. Available in `on_view` hooks and `add_tree_action` closures. Use this to display backlinks on a target note (e.g. show all Tasks that link to a Project).
- **Tags** — Any note can carry free-form tags. Add and remove tags from the tag pill editor in the InfoPanel. Tag pills are shown in the default note view. A resizable tag cloud panel in the tree sidebar lets you browse all tags in the workspace at a glance.
- **Tag search** — The search bar now matches tags in addition to note titles and text fields.
- **Template gallery** — `templates/` ships two ready-to-use template scripts: `book_collection.rhai` (a library organiser with an `on_view` table and sort actions) and `zettelkasten.rhai` (an atomic-note system with auto-titling and related-note discovery via shared tags). Copy a template into the Script Manager to activate it.
- **`note.tags` in `on_view` hooks** — The note map passed to `on_view` now includes a `tags` array, enabling scripts to read and display the note's tags.
- **`render_tags(tags)` display helper** — Renders a `note.tags` array as coloured pill badges.
- **`get_notes_for_tag(tags)` query function** — Returns all notes that carry any of the given tags (OR semantics, deduplicated). Available in `on_view` hooks and `add_tree_action` closures.
- **`today()` scripting function** — Returns today's date as a `"YYYY-MM-DD"` string. Useful in `on_save` hooks to auto-stamp date fields or derive a date-prefixed title.
- **Tags in export / import** — `workspace.json` now includes a global tag list and each note's tags array. Import restores all tag assignments.
- **Book collection template** — A full library management template (previously a bundled system script) moved to the template gallery as `templates/book_collection.rhai`.

---

## [0.2.2] — 2026-02-26

### Added
- **`create_note` and `update_note` in tree actions** — `add_tree_action` closures can now create new notes and modify existing ones, not just reorder children. `create_note(parent_id, node_type)` inserts a note with schema defaults and returns a map you can edit; `update_note(note)` persists title and field changes back to the database. All writes from a single action execute inside one SQLite transaction — any error rolls back everything. Notes created earlier in the same closure are immediately visible to `get_children()` and `get_note()`, so full subtrees can be built in one action.

---

## [0.2.1] — 2026-02-25

### Added
- **`on_add_child` hook** — Scripts can now define an `on_add_child` hook that fires whenever a child note is created under or moved to a parent note. The hook receives the parent and the new child, and can modify either before the operation completes.
- **Tree context menu actions** — Scripts can register custom actions via `add_tree_action(label, fn)`. Registered actions appear in the right-click context menu of tree nodes and are invoked with the selected note as an argument. The bundled Text Note script includes a "Sort Children A→Z" example action.
- **Schema name collision detection** — Krillnotes now detects when two scripts register schemas with the same name and reports an error at load time instead of silently overwriting one with the other.

### Fixed
- Note struct state is now synced with any `on_add_child` hook modifications before being written to the operations log, ensuring the logged snapshot reflects the final saved values.

---

## [0.2.0] — 2026-02-24

> **Breaking change:** The workspace file format has changed due to database encryption. Workspaces created with v0.1.x cannot be opened directly — export them from the old version and re-import into v0.2.0.

### Added
- **Database encryption** — All workspaces are now encrypted at rest using SQLCipher (AES-256). Passwords are stored in the OS keychain by default, with a toggle to cache them in-session only. Existing unencrypted workspaces must be exported and re-imported.
- **Encrypted exports** — Export archives can be password-protected with AES-256. Krillnotes automatically detects encrypted archives on import and prompts for the password.
- **Markdown rendering** — Textarea fields are rendered as Markdown in view mode. The raw text is still accessible in scripts and edit mode. A `markdown()` helper is also available in `on_view` scripts.
- **Hooks inside schema** — `on_save` and `on_view` hooks are now defined directly inside the `schema()` block, making scripts self-contained and removing any ambiguity about which hook runs for a given note type.
- **Script compile error reporting** — Saving a user script that contains a syntax or compile error now shows an error message instead of silently discarding the save.
- **Script name in hook error messages** — Runtime errors thrown by `on_save` or `on_view` hooks now include the name of the script that caused the error, making debugging much easier.
- **Copy and paste notes** — Any note (and its entire descendant subtree) can be copied and pasted as a child or sibling of any compatible target note. Available via right-click context menu, Edit menu, and keyboard shortcuts (⌘C / ⌘V / ⌘⇧V). Schema constraints are enforced silently — invalid paste targets are ignored, matching the behaviour of drag-and-drop move.
- **Humanised field labels** — field names are now displayed in Title Case in both view and edit mode (e.g. `note_title` → "Note Title", `first_name` → "First Name").
- **Script load-order drag reordering** — User scripts in the Script Manager can now be reordered by dragging the grip handle on the left of each row. The visual order in the list is immediately persisted to the database and the script engine reloads in the new order.

### Fixed
- Workspace names containing spaces are now accepted; the name is stored as-is and only the filename is slugified automatically.
- Exported archive filenames now default to the workspace name instead of a generic placeholder.
- `on_view` hook runtime errors are now surfaced to the user instead of silently falling back to the default view.

---

## [0.1.2] — 2026-02-23

### Fixed
- On Windows, workspace windows opened after startup were missing the menu bar. They now correctly receive the full application menu at creation time.

---

## [0.1.1] — 2026-02-23

### Fixed
- On Windows, menu events were incorrectly broadcast to all open windows. Events are now routed only to the focused window.

---

## [0.1.0] — 2026-02-23 — First release

### Added

#### Core note-taking
- Hierarchical tree-based note structure with unlimited nesting.
- Create, view, edit, and delete notes from the tree or via keyboard shortcuts.
- Notes are auto-selected and opened in edit mode immediately on creation.
- Drag-and-drop reordering: move notes among siblings or reparent them anywhere in the tree.
- Keyboard navigation: arrow keys move through the tree, Enter opens edit mode, Escape cancels.
- Resizable split between the tree panel and the note view/edit panel.
- Global search bar with instant dropdown results and automatic ancestor expansion so the matched note is always visible in the tree.

#### Scripting and note schemas
- Note types are defined via [Rhai](https://rhai.rs) scripts, giving full control over fields, validation, and display.
- **User scripts** are stored inside the workspace database — no separate files to manage. Each workspace has its own independent set of scripts.
- **Script Manager** UI: list, create, edit (CodeMirror editor), reload, and delete scripts. A warning is shown before deleting a script that defines a schema with existing data.
- System scripts are seeded into every new workspace and can be edited or deleted freely.
- **Field types**: `text` (single-line), `textarea` (multi-line), `date`, `email`, `boolean`, `select` (dropdown), `rating` (star widget).
- **Field visibility flags**: control whether a field appears in view mode, edit mode, or both. Optionally lock the note title from being edited (e.g. when it is derived by an `on_save` hook).
- **`on_save` hook**: transform or derive field values before a note is saved (e.g. auto-build a contact's display name from first and last name fields).
- **`on_view` hook**: return custom HTML to render a note, with access to the note's children. Includes a simple DSL — `table()`, `heading()`, `paragraph()`, `link_to()`, and more — so scripts stay readable without raw HTML string building.
- **`link_to(note)`**: creates a clickable link in a view that navigates to another note. Includes full back-navigation history and a back button.
- **Children sort**: schemas can specify whether child notes are sorted by title (ascending or descending) or kept in manual drag-and-drop order.
- **Parent/child constraints**: a schema can declare which parent types it may be placed under, and which child types are allowed beneath it. The tree enforces these constraints during drag-and-drop and note creation.

#### Built-in note types (bundled scripts)
- **Text Note** — title and multi-line body
- **Contact** — first name, last name, email, phone, address, notes, family flag; title auto-derived
- **Book** — title, author, genre, status, rating, date started/finished, notes
- **Task** — title, description, due date, priority, status, tags
- **Project** — title, description, status, start/end dates, owner, budget, notes
- **Product** — name, SKU, category, price, stock, description
- **Recipe** — title, cuisine, servings, prep/cook time, ingredients, instructions

#### Workspaces
- Each workspace is a self-contained SQLite database file.
- Configurable default workspace directory with sensible OS defaults (`~/Documents/Krillnotes`).
- New Workspace dialog and Open Workspace list dialog; no raw file pickers needed.
- Multiple workspaces can be open simultaneously, each in its own window.

#### Operations log
- Every create, update, and delete action is recorded with a timestamp and the affected note title.
- Operations log viewer with filtering by type and date range.
- Purge button to compact the log and reduce database size.

#### Export / Import
- Export a workspace as a ZIP archive containing a JSON data file and all user scripts as `.rhai` files — suitable for sharing or backup.
- Import a ZIP archive into a new workspace.

#### UI and application
- Compact grid layout for note fields in view mode; empty fields are hidden automatically.
- Collapsible metadata section for system-level fields.
- Right-click context menus on tree nodes (edit, delete with confirmation).
- Platform-aware menus: macOS app menu, Edit menu with standard shortcuts; Tools menu for Operations Log and Script Manager.
- Cross-platform release workflow via GitHub Actions (macOS, Windows, Linux).

[0.9.0]: https://github.com/2pisoftware/krillnotes/compare/v0.4.1...v0.9.0
[0.4.1]: https://github.com/2pisoftware/krillnotes/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/2pisoftware/krillnotes/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/2pisoftware/krillnotes/compare/v0.2.6...v0.3.0
[0.2.6]: https://github.com/2pisoftware/krillnotes/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/2pisoftware/krillnotes/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/2pisoftware/krillnotes/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/2pisoftware/krillnotes/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/2pisoftware/krillnotes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/2pisoftware/krillnotes/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/2pisoftware/krillnotes/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/2pisoftware/krillnotes/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/2pisoftware/krillnotes/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/2pisoftware/krillnotes/releases/tag/v0.1.0
