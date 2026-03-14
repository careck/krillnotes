// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Per-workspace cryptographic identity management.
//!
//! Manages Ed25519 keypairs protected by Argon2id-derived passphrases.
//! Each identity is stored as an encrypted JSON file. A separate settings
//! file binds workspaces to identities with encrypted DB passwords.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use serde::{Deserialize, Serialize};
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

/// On-disk identity file: `~/.config/krillnotes/identities/<uuid>.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityFile {
    pub identity_uuid: Uuid,
    pub display_name: String,
    pub public_key: String,
    pub private_key_enc: EncryptedKey,
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
// Identity settings (workspace registry)
// ---------------------------------------------------------------------------

/// `~/.config/krillnotes/identity_settings.json`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentitySettings {
    #[serde(default)]
    pub identities: Vec<IdentityRef>,
    /// Migration-only: readable from old files, never written back.
    #[serde(default, skip_serializing)]
    pub workspaces: std::collections::HashMap<String, LegacyWorkspaceBinding>,
}

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

/// Legacy workspace binding as stored in `identity_settings.json.workspaces`.
/// Read-only during migration; never written after migration runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyWorkspaceBinding {
    pub db_path: String,
    pub identity_uuid: Uuid,
    pub db_password_enc: String,
}

// ---------------------------------------------------------------------------
// IdentityManager
// ---------------------------------------------------------------------------

use std::path::PathBuf;

/// Manages identity files and the identity settings registry.
pub struct IdentityManager {
    config_dir: PathBuf,
}

/// Returned after successful unlock — caller holds this and wipes on lock.
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
}

// ---------------------------------------------------------------------------
// .swarmid portable identity file
// ---------------------------------------------------------------------------

/// Portable identity export file (`<name>.swarmid`).
/// The `identity` field is the same on-disk `IdentityFile` — private key
/// is already encrypted with Argon2id + AES-256-GCM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmIdFile {
    pub format: String,   // always "swarmid"
    pub version: u32,     // always 1
    pub identity: IdentityFile,
}

impl SwarmIdFile {
    pub const FORMAT: &'static str = "swarmid";
    pub const VERSION: u32 = 1;
}

impl IdentityManager {
    /// Create a new `IdentityManager`.
    ///
    /// Ensures the `identities/` subdirectory exists under `config_dir`.
    /// Runs on-disk migrations (idempotent).
    pub fn new(config_dir: PathBuf) -> Result<Self> {
        let identities_dir = config_dir.join("identities");
        std::fs::create_dir_all(&identities_dir)?;
        Self::migrate(&config_dir);
        // Remove vestigial top-level contacts/ dir (empty, superseded by per-identity folders)
        let legacy_contacts = config_dir.join("contacts");
        if legacy_contacts.is_dir() {
            let is_empty = std::fs::read_dir(&legacy_contacts)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            if is_empty {
                let _ = std::fs::remove_dir(&legacy_contacts);
            }
        }
        Ok(Self { config_dir })
    }

    /// Runs on-disk migrations. Called once from `new()`. Idempotent.
    fn migrate(config_dir: &std::path::Path) {
        Self::migrate_pass1_identity_files(config_dir);
        Self::migrate_pass2_workspace_bindings(config_dir);
    }

    /// Pass 2: migrate workspace bindings from `identity_settings.json.workspaces`
    /// into per-workspace `binding.json` files.
    fn migrate_pass2_workspace_bindings(config_dir: &std::path::Path) {
        let settings_path = config_dir.join("identity_settings.json");
        let raw = match std::fs::read_to_string(&settings_path) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut settings: IdentitySettings = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(_) => return,
        };

        if settings.workspaces.is_empty() { return; }

