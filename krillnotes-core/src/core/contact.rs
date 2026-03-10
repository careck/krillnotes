// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Cross-workspace contacts address book.
//!
//! Each contact is stored as an AES-256-GCM encrypted JSON file in the
//! per-identity contacts directory.  The on-disk format is `EncryptedContactFile`
//! (a JSON envelope containing a base64 nonce + ciphertext).  A legacy
//! unencrypted path is retained via `ContactManager::new()` for backward
//! compatibility — new code should always use `ContactManager::for_identity()`.

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

/// How much the local user trusts this contact's claimed identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Keys compared in person via QR code or side-by-side display.
    VerifiedInPerson,
    /// Verification code confirmed over phone/video.
    CodeVerified,
    /// A verified peer vouched for this identity.
    Vouched,
    /// Accepted at first use without independent verification.
    Tofu,
}

/// A contact in the local address book.
///
/// Stored at `<contacts_dir>/<contact_id>.json` (encrypted).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub contact_id: Uuid,
    /// The name the person declared when creating their identity.
    pub declared_name: String,
    /// Optional local override — never propagated to peers.
    pub local_name: Option<String>,
    /// Ed25519 public key, base64-encoded.
    pub public_key: String,
    /// BLAKE3(pubkey_bytes) → 4 BIP-39 words, hyphen-separated.
    pub fingerprint: String,
    pub trust_level: TrustLevel,
    /// UUID of the contact who vouched for this one, if trust_level == Vouched.
    pub vouched_by: Option<Uuid>,
    pub first_seen: DateTime<Utc>,
    pub notes: Option<String>,
}

impl Contact {
    /// The name to display in the UI: local override if set, else declared name.
    pub fn display_name(&self) -> &str {
        self.local_name.as_deref().unwrap_or(&self.declared_name)
    }
}

/// On-disk format for an encrypted contact file.
#[derive(Serialize, Deserialize)]
struct EncryptedContactFile {
    /// base64-encoded AES-256-GCM ciphertext (includes the 16-byte authentication tag).
    ciphertext: String,
    /// base64-encoded 12-byte nonce.
    nonce: String,
}

/// Manages the contacts directory.
///
/// When constructed via [`ContactManager::for_identity`] all existing contacts
/// are decrypted and held in an in-memory cache for the lifetime of this
/// manager.  Writes update both the cache and the encrypted on-disk file
/// atomically (cache is updated after a successful disk write).
pub struct ContactManager {
    contacts_dir: PathBuf,
    /// `None` in legacy unencrypted mode (`ContactManager::new`).
    encryption_key: Option<[u8; 32]>,
    cache: RwLock<HashMap<Uuid, Contact>>,
}

impl ContactManager {
    /// Legacy constructor — unencrypted, for backward compatibility.
    ///
    /// Creates `config_dir/contacts/` if it does not exist.  The in-memory
    /// cache starts empty; `list_contacts()` on a manager created this way
    /// will return an empty list.  Do not use this for new code.
    pub fn new(config_dir: PathBuf) -> Result<Self> {
        let contacts_dir = config_dir.join("contacts");
        std::fs::create_dir_all(&contacts_dir)?;
        Ok(Self {
            contacts_dir,
            encryption_key: None,
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Per-identity constructor — contacts are AES-256-GCM encrypted with `key`.
    ///
    /// Creates `contacts_dir` if it does not exist, then decrypts and caches
    /// all existing `.json` files found there.  Returns an error if any
    /// existing file cannot be decrypted (e.g. wrong key).
    pub fn for_identity(contacts_dir: PathBuf, key: [u8; 32]) -> Result<Self> {
        std::fs::create_dir_all(&contacts_dir)?;
        let mgr = Self {
            contacts_dir,
            encryption_key: Some(key),
            cache: RwLock::new(HashMap::new()),
        };
        mgr.load_all_into_cache()?;
        Ok(mgr)
    }

    // ── private helpers ──────────────────────────────────────────────────────

    fn load_all_into_cache(&self) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        for entry in std::fs::read_dir(&self.contacts_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let contact = self.decrypt_file(&path)?;
            cache.insert(contact.contact_id, contact);
        }
        Ok(())
    }

    fn encrypt_contact(&self, contact: &Contact) -> Result<EncryptedContactFile> {
        let key_bytes = self.encryption_key.ok_or_else(|| {
            crate::KrillnotesError::ContactEncryption("No encryption key".into())
        })?;
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = serde_json::to_vec(contact)?;
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| crate::KrillnotesError::ContactEncryption(e.to_string()))?;

        Ok(EncryptedContactFile {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
        })
    }

    fn decrypt_file(&self, path: &std::path::Path) -> Result<Contact> {
        let key_bytes = self.encryption_key.ok_or_else(|| {
            crate::KrillnotesError::ContactEncryption("No encryption key".into())
        })?;
        let raw = std::fs::read_to_string(path)?;
        let enc: EncryptedContactFile = serde_json::from_str(&raw)?;

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let nonce_bytes = BASE64
            .decode(&enc.nonce)
            .map_err(|e| crate::KrillnotesError::ContactEncryption(e.to_string()))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = BASE64
            .decode(&enc.ciphertext)
            .map_err(|e| crate::KrillnotesError::ContactEncryption(e.to_string()))?;

        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| {
                crate::KrillnotesError::ContactEncryption(
                    "Decryption failed — wrong key?".into(),
                )
            })?;

        let contact: Contact = serde_json::from_slice(&plaintext)?;
        Ok(contact)
    }

