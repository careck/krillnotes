# Unified Storage Layout

**Date:** 2026-04-07
**Status:** Draft

## Summary

Consolidate all Krillnotes data — identities, workspaces, and settings — into a single user-visible "home folder" instead of splitting between a hidden system config directory and a user-chosen workspace directory. Every identity gets its own folder (named by display name) containing both its cryptographic data and all its workspaces.

## Motivation

The current layout splits data across two unrelated locations:
- **Hidden config dir** (`~/.config/krillnotes/`): identity keypairs, contacts, relay accounts, invites, device IDs, settings
- **User-visible dir** (`~/Documents/Krillnotes/`): workspace databases, attachments, binding files

This creates problems:
- Identity data is in a system folder that users don't know about and can't easily back up
- The two locations are coupled cryptographically (via `binding.json`) but physically separated
- Users have no single folder they can point a backup tool at

## Design

### Directory Layout

```
~/Krillnotes/                          ← home folder (user-configurable)
├── settings.json                      ← app-level: theme, language, window state
│
├── Alice (Work)/                      ← identity folder (display name)
│   ├── .identity/                     ← identity internals
│   │   ├── identity.json             ← encrypted keypair + uuid + display name
│   │   ├── device_id                 ← per-machine device UUID
│   │   ├── contacts/                 ← encrypted contact files
│   │   ├── relays/                   ← encrypted relay account files
│   │   ├── invites/                  ← sent invite records
│   │   ├── accepted_invites/         ← accepted invite states
│   │   └── invite_responses/         ← received responses
│   │
│   ├── My Project/                    ← workspace (direct child of identity)
│   │   ├── notes.db                  ← SQLCipher database
│   │   ├── binding.json              ← workspace UUID + encrypted DB password
│   │   ├── info.json                 ← workspace metadata
│   │   └── attachments/              ← file attachments
│   │
│   └── Personal Notes/                ← another workspace
│       └── ...
│
└── Bob (Personal)/                    ← second identity
    ├── .identity/
    │   └── ...
    └── Journal/
        └── ...
```

### Key Rules

- **`.identity/` is the marker.** Any direct child of the home folder containing a `.identity/identity.json` file is an identity folder. Any direct child of an identity folder containing a `binding.json` is a workspace.
- **`identity.json` is the source of truth** for identity UUID and display name — the folder name is cosmetic.
- **Folder names are display names.** If a collision occurs on creation, append ` (2)`, ` (3)`, etc.
- **User can rename folders freely.** The app discovers identities and workspaces by scanning for marker files, not by remembering paths.
- **No `workspaces/` subdirectory.** Workspaces are direct children of the identity folder to keep paths short and browsable.
- **`.identity/` is not forcibly hidden on Windows.** The dot-prefix doesn't hide on Windows, but the name signals "internal" clearly enough. No platform-specific file attribute hacks.

### Cross-Platform Paths

**Home folder defaults:**

| Platform | Default home folder |
|----------|-------------------|
| macOS    | `~/Krillnotes/` |
| Linux    | `~/Krillnotes/` |
| Windows  | `%USERPROFILE%\Krillnotes\` |

**Breadcrumb file** (stores custom home path if user overrides the default):

| Platform | Breadcrumb location |
|----------|-------------------|
| macOS    | `~/.config/krillnotes/home_path` |
| Linux    | `~/.config/krillnotes/home_path` |
| Windows  | `%APPDATA%\Krillnotes\home_path` |

The breadcrumb file contains only the path string, nothing else. It is the one piece of data that must remain at a well-known system location so the app can bootstrap.

### Discovery & Bootstrap

On launch:

1. **Find home folder:** Read breadcrumb file if it exists, otherwise use platform default. Create the home folder if it doesn't exist (first launch).
2. **Load settings:** Read `{home}/settings.json`. Create with defaults if missing.
3. **Discover identities:** Scan direct children of home folder. Any subfolder containing `.identity/identity.json` is an identity. Build in-memory registry.
4. **Discover workspaces (per identity):** When an identity is unlocked, scan its direct children. Any subfolder containing `binding.json` is a workspace.

**Error case:** If the home folder doesn't exist on a non-first launch (e.g., external drive unmounted), show an error dialog and let the user pick a new location or quit. Don't silently create an empty home folder.

**Eliminated files/concepts:**
- `identity_settings.json` — gone, filesystem is the registry
- `settings.json.workspaceDirectory` — gone, workspaces discovered per-identity
- Flat scan of a global workspace directory — gone

### Identity Lifecycle

**Create:** Generate keypair → create `{home}/{display_name}/.identity/` with `identity.json`, `device_id`, and empty subdirs. Collision → append ` (2)`.

**Rename:** Update `display_name` in `identity.json` → rename folder on disk. Collision → append suffix.

**Delete:** Refuse if workspaces still exist (existing rule). User must delete all workspaces first. Then remove the entire identity folder.

### Workspace Lifecycle

**Create:** User picks identity + provides workspace name → create `{home}/{identity_folder}/{workspace_name}/` with `notes.db`, `binding.json`, `info.json`, `attachments/`. Collision → append ` (2)`.

**Open:** App scans identity folder for `binding.json` children → user picks one → decrypt DB password from `binding.json` → open `notes.db`.

**Delete:** Remove workspace folder. No other cleanup needed.

### What Doesn't Change

- **Encryption:** Argon2id + AES-256-GCM for identity, HKDF-derived keys for contacts/relays, SQLCipher for databases
- **`binding.json` format:** Still workspace UUID, identity UUID, encrypted DB password
- **Contact/relay/invite file formats:** Same encryption, same JSON structure — only the parent directory changes
- **Database schema, CRDT operations, scripting, export/import**
- **Relay protocol, sync logic**
- **All React components** except settings dialog and workspace open/list UI

## Code Impact

### Significant Changes

| File | What changes |
|------|-------------|
| `settings.rs` | Replace `config_dir()` with `home_dir()` that reads breadcrumb or uses platform default. Remove `default_workspace_directory()`. Load `settings.json` from home folder. |
| `identity.rs` (`IdentityManager`) | Base path changes from `config_dir/identities/` to `home_dir/{display_name}/.identity/`. Discovery scans home folder for `.identity/` markers instead of reading `identity_settings.json`. `get_workspaces_for_identity()` scans identity folder children. |
| `lib.rs` (`AppState`) | Remove `workspace_directory` field. Replace with `home_dir`. Update Tauri commands that reference workspace or identity paths. |
| `identity_settings.json` | **Deleted entirely.** |

### Minor/Path-Only Changes

| File | What changes |
|------|-------------|
| `contact.rs`, `relay_account.rs`, `invite.rs` | Base path passed in changes. Managers unchanged — they already take a directory path. |
| `workspace.rs` | `create_workspace()` takes identity folder as parent. |
| `storage.rs` | No change — opens whatever path it's given. |

### Frontend Changes

| File | What changes |
|------|-------------|
| Settings dialog | Remove "workspace directory" picker. Add "Krillnotes home folder" display (or picker for override). |
| Workspace list/open flow | Workspaces fetched per-identity, not from a global folder. |
| `types.ts` | Update types referencing workspace directory settings. |

## Migration

No migration. This is a clean break — existing data in the old layout is not carried over. The app is still in prototype/testing phase.
