// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Per-identity relay account management.
//!
//! Each relay account is stored as an AES-256-GCM encrypted JSON file in the
//! per-identity relays directory (`identities/<uuid>/relays/<account_id>.json`).
//! The on-disk format is `EncryptedRelayAccountFile` (a JSON envelope containing
//! a base64 nonce + ciphertext).  This mirrors the `ContactManager` pattern.

use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use uuid::Uuid;

use crate::Result;

/// A relay account stored per-identity.
///
/// Stored at `<relays_dir>/<relay_account_id>.json` (encrypted).
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for RelayAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayAccount")
            .field("relay_account_id", &self.relay_account_id)
            .field("relay_url", &self.relay_url)
            .field("email", &self.email)
            .field("password", &"[REDACTED]")
            .field("session_token", &"[REDACTED]")
            .field("session_expires_at", &self.session_expires_at)
            .field("device_public_key", &self.device_public_key)
            .finish()
    }
}

impl Drop for RelayAccount {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        self.session_token.zeroize();
    }
}

/// On-disk format for an encrypted relay account file.
#[derive(Serialize, Deserialize)]
struct EncryptedRelayAccountFile {
    /// base64-encoded 12-byte nonce.
    nonce: String,
    /// base64-encoded AES-256-GCM ciphertext (includes the 16-byte authentication tag).
    ciphertext: String,
}

/// Manages the relay accounts directory for a single identity.
///
/// When constructed via [`RelayAccountManager::for_identity`] all existing
/// accounts are decrypted and held in an in-memory cache for the lifetime of
/// this manager.  Writes update both the cache and the encrypted on-disk file
/// atomically (cache is updated after a successful disk write).
pub struct RelayAccountManager {
    relays_dir: PathBuf,
    /// AES-256-GCM encryption key derived from the identity passphrase.
    encryption_key: Option<[u8; 32]>,
    cache: RwLock<HashMap<Uuid, RelayAccount>>,
}

impl RelayAccountManager {
    /// Per-identity constructor — accounts are AES-256-GCM encrypted with `key`.
    ///
    /// Creates `relays_dir` if it does not exist, then decrypts and caches
    /// all existing `.json` files found there.  Returns an error if any
    /// existing file cannot be decrypted (e.g. wrong key).
    pub fn for_identity(relays_dir: PathBuf, key: [u8; 32]) -> Result<Self> {
        std::fs::create_dir_all(&relays_dir)?;
        let mgr = Self {
            relays_dir,
            encryption_key: Some(key),
            cache: RwLock::new(HashMap::new()),
        };
        mgr.load_all_into_cache()?;
        Ok(mgr)
    }

    // -- private helpers ------------------------------------------------------

