# Unified Storage Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate all Krillnotes data — identities, workspaces, settings — into a single user-visible "home folder" with per-identity subfolders named by display name.

**Architecture:** Replace the split `config_dir` (hidden system folder) / `workspace_directory` (user-visible) layout with a single `home_dir` (default `~/Krillnotes/`). Each identity gets a display-name folder containing `.identity/` (cryptographic data) and workspace folders as direct children. Discovery is filesystem-based (scan for `.identity/` markers) — `identity_settings.json` is eliminated.

**Tech Stack:** Rust (krillnotes-core, krillnotes-desktop/src-tauri), React 19, TypeScript, Tauri v2

**Design spec:** `docs/superpowers/specs/2026-04-07-unified-storage-layout-design.md`

---

## New directory layout

```
~/Krillnotes/                          ← home_dir (user-configurable)
├── settings.json                      ← app-level settings
├── Alice (Work)/                      ← identity folder (display name)
│   ├── .identity/
│   │   ├── identity.json
│   │   ├── device_id
│   │   ├── contacts/
│   │   ├── relays/
│   │   ├── invites/
│   │   ├── accepted_invites/
│   │   └── invite_responses/
│   ├── My Project/                    ← workspace
│   │   ├── notes.db
│   │   ├── binding.json
│   │   ├── info.json
│   │   └── attachments/
│   └── Personal Notes/
└── Bob (Personal)/
    ├── .identity/
    └── Journal/
```

## Key API changes summary

| Before | After |
|--------|-------|
| `settings::config_dir()` → `~/.config/krillnotes/` | `settings::home_dir()` → `~/Krillnotes/` |
| `settings::default_workspace_directory()` → `~/Documents/Krillnotes/` | Removed — workspaces live inside identity folders |
| `AppSettings.workspace_directory` | Removed |
| `IdentityManager::new(config_dir)` | `IdentityManager::new(home_dir)` |
| `identity_dir(uuid)` → `config_dir/identities/{uuid}/` | `identity_dir(uuid)` → `home_dir/{display_name}/.identity/` |
| `list_identities()` reads `identity_settings.json` | `list_identities()` scans filesystem for `.identity/` markers |
| `delete_identity(uuid, workspace_base_dir)` | `delete_identity(uuid)` — scans identity folder children |
| `get_workspaces_for_identity(uuid, workspace_base_dir)` | `get_workspaces_for_identity(uuid)` — scans identity folder children |
| `identity_settings.json` | Eliminated |
| `IdentitySettings`, `LegacyWorkspaceBinding` structs | Removed |
| Migration code (pass1, pass2) | Removed (clean break) |

---

## Task 1: settings.rs — home_dir foundation

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/settings.rs`

### Goal
Replace `config_dir()`, `default_workspace_directory()`, and `workspace_directory` field with a `home_dir()` function that reads an optional breadcrumb file or defaults to `~/Krillnotes/`.

- [ ] **Step 1: Write failing tests for home_dir**

Add these tests to the `#[cfg(test)] mod tests` block in `settings.rs`:

```rust
#[test]
fn home_dir_returns_default_when_no_breadcrumb() {
    // home_dir() should return ~/Krillnotes/ when no breadcrumb exists
    let dir = home_dir();
    assert!(dir.to_string_lossy().ends_with("Krillnotes"));
}

#[test]
fn settings_deserializes_without_workspace_directory() {
    let json = r#"{"activeThemeMode":"dark","language":"fr"}"#;
    let s: AppSettings = serde_json::from_str(json).unwrap();
    assert_eq!(s.active_theme_mode, "dark");
    assert_eq!(s.language, "fr");
}

#[test]
fn settings_ignores_legacy_workspace_directory_field() {
    // Old settings files with workspaceDirectory should still deserialize
    let json = r#"{"workspaceDirectory":"/old/path","language":"en"}"#;
    let s: AppSettings = serde_json::from_str(json).unwrap();
    assert_eq!(s.language, "en");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-desktop settings::tests --no-default-features --features rbac -- --nocapture 2>&1 | tail -20`
Expected: Compilation errors (home_dir doesn't exist, AppSettings still has workspace_directory)

- [ ] **Step 3: Rewrite settings.rs**

Replace the full content of `krillnotes-desktop/src-tauri/src/settings.rs` with:

```rust
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Application settings persistence for Krillnotes.
//!
//! Settings live in the Krillnotes home folder (`~/Krillnotes/settings.json`).
//! A breadcrumb file at the OS config directory stores a custom home path
//! if the user overrides the default.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Persisted application settings.
///
/// Stored at `{home_dir}/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Current theme mode: "light", "dark", or "system".
    #[serde(default = "default_theme_mode")]
    pub active_theme_mode: String,
    /// Name of the theme to use in light mode.
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    /// Name of the theme to use in dark mode.
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
    /// Language code for the UI ("en", "de", "fr", "es", "ja", "ko", "zh").
    #[serde(default = "default_language")]
    pub language: String,
    /// Controls when sharing permission indicators (coloured dots) appear in the tree.
    /// "on" = always, "off" = never, "auto" = only when the workspace has peers.
    #[serde(default = "default_sharing_indicator_mode")]
    pub sharing_indicator_mode: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_theme_mode: default_theme_mode(),
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
            language: default_language(),
            sharing_indicator_mode: default_sharing_indicator_mode(),
        }
    }
}

// ── Breadcrumb (custom home folder override) ─────────────────────────

/// Returns the OS-appropriate breadcrumb directory.
/// - macOS / Linux: `~/.config/krillnotes/`
/// - Windows: `%APPDATA%/Krillnotes/`
fn breadcrumb_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Krillnotes")
    }
    #[cfg(not(target_os = "windows"))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("krillnotes")
    }
}

/// Path to the breadcrumb file that stores a custom home folder location.
fn breadcrumb_path() -> PathBuf {
    breadcrumb_dir().join("home_path")
}

// ── Home directory ───────────────────────────────────────────────────

