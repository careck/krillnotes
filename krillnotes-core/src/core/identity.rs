// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Per-workspace cryptographic identity management.
//!
//! Manages Ed25519 keypairs protected by Argon2id-derived passphrases.
//! Each identity is stored in a display-name folder under `home_dir/`,
//! with its encrypted key file inside `<folder>/.identity/identity.json`.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use aes_gcm::aead::rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::Result;

// ---------------------------------------------------------------------------
// Argon2id parameters
// ---------------------------------------------------------------------------

#[cfg(test)]
const ARGON2_M_COST: u32 = 1024; // 1 MiB — fast for tests
#[cfg(test)]
const ARGON2_T_COST: u32 = 1;

#[cfg(not(test))]
const ARGON2_M_COST: u32 = 65536; // 64 MiB — production
#[cfg(not(test))]
const ARGON2_T_COST: u32 = 3;

const ARGON2_P_COST: u32 = 1;

// ---------------------------------------------------------------------------
// Identity file format (on-disk JSON)
// ---------------------------------------------------------------------------

/// On-disk identity file: `<home_dir>/<display-name>/.identity/identity.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityFile {
    pub identity_uuid: Uuid,
    pub display_name: String,
    pub public_key: String,
    pub private_key_enc: EncryptedKey,
    #[serde(default)]
    pub last_used: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedKey {
    pub ciphertext: String,
    pub nonce: String,
    pub kdf: String,
    pub kdf_params: KdfParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub salt: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

// ---------------------------------------------------------------------------
// Identity references
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRef {
    pub uuid: Uuid,
    #[serde(alias = "display_name")]
    pub display_name: String,
    pub file: String,
    #[serde(alias = "last_used")]
    pub last_used: DateTime<Utc>,
}

/// Per-workspace binding stored in `<workspace_dir>/binding.json`.
/// `workspace_uuid` is included so callers can derive the HKDF key without
/// reading `info.json` separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBinding {
    pub workspace_uuid: String,
    pub identity_uuid: Uuid,
    pub db_password_enc: String,
}

// ---------------------------------------------------------------------------
// IdentityManager
// ---------------------------------------------------------------------------

/// Manages identity files in a display-name folder layout.
///
/// Layout: `home_dir/<display-name>/.identity/identity.json`
pub struct IdentityManager {
    home_dir: PathBuf,
    /// UUID -> folder name (the display-name folder as it appears on disk).
    folder_cache: HashMap<Uuid, String>,
}

/// Returned after successful unlock -- caller holds this and wipes on lock.
#[derive(Debug)]
pub struct UnlockedIdentity {
    pub identity_uuid: Uuid,
    pub display_name: String,
    pub signing_key: ed25519_dalek::SigningKey,
    pub verifying_key: ed25519_dalek::VerifyingKey,
}

impl UnlockedIdentity {
    /// Derives a 32-byte encryption key for this identity's contact book.
    /// Uses HKDF-SHA256 with the Ed25519 seed as IKM.
    pub fn contacts_key(&self) -> [u8; 32] {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, self.signing_key.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(b"krillnotes-contacts-v1", &mut okm)
            .expect("HKDF expand failed — output length is valid");
        okm
    }

    /// Derives a 32-byte encryption key for this identity's relay credentials.
    /// Uses HKDF-SHA256 with the Ed25519 seed as IKM.
    pub fn relay_key(&self) -> [u8; 32] {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, self.signing_key.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(b"krillnotes-relay-v1", &mut okm)
            .expect("HKDF expand failed — output length is valid");
        okm
    }

    /// Derives a per-device Ed25519 signing key for relay device registration.
    ///
    /// Each device + identity combination produces a unique keypair, derived via
    /// HKDF-SHA256 from the identity's seed and the device ID (a MAC-address hash).
    /// This ensures the relay can distinguish devices that share the same identity.
    pub fn device_signing_key(&self, device_id: &str) -> SigningKey {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, self.signing_key.as_bytes());
        let info = format!("krillnotes-device-key-v1:{device_id}");
        let mut okm = [0u8; 32];
        hk.expand(info.as_bytes(), &mut okm)
            .expect("HKDF expand failed — output length is valid");
        SigningKey::from_bytes(&okm)
    }
}