    // ── public API ───────────────────────────────────────────────────────────

    /// Returns the on-disk path for the given contact UUID.
    pub fn path_for(&self, id: Uuid) -> PathBuf {
        self.contacts_dir.join(format!("{id}.json"))
    }

    /// Create a new contact and persist it.
    ///
    /// Returns an error if `public_key` is not valid base64.
    pub fn create_contact(
        &self,
        declared_name: &str,
        public_key: &str,
        trust_level: TrustLevel,
    ) -> Result<Contact> {
        let fingerprint = generate_fingerprint(public_key)?;
        let contact = Contact {
            contact_id: Uuid::new_v4(),
            declared_name: declared_name.to_string(),
            local_name: None,
            public_key: public_key.to_string(),
            fingerprint,
            trust_level,
            vouched_by: None,
            first_seen: Utc::now(),
            notes: None,
        };
        self.save_contact(&contact)?;
        Ok(contact)
    }

    /// Save (create or overwrite) a contact.
    ///
    /// Writes the encrypted (or plain, in legacy mode) file to disk, then
    /// updates the in-memory cache.
    pub fn save_contact(&self, contact: &Contact) -> Result<()> {
        if self.encryption_key.is_some() {
            let enc = self.encrypt_contact(contact)?;
            let json = serde_json::to_string_pretty(&enc)?;
            std::fs::write(self.path_for(contact.contact_id), json)?;
        } else {
            // Legacy unencrypted path
            let json = serde_json::to_string_pretty(contact)?;
            std::fs::write(self.path_for(contact.contact_id), json)?;
        }
        self.cache
            .write()
            .unwrap()
            .insert(contact.contact_id, contact.clone());
        Ok(())
    }

    /// Load a contact by UUID from the in-memory cache.
    ///
    /// Returns `None` if the contact does not exist.
    pub fn get_contact(&self, id: Uuid) -> Result<Option<Contact>> {
        Ok(self.cache.read().unwrap().get(&id).cloned())
    }

    /// Return all contacts sorted by display name.
    pub fn list_contacts(&self) -> Result<Vec<Contact>> {
        let cache = self.cache.read().unwrap();
        let mut list: Vec<Contact> = cache.values().cloned().collect();
        list.sort_by(|a, b| a.display_name().cmp(b.display_name()));
        Ok(list)
    }

    /// Find a contact by exact public key match.
    pub fn find_by_public_key(&self, public_key: &str) -> Result<Option<Contact>> {
        Ok(self
            .cache
            .read()
            .unwrap()
            .values()
            .find(|c| c.public_key == public_key)
            .cloned())
    }

    /// Find an existing contact by public key, or create a new one.
    pub fn find_or_create_by_public_key(
        &self,
        declared_name: &str,
        public_key: &str,
        trust_level: TrustLevel,
    ) -> Result<Contact> {
        if let Some(existing) = self.find_by_public_key(public_key)? {
            return Ok(existing);
        }
        self.create_contact(declared_name, public_key, trust_level)
    }

    /// Delete a contact from disk and the in-memory cache.
    pub fn delete_contact(&self, id: Uuid) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        self.cache.write().unwrap().remove(&id);
        Ok(())
    }
}