/// Returns the default home directory: `~/Krillnotes/`.
fn default_home_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Krillnotes")
}

/// Returns the Krillnotes home directory.
///
/// Reads the breadcrumb file if it exists; otherwise returns the
/// platform default (`~/Krillnotes/` on all platforms).
pub fn home_dir() -> PathBuf {
    let bp = breadcrumb_path();
    if let Ok(content) = fs::read_to_string(&bp) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_home_dir()
}

/// Writes a custom home directory path to the breadcrumb file.
pub fn set_home_dir(path: &str) -> Result<(), String> {
    let bp = breadcrumb_path();
    if let Some(parent) = bp.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create breadcrumb directory: {e}"))?;
    }
    fs::write(&bp, path.trim())
        .map_err(|e| format!("Failed to write breadcrumb file: {e}"))
}

// ── Settings I/O ─────────────────────────────────────────────────────

fn default_theme_mode() -> String { "system".to_string() }
fn default_light_theme() -> String { "light".to_string() }
fn default_dark_theme() -> String { "dark".to_string() }
fn default_language() -> String { "en".to_string() }
fn default_sharing_indicator_mode() -> String { "auto".to_string() }

/// Returns the path to the settings JSON file: `{home_dir}/settings.json`.
pub fn settings_file_path() -> PathBuf {
    home_dir().join("settings.json")
}

/// Loads settings from disk; returns defaults if the file is missing or corrupt.
pub fn load_settings() -> AppSettings {
    let path = settings_file_path();
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => AppSettings::default(),
    }
}

/// Saves settings to disk, creating parent directories as needed.
pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create settings directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write settings: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_dir_returns_default_when_no_breadcrumb() {
        let dir = home_dir();
        assert!(dir.to_string_lossy().ends_with("Krillnotes"));
    }

    #[test]
    fn settings_deserializes_without_workspace_directory() {
        let json = r#"{"activeThemeMode":"dark","language":"fr"}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "dark");
        assert_eq!(s.language, "fr");
    }

    #[test]
    fn settings_ignores_legacy_workspace_directory_field() {
        let json = r#"{"workspaceDirectory":"/old/path","language":"en"}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.language, "en");
    }

    #[test]
    fn settings_defaults_are_applied() {
        let json = r#"{}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "system");
        assert_eq!(s.light_theme, "light");
        assert_eq!(s.dark_theme, "dark");
        assert_eq!(s.language, "en");
        assert_eq!(s.sharing_indicator_mode, "auto");
    }
}
```

- [ ] **Step 4: Fix compilation errors in other files that reference removed API**

Many files reference `settings::config_dir()` and `AppSettings.workspace_directory`. These will fail to compile after this change. For now, add a temporary shim to keep things compiling while we work through subsequent tasks:

```rust
/// Temporary shim — remove after all callers are migrated.
#[deprecated(note = "Use home_dir() instead")]
pub fn config_dir() -> PathBuf {
    home_dir()
}
```

This allows the project to compile while tasks 2-6 migrate each caller.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p krillnotes-desktop settings::tests --no-default-features --features rbac 2>&1 | tail -10`
Expected: All 4 tests pass

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/settings.rs
git commit -m "feat: replace config_dir/workspace_directory with home_dir + breadcrumb"
```

---

## Task 2: IdentityManager — struct refactor + scanning

**Files:**
- Modify: `krillnotes-core/src/core/identity.rs`

### Goal
Replace UUID-based identity folder layout with display-name folders containing `.identity/`. Add in-memory cache for UUID → folder name mapping. Remove `IdentitySettings`, `LegacyWorkspaceBinding`, migration code.

- [ ] **Step 1: Remove dead types and migration code**

Remove these items from `identity.rs`:

1. `IdentitySettings` struct and its `Default` impl (lines 77-84)
2. `LegacyWorkspaceBinding` struct (lines 109-114)
3. `migrate()` method and both `migrate_pass1_*` and `migrate_pass2_*` methods
4. `load_settings()` / `save_settings()` methods (the identity_settings.json ones)
5. Legacy `contacts/` cleanup in `new()`

- [ ] **Step 2: Add `last_used` field to IdentityFile**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityFile {
    pub identity_uuid: Uuid,
    pub display_name: String,
    pub public_key: String,
    pub private_key_enc: EncryptedKey,
    /// Last time this identity was unlocked. Added in unified-storage layout.
    #[serde(default)]
    pub last_used: Option<DateTime<Utc>>,
}
```

- [ ] **Step 3: Rewrite IdentityManager struct and new()**

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Manages identity folders and their cryptographic data.
///
/// Identities are stored in display-name folders under `home_dir`, each
/// containing a `.identity/` subdirectory with the encrypted keypair,
/// contacts, relay accounts, etc. Discovery is filesystem-based.
pub struct IdentityManager {
    home_dir: PathBuf,
    /// UUID → folder name (the display-name folder as it appears on disk).
    /// Kept in sync by create/delete/rename/import operations.
    folder_cache: HashMap<Uuid, String>,
}