// ---------------------------------------------------------------------------
// .swarmid portable identity file
// ---------------------------------------------------------------------------

/// A relay account file embedded in a `.swarmid` export.
/// The `contents` field is the raw JSON of an already-encrypted relay account
/// file (AES-256-GCM with the identity's HKDF-derived relay key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedRelayFile {
    pub filename: String,
    pub contents: String,
}

/// Portable identity export file (`<name>.swarmid`).
/// The `identity` field is the same on-disk `IdentityFile` — private key
/// is already encrypted with Argon2id + AES-256-GCM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmIdFile {
    pub format: String,   // always "swarmid"
    pub version: u32,     // always 1
    pub identity: IdentityFile,
    /// Encrypted relay account files. Empty for old exports (backward compat).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<ExportedRelayFile>,
}

impl SwarmIdFile {
    pub const FORMAT: &'static str = "swarmid";
    pub const VERSION: u32 = 1;
}

impl IdentityManager {
    /// Create a new `IdentityManager`.
    ///
    /// Ensures `home_dir` exists and scans for identity folders.
    pub fn new(home_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&home_dir)?;
        let folder_cache = Self::scan_identities(&home_dir);
        Ok(Self { home_dir, folder_cache })
    }

    /// Scans `home_dir` for display-name folders that contain `.identity/identity.json`.
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

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Returns the display-name base directory for an identity, e.g. `home_dir/Alice`.
    pub fn identity_base_dir(&self, identity_uuid: &Uuid) -> Option<PathBuf> {
        self.folder_cache
            .get(identity_uuid)
            .map(|name| self.home_dir.join(name))
    }

    /// Returns the `.identity` directory for a given UUID.
    /// Falls back to `home_dir/<uuid>/.identity` if not in the cache (with a warning).
    pub fn identity_dir(&self, identity_uuid: &Uuid) -> PathBuf {
        match self.identity_base_dir(identity_uuid) {
            Some(base) => base.join(".identity"),
            None => {
                log::warn!("identity_dir: UUID {identity_uuid} not in cache");
                self.home_dir.join(identity_uuid.to_string()).join(".identity")
            }
        }
    }

    /// Returns the absolute path to the identity key file for a given UUID.
    pub fn identity_file_path(&self, identity_uuid: &Uuid) -> PathBuf {
        self.identity_dir(identity_uuid).join("identity.json")
    }

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

    /// Helper to create the standard `.identity/` subdirectory tree.
    fn create_identity_subdirs(identity_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(identity_dir)?;
        std::fs::create_dir_all(identity_dir.join("contacts"))?;
        std::fs::create_dir_all(identity_dir.join("invites"))?;
        std::fs::create_dir_all(identity_dir.join("relays"))?;
        std::fs::create_dir_all(identity_dir.join("accepted_invites"))?;
        std::fs::create_dir_all(identity_dir.join("invite_responses"))?;
        Ok(())
    }

    /// Create a new identity with the given display name and passphrase.
    ///
    /// Generates an Ed25519 keypair, encrypts the seed with Argon2id + AES-256-GCM,
    /// writes the identity file, and registers it in the folder cache.
    pub fn create_identity(&mut self, display_name: &str, passphrase: &str) -> Result<IdentityFile> {
        // Generate Ed25519 keypair
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        // Argon2id: derive encryption key from passphrase
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);

        let mut derived_key = [0u8; 32];
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
                .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 params: {e}")))?,
        );
        argon2
            .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 hash: {e}")))?;

        // AES-256-GCM: encrypt seed
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES key: {e}")))?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, seed.as_ref())
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES encrypt: {e}")))?;

        // Zero the seed from stack
        seed.fill(0);
        derived_key.fill(0);

        // Build identity file
        let identity_uuid = Uuid::new_v4();
        let identity_file = IdentityFile {
            identity_uuid,
            display_name: display_name.to_string(),
            public_key: BASE64.encode(verifying_key.as_bytes()),
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

        // Create display-name folder with .identity/ subdirs
        let folder_name = self.pick_folder_name(display_name);
        let identity_dir = self.home_dir.join(&folder_name).join(".identity");
        Self::create_identity_subdirs(&identity_dir)?;

        let file_path = identity_dir.join("identity.json");
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&file_path, json)?;

        // Update folder cache
        self.folder_cache.insert(identity_uuid, folder_name);

        Ok(identity_file)
    }

    /// Unlock an identity by decrypting its Ed25519 seed with the given passphrase.
    pub fn unlock_identity(&mut self, identity_uuid: &Uuid, passphrase: &str) -> Result<UnlockedIdentity> {
        // Load identity file
        let file_path = self.identity_file_path(identity_uuid);
        if !file_path.exists() {
            return Err(crate::KrillnotesError::IdentityNotFound(identity_uuid.to_string()));
        }
        let data = std::fs::read_to_string(&file_path)?;
        let mut identity_file: IdentityFile = serde_json::from_str(&data)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("JSON parse: {e}")))?;

        // Decode stored values
        let salt = BASE64.decode(&identity_file.private_key_enc.kdf_params.salt)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("salt decode: {e}")))?;
        let nonce_bytes = BASE64.decode(&identity_file.private_key_enc.nonce)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("nonce decode: {e}")))?;
        let ciphertext = BASE64.decode(&identity_file.private_key_enc.ciphertext)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("ciphertext decode: {e}")))?;

        // Argon2id: derive decryption key
        let params = &identity_file.private_key_enc.kdf_params;
        let mut derived_key = [0u8; 32];
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
                .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 params: {e}")))?,
        );
        argon2
            .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 hash: {e}")))?;

        // AES-256-GCM: decrypt seed
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES key: {e}")))?;
        derived_key.fill(0);

        let nonce = Nonce::from_slice(&nonce_bytes);
        let seed_bytes = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| crate::KrillnotesError::IdentityWrongPassphrase)?;

        let seed: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| crate::KrillnotesError::IdentityCorrupt("seed is not 32 bytes".to_string()))?;
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        // Update last_used timestamp directly in identity.json
        identity_file.last_used = Some(Utc::now());
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&file_path, json)?;

        Ok(UnlockedIdentity {
            identity_uuid: *identity_uuid,
            display_name: identity_file.display_name,
            signing_key,
            verifying_key,
        })
    }

    /// List all registered identities.
    pub fn list_identities(&self) -> Result<Vec<IdentityRef>> {
        let mut refs = Vec::new();
        for (uuid, folder_name) in &self.folder_cache {
            let identity_json = self.home_dir.join(folder_name).join(".identity").join("identity.json");
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
        refs.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        Ok(refs)
    }

    /// Look up the display name for a given base64-encoded public key.
    /// Reads each identity file until a match is found. Returns `None` if
    /// no local identity has that public key.
    pub fn lookup_display_name(&self, public_key: &str) -> Option<String> {
        for (_uuid, folder_name) in &self.folder_cache {
            let identity_json = self.home_dir.join(folder_name).join(".identity").join("identity.json");
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

    /// Delete an identity. Fails if any workspaces are still bound to it.
    pub fn delete_identity(&mut self, identity_uuid: &Uuid) -> Result<()> {
        let workspaces = self.get_workspaces_for_identity(identity_uuid)?;
        if !workspaces.is_empty() {
            return Err(crate::KrillnotesError::IdentityHasBoundWorkspaces(
                identity_uuid.to_string(),
            ));
        }

        if let Some(base_dir) = self.identity_base_dir(identity_uuid) {
            std::fs::remove_dir_all(&base_dir)?;
        }
        self.folder_cache.remove(identity_uuid);

        Ok(())
    }

    /// Change the passphrase for an identity.
    ///
    /// Decrypts the seed with the old passphrase, generates a new Argon2id salt,
    /// and re-encrypts with the new passphrase. The keypair is unchanged.
    pub fn change_passphrase(
        &mut self,
        identity_uuid: &Uuid,
        old_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<()> {
        // Unlock with old passphrase to get the seed
        let unlocked = self.unlock_identity(identity_uuid, old_passphrase)?;
        let seed = unlocked.signing_key.to_bytes();

        // Generate new Argon2id salt and derive new key
        let mut new_salt = [0u8; 16];
        OsRng.fill_bytes(&mut new_salt);

        let mut new_derived_key = [0u8; 32];
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
                .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 params: {e}")))?,
        );
        argon2
            .hash_password_into(new_passphrase.as_bytes(), &new_salt, &mut new_derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("Argon2 hash: {e}")))?;

        // Re-encrypt seed with new key
        let cipher = Aes256Gcm::new_from_slice(&new_derived_key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES key: {e}")))?;
        new_derived_key.fill(0);

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, seed.as_ref())
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES encrypt: {e}")))?;

        // Load and update identity file (preserves last_used)
        let file_path = self.identity_file_path(identity_uuid);
        let data = std::fs::read_to_string(&file_path)?;
        let mut identity_file: IdentityFile = serde_json::from_str(&data)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("JSON parse: {e}")))?;

        identity_file.private_key_enc = EncryptedKey {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
            kdf: "argon2id".to_string(),
            kdf_params: KdfParams {
                salt: BASE64.encode(new_salt),
                m_cost: ARGON2_M_COST,
                t_cost: ARGON2_T_COST,
                p_cost: ARGON2_P_COST,
            },
        };

        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&file_path, json)?;

        Ok(())
    }

    /// Renames an identity's display name and moves its folder.
    pub fn rename_identity(&mut self, identity_uuid: &Uuid, new_name: &str) -> Result<()> {
        // Update identity file
        let file_path = self.identity_file_path(identity_uuid);
        let raw = std::fs::read_to_string(&file_path)
            .map_err(|_| crate::KrillnotesError::IdentityNotFound(identity_uuid.to_string()))?;
        let mut id_file: IdentityFile = serde_json::from_str(&raw)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(e.to_string()))?;
        id_file.display_name = new_name.to_string();
        let json = serde_json::to_string_pretty(&id_file)?;
        std::fs::write(&file_path, json)?;

        // Rename folder if needed
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

    /// Verifies the passphrase, then returns a `SwarmIdFile` ready to be serialised
    /// and written to disk by the caller. Does NOT write any file itself.
    pub fn export_swarmid(&mut self, identity_uuid: &Uuid, passphrase: &str) -> Result<SwarmIdFile> {
        // Verify passphrase by attempting an unlock (propagates IdentityWrongPassphrase on mismatch)
        self.unlock_identity(identity_uuid, passphrase)?;
        self.export_swarmid_no_verify(identity_uuid)
    }

    /// Export without passphrase verification.
    /// Caller MUST ensure the identity is already unlocked (ownership proven).
    pub fn export_swarmid_no_verify(&self, identity_uuid: &Uuid) -> Result<SwarmIdFile> {
        // Read the raw IdentityFile from disk
        let file_path = self.identity_file_path(identity_uuid);
        let data = std::fs::read_to_string(&file_path)
            .map_err(|_| crate::KrillnotesError::IdentityNotFound(identity_uuid.to_string()))?;
        let identity: IdentityFile = serde_json::from_str(&data)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("JSON parse: {e}")))?;

        // Collect encrypted relay account files (if any exist).
        let relays_dir = self.identity_dir(identity_uuid).join("relays");
        let mut relays = Vec::new();
        if relays_dir.is_dir() {
            for entry in std::fs::read_dir(&relays_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let filename = path.file_name().unwrap().to_string_lossy().to_string();
                    let contents = std::fs::read_to_string(&path)?;
                    relays.push(ExportedRelayFile { filename, contents });
                }
            }
        }

        Ok(SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity,
            relays,
        })
    }

    /// Import a `.swarmid` file into this identity store (identity added in locked state).
    ///
    /// Returns `IdentityAlreadyExists` if the same UUID is already registered.
    /// Call `import_swarmid_overwrite` if the user confirms they want to replace it.
    pub fn import_swarmid(&mut self, file: SwarmIdFile) -> Result<IdentityRef> {
        self.validate_swarmid_file(&file)?;
        let uuid = file.identity.identity_uuid;
        if self.folder_cache.contains_key(&uuid) {
            return Err(crate::KrillnotesError::IdentityAlreadyExists(uuid.to_string()));
        }
        self.write_swarmid_to_store(file)
    }

    /// Import a `.swarmid` file, overwriting any existing identity with the same UUID.
    ///
    /// Workspace bindings for the overwritten UUID are intentionally preserved — the
    /// imported `.swarmid` is assumed to carry the same Ed25519 key material, so the
    /// bound DB passwords remain decryptable after import.
    pub fn import_swarmid_overwrite(&mut self, file: SwarmIdFile) -> Result<IdentityRef> {
        self.validate_swarmid_file(&file)?;
        let uuid = file.identity.identity_uuid;
        // Remove existing folder if present
        if let Some(base_dir) = self.identity_base_dir(&uuid) {
            std::fs::remove_dir_all(&base_dir)?;
        }
        self.folder_cache.remove(&uuid);
        self.write_swarmid_to_store(file)
    }

    fn validate_swarmid_file(&self, file: &SwarmIdFile) -> Result<()> {
        if file.format != SwarmIdFile::FORMAT {
            return Err(crate::KrillnotesError::SwarmIdInvalidFormat(format!(
                "expected \"swarmid\", got \"{}\"",
                file.format
            )));
        }
        if file.version != SwarmIdFile::VERSION {
            return Err(crate::KrillnotesError::SwarmIdVersionUnsupported(
                file.version,
            ));
        }
        Ok(())
    }

    fn write_swarmid_to_store(&mut self, file: SwarmIdFile) -> Result<IdentityRef> {
        let identity = file.identity;
        let uuid = identity.identity_uuid;
        let display_name = identity.display_name.clone();

        // Create display-name folder with .identity/ subdirs
        let folder_name = self.pick_folder_name(&display_name);
        let identity_dir = self.home_dir.join(&folder_name).join(".identity");
        Self::create_identity_subdirs(&identity_dir)?;

        let file_path = identity_dir.join("identity.json");
        let json = serde_json::to_string_pretty(&identity)?;
        std::fs::write(&file_path, json)?;

        // Restore encrypted relay account files from the export.
        let relays_dir = identity_dir.join("relays");
        for relay in &file.relays {
            let safe_name = match std::path::Path::new(&relay.filename).file_name() {
                Some(name) => name,
                None => continue,
            };
            std::fs::write(relays_dir.join(safe_name), &relay.contents)?;
        }

        // Update folder cache
        self.folder_cache.insert(uuid, folder_name);

        let identity_ref = IdentityRef {
            uuid,
            display_name,
            file: file_path.to_string_lossy().to_string(),
            last_used: identity.last_used.unwrap_or_else(Utc::now),
        };

        Ok(identity_ref)
    }

    /// Encrypts `db_password` with `seed` and writes a `binding.json` into `workspace_dir`.
    pub fn bind_workspace(
        &self,
        identity_uuid: &Uuid,
        workspace_uuid: &str,
        workspace_dir: &std::path::Path,
        db_password: &str,
        seed: &[u8; 32],
    ) -> Result<()> {
        let db_password_enc = self.encrypt_db_password(seed, workspace_uuid, db_password)?;
        let binding = WorkspaceBinding {
            workspace_uuid: workspace_uuid.to_string(),
            identity_uuid: *identity_uuid,
            db_password_enc,
        };
        let json = serde_json::to_string_pretty(&binding)?;
        std::fs::write(workspace_dir.join("binding.json"), json)?;
        Ok(())
    }

    /// Removes `binding.json` from `workspace_dir`. Returns `Ok(())` if already absent.
    pub fn unbind_workspace(&self, workspace_dir: &std::path::Path) -> Result<()> {
        let path = workspace_dir.join("binding.json");
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Reads `<workspace_dir>/binding.json`. Returns `None` if the file is absent.
    pub fn get_workspace_binding(&self, workspace_dir: &std::path::Path) -> Result<Option<WorkspaceBinding>> {
        let path = workspace_dir.join("binding.json");
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)?;
        let binding: WorkspaceBinding = serde_json::from_str(&raw)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(
                format!("binding.json in {:?}: {e}", workspace_dir)
            ))?;
        Ok(Some(binding))
    }

    /// Decrypts the DB password from `<workspace_dir>/binding.json`.
    /// Uses `workspace_uuid` stored in the binding for HKDF key derivation.
    pub fn decrypt_db_password(&self, workspace_dir: &std::path::Path, seed: &[u8; 32]) -> Result<String> {
        let binding = self.get_workspace_binding(workspace_dir)?
            .ok_or_else(|| crate::KrillnotesError::WorkspaceNotBound(
                workspace_dir.display().to_string()
            ))?;

        let key = self.derive_db_password_key(seed, &binding.workspace_uuid)?;
        let blob = BASE64.decode(&binding.db_password_enc)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("db_password_enc: {e}")))?;

        if blob.len() < 12 {
            return Err(crate::KrillnotesError::IdentityCorrupt("db_password_enc too short".into()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(e.to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|_| crate::KrillnotesError::IdentityCorrupt("decrypt failed".into()))?;
        String::from_utf8(plaintext)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(e.to_string()))
    }

    /// Scans the identity's base directory for workspace folders that contain
    /// `binding.json` files belonging to `identity_uuid`.
    /// Returns `(workspace_folder, WorkspaceBinding)` pairs.
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

    // --- private helpers ---

    fn derive_db_password_key(&self, seed: &[u8; 32], workspace_uuid: &str) -> Result<[u8; 32]> {
        use hkdf::Hkdf;
        use sha2::Sha256;

        let hk = Hkdf::<Sha256>::new(Some(workspace_uuid.as_bytes()), seed);
        let mut key = [0u8; 32];
        hk.expand(b"krillnotes-db-password-v1", &mut key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("HKDF expand: {e}")))?;
        Ok(key)
    }

    fn encrypt_db_password(&self, seed: &[u8; 32], workspace_uuid: &str, db_password: &str) -> Result<String> {
        let key = self.derive_db_password_key(seed, workspace_uuid)?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES key: {e}")))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, db_password.as_bytes())
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES encrypt: {e}")))?;

        // Store as nonce || ciphertext+tag
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        Ok(BASE64.encode(&blob))
    }
}

