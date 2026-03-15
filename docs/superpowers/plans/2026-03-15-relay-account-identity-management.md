# Relay Account Identity-Level Management — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move relay account storage and management from a per-peer concern to the Identity Manager, with auto-login and a simple relay picker in Workspace Peers.

**Architecture:** New `RelayAccountManager` in krillnotes-core mirrors the existing `ContactManager` pattern — per-identity encrypted files in `identities/<uuid>/relays/`, in-memory cache, initialized on unlock. Tauri commands wrap it. React gets a RelayBookDialog (like ContactBookDialog) and a relay picker dropdown in WorkspacePeersDialog.

**Tech Stack:** Rust (krillnotes-core), Tauri v2 commands, React 19 + Tailwind v4, AES-256-GCM encryption, reqwest::blocking (via spawn_blocking)

**Spec:** `docs/superpowers/specs/2026-03-15-relay-account-identity-management-design.md`

---

## Chunk 1: Core Rust — RelayAccountManager

### Task 1: Add RelayEncryption error variant

**Files:**
- Modify: `krillnotes-core/src/core/error.rs:62` (add variant near ContactEncryption)

- [ ] **Step 1: Add the error variant**

In `error.rs`, add after the `ContactEncryption` variant (line 62-63):

```rust
#[error("Relay encryption error: {0}")]
RelayEncryption(String),
```

Also add the user_message match arm in the `user_message()` method (near line 178 where ContactEncryption is handled):

```rust
KrillnotesError::RelayEncryption(msg) => msg.clone(),
```

- [ ] **Step 2: Run tests to verify no breakage**

Run: `cargo test -p krillnotes-core --lib`
Expected: All existing tests pass

- [ ] **Step 3: Commit**

```
feat(core): add RelayEncryption error variant
```

---

### Task 2: Create RelayAccount struct and RelayAccountManager

**Files:**
- Create: `krillnotes-core/src/core/sync/relay/relay_account.rs`
- Modify: `krillnotes-core/src/core/sync/relay/mod.rs` (add pub mod + re-exports)

- [ ] **Step 1: Write tests for RelayAccountManager**