impl IdentityManager {
    /// Create a new `IdentityManager` rooted at `home_dir`.
    ///
    /// Scans `home_dir` for identity folders (those containing `.identity/identity.json`)
    /// and builds an in-memory UUID → folder-name cache.
    pub fn new(home_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&home_dir)?;
        let folder_cache = Self::scan_identities(&home_dir);
        Ok(Self { home_dir, folder_cache })
    }

    /// Scan `home_dir` for identity folders and build UUID → folder-name map.
    fn scan_identities(home_dir: &Path) -> HashMap<Uuid, String> {
        let mut cache = HashMap::new();
        let entries = match std::fs::read_dir(home_dir) {
            Ok(e) => e,
            Err(_) => return cache,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let identity_json = path.join(".identity").join("identity.json");
            if let Ok(content) = std::fs::read_to_string(&identity_json) {
                if let Ok(file) = serde_json::from_str::<IdentityFile>(&content) {
                    if let Some(folder_name) = path.file_name().and_then(|n| n.to_str()) {
                        cache.insert(file.identity_uuid, folder_name.to_string());
                    }
                }
            }
        }
        cache
    }

    /// Returns the home directory.
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Returns the identity's root folder (contains workspaces + `.identity/`).
    ///
    /// Returns `None` if the identity is not in the cache.
    pub fn identity_base_dir(&self, identity_uuid: &Uuid) -> Option<PathBuf> {
        self.folder_cache
            .get(identity_uuid)
            .map(|name| self.home_dir.join(name))
    }

    /// Returns the `.identity/` data directory for the given identity.
    ///
    /// Contains `identity.json`, `contacts/`, `relays/`, etc.
    /// Panics in debug builds if the identity is not in the cache.
    pub fn identity_dir(&self, identity_uuid: &Uuid) -> PathBuf {
        match self.identity_base_dir(identity_uuid) {
            Some(base) => base.join(".identity"),
            None => {
                log::warn!("identity_dir: UUID {identity_uuid} not in cache");
                self.home_dir.join(identity_uuid.to_string()).join(".identity")
            }
        }
    }

    /// Returns the path to `identity.json` for the given identity.
    pub fn identity_file_path(&self, identity_uuid: &Uuid) -> PathBuf {
        self.identity_dir(identity_uuid).join("identity.json")
    }

    /// Pick a non-colliding folder name for a display name.
    fn pick_folder_name(&self, display_name: &str) -> String {
        let base = display_name.to_string();
        if !self.home_dir.join(&base).exists() {
            return base;
        }
        for i in 2u32.. {
            let candidate = format!("{} ({})", display_name, i);
            if !self.home_dir.join(&candidate).exists() {
                return candidate;
            }
        }
        unreachable!()
    }
```

- [ ] **Step 4: Rewrite `list_identities()`**

```rust
    /// Lists all identities discovered in the home directory.
    ///
    /// Reads each `.identity/identity.json` file to build `IdentityRef` entries.
    pub fn list_identities(&self) -> Result<Vec<IdentityRef>> {
        let mut refs = Vec::new();
        for (uuid, folder_name) in &self.folder_cache {
            let identity_json = self.home_dir
                .join(folder_name)
                .join(".identity")
                .join("identity.json");
            if let Ok(content) = std::fs::read_to_string(&identity_json) {
                if let Ok(file) = serde_json::from_str::<IdentityFile>(&content) {
                    refs.push(IdentityRef {
                        uuid: *uuid,
                        display_name: file.display_name,
                        file: identity_json.to_string_lossy().to_string(),
                        last_used: file.last_used.unwrap_or_else(Utc::now),
                    });
                }
            }
        }
        // Sort by last_used descending (most recent first)
        refs.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        Ok(refs)
    }
```

- [ ] **Step 5: Rewrite `create_identity()`**

The method signature changes to `&mut self` since it updates the folder cache:

```rust
    /// Create a new identity with the given display name and passphrase.
    ///
    /// Creates a display-name folder with `.identity/` subdirectory containing
    /// the encrypted keypair file. Updates the in-memory folder cache.
    pub fn create_identity(
        &mut self,
        display_name: &str,
        passphrase: &str,
    ) -> Result<IdentityFile> {
        let identity_uuid = Uuid::new_v4();
        let folder_name = self.pick_folder_name(display_name);
        let identity_dir = self.home_dir.join(&folder_name).join(".identity");

        // Create directory structure
        std::fs::create_dir_all(&identity_dir)?;
        std::fs::create_dir_all(identity_dir.join("contacts"))?;
        std::fs::create_dir_all(identity_dir.join("invites"))?;
        std::fs::create_dir_all(identity_dir.join("relays"))?;
        std::fs::create_dir_all(identity_dir.join("accepted_invites"))?;
        std::fs::create_dir_all(identity_dir.join("invite_responses"))?;

        // Generate Ed25519 keypair
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let public_key = BASE64.encode(signing_key.verifying_key().to_bytes());

        // Encrypt seed with Argon2id + AES-256-GCM
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let mut derived_key = [0u8; 32];
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
                .expect("valid Argon2 params"),
        )
        .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
        .map_err(|e| crate::KrillnotesError::Generic(format!("Argon2 failed: {e}")))?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| crate::KrillnotesError::Generic(format!("AES init failed: {e}")))?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, seed.as_ref())
            .map_err(|e| crate::KrillnotesError::Generic(format!("Encryption failed: {e}")))?;

        let identity_file = IdentityFile {
            identity_uuid,
            display_name: display_name.to_string(),
            public_key,
            private_key_enc: EncryptedKey {
                ciphertext: BASE64.encode(&ciphertext),
                nonce: BASE64.encode(nonce_bytes),
                kdf: "argon2id".to_string(),
                kdf_params: KdfParams {
                    salt: BASE64.encode(salt),
                    m_cost: ARGON2_M_COST,
                    t_cost: ARGON2_T_COST,
                    p_cost: ARGON2_P_COST,
                },
            },
            last_used: Some(Utc::now()),
        };

        // Write identity file
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(identity_dir.join("identity.json"), json)?;

        // Update cache
        self.folder_cache.insert(identity_uuid, folder_name);

        Ok(identity_file)
    }