/// Generate a 4-word BIP-39 fingerprint from a base64-encoded public key.
///
/// Algorithm: BLAKE3(decoded_pubkey_bytes) → take first 44 bits →
/// split into four 11-bit indices → look up in BIP-39 word list.
pub fn generate_fingerprint(public_key_b64: &str) -> Result<String> {
    let key_bytes = BASE64.decode(public_key_b64).map_err(|e| {
        crate::KrillnotesError::IdentityCorrupt(format!("invalid public key base64: {e}"))
    })?;
    let hash = blake3::hash(&key_bytes);
    let hash_bytes = hash.as_bytes();

    // Extract four 11-bit indices from the first 6 bytes (48 bits, use 44).
    let b = hash_bytes;
    let idx0 = (((b[0] as u16) << 3) | ((b[1] as u16) >> 5)) & 0x7FF;
    let idx1 = (((b[1] as u16) << 6) | ((b[2] as u16) >> 2)) & 0x7FF;
    let idx2 =
        (((b[2] as u16) << 9) | ((b[3] as u16) << 1) | ((b[4] as u16) >> 7)) & 0x7FF;
    let idx3 = (((b[4] as u16) << 4) | ((b[5] as u16) >> 4)) & 0x7FF;

    // Use bip39 crate to get the English word list.
    let wordlist = bip39::Language::English.word_list();
    let words = [
        wordlist[idx0 as usize],
        wordlist[idx1 as usize],
        wordlist[idx2 as usize],
        wordlist[idx3 as usize],
    ];
    Ok(words.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tempfile::tempdir;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    // ── legacy (unencrypted) tests ───────────────────────────────────────────

    fn mgr(tmp: &TempDir) -> ContactManager {
        ContactManager::new(tmp.path().to_path_buf()).unwrap()
    }

    #[test]
    fn test_create_and_read_contact() {
        let tmp = TempDir::new().unwrap();
        let mgr = mgr(&tmp);
        let pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let c = mgr.create_contact("Alice", pubkey, TrustLevel::Tofu).unwrap();
        assert_eq!(c.declared_name, "Alice");
        assert_eq!(c.local_name, None);
        // Legacy new() cache is empty after construction — save_contact populates it.
        let fetched = mgr.get_contact(c.contact_id).unwrap().unwrap();
        assert_eq!(fetched.declared_name, "Alice");
    }

    #[test]
    fn test_display_name_prefers_local_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = mgr(&tmp);
        let pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let mut c = mgr.create_contact("Bob Chen", pubkey, TrustLevel::Tofu).unwrap();
        c.local_name = Some("Robert — Field Lead".to_string());
        mgr.save_contact(&c).unwrap();
        let fetched = mgr.get_contact(c.contact_id).unwrap().unwrap();
        assert_eq!(fetched.display_name(), "Robert — Field Lead");
    }

    #[test]
    fn test_find_by_public_key_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let mgr = mgr(&tmp);
        let pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let c1 = mgr.create_contact("Alice", pubkey, TrustLevel::Tofu).unwrap();
        // Second create with same pubkey should return existing
        let c2 = mgr
            .find_or_create_by_public_key("Alice", pubkey, TrustLevel::Tofu)
            .unwrap();
        assert_eq!(c1.contact_id, c2.contact_id);
    }

    #[test]
    fn test_fingerprint_is_four_words() {
        let pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let fp = generate_fingerprint(pubkey).unwrap();
        let words: Vec<&str> = fp.split('-').collect();
        assert_eq!(words.len(), 4);
        assert!(words.iter().all(|w| !w.is_empty()));
    }

    #[test]
    fn test_list_contacts() {
        let tmp = TempDir::new().unwrap();
        let mgr = mgr(&tmp);
        mgr.create_contact(
            "Alice",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            TrustLevel::Tofu,
        )
        .unwrap();
        mgr.create_contact(
            "Bob",
            "BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            TrustLevel::Tofu,
        )
        .unwrap();
        let list = mgr.list_contacts().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_delete_contact() {
        let tmp = TempDir::new().unwrap();
        let mgr = mgr(&tmp);
        let c = mgr
            .create_contact(
                "Alice",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                TrustLevel::Tofu,
            )
            .unwrap();
        mgr.delete_contact(c.contact_id).unwrap();
        assert!(mgr.get_contact(c.contact_id).unwrap().is_none());
    }

    // ── encrypted (for_identity) tests ──────────────────────────────────────

    #[test]
    fn encrypted_contact_roundtrip() {
        let dir = tempdir().unwrap();
        let contacts_dir = dir.path().join("contacts");

        // Create a contact
        let mgr =
            ContactManager::for_identity(contacts_dir.clone(), test_key()).unwrap();
        let contact = mgr
            .create_contact(
                "Alice",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                TrustLevel::Tofu,
            )
            .unwrap();

        // On-disk file must NOT be readable as plain JSON Contact
        let raw =
            std::fs::read_to_string(mgr.path_for(contact.contact_id)).unwrap();
        assert!(
            serde_json::from_str::<Contact>(&raw).is_err(),
            "File must not be plain JSON"
        );

        // Load fresh manager from same dir — contact must survive
        let mgr2 =
            ContactManager::for_identity(contacts_dir, test_key()).unwrap();
        let loaded = mgr2.get_contact(contact.contact_id).unwrap().unwrap();
        assert_eq!(loaded.declared_name, "Alice");
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let dir = tempdir().unwrap();
        let contacts_dir = dir.path().join("contacts");

        let mgr =
            ContactManager::for_identity(contacts_dir.clone(), test_key()).unwrap();
        mgr.create_contact(
            "Bob",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            TrustLevel::Tofu,
        )
        .unwrap();

        let wrong_key = [99u8; 32];
        let result = ContactManager::for_identity(contacts_dir, wrong_key);
        assert!(result.is_err(), "Wrong key must fail to load contacts");
    }

    #[test]
    fn list_and_delete_contact() {
        let dir = tempdir().unwrap();
        let mgr =
            ContactManager::for_identity(dir.path().join("c"), test_key()).unwrap();
        mgr.create_contact(
            "Alice",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            TrustLevel::Tofu,
        )
        .unwrap();
        mgr.create_contact(
            "Bob",
            "BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            TrustLevel::CodeVerified,
        )
        .unwrap();
        let list = mgr.list_contacts().unwrap();
        assert_eq!(list.len(), 2);

        let alice = list.iter().find(|c| c.declared_name == "Alice").unwrap();
        mgr.delete_contact(alice.contact_id).unwrap();
        assert_eq!(mgr.list_contacts().unwrap().len(), 1);
    }
}