Create the test module at the bottom of `relay_account.rs`. Tests mirror the contact.rs test patterns:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use chrono::Utc;

    fn test_key() -> [u8; 32] {
        [0xAA; 32]
    }

    fn make_account(url: &str, email: &str) -> RelayAccount {
        RelayAccount {
            relay_account_id: Uuid::new_v4(),
            relay_url: url.to_string(),
            email: email.to_string(),
            password: "secret123".to_string(),
            session_token: "tok_abc".to_string(),
            session_expires_at: Utc::now() + chrono::Duration::days(30),
            device_public_key: "deadbeef".to_string(),
        }
    }

    #[test]
    fn test_create_and_list_relay_accounts() {
        let dir = TempDir::new().unwrap();
        let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();

        let acct = mgr.create_relay_account(
            "https://relay.example.com",
            "user@example.com",
            "password123",
            "tok_abc",
            Utc::now() + chrono::Duration::days(30),
            "deadbeef",
        ).unwrap();

        assert_eq!(acct.relay_url, "https://relay.example.com");
        assert_eq!(acct.email, "user@example.com");
        assert_eq!(acct.password, "password123");

        let list = mgr.list_relay_accounts().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].relay_account_id, acct.relay_account_id);
    }

    #[test]
    fn test_find_by_url_deduplication() {
        let dir = TempDir::new().unwrap();
        let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();

        mgr.create_relay_account(
            "https://relay.example.com", "user@example.com",
            "pass", "tok", Utc::now(), "key",
        ).unwrap();

        let result = mgr.create_relay_account(
            "https://relay.example.com", "other@example.com",
            "pass", "tok", Utc::now(), "key",
        );
        assert!(result.is_err()); // duplicate URL rejected
    }

    #[test]
    fn test_get_relay_account() {
        let dir = TempDir::new().unwrap();
        let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();

        let acct = mgr.create_relay_account(
            "https://relay.example.com", "user@example.com",
            "pass", "tok", Utc::now(), "key",
        ).unwrap();

        let found = mgr.get_relay_account(acct.relay_account_id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().email, "user@example.com");
    }

    #[test]
    fn test_save_updates_existing() {
        let dir = TempDir::new().unwrap();
        let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();

        let mut acct = mgr.create_relay_account(
            "https://relay.example.com", "user@example.com",
            "pass", "old_tok", Utc::now(), "key",
        ).unwrap();

        acct.session_token = "new_tok".to_string();
        mgr.save_relay_account(&acct).unwrap();

        let found = mgr.get_relay_account(acct.relay_account_id).unwrap().unwrap();
        assert_eq!(found.session_token, "new_tok");
    }

    #[test]
    fn test_delete_relay_account() {
        let dir = TempDir::new().unwrap();
        let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();

        let acct = mgr.create_relay_account(
            "https://relay.example.com", "user@example.com",
            "pass", "tok", Utc::now(), "key",
        ).unwrap();

        mgr.delete_relay_account(acct.relay_account_id).unwrap();
        let list = mgr.list_relay_accounts().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = TempDir::new().unwrap();
        let key = test_key();

        {
            let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), key).unwrap();
            mgr.create_relay_account(
                "https://relay.example.com", "user@example.com",
                "pass", "tok", Utc::now(), "key",
            ).unwrap();
        }

        // New instance should load from disk
        let mgr2 = RelayAccountManager::for_identity(dir.path().to_path_buf(), key).unwrap();
        let list = mgr2.list_relay_accounts().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].email, "user@example.com");
        assert_eq!(list[0].password, "pass"); // password survives roundtrip
    }

    #[test]
    fn test_wrong_key_fails_to_load() {
        let dir = TempDir::new().unwrap();

        {
            let mgr = RelayAccountManager::for_identity(dir.path().to_path_buf(), test_key()).unwrap();
            mgr.create_relay_account(
                "https://relay.example.com", "user@example.com",
                "pass", "tok", Utc::now(), "key",
            ).unwrap();
        }

        let wrong_key = [0xBB; 32];
        let result = RelayAccountManager::for_identity(dir.path().to_path_buf(), wrong_key);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (no implementation yet)**

Run: `cargo test -p krillnotes-core --lib relay_account`
Expected: Compilation failure (types don't exist yet)

- [ ] **Step 3: Implement RelayAccount struct and EncryptedRelayAccountFile**

In `relay_account.rs`, add the structs:

```rust
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

use crate::KrillnotesError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccount {
    pub relay_account_id: Uuid,
    pub relay_url: String,
    pub email: String,
    pub password: String,
    pub session_token: String,
    pub session_expires_at: DateTime<Utc>,
    pub device_public_key: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedRelayAccountFile {
    nonce: String,      // base64
    ciphertext: String, // base64
}
```

- [ ] **Step 4: Implement RelayAccountManager**

In `relay_account.rs`, add the manager struct and methods. Mirror the `ContactManager` pattern from `contact.rs`:

```rust
pub struct RelayAccountManager {
    relays_dir: PathBuf,
    encryption_key: Option<[u8; 32]>,
    cache: RwLock<HashMap<Uuid, RelayAccount>>,
}

impl RelayAccountManager {
    /// Create manager and load all existing relay accounts from disk.
    pub fn for_identity(relays_dir: PathBuf, key: [u8; 32]) -> Result<Self, KrillnotesError> {
        std::fs::create_dir_all(&relays_dir).map_err(|e| {
            KrillnotesError::RelayEncryption(format!("Failed to create relays dir: {e}"))
        })?;
        let mgr = Self {
            relays_dir,
            encryption_key: Some(key),
            cache: RwLock::new(HashMap::new()),
        };
        mgr.load_all_into_cache()?;
        Ok(mgr)
    }

    fn load_all_into_cache(&self) -> Result<(), KrillnotesError> {
        let mut cache = self.cache.write().unwrap();
        for entry in std::fs::read_dir(&self.relays_dir)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("read_dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| KrillnotesError::RelayEncryption(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let acct = self.decrypt_file(&path)?;
                cache.insert(acct.relay_account_id, acct);
            }
        }
        Ok(())
    }

    fn encrypt_account(&self, account: &RelayAccount) -> Result<EncryptedRelayAccountFile, KrillnotesError> {
        let key_bytes = self.encryption_key.as_ref()
            .ok_or_else(|| KrillnotesError::RelayEncryption("No encryption key".into()))?;
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(key_bytes);
        let cipher = Aes256Gcm::new(key);
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = serde_json::to_vec(account)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("serialize: {e}")))?;
        let ct = cipher.encrypt(nonce, plaintext.as_ref())
            .map_err(|e| KrillnotesError::RelayEncryption(format!("encrypt: {e}")))?;
        Ok(EncryptedRelayAccountFile {
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(ct),
        })
    }

    fn decrypt_file(&self, path: &std::path::Path) -> Result<RelayAccount, KrillnotesError> {
        let key_bytes = self.encryption_key.as_ref()
            .ok_or_else(|| KrillnotesError::RelayEncryption("No encryption key".into()))?;
        let data = std::fs::read_to_string(path)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("read: {e}")))?;
        let envelope: EncryptedRelayAccountFile = serde_json::from_str(&data)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("parse envelope: {e}")))?;
        let nonce_bytes = BASE64.decode(&envelope.nonce)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("nonce decode: {e}")))?;
        let ct_bytes = BASE64.decode(&envelope.ciphertext)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("ct decode: {e}")))?;

        if nonce_bytes.len() != 12 {
            return Err(KrillnotesError::RelayEncryption(
                format!("invalid nonce length: {}", nonce_bytes.len()),
            ));
        }
        let nonce = Nonce::from_slice(&nonce_bytes);
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(key_bytes);
        let cipher = Aes256Gcm::new(key);
        let plaintext = cipher.decrypt(nonce, ct_bytes.as_ref())
            .map_err(|e| KrillnotesError::RelayEncryption(format!("decrypt: {e}")))?;
        serde_json::from_slice(&plaintext)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("deserialize: {e}")))
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.relays_dir.join(format!("{id}.json"))
    }

    pub fn create_relay_account(
        &self,
        relay_url: &str,
        email: &str,
        password: &str,
        session_token: &str,
        session_expires_at: DateTime<Utc>,
        device_public_key: &str,
    ) -> Result<RelayAccount, KrillnotesError> {
        // Enforce one account per URL
        if self.find_by_url(relay_url)?.is_some() {
            return Err(KrillnotesError::RelayEncryption(
                format!("Relay account already exists for URL: {relay_url}"),
            ));
        }
        let account = RelayAccount {
            relay_account_id: Uuid::new_v4(),
            relay_url: relay_url.to_string(),
            email: email.to_string(),
            password: password.to_string(),
            session_token: session_token.to_string(),
            session_expires_at,
            device_public_key: device_public_key.to_string(),
        };
        self.save_relay_account(&account)?;
        Ok(account)
    }

    pub fn save_relay_account(&self, account: &RelayAccount) -> Result<(), KrillnotesError> {
        let encrypted = self.encrypt_account(account)?;
        let json = serde_json::to_string_pretty(&encrypted)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("serialize: {e}")))?;
        std::fs::write(self.path_for(account.relay_account_id), json)
            .map_err(|e| KrillnotesError::RelayEncryption(format!("write: {e}")))?;
        self.cache.write().unwrap().insert(account.relay_account_id, account.clone());
        Ok(())
    }

    pub fn get_relay_account(&self, id: Uuid) -> Result<Option<RelayAccount>, KrillnotesError> {
        Ok(self.cache.read().unwrap().get(&id).cloned())
    }

    pub fn list_relay_accounts(&self) -> Result<Vec<RelayAccount>, KrillnotesError> {
        let cache = self.cache.read().unwrap();
        let mut accounts: Vec<_> = cache.values().cloned().collect();
        accounts.sort_by(|a, b| a.relay_url.cmp(&b.relay_url));
        Ok(accounts)
    }

    pub fn find_by_url(&self, url: &str) -> Result<Option<RelayAccount>, KrillnotesError> {
        Ok(self.cache.read().unwrap().values().find(|a| a.relay_url == url).cloned())
    }

    pub fn delete_relay_account(&self, id: Uuid) -> Result<(), KrillnotesError> {
        let path = self.path_for(id);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| KrillnotesError::RelayEncryption(format!("delete: {e}")))?;
        }
        self.cache.write().unwrap().remove(&id);
        Ok(())
    }
}

impl Drop for RelayAccountManager {
    fn drop(&mut self) {
        if let Some(ref mut key) = self.encryption_key {
            key.fill(0);
        }
    }
}
```

- [ ] **Step 5: Add module to mod.rs**

In `krillnotes-core/src/core/sync/relay/mod.rs`, add after existing module declarations. **Note:** this is NOT feature-gated (unlike `pub mod client` which is `#[cfg(feature = "relay")]`), because `RelayAccountManager` only depends on `aes_gcm`/`serde`/`uuid` which are always available:

```rust
pub mod relay_account;
pub use relay_account::{RelayAccount, RelayAccountManager};
```

- [ ] **Step 6: Verify crate access path**

The Tauri commands will use the full path `krillnotes_core::core::sync::relay::RelayAccountManager` (via the re-export in `mod.rs`). No additional re-export in `krillnotes-core/src/lib.rs` is needed — the existing `pub mod core` chain is sufficient.

- [ ] **Step 7: Run tests**

Run: `cargo test -p krillnotes-core --lib relay_account`
Expected: All 7 tests pass

- [ ] **Step 8: Commit**

```
feat(core): add RelayAccountManager with encrypted per-identity storage
```

---

## Chunk 2: Tauri Command Layer

### Task 3: Add relay_account_managers to AppState and lifecycle

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:39-80` (AppState struct)
- Modify: `krillnotes-desktop/src-tauri/src/commands/identity.rs:119-163` (unlock_identity)
- Modify: `krillnotes-desktop/src-tauri/src/commands/identity.rs:166-206` (lock_identity)

- [ ] **Step 1: Add field to AppState**

In `lib.rs`, add after `invite_managers` field (around line 57):

```rust
pub relay_account_managers: Arc<Mutex<HashMap<Uuid, krillnotes_core::core::sync::relay::relay_account::RelayAccountManager>>>,
```

And initialize it in the `Default` or constructor (wherever `contact_managers` is initialized):

```rust
relay_account_managers: Arc::new(Mutex::new(HashMap::new())),
```

- [ ] **Step 2: Initialize RelayAccountManager on unlock_identity**

In `identity.rs`, in the `unlock_identity` function (after ContactManager initialization around line 148), add:

```rust
// Initialize RelayAccountManager
let relays_dir = identity_dir.join("relays");
let relay_key = unlocked.relay_key();
let relay_mgr = krillnotes_core::core::sync::relay::relay_account::RelayAccountManager::for_identity(
    relays_dir, relay_key,
).map_err(|e| e.to_string())?;
state.relay_account_managers.lock().unwrap().insert(uuid, relay_mgr);
```

- [ ] **Step 3: Add migration from old relay credentials**

In `unlock_identity`, after creating the `RelayAccountManager`, add migration logic:

```rust
// Migrate old-style relay credentials if they exist.
// NOTE: This runs synchronously before unlock_identity returns,
// so RelayAccountManager is fully populated before any workspace opens.
// Lock acquisition order: relay_account_managers is acquired and released
// before any other state locks in this block.
let old_relay_dir = crate::settings::config_dir().join("relay");
if let Ok(Some(old_creds)) = krillnotes_core::core::sync::relay::load_relay_credentials(
    &old_relay_dir, &identity_uuid, &relay_key,
) {
    // Acquire lock, do work, release — same pattern as contact_managers usage
    {
        let managers = state.relay_account_managers.lock().unwrap();
        if let Some(mgr) = managers.get(&uuid) {
            if mgr.find_by_url(&old_creds.relay_url).unwrap_or(None).is_none() {
                let _ = mgr.create_relay_account(
                    &old_creds.relay_url,
                    &old_creds.email,
                    "",  // old format has no password — user must re-login once
                    &old_creds.session_token,
                    old_creds.session_expires_at,
                    &old_creds.device_public_key,
                );
            }
        }
    } // managers lock released here

    // Delete old file (uses existing delete_relay_credentials from auth.rs)
    let _ = krillnotes_core::core::sync::relay::delete_relay_credentials(
        &old_relay_dir, &identity_uuid,
    );
}
```

- [ ] **Step 4: Clear RelayAccountManager on lock_identity**

In `lock_identity`, add alongside the existing `contact_managers.remove` (around line 202):

```rust
state.relay_account_managers.lock().unwrap().remove(&uuid);
```

- [ ] **Step 5: Run existing tests to verify no breakage**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles without errors

- [ ] **Step 6: Commit**

```
feat(desktop): wire RelayAccountManager into AppState lifecycle with migration
```

---

### Task 4: New Tauri commands for relay account CRUD

**Files:**
- Create: `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/mod.rs` (add pub mod)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:286-421` (generate_handler!)

- [ ] **Step 1: Create relay_accounts.rs with list and delete commands**

```rust
use serde::Serialize;
use tauri::State;

use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccountInfo {
    pub relay_account_id: String,
    pub relay_url: String,
    pub email: String,
    pub session_valid: bool,
}

#[tauri::command]
pub fn list_relay_accounts(
    state: State<'_, AppState>,
    identity_uuid: String,
) -> Result<Vec<RelayAccountInfo>, String> {
    let uuid = uuid::Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let managers = state.relay_account_managers.lock().unwrap();
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let accounts = mgr.list_relay_accounts().map_err(|e| e.to_string())?;
    Ok(accounts
        .into_iter()
        .map(|a| RelayAccountInfo {
            relay_account_id: a.relay_account_id.to_string(),
            relay_url: a.relay_url,
            email: a.email,
            session_valid: a.session_expires_at > chrono::Utc::now(),
        })
        .collect())
}

#[tauri::command]
pub fn delete_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_account_id: String,
) -> Result<(), String> {
    let uuid = uuid::Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;
    let acct_id = uuid::Uuid::parse_str(&relay_account_id).map_err(|e| e.to_string())?;
    let managers = state.relay_account_managers.lock().unwrap();
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    mgr.delete_relay_account(acct_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Add register_relay_account command**

This mirrors the existing `configure_relay` logic but stores via `RelayAccountManager`:

```rust
#[tauri::command]
pub async fn register_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<RelayAccountInfo, String> {
    let uuid = uuid::Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    // Get identity keys
    let (signing_key, verifying_key, relay_key) = {
        let identities = state.unlocked_identities.lock().unwrap();
        let id = identities.get(&uuid).ok_or("Identity not unlocked")?;
        (id.signing_key.clone(), id.verifying_key, id.relay_key())
    };

    let device_public_key = hex::encode(verifying_key.to_bytes());
    let relay_url_clone = relay_url.clone();
    let email_clone = email.clone();
    let password_clone = password.clone();
    let dpk = device_public_key.clone();

    // Register on relay server (blocking HTTP via spawn_blocking)
    let session = tokio::task::spawn_blocking(move || {
        let client = krillnotes_core::core::sync::relay::RelayClient::new(&relay_url_clone);

        // RegisterResult is a struct { account_id, challenge }, not an enum
        let result = client.register(&email_clone, &password_clone, &uuid.to_string(), &dpk)
            .map_err(|e| e.to_string())?;

        // Decrypt PoP challenge using identity's Ed25519 key
        let nonce = krillnotes_core::core::sync::relay::auth::decrypt_pop_challenge(
            &signing_key,
            &result.challenge.encrypted_nonce,
            &result.challenge.server_public_key,
        ).map_err(|e| e.to_string())?;

        let nonce_hex = hex::encode(&nonce);
        let session = client.register_verify(&dpk, &nonce_hex)
            .map_err(|e| e.to_string())?;

        Ok::<_, String>(session)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e: String| e)?;

    // Store in RelayAccountManager
    let expires = chrono::Utc::now() + chrono::Duration::days(30);
    let managers = state.relay_account_managers.lock().unwrap();
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;
    let account = mgr.create_relay_account(
        &relay_url, &email, &password,
        &session.session_token, expires, &device_public_key,
    ).map_err(|e| e.to_string())?;

    Ok(RelayAccountInfo {
        relay_account_id: account.relay_account_id.to_string(),
        relay_url: account.relay_url,
        email: account.email,
        session_valid: true,
    })
}
```

- [ ] **Step 3: Add login_relay_account command**

```rust
#[tauri::command]
pub async fn login_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<RelayAccountInfo, String> {
    let uuid = uuid::Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let verifying_key = {
        let identities = state.unlocked_identities.lock().unwrap();
        let id = identities.get(&uuid).ok_or("Identity not unlocked")?;
        id.verifying_key
    };

    let device_public_key = hex::encode(verifying_key.to_bytes());
    let relay_url_clone = relay_url.clone();
    let email_clone = email.clone();
    let password_clone = password.clone();
    let dpk = device_public_key.clone();

    let session = tokio::task::spawn_blocking(move || {
        let client = krillnotes_core::core::sync::relay::RelayClient::new(&relay_url_clone);
        client.login(&email_clone, &password_clone, &dpk)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e: String| e)?;

    let expires = chrono::Utc::now() + chrono::Duration::days(30);
    let managers = state.relay_account_managers.lock().unwrap();
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;

    // Check if account for this URL already exists (update it) or create new
    if let Some(mut existing) = mgr.find_by_url(&relay_url).map_err(|e| e.to_string())? {
        existing.email = email;
        existing.password = password;
        existing.session_token = session.session_token;
        existing.session_expires_at = expires;
        existing.device_public_key = device_public_key;
        mgr.save_relay_account(&existing).map_err(|e| e.to_string())?;
        Ok(RelayAccountInfo {
            relay_account_id: existing.relay_account_id.to_string(),
            relay_url: existing.relay_url,
            email: existing.email,
            session_valid: true,
        })
    } else {
        let account = mgr.create_relay_account(
            &relay_url, &email, &password,
            &session.session_token, expires, &device_public_key,
        ).map_err(|e| e.to_string())?;
        Ok(RelayAccountInfo {
            relay_account_id: account.relay_account_id.to_string(),
            relay_url: account.relay_url,
            email: account.email,
            session_valid: true,
        })
    }
}
```

- [ ] **Step 4: Add set_peer_relay command**

```rust
#[tauri::command]
pub async fn set_peer_relay(
    window: tauri::Window,
    state: State<'_, AppState>,
    peer_device_id: String,
    relay_account_id: String,
) -> Result<(), String> {
    let workspace_label = window.label().to_string();
    let workspaces = state.workspaces.lock().unwrap();
    let ws = workspaces.get(&workspace_label)
        .ok_or_else(|| format!("Workspace not found: {workspace_label}"))?;
    let channel_params = serde_json::json!({
        "relay_account_id": relay_account_id
    }).to_string();
    ws.update_peer_channel(&peer_device_id, "relay", &channel_params)
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Register module and commands in mod.rs and lib.rs**

In `commands/mod.rs`, add:
```rust
pub mod relay_accounts;
```

In `lib.rs`, add to `generate_handler![]`:
```rust
commands::relay_accounts::list_relay_accounts,
commands::relay_accounts::register_relay_account,
commands::relay_accounts::login_relay_account,
commands::relay_accounts::delete_relay_account,
commands::relay_accounts::set_peer_relay,
```

- [ ] **Step 6: Build to verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles

- [ ] **Step 7: Commit**

```
feat(desktop): add Tauri commands for relay account CRUD
```

---

### Task 5: Update poll_sync to use RelayAccountManager

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs:69-173` (poll_sync)

- [ ] **Step 1: Refactor poll_sync relay channel creation**

Replace the current relay credential loading (lines 101-104, 127-134) with:

```rust
// Load relay accounts for peers that use relay channel
let relay_accounts: Vec<krillnotes_core::core::sync::relay::RelayAccount> = {
    let ram = state.relay_account_managers.lock().unwrap();
    if let Some(mgr) = ram.get(&identity_uuid) {
        mgr.list_relay_accounts().unwrap_or_default()
    } else {
        vec![]
    }
};
```

Then when registering channels, instead of creating a single RelayChannel, create one per distinct relay account referenced by peers:

```rust
// Register relay channels for each relay account that has peers
for acct in &relay_accounts {
    let relay_client = RelayClient::new(&acct.relay_url)
        .with_session_token(&acct.session_token);
    engine.register_channel(Box::new(RelayChannel::new(
        relay_client,
        workspace_id_str.clone(),
        acct.device_public_key.clone(),
    )));
}
```

**Auto-login in poll_sync**: If a relay account's session is expired and it has a stored password, attempt login inside the `spawn_blocking` closure before creating the channel. Since the closure only has cloned `Vec<RelayAccount>` (not the manager), the refreshed token is used for this sync but NOT persisted to disk. Persistence of refreshed tokens is handled by the fire-and-forget auto-login in `unlock_identity` (Task 6). This is acceptable because `poll_sync` runs frequently and the next `unlock_identity` will persist.

```rust
for acct in &relay_accounts {
    let mut token = acct.session_token.clone();
    if acct.session_expires_at < chrono::Utc::now() && !acct.password.is_empty() {
        let client = RelayClient::new(&acct.relay_url);
        if let Ok(session) = client.login(&acct.email, &acct.password, &acct.device_public_key) {
            token = session.session_token;
        }
    }
    let relay_client = RelayClient::new(&acct.relay_url)
        .with_session_token(&token);
    engine.register_channel(Box::new(RelayChannel::new(
        relay_client,
        workspace_id_str.clone(),
        acct.device_public_key.clone(),
    )));
}
```

- [ ] **Step 2: Add channel_params migration in poll_sync**

In `poll_sync`, after loading workspace peers but before the `spawn_blocking` closure, migrate old-format `channel_params`. This runs in the Tauri command context where `AppState` is available:

```rust
// Migrate old relay channel_params: {"relay_url": "..."} → {"relay_account_id": "<uuid>"}
let peers = ws.list_peers().map_err(|e| e.to_string())?;
for peer in &peers {
    if peer.channel_type == "relay" {
        if let Ok(params) = serde_json::from_str::<serde_json::Value>(&peer.channel_params) {
            if params.get("relay_url").is_some() && params.get("relay_account_id").is_none() {
                // Old format — look up relay account by URL
                if let Some(url) = params["relay_url"].as_str() {
                    let managers = state.relay_account_managers.lock().unwrap();
                    if let Some(mgr) = managers.get(&identity_uuid) {
                        if let Ok(Some(acct)) = mgr.find_by_url(url) {
                            let new_params = serde_json::json!({
                                "relay_account_id": acct.relay_account_id.to_string()
                            }).to_string();
                            let _ = ws.update_peer_channel(
                                &peer.peer_device_id, "relay", &new_params,
                            );
                        } else {
                            // No matching relay account — fall back to manual
                            log::warn!("No relay account for URL {url}, setting peer {} to manual", peer.peer_device_id);
                            let _ = ws.update_peer_channel(
                                &peer.peer_device_id, "manual", "{}",
                            );
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles

- [ ] **Step 4: Commit**

```
feat(desktop): update poll_sync to use RelayAccountManager with per-account channels
```

---

### Task 6: Add auto-login on unlock_identity

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/identity.rs` (unlock_identity)

- [ ] **Step 1: Add fire-and-forget auto-login after RelayAccountManager init**

After the migration logic in `unlock_identity`, spawn a background task. **IMPORTANT:** `unlock_identity` is `pub fn` (synchronous) — do NOT change it to `pub async fn`. `tokio::task::spawn` works here because Tauri v2 runs sync commands within a Tokio runtime context.

```rust
// Fire-and-forget: auto-login expired relay sessions.
// This runs in background — unlock_identity returns immediately.
let ram_clone = state.relay_account_managers.clone();
let uuid_clone = uuid;
tokio::task::spawn(async move {
    let accounts = {
        let mgrs = ram_clone.lock().unwrap();
        match mgrs.get(&uuid_clone) {
            Some(mgr) => mgr.list_relay_accounts().unwrap_or_default(),
            None => return,
        }
    };

    for acct in accounts {
        if acct.session_expires_at > chrono::Utc::now() || acct.password.is_empty() {
            continue;
        }

        let dpk = acct.device_public_key.clone();
        let url = acct.relay_url.clone();
        let email = acct.email.clone();
        let pw = acct.password.clone();
        let acct_id = acct.relay_account_id;

        let result = tokio::task::spawn_blocking(move || {
            let client = krillnotes_core::core::sync::relay::RelayClient::new(&url);
            client.login(&email, &pw, &dpk)
        })
        .await;

        let mgrs = ram_clone.lock().unwrap();
        if let Some(mgr) = mgrs.get(&uuid_clone) {
            if let Ok(Some(mut updated)) = mgr.get_relay_account(acct_id) {
                match result {
                    Ok(Ok(session)) => {
                        // Success: update token and expiry
                        updated.session_token = session.session_token;
                        updated.session_expires_at = chrono::Utc::now() + chrono::Duration::days(30);
                        let _ = mgr.save_relay_account(&updated);
                    }
                    Ok(Err(_login_err)) => {
                        // Auth failure (wrong password, account deleted server-side):
                        // mark session as invalid so UI shows expired status
                        updated.session_expires_at = chrono::DateTime::<chrono::Utc>::MIN_UTC;
                        let _ = mgr.save_relay_account(&updated);
                        log::warn!("Auto-login failed for relay {}, marking session invalid", acct.relay_url);
                    }
                    Err(_join_err) => {
                        // spawn_blocking panicked — skip silently
                    }
                }
            }
        }
    }
});
```

- [ ] **Step 2: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles

- [ ] **Step 3: Commit**

```
feat(desktop): auto-login expired relay sessions on identity unlock
```

---

## Chunk 3: Frontend — Identity Manager Integration

### Task 7: Add TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:263-266` (replace RelayInfo with RelayAccountInfo)

- [ ] **Step 1: Add RelayAccountInfo type and update RelayInfo**

In `types.ts`, replace or add alongside the existing `RelayInfo`:

```typescript
export interface RelayAccountInfo {
  relayAccountId: string;
  relayUrl: string;
  email: string;
  sessionValid: boolean;
}
```

Keep `RelayInfo` if it's still referenced elsewhere, or remove if all uses will be migrated.

- [ ] **Step 2: Commit**

```
feat(ui): add RelayAccountInfo TypeScript type
```

---

### Task 8: Create RelayBookDialog

**Files:**
- Create: `krillnotes-desktop/src/components/RelayBookDialog.tsx`

- [ ] **Step 1: Implement RelayBookDialog**

Model on `ContactBookDialog.tsx` (170 lines). Props:

```typescript
interface Props {
  identityUuid: string;
  identityName: string;
  onClose: () => void;
}
```

The dialog:
- Calls `invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid })` on mount
- Shows list of relay accounts: URL, email, session status badge (green/red dot)
- "Add Relay Account" button opens `AddRelayAccountDialog`
- Click on an account opens `EditRelayAccountDialog`
- Has a Close button

Use `useTranslation()` for i18n strings — add relay-specific keys to `en.json` and other locale files.

- [ ] **Step 2: Commit**

```
feat(ui): add RelayBookDialog component
```

---

### Task 9: Create AddRelayAccountDialog

**Files:**
- Create: `krillnotes-desktop/src/components/AddRelayAccountDialog.tsx`

- [ ] **Step 1: Implement AddRelayAccountDialog**

Adapt from `ConfigureRelayDialog.tsx` (218 lines) but simpler — no `peerDeviceId` prop:

```typescript
interface Props {
  identityUuid: string;
  onClose: () => void;
  onCreated: () => void;
}
```

Two tabs: Register and Login.

**Register tab:**
- Relay URL input (default: `https://swarm.krillnotes.org`)
- Email input
- Password input
- Confirm Password input
- Submit calls `invoke('register_relay_account', { identityUuid, relayUrl, email, password })`

**Login tab:**
- Relay URL input
- Email input
- Password input
- Submit calls `invoke('login_relay_account', { identityUuid, relayUrl, email, password })`

On success: calls `onCreated()` which refreshes the list in `RelayBookDialog`.

Reuse the same error mapping from `ConfigureRelayDialog`.

- [ ] **Step 2: Commit**

```
feat(ui): add AddRelayAccountDialog component
```

---

### Task 10: Create EditRelayAccountDialog

**Files:**
- Create: `krillnotes-desktop/src/components/EditRelayAccountDialog.tsx`

- [ ] **Step 1: Implement EditRelayAccountDialog**

Simple dialog:

```typescript
interface Props {
  identityUuid: string;
  account: RelayAccountInfo;
  onClose: () => void;
  onDeleted: () => void;
}
```

Shows:
- Relay URL (read-only)
- Email (read-only)
- Session status (valid/expired indicator)
- Delete button with confirmation prompt

Delete calls `invoke('delete_relay_account', { identityUuid, relayAccountId: account.relayAccountId })`.

- [ ] **Step 2: Commit**

```
feat(ui): add EditRelayAccountDialog component
```

---

### Task 11: Add "Relays (N)" button to IdentityManagerDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/IdentityManagerDialog.tsx:496-502` (toolbar area)

- [ ] **Step 1: Add relay count state and loading**

Add state alongside the existing `contactCounts`:

```typescript
const [relayCounts, setRelayCounts] = useState<Record<string, number>>({});
```

In the identity loading effect, for each unlocked identity, call:

```typescript
const relays = await invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid: id.uuid });
```

And store the count.

- [ ] **Step 2: Add "Relays" button next to "Contacts" button**

In the toolbar area (near line 498), add:

```tsx
<button
  onClick={() => setShowRelayBook(selectedUuid)}
  disabled={!selectedUuid || !unlockedUuids.includes(selectedUuid!)}
>
  {t('identityManager.relays')} ({relayCounts[selectedUuid!] ?? 0})
</button>
```

- [ ] **Step 3: Add RelayBookDialog rendering**

Add state: `const [showRelayBook, setShowRelayBook] = useState<string | null>(null);`

Render when open:
```tsx
{showRelayBook && (
  <RelayBookDialog
    identityUuid={showRelayBook}
    identityName={identities.find(i => i.uuid === showRelayBook)?.displayName ?? ''}
    onClose={() => { setShowRelayBook(null); loadRelayCounts(); }}
  />
)}
```

- [ ] **Step 4: Add i18n keys**

Add to `en.json` and other locale files:
```json
"identityManager.relays": "Relays",
"relayBook.title": "Relay Accounts",
"relayBook.addRelay": "Add Relay Account",
"relayBook.noRelays": "No relay accounts configured.",
"relayBook.sessionValid": "Session active",
"relayBook.sessionExpired": "Session expired",
"addRelay.title": "Add Relay Account",
"addRelay.register": "Register",
"addRelay.login": "Login",
"editRelay.title": "Relay Account",
"editRelay.delete": "Delete",
"editRelay.confirmDelete": "Delete this relay account?"
```

- [ ] **Step 5: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No type errors

- [ ] **Step 6: Commit**

```
feat(ui): add Relays button to Identity Manager with relay book dialog
```

---

## Chunk 4: Frontend — Workspace Peers Relay Picker + Cleanup

### Task 12: Replace relay config with dropdown picker in WorkspacePeersDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx:264-279, 452-462`

- [ ] **Step 1: Load relay accounts for the bound identity**

Add state and loading:

```typescript
const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
```

On dialog mount (or when identity is known), load:

```typescript
const accounts = await invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid });
setRelayAccounts(accounts);
```

- [ ] **Step 2: Replace relay channel configuration flow**

Where the current code (lines 274-279) checks `selectedChannelType === 'relay'` and opens `ConfigureRelayDialog`, replace with:

If `relayAccounts.length === 0`:
- Show inline message: "No relay accounts configured. Add one in Identity Manager → Relays."

If `relayAccounts.length > 0`:
- Show a `<select>` dropdown with relay accounts formatted as `email @ relay_url`
- On selection, call `invoke('set_peer_relay', { peerDeviceId, relayAccountId: selectedAccountId })`
- Reload peers

- [ ] **Step 3: Remove ConfigureRelayDialog import and rendering**

Remove the `showConfigureRelay` state, the `<ConfigureRelayDialog>` rendering block (lines 452-462), and the import.

- [ ] **Step 4: Verify TypeScript compiles**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No type errors

- [ ] **Step 5: Commit**

```
feat(ui): replace relay config dialog with dropdown picker in Workspace Peers
```

---

### Task 13: Delete ConfigureRelayDialog and remove old Tauri commands

**Files:**
- Delete: `krillnotes-desktop/src/components/ConfigureRelayDialog.tsx`
- Modify: `krillnotes-desktop/src-tauri/src/commands/sync.rs` (remove configure_relay, relay_login, get_relay_info)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (remove from generate_handler!)

- [ ] **Step 1: Delete ConfigureRelayDialog.tsx**

Remove the file entirely.

- [ ] **Step 2: Remove old Tauri commands**

In `sync.rs`, remove the `configure_relay` function (lines 179-250), `relay_login` function (lines 256-318), and `get_relay_info` function (lines 389-419). Also remove the `RelayInfo` struct (lines 29-34) if no longer used.

In `lib.rs`, remove `configure_relay`, `relay_login`, `get_relay_info` from `generate_handler![]`.

- [ ] **Step 3: Search for any remaining references**

Grep for `configure_relay`, `relay_login`, `get_relay_info`, `ConfigureRelayDialog` across the codebase. Fix any remaining imports or references.

- [ ] **Step 4: Build and type-check**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop && npx tsc --noEmit`
Expected: Both pass

- [ ] **Step 5: Commit**

```
refactor: remove old per-peer relay configuration code
```

---

### Task 14: Final integration test

**Files:** None (manual testing)

- [ ] **Step 1: Run all Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: All pass

- [ ] **Step 2: Run dev server**

Run: `cd krillnotes-desktop && npm run tauri dev`
Expected: App launches

- [ ] **Step 3: Manual smoke test**

1. Create/unlock an identity
2. Open Identity Manager → verify "Relays (0)" button appears
3. Click Relays → verify empty state message
4. Add a relay account (register or login)
5. Verify it appears in the list with session status
6. Close and reopen → verify persistence
7. Open a workspace → add a peer → select relay channel
8. Verify dropdown shows the relay account
9. Select it → verify peer is configured
10. Lock identity → unlock again → verify relay accounts still present

- [ ] **Step 4: Commit any fixes from smoke test**

```
fix: address issues found during relay account integration testing
```

---

## Summary of Files Changed

### Created
- `krillnotes-core/src/core/sync/relay/relay_account.rs` — RelayAccount + RelayAccountManager
- `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs` — Tauri CRUD commands
- `krillnotes-desktop/src/components/RelayBookDialog.tsx` — Relay list dialog
- `krillnotes-desktop/src/components/AddRelayAccountDialog.tsx` — Register/login dialog
- `krillnotes-desktop/src/components/EditRelayAccountDialog.tsx` — View/delete dialog

### Modified
- `krillnotes-core/src/core/error.rs` — Add `RelayEncryption` variant
- `krillnotes-core/src/core/sync/relay/mod.rs` — Add module + re-exports
- `krillnotes-desktop/src-tauri/src/lib.rs` — AppState field + generate_handler
- `krillnotes-desktop/src-tauri/src/commands/identity.rs` — Lifecycle + migration + auto-login
- `krillnotes-desktop/src-tauri/src/commands/sync.rs` — poll_sync refactor, remove old commands
- `krillnotes-desktop/src-tauri/src/commands/mod.rs` — Add module
- `krillnotes-desktop/src/types.ts` — Add RelayAccountInfo
- `krillnotes-desktop/src/components/IdentityManagerDialog.tsx` — Relays button
- `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx` — Relay picker dropdown
- `krillnotes-desktop/src/i18n/locales/*.json` — i18n keys

### Deleted
- `krillnotes-desktop/src/components/ConfigureRelayDialog.tsx`