```

- [ ] **Step 6: Run core tests to check compilation**

Run: `cargo check -p krillnotes-core 2>&1 | head -30`
Expected: May show errors from test file — that's OK, we'll fix tests in Task 5

- [ ] **Step 7: Commit**

```bash
git add krillnotes-core/src/core/identity.rs
git commit -m "feat: rewrite IdentityManager for display-name folder layout"
```

---

## Task 3: IdentityManager — lifecycle methods

**Files:**
- Modify: `krillnotes-core/src/core/identity.rs`

### Goal
Update `delete_identity`, `rename_identity`, `unlock_identity`, `get_workspaces_for_identity`, and import/export methods for the new layout.

- [ ] **Step 1: Rewrite `unlock_identity()`**

The core logic stays the same (Argon2id decrypt), but `last_used` is now updated in `identity.json` instead of `identity_settings.json`:

```rust
    /// Unlock an identity by decrypting its seed with the given passphrase.
    pub fn unlock_identity(
        &self,
        identity_uuid: &Uuid,
        passphrase: &str,
    ) -> Result<UnlockedIdentity> {
        let file_path = self.identity_file_path(identity_uuid);
        let raw = std::fs::read_to_string(&file_path)?;
        let mut id_file: IdentityFile = serde_json::from_str(&raw)?;

        // Decrypt seed
        let salt = BASE64.decode(&id_file.private_key_enc.kdf_params.salt)
            .map_err(|e| crate::KrillnotesError::Generic(format!("Bad salt: {e}")))?;
        let mut derived_key = [0u8; 32];
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(
                id_file.private_key_enc.kdf_params.m_cost,
                id_file.private_key_enc.kdf_params.t_cost,
                id_file.private_key_enc.kdf_params.p_cost,
                Some(32),
            ).map_err(|e| crate::KrillnotesError::Generic(format!("Argon2 params: {e}")))?,
        )
        .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
        .map_err(|_| crate::KrillnotesError::IdentityWrongPassphrase)?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| crate::KrillnotesError::Generic(format!("AES init: {e}")))?;
        let nonce_bytes = BASE64.decode(&id_file.private_key_enc.nonce)
            .map_err(|e| crate::KrillnotesError::Generic(format!("Bad nonce: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = BASE64.decode(&id_file.private_key_enc.ciphertext)
            .map_err(|e| crate::KrillnotesError::Generic(format!("Bad ciphertext: {e}")))?;
        let seed = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| crate::KrillnotesError::IdentityWrongPassphrase)?;

        let seed_array: [u8; 32] = seed
            .try_into()
            .map_err(|_| crate::KrillnotesError::Generic("Bad seed length".to_string()))?;
        let signing_key = SigningKey::from_bytes(&seed_array);
        let verifying_key = signing_key.verifying_key();

        // Update last_used timestamp in identity.json
        id_file.last_used = Some(Utc::now());
        let json = serde_json::to_string_pretty(&id_file)?;
        std::fs::write(&file_path, json)?;

        Ok(UnlockedIdentity {
            identity_uuid: *identity_uuid,
            display_name: id_file.display_name,
            signing_key,
            verifying_key,
        })
    }
```

- [ ] **Step 2: Rewrite `delete_identity()`**

Signature changes: no more `workspace_base_dir` parameter.

```rust
    /// Delete an identity. Fails if the identity still has workspaces.
    ///
    /// Scans the identity folder for workspace children (folders with `binding.json`).
    pub fn delete_identity(&mut self, identity_uuid: &Uuid) -> Result<()> {
        // Check for bound workspaces
        let workspaces = self.get_workspaces_for_identity(identity_uuid)?;
        if !workspaces.is_empty() {
            return Err(crate::KrillnotesError::IdentityHasBoundWorkspaces);
        }

        // Remove the identity folder
        if let Some(base_dir) = self.identity_base_dir(identity_uuid) {
            std::fs::remove_dir_all(&base_dir)?;
        }

        // Update cache
        self.folder_cache.remove(identity_uuid);
        Ok(())
    }
```

- [ ] **Step 3: Rewrite `rename_identity()`**

```rust
    /// Rename an identity. Updates `identity.json` and renames the folder on disk.
    pub fn rename_identity(&mut self, identity_uuid: &Uuid, new_name: &str) -> Result<()> {
        // Update identity.json
        let file_path = self.identity_file_path(identity_uuid);
        let raw = std::fs::read_to_string(&file_path)?;
        let mut id_file: IdentityFile = serde_json::from_str(&raw)?;
        id_file.display_name = new_name.to_string();
        let json = serde_json::to_string_pretty(&id_file)?;
        std::fs::write(&file_path, json)?;

        // Rename folder if the display name is different from the current folder
        if let Some(old_base) = self.identity_base_dir(identity_uuid) {
            let new_folder_name = self.pick_folder_name(new_name);
            let new_base = self.home_dir.join(&new_folder_name);
            if old_base != new_base {
                std::fs::rename(&old_base, &new_base)?;
                self.folder_cache.insert(*identity_uuid, new_folder_name);
            }
        }

        Ok(())
    }
```

- [ ] **Step 4: Rewrite `get_workspaces_for_identity()`**

Signature changes: no more `workspace_base_dir` parameter.

```rust
    /// Lists all workspaces belonging to an identity.
    ///
    /// Scans direct children of the identity's root folder for `binding.json`.
    pub fn get_workspaces_for_identity(
        &self,
        identity_uuid: &Uuid,
    ) -> Result<Vec<(PathBuf, WorkspaceBinding)>> {
        let base_dir = match self.identity_base_dir(identity_uuid) {
            Some(dir) => dir,
            None => return Ok(vec![]),
        };

        let mut result = Vec::new();
        let entries = match std::fs::read_dir(&base_dir) {
            Ok(e) => e,
            Err(_) => return Ok(vec![]),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            // Skip the .identity directory itself
            if path.file_name().map(|n| n == ".identity").unwrap_or(false) { continue; }
            let binding_path = path.join("binding.json");
            if let Ok(content) = std::fs::read_to_string(&binding_path) {
                if let Ok(binding) = serde_json::from_str::<WorkspaceBinding>(&content) {
                    if binding.identity_uuid == *identity_uuid {
                        result.push((path, binding));
                    }
                }
            }
        }
        Ok(result)
    }
```

- [ ] **Step 5: Rewrite `change_passphrase()`**

The logic is the same — unlock with old passphrase, re-encrypt with new. The only change is that `last_used` is preserved:

```rust
    pub fn change_passphrase(
        &self,
        identity_uuid: &Uuid,
        old_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<()> {
        // Unlock to get seed
        let unlocked = self.unlock_identity(identity_uuid, old_passphrase)?;
        let seed = unlocked.signing_key.to_bytes();

        // Re-encrypt with new passphrase
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let mut derived_key = [0u8; 32];
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
                .expect("valid Argon2 params"),
        )
        .hash_password_into(new_passphrase.as_bytes(), &salt, &mut derived_key)
        .map_err(|e| crate::KrillnotesError::Generic(format!("Argon2 failed: {e}")))?;

        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| crate::KrillnotesError::Generic(format!("AES init: {e}")))?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, seed.as_ref())
            .map_err(|e| crate::KrillnotesError::Generic(format!("Encryption failed: {e}")))?;

        // Read current file to preserve all fields
        let file_path = self.identity_file_path(identity_uuid);
        let raw = std::fs::read_to_string(&file_path)?;
        let mut id_file: IdentityFile = serde_json::from_str(&raw)?;
        id_file.private_key_enc = EncryptedKey {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
            kdf: "argon2id".to_string(),
            kdf_params: KdfParams {
                salt: BASE64.encode(salt),
                m_cost: ARGON2_M_COST,
                t_cost: ARGON2_T_COST,
                p_cost: ARGON2_P_COST,
            },
        };
        let json = serde_json::to_string_pretty(&id_file)?;
        std::fs::write(&file_path, json)?;

        Ok(())
    }