    fn load_all_into_cache(&self) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        for entry in std::fs::read_dir(&self.relays_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let account = self.decrypt_file(&path)?;
            cache.insert(account.relay_account_id, account);
        }
        Ok(())
    }

    fn encrypt_account(&self, account: &RelayAccount) -> Result<EncryptedRelayAccountFile> {
        let key_bytes = self
            .encryption_key
            .ok_or_else(|| crate::KrillnotesError::RelayEncryption("No encryption key".into()))?;
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = serde_json::to_vec(account)?;
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| crate::KrillnotesError::RelayEncryption(e.to_string()))?;

        Ok(EncryptedRelayAccountFile {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
        })
    }

    fn decrypt_file(&self, path: &std::path::Path) -> Result<RelayAccount> {
        let key_bytes = self
            .encryption_key
            .ok_or_else(|| crate::KrillnotesError::RelayEncryption("No encryption key".into()))?;
        let raw = std::fs::read_to_string(path)?;
        let enc: EncryptedRelayAccountFile = serde_json::from_str(&raw)?;

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let nonce_bytes = BASE64
            .decode(&enc.nonce)
            .map_err(|e| crate::KrillnotesError::RelayEncryption(e.to_string()))?;
        if nonce_bytes.len() != 12 {
            return Err(crate::KrillnotesError::RelayEncryption(format!(
                "invalid nonce length: {} bytes",
                nonce_bytes.len()
            )));
        }
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = BASE64
            .decode(&enc.ciphertext)
            .map_err(|e| crate::KrillnotesError::RelayEncryption(e.to_string()))?;

        let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|_| {
            crate::KrillnotesError::RelayEncryption("Decryption failed — wrong key?".into())
        })?;

        let account: RelayAccount = serde_json::from_slice(&plaintext)?;
        Ok(account)
    }

    // -- public API -----------------------------------------------------------

    /// Returns the on-disk path for the given relay account UUID.
    pub fn path_for(&self, id: Uuid) -> PathBuf {
        self.relays_dir.join(format!("{id}.json"))
    }

    /// Create a new relay account and persist it.
    ///
    /// Returns an error if a relay account with the same URL already exists.
    pub fn create_relay_account(
        &self,
        relay_url: &str,
        email: &str,
        password: &str,
        session_token: &str,
        session_expires_at: DateTime<Utc>,
        device_public_key: &str,
    ) -> Result<RelayAccount> {
        if let Some(existing) = self.find_by_url(relay_url)? {
            return Err(crate::KrillnotesError::RelayEncryption(format!(
                "A relay account for {} already exists (id: {})",
                existing.relay_url, existing.relay_account_id
            )));
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

    /// Save (create or overwrite) a relay account.
    ///
    /// Writes the encrypted file to disk, then updates the in-memory cache.
    pub fn save_relay_account(&self, account: &RelayAccount) -> Result<()> {
        let enc = self.encrypt_account(account)?;
        let json = serde_json::to_string_pretty(&enc)?;
        std::fs::write(self.path_for(account.relay_account_id), json)?;
        self.cache
            .write()
            .unwrap()
            .insert(account.relay_account_id, account.clone());
        Ok(())
    }

    /// Load a relay account by UUID from the in-memory cache.
    ///
    /// Returns `None` if the account does not exist.
    pub fn get_relay_account(&self, id: Uuid) -> Result<Option<RelayAccount>> {
        Ok(self.cache.read().unwrap().get(&id).cloned())
    }

    /// Return all relay accounts sorted by relay URL.
    pub fn list_relay_accounts(&self) -> Result<Vec<RelayAccount>> {
        let cache = self.cache.read().unwrap();
        let mut list: Vec<RelayAccount> = cache.values().cloned().collect();
        list.sort_by(|a, b| a.relay_url.cmp(&b.relay_url));
        Ok(list)
    }

    /// Find a relay account by exact URL match.
    pub fn find_by_url(&self, url: &str) -> Result<Option<RelayAccount>> {
        Ok(self
            .cache
            .read()
            .unwrap()
            .values()
            .find(|a| a.relay_url == url)
            .cloned())
    }

    /// Delete a relay account from disk and the in-memory cache.
    pub fn delete_relay_account(&self, id: Uuid) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        self.cache.write().unwrap().remove(&id);
        Ok(())
    }
}

impl Drop for RelayAccountManager {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(key) = self.encryption_key.as_mut() {
            key.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn test_create_and_list_relay_accounts() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");

        let mgr = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let expires = Utc::now() + chrono::Duration::days(30);
        let account = mgr
            .create_relay_account(
                "https://relay.example.com",
                "alice@example.com",
                "s3cret",
                "tok_abc123",
                expires,
                "deadbeef",
            )
            .unwrap();

        assert_eq!(account.relay_url, "https://relay.example.com");
        assert_eq!(account.email, "alice@example.com");
        assert_eq!(account.password, "s3cret");
        assert_eq!(account.session_token, "tok_abc123");
        assert_eq!(account.device_public_key, "deadbeef");