        for (ws_uuid, legacy) in &settings.workspaces {
            // Derive workspace folder from db_path (parent of the .db file)
            let workspace_dir = std::path::Path::new(&legacy.db_path)
                .parent()
                .map(|p| p.to_path_buf());

            let workspace_dir = match workspace_dir {
                Some(d) if d.is_dir() => d,
                _ => {
                    eprintln!("[migration] Workspace folder missing for {ws_uuid}, dropping binding");
                    continue;
                }
            };

            let binding = WorkspaceBinding {
                workspace_uuid: ws_uuid.clone(),
                identity_uuid: legacy.identity_uuid,
                db_password_enc: legacy.db_password_enc.clone(),
            };
            let binding_path = workspace_dir.join("binding.json");
            match serde_json::to_string_pretty(&binding) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&binding_path, json) {
                        eprintln!("[migration] Cannot write binding.json to {binding_path:?}: {e}");
                    }
                }
                Err(e) => eprintln!("[migration] Cannot serialise binding for {ws_uuid}: {e}"),
            }
        }

        // Clear workspaces from settings regardless (stale entries are dropped)
        settings.workspaces.clear();
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            let _ = std::fs::write(&settings_path, json);
        }
    }

    /// Pass 1: move flat `identities/<uuid>.json` → `identities/<uuid>/identity.json`.
    fn migrate_pass1_identity_files(config_dir: &std::path::Path) {
        let identities_dir = config_dir.join("identities");
        let settings_path = config_dir.join("identity_settings.json");

        // Collect flat .json files (entries like `<uuid>.json` at root of identities/)
        let flat_files: Vec<(Uuid, std::path::PathBuf)> = match std::fs::read_dir(&identities_dir) {
            Ok(rd) => rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.is_file() && p.extension().map(|x| x == "json").unwrap_or(false) {
                        let stem = p.file_stem()?.to_str()?;
                        let uuid = Uuid::parse_str(stem).ok()?;
                        Some((uuid, p))
                    } else {
                        None
                    }
                })
                .collect(),
            Err(_) => return,
        };

        if flat_files.is_empty() { return; }

        // Load settings to update file refs
        let raw = match std::fs::read_to_string(&settings_path) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut settings: IdentitySettings = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut changed = false;
        for (uuid, src_path) in flat_files {
            let dest_dir = identities_dir.join(uuid.to_string());
            let dest_path = dest_dir.join("identity.json");

            if dest_path.exists() {
                // Already migrated — remove the now-orphaned flat file
                let _ = std::fs::remove_file(&src_path);
                changed = true;
                continue;
            }

            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                eprintln!("[migration] Cannot create {dest_dir:?}: {e}");
                continue;
            }
            if let Err(e) = std::fs::rename(&src_path, &dest_path) {
                eprintln!("[migration] Cannot move {src_path:?}: {e}");
                continue;
            }

            // Update IdentityRef.file in settings
            let new_file = format!("identities/{uuid}/identity.json");
            for id_ref in settings.identities.iter_mut() {
                if id_ref.uuid == uuid {
                    id_ref.file = new_file.clone();
                    break;
                }
            }
            changed = true;
        }

        if changed {
            if let Ok(json) = serde_json::to_string_pretty(&settings) {
                let _ = std::fs::write(&settings_path, json);
            }
        }
    }

    fn identities_dir(&self) -> PathBuf {
        self.config_dir.join("identities")
    }

    /// Returns the directory for a single identity (contains identity.json, contacts/, invites/).
    pub fn identity_dir(&self, identity_uuid: &Uuid) -> PathBuf {
        self.identities_dir().join(identity_uuid.to_string())
    }

    /// Returns the absolute path to the identity key file for a given UUID.
    /// Replaces the pattern: `config_dir.join(&identity_ref.file)` in lib.rs.
    pub fn identity_file_path(&self, identity_uuid: &Uuid) -> PathBuf {
        self.identity_dir(identity_uuid).join("identity.json")
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join("identity_settings.json")
    }

    fn load_settings(&self) -> Result<IdentitySettings> {
        let path = self.settings_path();
        if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            let settings: IdentitySettings = serde_json::from_str(&data)
                .map_err(|e| crate::KrillnotesError::IdentityCorrupt(
                    format!("identity_settings.json: {e}")
                ))?;
            Ok(settings)
        } else {
            Ok(IdentitySettings::default())
        }
    }

    fn save_settings(&self, settings: &IdentitySettings) -> Result<()> {
        let path = self.settings_path();
        let data = serde_json::to_string_pretty(settings)?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Create a new identity with the given display name and passphrase.
    ///
    /// Generates an Ed25519 keypair, encrypts the seed with Argon2id + AES-256-GCM,
    /// writes the identity file, and registers it in settings.
    pub fn create_identity(&self, display_name: &str, passphrase: &str) -> Result<IdentityFile> {
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
        };

        // Write identity file into per-identity directory
        let identity_dir = self.identity_dir(&identity_uuid);
        std::fs::create_dir_all(&identity_dir)?;
        // Pre-create data subdirs so the identity folder is complete from the start
        std::fs::create_dir_all(identity_dir.join("contacts"))?;
        std::fs::create_dir_all(identity_dir.join("invites"))?;
        let file_path = identity_dir.join("identity.json");
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&file_path, json)?;

        // Register in settings
        let mut settings = self.load_settings()?;
        settings.identities.push(IdentityRef {
            uuid: identity_uuid,
            display_name: display_name.to_string(),
            file: format!("identities/{identity_uuid}/identity.json"),
            last_used: Utc::now(),
        });
        self.save_settings(&settings)?;

        Ok(identity_file)
    }

    /// Unlock an identity by decrypting its Ed25519 seed with the given passphrase.
    pub fn unlock_identity(&self, identity_uuid: &Uuid, passphrase: &str) -> Result<UnlockedIdentity> {
        // Load identity file
        let file_path = self.identity_file_path(identity_uuid);
        if !file_path.exists() {
            return Err(crate::KrillnotesError::IdentityNotFound(identity_uuid.to_string()));
        }
        let data = std::fs::read_to_string(&file_path)?;
        let identity_file: IdentityFile = serde_json::from_str(&data)
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

        // Update last_used timestamp
        let mut settings = self.load_settings()?;
        if let Some(entry) = settings.identities.iter_mut().find(|i| i.uuid == *identity_uuid) {
            entry.last_used = Utc::now();
            self.save_settings(&settings)?;
        }

        Ok(UnlockedIdentity {
            identity_uuid: *identity_uuid,
            display_name: identity_file.display_name,
            signing_key,
            verifying_key,
        })
    }

    /// List all registered identities.
    pub fn list_identities(&self) -> Result<Vec<IdentityRef>> {
        let settings = self.load_settings()?;
        Ok(settings.identities)
    }

    /// Look up the display name for a given base64-encoded public key.
    /// Reads each identity file until a match is found. Returns `None` if
    /// no local identity has that public key.
    pub fn lookup_display_name(&self, public_key: &str) -> Option<String> {
        let settings = self.load_settings().ok()?;
        for identity_ref in &settings.identities {
            let file_path = self.identity_file_path(&identity_ref.uuid);
            let Ok(data) = std::fs::read_to_string(&file_path) else { continue };
            let Ok(identity_file) = serde_json::from_str::<IdentityFile>(&data) else { continue };
            if identity_file.public_key == public_key {
                return Some(identity_file.display_name);
            }
        }
        None
    }

    /// Delete an identity. Fails if any workspaces are still bound to it.
    pub fn delete_identity(&self, identity_uuid: &Uuid, workspace_base_dir: &std::path::Path) -> Result<()> {
        // Check for bound workspaces in the workspace base directory
        let bound = self.get_workspaces_for_identity(identity_uuid, workspace_base_dir)?;
        if !bound.is_empty() {
            return Err(crate::KrillnotesError::IdentityHasBoundWorkspaces(
                identity_uuid.to_string(),
            ));
        }

        let mut settings = self.load_settings()?;

        // Remove from settings
        settings.identities.retain(|i| i.uuid != *identity_uuid);
        self.save_settings(&settings)?;

        // Delete entire identity directory
        let identity_dir = self.identity_dir(identity_uuid);
        if identity_dir.exists() {
            std::fs::remove_dir_all(&identity_dir)?;
        }

        Ok(())
    }

    /// Change the passphrase for an identity.
    ///
    /// Decrypts the seed with the old passphrase, generates a new Argon2id salt,
    /// and re-encrypts with the new passphrase. The keypair is unchanged.
    pub fn change_passphrase(
        &self,
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

        // Load and update identity file
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

    /// Renames an identity's display name in both the identity file and the settings registry.
    pub fn rename_identity(&self, identity_uuid: &Uuid, new_name: &str) -> Result<()> {
        // Update identity file
        let identity_path = self.identity_file_path(identity_uuid);
        let content = std::fs::read_to_string(&identity_path)
            .map_err(|_| crate::KrillnotesError::IdentityNotFound(identity_uuid.to_string()))?;
        let mut identity_file: IdentityFile = serde_json::from_str(&content)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(e.to_string()))?;
        identity_file.display_name = new_name.to_string();
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&identity_path, json)?;

        // Update settings registry
        let mut settings = self.load_settings()?;
        if let Some(identity_ref) = settings.identities.iter_mut().find(|i| i.uuid == *identity_uuid) {
            identity_ref.display_name = new_name.to_string();
        }
        self.save_settings(&settings)?;

        Ok(())
    }

    /// Verifies the passphrase, then returns a `SwarmIdFile` ready to be serialised
    /// and written to disk by the caller. Does NOT write any file itself.
    pub fn export_swarmid(&self, identity_uuid: &Uuid, passphrase: &str) -> Result<SwarmIdFile> {
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

        Ok(SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity,
        })
    }

    /// Import a `.swarmid` file into this identity store (identity added in locked state).
    ///
    /// Returns `IdentityAlreadyExists` if the same UUID is already registered.
    /// Call `import_swarmid_overwrite` if the user confirms they want to replace it.
    pub fn import_swarmid(&self, file: SwarmIdFile) -> Result<IdentityRef> {
        self.validate_swarmid_file(&file)?;
        let uuid = file.identity.identity_uuid;
        let settings = self.load_settings()?;
        if settings.identities.iter().any(|i| i.uuid == uuid) {
            return Err(crate::KrillnotesError::IdentityAlreadyExists(uuid.to_string()));
        }
        self.write_swarmid_to_store(file)
    }

    /// Import a `.swarmid` file, overwriting any existing identity with the same UUID.
    ///
    /// Workspace bindings for the overwritten UUID are intentionally preserved — the
    /// imported `.swarmid` is assumed to carry the same Ed25519 key material, so the
    /// bound DB passwords remain decryptable after import.
    pub fn import_swarmid_overwrite(&self, file: SwarmIdFile) -> Result<IdentityRef> {
        self.validate_swarmid_file(&file)?;
        let uuid = file.identity.identity_uuid;
        // Remove existing entry if present
        let mut settings = self.load_settings()?;
        settings.identities.retain(|i| i.uuid != uuid);
        self.save_settings(&settings)?;
        let identity_dir = self.identity_dir(&uuid);
        if identity_dir.exists() {
            std::fs::remove_dir_all(&identity_dir)?;
        }
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

    fn write_swarmid_to_store(&self, file: SwarmIdFile) -> Result<IdentityRef> {
        let identity = file.identity;
        let uuid = identity.identity_uuid;
        let display_name = identity.display_name.clone();

        // Write identity file into per-identity directory
        let identity_dir = self.identity_dir(&uuid);
        std::fs::create_dir_all(&identity_dir)?;
        std::fs::create_dir_all(identity_dir.join("contacts"))?;
        std::fs::create_dir_all(identity_dir.join("invites"))?;
        let file_path = identity_dir.join("identity.json");
        let json = serde_json::to_string_pretty(&identity)?;
        std::fs::write(&file_path, json)?;

        // Register in settings registry
        let mut settings = self.load_settings()?;
        let identity_ref = IdentityRef {
            uuid,
            display_name: display_name.clone(),
            file: format!("identities/{uuid}/identity.json"),
            last_used: Utc::now(),
        };
        settings.identities.push(identity_ref.clone());
        self.save_settings(&settings)?;

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

    /// Scans all subdirectories of `workspace_base_dir` for `binding.json` files
    /// that belong to `identity_uuid`. Returns `(workspace_folder, WorkspaceBinding)` pairs.
    pub fn get_workspaces_for_identity(
        &self,
        identity_uuid: &Uuid,
        workspace_base_dir: &std::path::Path,
    ) -> Result<Vec<(PathBuf, WorkspaceBinding)>> {
        let mut results = Vec::new();
        let entries = match std::fs::read_dir(workspace_base_dir) {
            Ok(rd) => rd,
            Err(_) => return Ok(results), // directory doesn't exist yet
        };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() { continue; }
            let binding_path = folder.join("binding.json");
            if !binding_path.exists() { continue; }
            if let Ok(raw) = std::fs::read_to_string(&binding_path) {
                if let Ok(b) = serde_json::from_str::<WorkspaceBinding>(&raw) {
                    if b.identity_uuid == *identity_uuid {
                        results.push((folder, b));
                    }
                }
            }
        }
        Ok(results)
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

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