```

- [ ] **Step 6: Rewrite `lookup_display_name()`**

```rust
    /// Look up the display name for a given public key across all identities.
    pub fn lookup_display_name(&self, public_key: &str) -> Option<String> {
        for (_uuid, folder_name) in &self.folder_cache {
            let identity_json = self.home_dir
                .join(folder_name)
                .join(".identity")
                .join("identity.json");
            if let Ok(content) = std::fs::read_to_string(&identity_json) {
                if let Ok(file) = serde_json::from_str::<IdentityFile>(&content) {
                    if file.public_key == public_key {
                        return Some(file.display_name);
                    }
                }
            }
        }
        None
    }
```

- [ ] **Step 7: Update `export_swarmid()` and `import_swarmid()`**

`export_swarmid()` — the existing logic works because `identity_dir()` still returns the `.identity/` folder. Review the method and ensure it reads from the correct path. The main change: remove any references to `identity_settings.json`.

`import_swarmid()` — change to `&mut self`, create a display-name folder, write identity data into `.identity/`:

```rust
    /// Import a `.swarmid` file.
    pub fn import_swarmid(
        &mut self,
        swarmid_json: &str,
        passphrase: &str,
    ) -> Result<Uuid> {
        let swarmid: SwarmIdFile = serde_json::from_str(swarmid_json)
            .map_err(|e| crate::KrillnotesError::Generic(format!("Invalid swarmid: {e}")))?;
        if swarmid.format != SwarmIdFile::FORMAT || swarmid.version != SwarmIdFile::VERSION {
            return Err(crate::KrillnotesError::Generic("Unsupported swarmid format/version".to_string()));
        }

        // Verify passphrase by attempting to decrypt
        let _ = self.decrypt_seed(&swarmid.identity, passphrase)?;

        let uuid = swarmid.identity.identity_uuid;

        // Check for collision
        if self.folder_cache.contains_key(&uuid) {
            return Err(crate::KrillnotesError::IdentityAlreadyExists(uuid));
        }

        // Create folder structure
        let folder_name = self.pick_folder_name(&swarmid.identity.display_name);
        let identity_dir = self.home_dir.join(&folder_name).join(".identity");
        std::fs::create_dir_all(&identity_dir)?;
        std::fs::create_dir_all(identity_dir.join("contacts"))?;
        std::fs::create_dir_all(identity_dir.join("invites"))?;
        std::fs::create_dir_all(identity_dir.join("relays"))?;
        std::fs::create_dir_all(identity_dir.join("accepted_invites"))?;
        std::fs::create_dir_all(identity_dir.join("invite_responses"))?;

        // Write identity file
        let json = serde_json::to_string_pretty(&swarmid.identity)?;
        std::fs::write(identity_dir.join("identity.json"), json)?;

        // Write relay account files
        for relay_file in &swarmid.relays {
            let relay_path = identity_dir.join("relays").join(&relay_file.filename);
            std::fs::write(&relay_path, &relay_file.contents)?;
        }

        // Update cache
        self.folder_cache.insert(uuid, folder_name);

        Ok(uuid)
    }