// ---------------------------------------------------------------------------
// Per-machine device UUID helpers
// ---------------------------------------------------------------------------

/// Returns the stable device UUID for this identity on this machine.
/// Creates the UUID file if it doesn't exist.
/// The UUID is stored as a plain string in `identity_dir/device_id`.
pub fn ensure_device_uuid(identity_dir: &std::path::Path) -> crate::Result<String> {
    let device_id_path = identity_dir.join("device_id");
    let new_uuid = Uuid::new_v4().to_string();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&device_id_path)
    {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(new_uuid.as_bytes())?;
            Ok(new_uuid)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let content = std::fs::read_to_string(&device_id_path)?;
            Ok(content.trim().to_string())
        }
        Err(e) => Err(crate::KrillnotesError::Io(e)),
    }
}

/// Extracts the identity UUID prefix from a composite `{identity_uuid}:{device_uuid}` device_id.
/// If the string contains ':', returns the part before ':'; otherwise returns the whole string.
pub fn identity_from_device_id(device_id: &str) -> &str {
    if let Some(pos) = device_id.find(':') {
        &device_id[..pos]
    } else {
        device_id
    }
}

/// Extracts the device UUID suffix from a composite `{identity_uuid}:{device_uuid}` device_id.
/// If the string contains ':', returns the part after ':'; otherwise returns the whole string.
/// This is the part used as the HLC node ID.
pub fn device_part_from_device_id(device_id: &str) -> &str {
    if let Some(pos) = device_id.find(':') {
        &device_id[pos + 1..]
    } else {
        device_id
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