        let list = mgr.list_relay_accounts().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].relay_account_id, account.relay_account_id);
    }

    #[test]
    fn test_find_by_url_deduplication() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");

        let mgr = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let expires = Utc::now() + chrono::Duration::days(30);

        mgr.create_relay_account(
            "https://relay.example.com",
            "alice@example.com",
            "pass1",
            "tok_1",
            expires,
            "key1",
        )
        .unwrap();

        // Trying to create a second account with the same URL should fail.
        let result = mgr.create_relay_account(
            "https://relay.example.com",
            "bob@example.com",
            "pass2",
            "tok_2",
            expires,
            "key2",
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("already exists"),
            "Error should mention 'already exists', got: {err_msg}"
        );
    }

    #[test]
    fn test_get_relay_account() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");

        let mgr = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let expires = Utc::now() + chrono::Duration::days(30);

        let account = mgr
            .create_relay_account(
                "https://relay.example.com",
                "alice@example.com",
                "pass",
                "tok_abc",
                expires,
                "pubkey",
            )
            .unwrap();

        let fetched = mgr
            .get_relay_account(account.relay_account_id)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.relay_url, "https://relay.example.com");
        assert_eq!(fetched.email, "alice@example.com");

        // Non-existent ID returns None.
        assert!(mgr.get_relay_account(Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn test_save_updates_existing() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");

        let mgr = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let expires = Utc::now() + chrono::Duration::days(30);

        let mut account = mgr
            .create_relay_account(
                "https://relay.example.com",
                "alice@example.com",
                "pass",
                "tok_old",
                expires,
                "pubkey",
            )
            .unwrap();

        // Update the session token.
        account.session_token = "tok_new".to_string();
        mgr.save_relay_account(&account).unwrap();

        let fetched = mgr
            .get_relay_account(account.relay_account_id)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.session_token, "tok_new");

        // List should still have exactly one account.
        assert_eq!(mgr.list_relay_accounts().unwrap().len(), 1);
    }

    #[test]
    fn test_delete_relay_account() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");

        let mgr = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let expires = Utc::now() + chrono::Duration::days(30);

        let account = mgr
            .create_relay_account(
                "https://relay.example.com",
                "alice@example.com",
                "pass",
                "tok",
                expires,
                "key",
            )
            .unwrap();

        mgr.delete_relay_account(account.relay_account_id).unwrap();
        assert!(mgr.list_relay_accounts().unwrap().is_empty());
        assert!(mgr
            .get_relay_account(account.relay_account_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");
        let expires = Utc::now() + chrono::Duration::days(30);

        let account_id;
        {
            let mgr = RelayAccountManager::for_identity(relays_dir.clone(), test_key()).unwrap();
            let account = mgr
                .create_relay_account(
                    "https://relay.example.com",
                    "alice@example.com",
                    "s3cret-password",
                    "tok_persist",
                    expires,
                    "pubkey123",
                )
                .unwrap();
            account_id = account.relay_account_id;
        }

        // Load a fresh manager from the same directory.
        let mgr2 = RelayAccountManager::for_identity(relays_dir, test_key()).unwrap();
        let loaded = mgr2.get_relay_account(account_id).unwrap().unwrap();

        assert_eq!(loaded.relay_url, "https://relay.example.com");
        assert_eq!(loaded.email, "alice@example.com");
        assert_eq!(loaded.password, "s3cret-password");
        assert_eq!(loaded.session_token, "tok_persist");
        assert_eq!(loaded.device_public_key, "pubkey123");
    }

    #[test]
    fn test_debug_redacts_password_and_token() {
        let account = RelayAccount {
            relay_account_id: Uuid::new_v4(),
            relay_url: "https://relay.example.com".to_string(),
            email: "test@test.com".to_string(),
            password: "super-secret-password".to_string(),
            session_token: "secret-token-value".to_string(),
            session_expires_at: Utc::now(),
            device_public_key: "pk-123".to_string(),
        };
        let debug_output = format!("{:?}", account);
        assert!(
            !debug_output.contains("super-secret-password"),
            "Debug output must not contain password"
        );
        assert!(
            !debug_output.contains("secret-token-value"),
            "Debug output must not contain session_token"
        );
        assert!(debug_output.contains("[REDACTED]"));
        assert!(debug_output.contains("relay.example.com"));
    }

    #[test]
    fn test_wrong_key_fails_to_load() {
        let dir = tempdir().unwrap();
        let relays_dir = dir.path().join("relays");
        let expires = Utc::now() + chrono::Duration::days(30);

        {
            let mgr = RelayAccountManager::for_identity(relays_dir.clone(), test_key()).unwrap();
            mgr.create_relay_account(
                "https://relay.example.com",
                "alice@example.com",
                "pass",
                "tok",
                expires,
                "key",
            )
            .unwrap();
        }

        let wrong_key = [99u8; 32];
        let result = RelayAccountManager::for_identity(relays_dir, wrong_key);
        assert!(
            result.is_err(),
            "Wrong key must fail to load relay accounts"
        );
    }
}