```

Similarly update `import_swarmid_overwrite()` — same pattern but removes existing entry first.

- [ ] **Step 8: Keep `bind_workspace`, `unbind_workspace`, `get_workspace_binding`, `encrypt_db_password`, `decrypt_db_password`, `ensure_device_uuid` unchanged**

These methods operate on workspace folders or identity directories via `identity_dir()` — they don't reference `config_dir` or `identity_settings.json`. Verify they compile correctly.

- [ ] **Step 9: Check compilation**

Run: `cargo check -p krillnotes-core 2>&1 | head -30`
Expected: Core crate compiles (test file may have errors — that's Task 5)

- [ ] **Step 10: Commit**

```bash
git add krillnotes-core/src/core/identity.rs
git commit -m "feat: identity lifecycle methods for display-name folder layout"
```

---

## Task 4: Identity tests rewrite

**Files:**
- Modify: `krillnotes-core/src/core/identity_tests.rs`

### Goal
Update all 43 identity tests for the new layout. Remove migration tests. Add tests for display-name folders, scanning, and collision handling.

- [ ] **Step 1: Remove migration and identity_settings tests**

Delete these tests:
- `test_identity_settings_default_empty`
- `test_settings_load_save_roundtrip`
- `migration_pass1_moves_flat_json_into_identity_subfolder`
- `migration_pass1_is_idempotent`
- `migration_pass2_writes_binding_json_for_existing_workspace`
- `migration_pass2_drops_stale_entry_for_missing_workspace`
- `old_identity_settings_with_workspaces_key_deserialises`
- `new_identity_settings_serialises_without_workspaces_key`

- [ ] **Step 2: Update test helper pattern**

All tests follow this pattern — update them from:
```rust
let tmp = tempdir().unwrap();
let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
```
To:
```rust
let tmp = tempdir().unwrap();
let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
```

The `mut` is needed because `create_identity`, `delete_identity`, `rename_identity`, `import_swarmid`, and `import_swarmid_overwrite` now take `&mut self`.

- [ ] **Step 3: Update path assertion tests**

Update `identity_dir_returns_uuid_subfolder` → rename to `identity_dir_returns_display_name_folder`:

```rust
#[test]
fn identity_dir_returns_display_name_folder() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let dir = mgr.identity_dir(&file.identity_uuid);
    assert_eq!(dir, tmp.path().join("Alice").join(".identity"));
}
```

Update `identity_file_path_returns_identity_json_inside_folder`:

```rust
#[test]
fn identity_file_path_returns_identity_json_inside_folder() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let path = mgr.identity_file_path(&file.identity_uuid);
    assert_eq!(path, tmp.path().join("Alice").join(".identity").join("identity.json"));
}
```

- [ ] **Step 4: Update `test_create_identity`**

```rust
#[test]
fn test_create_identity() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Test Identity", "my-passphrase").unwrap();
    assert_eq!(file.display_name, "Test Identity");
    assert!(file.last_used.is_some());
    // Verify folder structure
    let base = tmp.path().join("Test Identity");
    assert!(base.join(".identity").join("identity.json").exists());
    assert!(base.join(".identity").join("contacts").is_dir());
    assert!(base.join(".identity").join("relays").is_dir());
    assert!(base.join(".identity").join("invites").is_dir());
}
```

- [ ] **Step 5: Add display-name collision test**

```rust
#[test]
fn create_identity_handles_name_collision() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let _a = mgr.create_identity("Alice", "pass1").unwrap();
    let b = mgr.create_identity("Alice", "pass2").unwrap();
    // Second identity should get " (2)" suffix
    let base_b = mgr.identity_base_dir(&b.identity_uuid).unwrap();
    assert_eq!(base_b.file_name().unwrap().to_str().unwrap(), "Alice (2)");
}
```

- [ ] **Step 6: Add scanning test**

```rust
#[test]
fn new_discovers_existing_identities() {
    let tmp = tempdir().unwrap();
    {
        let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
        mgr.create_identity("Alice", "pass").unwrap();
        mgr.create_identity("Bob", "pass").unwrap();
    }
    // Re-create manager — should discover both identities
    let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let identities = mgr.list_identities().unwrap();
    assert_eq!(identities.len(), 2);
    let names: Vec<_> = identities.iter().map(|i| i.display_name.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Bob"));
}
```

- [ ] **Step 7: Update `delete_identity` tests**

Remove the `workspace_base_dir` parameter from `delete_identity` calls. Update `delete_identity_fails_if_workspaces_still_bound` to create workspace folders inside the identity folder:

```rust
#[test]
fn delete_identity_fails_if_workspaces_still_bound() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();

    // Create a workspace inside the identity folder
    let ws_dir = mgr.identity_base_dir(&uuid).unwrap().join("My Workspace");
    std::fs::create_dir_all(&ws_dir).unwrap();
    mgr.bind_workspace(&uuid, "ws-uuid-1", &ws_dir, "db-pass", &seed).unwrap();

    // Delete should fail
    let err = mgr.delete_identity(&uuid).unwrap_err();
    assert!(matches!(err, crate::KrillnotesError::IdentityHasBoundWorkspaces));
}
```

- [ ] **Step 8: Update `get_workspaces_for_identity` tests**

```rust
#[test]
fn get_workspaces_for_identity_scans_identity_folder() {
    let tmp = tempdir().unwrap();
    let mut mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
    let file = mgr.create_identity("Alice", "pass").unwrap();
    let uuid = file.identity_uuid;
    let unlocked = mgr.unlock_identity(&uuid, "pass").unwrap();
    let seed = unlocked.signing_key.to_bytes();

    // Create two workspaces inside the identity folder
    let base = mgr.identity_base_dir(&uuid).unwrap();
    for name in &["Work", "Personal"] {
        let ws_dir = base.join(name);
        std::fs::create_dir_all(&ws_dir).unwrap();
        mgr.bind_workspace(&uuid, &format!("uuid-{name}"), &ws_dir, "pass", &seed).unwrap();
    }

    let workspaces = mgr.get_workspaces_for_identity(&uuid).unwrap();
    assert_eq!(workspaces.len(), 2);
}
```

- [ ] **Step 9: Update all remaining tests**

Go through each remaining test and:
1. Change `let mgr` to `let mut mgr` where needed
2. Remove `workspace_base_dir` parameter from `delete_identity` calls
3. Remove `workspace_base_dir` parameter from `get_workspaces_for_identity` calls
4. Update any path assertions to match the new `.identity/` layout
5. Ensure workspace folders for tests are created inside identity folders

- [ ] **Step 10: Run all identity tests**

Run: `cargo test -p krillnotes-core identity 2>&1 | tail -20`
Expected: All tests pass

- [ ] **Step 11: Commit**

```bash
git add krillnotes-core/src/core/identity_tests.rs
git commit -m "test: update identity tests for unified storage layout"
```

---

## Task 5: Tauri backend — lib.rs + commands/identity.rs

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/identity.rs`

### Goal
Update AppState initialization and all identity Tauri commands to use the new `home_dir()` and `IdentityManager` API.

- [ ] **Step 1: Update lib.rs — IdentityManager initialization**

In the `run()` function, change:
```rust
identity_manager: Arc::new(Mutex::new(
    IdentityManager::new(settings::config_dir()).expect("Failed to init IdentityManager")
)),
```
To:
```rust
identity_manager: Arc::new(Mutex::new(
    IdentityManager::new(settings::home_dir()).expect("Failed to init IdentityManager")
)),
```

- [ ] **Step 2: Update lib.rs — remove workspace directory startup logic**

In the `setup()` callback, remove the block that:
1. Creates the workspace directory from `app_settings.workspace_directory`
2. Auto-migrates flat `.db` files to per-workspace folders

Replace with a simple home directory existence check:

```rust
// Ensure home directory exists
let home = settings::home_dir();
if !home.exists() {
    std::fs::create_dir_all(&home).expect("Failed to create Krillnotes home directory");
}
```

- [ ] **Step 3: Update lib.rs — get_settings and update_settings commands**

Find the `get_settings` and `update_settings` commands. If they reference `AppSettings.workspace_directory`, remove those references. The settings are now loaded from `home_dir()/settings.json` (which `load_settings()` already does after Task 1).

Add two new commands for home directory management:

```rust
#[tauri::command]
fn get_home_dir_path() -> String {
    settings::home_dir().to_string_lossy().to_string()
}

#[tauri::command]
fn set_home_dir_path(path: String) -> std::result::Result<(), String> {
    settings::set_home_dir(&path)
}
```

Register both in `tauri::generate_handler![...]`.

- [ ] **Step 4: Update commands/identity.rs — replace all config_dir path construction**

Every occurrence of this pattern:
```rust
crate::settings::config_dir()
    .join("identities")
    .join(uuid.to_string())
    .join("contacts")   // or "invites", "relays", etc.
```

Must change to:
```rust
mgr.identity_dir(&uuid).join("contacts")
```

Where `mgr` is `state.identity_manager.lock()...`. If the lock was already dropped, re-acquire it or save `identity_dir` to a local variable before dropping.

**In `create_identity` command** (approx lines 71-154):
1. Change `let mgr` → `let mut mgr` (create_identity now takes `&mut self`)
2. Save `let identity_dir = mgr.identity_dir(&uuid);` before `drop(mgr)`
3. Replace all 5 `config_dir().join("identities").join(uuid.to_string()).join(X)` with `identity_dir.join(X)`

**In `unlock_identity` command** (approx lines 158-409):
1. Save `let identity_dir = mgr.identity_dir(&uuid);` before `drop(mgr)`
2. Replace all `config_dir()...` path constructions with `identity_dir.join(X)`
3. Remove the old-style relay credential migration block (`let old_relay_dir = config_dir().join("relay")...`) — clean break means no relay migration

**In `delete_identity` command** (approx lines 459-474):
1. Remove `let workspace_base_dir = PathBuf::from(&crate::settings::load_settings().workspace_directory);`
2. Change `let mgr` → `let mut mgr`
3. Call `mgr.delete_identity(&uuid)` without `workspace_base_dir` parameter

**In `rename_identity` command** (approx lines 478-493):
1. Change `let mgr` → `let mut mgr`
2. Call `mgr.rename_identity(&uuid, &new_name)` (unchanged semantics)

**In `get_workspaces_for_identity` command** (approx lines 546-564):
1. Remove `let workspace_base_dir = PathBuf::from(&crate::settings::load_settings().workspace_directory);`
2. Call `mgr.get_workspaces_for_identity(&uuid)` without `workspace_base_dir`

**In `import_swarmid_cmd` and `import_swarmid_overwrite_cmd`:**
1. Change `let mgr` → `let mut mgr`
2. Rest of the logic should work since `identity_dir()` returns the right path

- [ ] **Step 5: Check compilation**

Run: `cargo check -p krillnotes-desktop --no-default-features --features rbac 2>&1 | head -30`
Expected: May still have errors in commands/workspace.rs — that's Task 6

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/src/commands/identity.rs
git commit -m "feat: update Tauri startup and identity commands for unified storage"
```

---

## Task 6: Tauri workspace commands

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/workspace.rs`

### Goal
Update `list_workspace_files`, `create_workspace`, `duplicate_workspace`, and `execute_import` to work with per-identity workspace folders.

- [ ] **Step 1: Rewrite `list_workspace_files`**

Currently scans one flat directory. New version scans all identity folders:

```rust
#[tauri::command]
pub fn list_workspace_files(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<WorkspaceEntry>, String> {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    let identities = mgr.list_identities().map_err(|e| e.to_string())?;

    // Build path → label map for open workspaces
    let open_labels: HashMap<PathBuf, String> = state
        .workspace_paths
        .lock()
        .expect("Mutex poisoned")
        .iter()
        .map(|(label, path)| (path.clone(), label.clone()))
        .collect();

    let mut entries = Vec::new();

    for identity_ref in &identities {
        let base_dir = match mgr.identity_base_dir(&identity_ref.uuid) {
            Some(dir) => dir,
            None => continue,
        };

        let read_dir = match std::fs::read_dir(&base_dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let folder = entry.path();
            if !folder.is_dir() { continue; }
            // Skip .identity directory
            if folder.file_name().map(|n| n == ".identity").unwrap_or(false) { continue; }
            let db_file = folder.join("notes.db");
            if !db_file.exists() { continue; }

            if let Some(name) = folder.file_name().and_then(|s| s.to_str()) {
                let is_open = open_labels.contains_key(&folder);

                // Refresh info.json for open workspaces
                if let Some(label) = open_labels.get(&folder) {
                    if let Some(ws) = state.workspaces.lock().expect("Mutex poisoned").get(label) {
                        let _ = ws.write_info_json();
                    }
                }

                let last_modified = std::fs::metadata(&folder)
                    .and_then(|m| m.modified())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
                    .unwrap_or(0);
                let size_bytes = dir_size_bytes(&folder);
                let (workspace_id, created_at, note_count, attachment_count) =
                    read_info_json_full(&folder);

                entries.push(WorkspaceEntry {
                    name: name.to_string(),
                    path: folder.display().to_string(),
                    is_open,
                    last_modified,
                    size_bytes,
                    created_at,
                    note_count,
                    attachment_count,
                    workspace_uuid: workspace_id,
                    identity_uuid: Some(identity_ref.uuid.to_string()),
                    identity_name: Some(identity_ref.display_name.clone()),
                });
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}
```

- [ ] **Step 2: Update `create_workspace` — use identity folder as parent**

Change the `path` parameter to `name`:

```rust
#[tauri::command]
pub async fn create_workspace(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    identity_uuid: String,
) -> std::result::Result<WorkspaceInfo, String> {
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Compute folder path from identity base dir
    let folder = {
        let mgr = state.identity_manager.lock().expect("Mutex poisoned");
        mgr.identity_base_dir(&uuid)
            .ok_or_else(|| format!("Identity folder not found for {identity_uuid}"))?
            .join(&name)
    };

    if folder.exists() {
        return Err("Workspace already exists. Use Open Workspace instead.".to_string());
    }

    // ... rest of the function stays the same but uses `folder` instead of `PathBuf::from(&path)`
```

The rest of the function body (`match find_window_for_path`, password generation, Workspace::create, bind_workspace, etc.) stays the same — just replace `PathBuf::from(&path)` with the computed `folder`.

- [ ] **Step 3: Update `duplicate_workspace` — compute dest path from identity folder**

Change the destination folder computation from:
```rust
let workspace_dir = PathBuf::from(&app_settings.workspace_directory);
let dest_folder = workspace_dir.join(&new_name);
```
To:
```rust
let dest_uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
let dest_folder = {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.identity_base_dir(&dest_uuid)
        .ok_or_else(|| format!("Identity folder not found for {identity_uuid}"))?
        .join(&new_name)
};
```

Remove `let app_settings = crate::settings::load_settings();` from this function.

- [ ] **Step 4: Update `execute_import`**

Find the `execute_import` command. Change the `folder_path` parameter to `name` + `identity_uuid`, and compute the path:

```rust
let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
let folder_path = {
    let mgr = state.identity_manager.lock().expect("Mutex poisoned");
    mgr.identity_base_dir(&uuid)
        .ok_or_else(|| format!("Identity folder not found for {identity_uuid}"))?
        .join(&name)
};
```

- [ ] **Step 5: Remove the deprecated `config_dir()` shim from settings.rs**

Now that all callers have been migrated, remove the `#[deprecated] pub fn config_dir()` shim added in Task 1.

- [ ] **Step 6: Full compilation check**

Run: `cargo check -p krillnotes-core -p krillnotes-desktop --no-default-features --features rbac 2>&1`
Expected: No errors

- [ ] **Step 7: Run all Rust tests**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/workspace.rs krillnotes-desktop/src-tauri/src/settings.rs
git commit -m "feat: update workspace commands for per-identity folder layout"
```

---

## Task 7: Frontend updates

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`
- Modify: `krillnotes-desktop/src/components/SettingsDialog.tsx`
- Modify: `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx`
- Modify: `krillnotes-desktop/src/components/WorkspaceManagerDialog.tsx`
- Modify: `krillnotes-desktop/src/App.tsx`

### Goal
Remove `workspaceDirectory` from types and UI. Update workspace creation to pass `name` instead of `path`. Update settings dialog to show home folder.

- [ ] **Step 1: Update types.ts**

Remove `workspaceDirectory` from `AppSettings`:

```typescript
export interface AppSettings {
  activeThemeMode?: string;
  lightTheme?: string;
  darkTheme?: string;
  language?: string;
  sharingIndicatorMode?: string;
}
```

- [ ] **Step 2: Update SettingsDialog.tsx**

1. Remove the workspace directory state variable and browse button
2. Remove `workspaceDirectory` from the `update_settings` patch
3. Optionally add a read-only display of the home folder path using the new `get_home_dir_path` command:

```typescript
const [homeDir, setHomeDir] = useState('');

useEffect(() => {
  invoke<string>('get_home_dir_path').then(setHomeDir);
}, []);
```

Display `homeDir` as read-only text in the General tab. Remove the old "Workspace Directory" picker.

- [ ] **Step 3: Update NewWorkspaceDialog.tsx**

Change the workspace creation from constructing a full path to passing just the name:

From:
```typescript
const path = `${workspaceDir}/${slug}`;
await invoke('create_workspace', { path, identityUuid });
```

To:
```typescript
await invoke('create_workspace', { name: slug, identityUuid });
```

Remove the `workspaceDir` state variable and the settings fetch for `workspaceDirectory`. Remove the path preview that showed `${workspaceDir}/${slug}`.

- [ ] **Step 4: Update WorkspaceManagerDialog.tsx**

The `list_workspace_files` command no longer depends on a settings directory. It should just work since the backend handles scanning all identity folders. Review the component and remove any references to `workspaceDirectory` if present.

- [ ] **Step 5: Update App.tsx import flow**

Find the import workspace flow that constructs:
```typescript
const folderPath = `${settings.workspaceDirectory}/${slug}`;
await invoke('execute_import', { zipPath, folderPath, password, identityUuid });
```

Change to:
```typescript
await invoke('execute_import', { zipPath, name: slug, password, identityUuid });
```

Remove the settings fetch for `workspaceDirectory` in the import flow.

- [ ] **Step 6: TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit 2>&1 | head -20`
Expected: No type errors

- [ ] **Step 7: Manual smoke test**

Run: `cd krillnotes-desktop && npm run tauri dev`

Verify:
1. App launches without errors
2. Can create a new identity (creates folder at `~/Krillnotes/{name}/`)
3. Can create a workspace under that identity
4. Can open the workspace
5. Settings dialog shows correctly (no workspace directory picker)
6. `~/Krillnotes/settings.json` exists

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/types.ts krillnotes-desktop/src/components/ krillnotes-desktop/src/App.tsx
git commit -m "feat: frontend updates for unified storage layout"
```

---

## Post-implementation cleanup

After all tasks are complete:

- [ ] Remove any remaining `#[allow(deprecated)]` or `config_dir()` references
- [ ] Run `cargo test -p krillnotes-core && cargo test -p krillnotes-desktop --no-default-features --features rbac`
- [ ] Run `cd krillnotes-desktop && npx tsc --noEmit`
- [ ] Full smoke test with `npm run tauri dev`
- [ ] Update CHANGELOG.md
