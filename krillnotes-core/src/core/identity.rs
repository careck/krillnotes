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
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::Signer;

    #[test]
    fn test_identity_file_roundtrip_serde() {
        let file = IdentityFile {
            identity_uuid: Uuid::new_v4(),
            display_name: "Test User".to_string(),
            public_key: "AAAA".to_string(),
            private_key_enc: EncryptedKey {
                ciphertext: "BBBB".to_string(),
                nonce: "CCCC".to_string(),
                kdf: "argon2id".to_string(),
                kdf_params: KdfParams {
                    salt: "DDDD".to_string(),
                    m_cost: 65536,
                    t_cost: 3,
                    p_cost: 1,
                },
            },
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        let parsed: IdentityFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.identity_uuid, file.identity_uuid);
        assert_eq!(parsed.display_name, "Test User");
        assert_eq!(parsed.private_key_enc.kdf, "argon2id");
    }

    #[test]
    fn test_identity_settings_default_empty() {
        let settings = IdentitySettings::default();
        assert!(settings.identities.is_empty());
        assert!(settings.workspaces.is_empty());
    }

    #[test]
    fn test_identity_manager_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        assert!(mgr.identities_dir().exists());
    }

    #[test]
    fn test_settings_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        // Fresh — no file yet, returns default
        let settings = mgr.load_settings().unwrap();
        assert!(settings.identities.is_empty());

        // Save and reload
        let mut settings = IdentitySettings::default();
        settings.identities.push(IdentityRef {
            uuid: Uuid::new_v4(),
            display_name: "Test".to_string(),
            file: "identities/test.json".to_string(),
            last_used: Utc::now(),
        });
        mgr.save_settings(&settings).unwrap();

        let reloaded = mgr.load_settings().unwrap();
        assert_eq!(reloaded.identities.len(), 1);
        assert_eq!(reloaded.identities[0].display_name, "Test");
    }

    #[test]
    fn test_create_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        let identity_file = mgr.create_identity("Alice", "password123").unwrap();

        // File was written in the new per-identity subfolder
        let file_path = dir.path().join("identities")
            .join(identity_file.identity_uuid.to_string())
            .join("identity.json");
        assert!(file_path.exists());

        // Settings updated
        let settings = mgr.load_settings().unwrap();
        assert_eq!(settings.identities.len(), 1);
        assert_eq!(settings.identities[0].display_name, "Alice");
        assert_eq!(settings.identities[0].uuid, identity_file.identity_uuid);

        // Public key is valid base64 and 32 bytes
        let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
        assert_eq!(pk_bytes.len(), 32);

        // KDF params match expectations
        assert_eq!(identity_file.private_key_enc.kdf, "argon2id");
        assert_eq!(identity_file.private_key_enc.kdf_params.p_cost, 1);
    }

    #[test]
    fn test_unlock_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity_file = mgr.create_identity("Bob", "secret").unwrap();

        let unlocked = mgr.unlock_identity(&identity_file.identity_uuid, "secret").unwrap();
        assert_eq!(unlocked.identity_uuid, identity_file.identity_uuid);
        assert_eq!(unlocked.display_name, "Bob");

        // Public key matches
        let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
        assert_eq!(unlocked.verifying_key.as_bytes(), pk_bytes.as_slice());
    }

    #[test]
    fn test_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity_file = mgr.create_identity("Carol", "correct").unwrap();

        let result = mgr.unlock_identity(&identity_file.identity_uuid, "wrong");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::KrillnotesError::IdentityWrongPassphrase
        ));
    }

    #[test]
    fn test_sign_and_verify() {
        use ed25519_dalek::Verifier;

        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity_file = mgr.create_identity("Dave", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&identity_file.identity_uuid, "pass").unwrap();

        let message = b"hello world";
        let signature = unlocked.signing_key.sign(message);
        assert!(unlocked.verifying_key.verify(message, &signature).is_ok());

        // Also verify using the public key loaded from the file (not from unlock)
        let pk_bytes = BASE64.decode(&identity_file.public_key).unwrap();
        let file_vk = ed25519_dalek::VerifyingKey::from_bytes(
            pk_bytes.as_slice().try_into().unwrap()
        ).unwrap();
        assert!(file_vk.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_list_identities() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        mgr.create_identity("Alice", "pass1").unwrap();
        mgr.create_identity("Bob", "pass2").unwrap();
        mgr.create_identity("Carol", "pass3").unwrap();

        let list = mgr.list_identities().unwrap();
        assert_eq!(list.len(), 3);
        let names: Vec<&str> = list.iter().map(|i| i.display_name.as_str()).collect();
        assert!(names.contains(&"Alice"));
        assert!(names.contains(&"Bob"));
        assert!(names.contains(&"Carol"));
    }

    #[test]
    fn test_delete_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("ToDelete", "pass").unwrap();

        let identity_dir = dir.path().join("identities").join(identity.identity_uuid.to_string());
        assert!(identity_dir.join("identity.json").exists());

        // Empty workspace base dir — no bound workspaces
        let ws_base = dir.path().join("workspaces");
        std::fs::create_dir_all(&ws_base).unwrap();
        mgr.delete_identity(&identity.identity_uuid, &ws_base).unwrap();

        assert!(!identity_dir.exists());
        let list = mgr.list_identities().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_change_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Eve", "old-pass").unwrap();

        // Unlock with old passphrase — get the public key for comparison
        let unlocked_before = mgr.unlock_identity(&identity.identity_uuid, "old-pass").unwrap();
        let pk_before = *unlocked_before.verifying_key.as_bytes();

        // Change passphrase
        mgr.change_passphrase(&identity.identity_uuid, "old-pass", "new-pass").unwrap();

        // Old passphrase no longer works
        let result = mgr.unlock_identity(&identity.identity_uuid, "old-pass");
        assert!(matches!(result.unwrap_err(), crate::KrillnotesError::IdentityWrongPassphrase));

        // New passphrase works and produces the same keypair
        let unlocked_after = mgr.unlock_identity(&identity.identity_uuid, "new-pass").unwrap();
        assert_eq!(*unlocked_after.verifying_key.as_bytes(), pk_before);
    }

    #[test]
    fn bind_and_get_workspace_binding_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();

        let identity_uuid = Uuid::new_v4();
        let workspace_uuid = Uuid::new_v4().to_string();
        let workspace_dir = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace_dir).unwrap();

        let seed = [42u8; 32];
        let password = "hunter2";

        mgr.bind_workspace(&identity_uuid, &workspace_uuid, &workspace_dir, password, &seed).unwrap();

        // binding.json must exist
        assert!(workspace_dir.join("binding.json").exists());

        let binding = mgr.get_workspace_binding(&workspace_dir).unwrap().unwrap();
        assert_eq!(binding.workspace_uuid, workspace_uuid);
        assert_eq!(binding.identity_uuid, identity_uuid);

        // Decrypt round-trip
        let decrypted = mgr.decrypt_db_password(&workspace_dir, &seed).unwrap();
        assert_eq!(decrypted, password);
    }

    #[test]
    fn get_workspace_binding_returns_none_when_no_binding_json() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();

        assert!(mgr.get_workspace_binding(&ws_dir).unwrap().is_none());
    }

    #[test]
    fn decrypt_db_password_round_trips_multiple_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();
        let identity_uuid = Uuid::new_v4();
        let seed = [7u8; 32];

        for i in 0..3 {
            let ws_uuid = Uuid::new_v4().to_string();
            let ws_dir = tmp.path().join(format!("ws{i}"));
            std::fs::create_dir_all(&ws_dir).unwrap();
            let password = format!("pass{i}");
            mgr.bind_workspace(&identity_uuid, &ws_uuid, &ws_dir, &password, &seed).unwrap();
            let decrypted = mgr.decrypt_db_password(&ws_dir, &seed).unwrap();
            assert_eq!(decrypted, password);
        }
    }

    #[test]
    fn unbind_workspace_removes_binding_json() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        let binding_path = ws_dir.join("binding.json");
        std::fs::write(&binding_path, "{}").unwrap();

        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();

        mgr.unbind_workspace(&ws_dir).unwrap();
        assert!(!binding_path.exists());
    }

    #[test]
    fn test_multiple_identities_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();

        let id_a = mgr.create_identity("IdentA", "passA").unwrap();
        let id_b = mgr.create_identity("IdentB", "passB").unwrap();
        let unlocked_a = mgr.unlock_identity(&id_a.identity_uuid, "passA").unwrap();
        let unlocked_b = mgr.unlock_identity(&id_b.identity_uuid, "passB").unwrap();

        let ws_a = tmp.path().join("ws_a");
        let ws_b = tmp.path().join("ws_b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();

        mgr.bind_workspace(&id_a.identity_uuid, "ws-a-uuid", &ws_a, "pw-a", unlocked_a.signing_key.as_bytes()).unwrap();
        mgr.bind_workspace(&id_b.identity_uuid, "ws-b-uuid", &ws_b, "pw-b", unlocked_b.signing_key.as_bytes()).unwrap();

        // A can decrypt A's workspace
        assert_eq!(mgr.decrypt_db_password(&ws_a, unlocked_a.signing_key.as_bytes()).unwrap(), "pw-a");

        // B can decrypt B's workspace
        assert_eq!(mgr.decrypt_db_password(&ws_b, unlocked_b.signing_key.as_bytes()).unwrap(), "pw-b");

        // A cannot decrypt B's workspace (wrong key, AES-GCM will fail)
        let result = mgr.decrypt_db_password(&ws_b, unlocked_a.signing_key.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn delete_identity_fails_if_workspaces_still_bound() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir.clone()).unwrap();

        let seed = [1u8; 32];
        let ws_dir = tmp.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        // Create the identity first
        let display_name = "Test";
        let passphrase = "testpass";
        mgr.create_identity(display_name, passphrase).unwrap();

        // Get the UUID we just created
        let settings = mgr.load_settings().unwrap();
        let id_ref = settings.identities.first().unwrap();
        let real_uuid = id_ref.uuid;

        // Bind a workspace to it
        let ws_uuid = Uuid::new_v4().to_string();
        mgr.bind_workspace(&real_uuid, &ws_uuid, &ws_dir, "pass", &seed).unwrap();

        // delete_identity must fail because a workspace is still bound
        let ws_base = tmp.path().to_path_buf();
        let result = mgr.delete_identity(&real_uuid, &ws_base);
        assert!(result.is_err(), "should fail when workspaces are bound");
    }

    #[test]
    fn test_rename_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let file = mgr.create_identity("Old Name", "pass123").unwrap();
        let uuid = file.identity_uuid;

        mgr.rename_identity(&uuid, "New Name").unwrap();

        // Check settings
        let identities = mgr.list_identities().unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].display_name, "New Name");

        // Check identity file
        let unlocked = mgr.unlock_identity(&uuid, "pass123").unwrap();
        assert_eq!(unlocked.display_name, "New Name");
    }

    #[test]
    fn get_workspaces_for_identity_scans_workspace_base_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let mgr = IdentityManager::new(config_dir).unwrap();

        let identity_a = Uuid::new_v4();
        let identity_b = Uuid::new_v4();
        let ws_base = tmp.path().join("workspaces");

        // Two workspaces for identity_a, one for identity_b
        for (name, owner) in &[("ws1", identity_a), ("ws2", identity_a), ("ws3", identity_b)] {
            let ws_dir = ws_base.join(name);
            std::fs::create_dir_all(&ws_dir).unwrap();
            let binding = WorkspaceBinding {
                workspace_uuid: Uuid::new_v4().to_string(),
                identity_uuid: *owner,
                db_password_enc: "enc".to_string(),
            };
            std::fs::write(
                ws_dir.join("binding.json"),
                serde_json::to_string(&binding).unwrap()
            ).unwrap();
        }
        // ws4 has no binding.json — must be ignored
        std::fs::create_dir_all(ws_base.join("ws4")).unwrap();

        let results = mgr.get_workspaces_for_identity(&identity_a, &ws_base).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, b)| b.identity_uuid == identity_a));
    }

    #[test]
    fn test_identity_file_format_matches_spec() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Spec Check", "pass").unwrap();

        // Read the raw JSON file from the new per-identity subfolder
        let file_path = dir.path().join("identities")
            .join(identity.identity_uuid.to_string())
            .join("identity.json");
        let raw = std::fs::read_to_string(&file_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

        // Verify top-level keys match spec
        assert!(json.get("identity_uuid").unwrap().is_string());
        assert!(json.get("display_name").unwrap().is_string());
        assert!(json.get("public_key").unwrap().is_string());

        let enc = json.get("private_key_enc").unwrap();
        assert!(enc.get("ciphertext").unwrap().is_string());
        assert!(enc.get("nonce").unwrap().is_string());
        assert_eq!(enc.get("kdf").unwrap().as_str().unwrap(), "argon2id");

        let params = enc.get("kdf_params").unwrap();
        assert!(params.get("salt").unwrap().is_string());
        assert!(params.get("m_cost").unwrap().is_u64());
        assert!(params.get("t_cost").unwrap().is_u64());
        assert!(params.get("p_cost").unwrap().is_u64());
    }

    #[test]
    fn swarmid_file_roundtrip() {
        let inner = IdentityFile {
            identity_uuid: Uuid::new_v4(),
            display_name: "Test".to_string(),
            public_key: "abc".to_string(),
            private_key_enc: EncryptedKey {
                ciphertext: "ct".to_string(),
                nonce: "nn".to_string(),
                kdf: "argon2id".to_string(),
                kdf_params: KdfParams {
                    salt: "sl".to_string(),
                    m_cost: 1,
                    t_cost: 1,
                    p_cost: 1,
                },
            },
        };
        let swarmid = SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity: inner.clone(),
        };
        let json = serde_json::to_string(&swarmid).unwrap();
        assert!(json.contains("\"format\":\"swarmid\""));
        assert!(json.contains("\"version\":1"));
        let parsed: SwarmIdFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.identity.display_name, "Test");
    }

    #[test]
    fn export_swarmid_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Alice", "correct-passphrase").unwrap();
        let result = mgr.export_swarmid(&identity.identity_uuid, "wrong-passphrase");
        assert!(matches!(result, Err(crate::KrillnotesError::IdentityWrongPassphrase)));
    }

    #[test]
    fn export_swarmid_correct_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Bob", "my-passphrase").unwrap();
        let swarmid = mgr.export_swarmid(&identity.identity_uuid, "my-passphrase").unwrap();
        assert_eq!(swarmid.format, "swarmid");
        assert_eq!(swarmid.version, 1);
        assert_eq!(swarmid.identity.display_name, "Bob");
        assert_eq!(swarmid.identity.identity_uuid, identity.identity_uuid);
        assert_eq!(swarmid.identity.public_key, identity.public_key);
    }

    #[test]
    fn import_swarmid_adds_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        // Create identity, export it, then delete to simulate a fresh device
        let original = mgr.create_identity("Charlie", "passphrase").unwrap();
        let swarmid = SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity: original.clone(),
        };
        let ws_base = dir.path().join("workspaces");
        std::fs::create_dir_all(&ws_base).unwrap();
        mgr.delete_identity(&original.identity_uuid, &ws_base).unwrap();
        assert!(mgr.list_identities().unwrap().is_empty());

        let identity_ref = mgr.import_swarmid(swarmid).unwrap();
        assert_eq!(identity_ref.display_name, "Charlie");
        assert_eq!(identity_ref.uuid, original.identity_uuid);

        let identities = mgr.list_identities().unwrap();
        assert_eq!(identities.len(), 1);
    }

    #[test]
    fn import_swarmid_collision_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let original = mgr.create_identity("Dave", "passphrase").unwrap();
        let swarmid = SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity: original.clone(),
        };
        // Import again — same UUID should fail with IdentityAlreadyExists
        let result = mgr.import_swarmid(swarmid);
        assert!(matches!(result, Err(crate::KrillnotesError::IdentityAlreadyExists(_))));
    }

    #[test]
    fn import_swarmid_overwrite_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let original = mgr.create_identity("Eve", "passphrase").unwrap();
        let mut swarmid = SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: SwarmIdFile::VERSION,
            identity: original.clone(),
        };
        swarmid.identity.display_name = "Eve Updated".to_string();
        let identity_ref = mgr.import_swarmid_overwrite(swarmid).unwrap();
        assert_eq!(identity_ref.display_name, "Eve Updated");
        // Only one identity in list
        assert_eq!(mgr.list_identities().unwrap().len(), 1);
    }

    #[test]
    fn import_swarmid_invalid_format_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Test", "pass").unwrap();

        let bad_format = SwarmIdFile {
            format: "notswarmid".to_string(),
            version: 1,
            identity: identity.clone(),
        };
        assert!(matches!(
            mgr.import_swarmid(bad_format),
            Err(crate::KrillnotesError::SwarmIdInvalidFormat(_))
        ));

        let bad_version = SwarmIdFile {
            format: SwarmIdFile::FORMAT.to_string(),
            version: 99,
            identity,
        };
        assert!(matches!(
            mgr.import_swarmid(bad_version),
            Err(crate::KrillnotesError::SwarmIdVersionUnsupported(99))
        ));
    }

    #[test]
    fn contacts_key_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr
            .create_identity("Test User", "passphrase123")
            .unwrap();
        let unlocked = mgr
            .unlock_identity(&identity.identity_uuid, "passphrase123")
            .unwrap();
        let key1 = unlocked.contacts_key();
        let key2 = unlocked.contacts_key();
        assert_eq!(key1, key2, "contacts_key must be deterministic");
        assert_eq!(key1.len(), 32);
        // Must differ from a different identity
        let identity2 = mgr
            .create_identity("Other User", "passphrase123")
            .unwrap();
        let unlocked2 = mgr
            .unlock_identity(&identity2.identity_uuid, "passphrase123")
            .unwrap();
        assert_ne!(unlocked.contacts_key(), unlocked2.contacts_key());
    }

    #[test]
    fn old_identity_settings_with_workspaces_key_deserialises() {
        // Old format still deserialises (workspaces key is readable)
        let json = r#"{
            "identities": [],
            "workspaces": {
                "ws-uuid-1": {
                    "db_path": "/tmp/foo/notes.db",
                    "identity_uuid": "00000000-0000-0000-0000-000000000001",
                    "db_password_enc": "aGVsbG8="
                }
            }
        }"#;
        let settings: IdentitySettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.workspaces.len(), 1);
        let binding = settings.workspaces.get("ws-uuid-1").unwrap();
        assert_eq!(binding.db_path, "/tmp/foo/notes.db");
    }

    #[test]
    fn new_identity_settings_serialises_without_workspaces_key() {
        let settings = IdentitySettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("workspaces"),
            "workspaces key must not appear in serialised output");
    }

    #[test]
    fn workspace_binding_serialises_with_workspace_uuid() {
        let b = WorkspaceBinding {
            workspace_uuid: "ws-1".to_string(),
            identity_uuid: Uuid::nil(),
            db_password_enc: "enc".to_string(),
        };
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains("workspace_uuid"));
        assert!(json.contains("identity_uuid"));
        assert!(!json.contains("db_path"));
    }

    #[test]
    fn identity_dir_returns_uuid_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
        let uuid = Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        assert_eq!(
            mgr.identity_dir(&uuid),
            tmp.path().join("identities").join("aaaaaaaa-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn identity_file_path_returns_identity_json_inside_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(tmp.path().to_path_buf()).unwrap();
        let uuid = Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        assert_eq!(
            mgr.identity_file_path(&uuid),
            tmp.path().join("identities").join("aaaaaaaa-0000-0000-0000-000000000001").join("identity.json")
        );
    }

    #[test]
    fn migration_pass1_moves_flat_json_into_identity_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();
        let identities_dir = config_dir.join("identities");
        std::fs::create_dir_all(&identities_dir).unwrap();

        // Create legacy flat identity file
        let uuid = Uuid::new_v4();
        let legacy_path = identities_dir.join(format!("{uuid}.json"));
        let identity_file = serde_json::json!({
            "identity_uuid": uuid.to_string(),
            "display_name": "Test",
            "public_key": "dGVzdA==",
            "private_key_enc": {
                "ciphertext": "dGVzdA==",
                "nonce": "dGVzdA==",
                "kdf": "argon2id",
                "kdf_params": { "salt": "dGVzdA==", "m_cost": 1024, "t_cost": 1, "p_cost": 1 }
            }
        });
        std::fs::write(&legacy_path, serde_json::to_string(&identity_file).unwrap()).unwrap();

        // Create identity_settings.json referencing the flat file
        let settings = serde_json::json!({
            "identities": [{
                "uuid": uuid.to_string(),
                "displayName": "Test",
                "file": format!("identities/{uuid}.json"),
                "lastUsed": "2026-01-01T00:00:00Z"
            }]
        });
        std::fs::write(config_dir.join("identity_settings.json"),
            serde_json::to_string(&settings).unwrap()).unwrap();

        // Trigger migration
        let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

        // Flat file must be gone
        assert!(!legacy_path.exists(), "flat file should be removed");

        // New path must exist
        let new_path = identities_dir.join(uuid.to_string()).join("identity.json");
        assert!(new_path.exists(), "identity.json inside folder must exist");

        // settings must be updated
        let raw = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
        let updated: IdentitySettings = serde_json::from_str(&raw).unwrap();
        assert_eq!(updated.identities[0].file,
            format!("identities/{uuid}/identity.json"));
    }

    #[test]
    fn migration_pass1_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();
        // First call (no legacy files) — should succeed silently
        let _m1 = IdentityManager::new(config_dir.clone()).unwrap();
        // Second call — must also succeed
        let _m2 = IdentityManager::new(config_dir.clone()).unwrap();
    }

    #[test]
    fn migration_pass2_writes_binding_json_for_existing_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();

        // Create a fake workspace folder with notes.db
        let ws_dir = tmp.path().join("workspaces").join("my-workspace");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("notes.db"), b"").unwrap();

        let ws_uuid = "aaaaaaaa-1111-0000-0000-000000000001";
        let identity_uuid = "bbbbbbbb-2222-0000-0000-000000000001";

        // Write legacy identity_settings.json with workspaces section
        let settings_json = serde_json::json!({
            "identities": [],
            "workspaces": {
                ws_uuid: {
                    "db_path": ws_dir.join("notes.db").display().to_string(),
                    "identity_uuid": identity_uuid,
                    "db_password_enc": "dGVzdA=="
                }
            }
        });
        std::fs::write(
            config_dir.join("identity_settings.json"),
            serde_json::to_string(&settings_json).unwrap()
        ).unwrap();

        // Trigger migration
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();
        let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

        // binding.json must exist in workspace folder
        let binding_path = ws_dir.join("binding.json");
        assert!(binding_path.exists(), "binding.json must be written");

        let raw = std::fs::read_to_string(&binding_path).unwrap();
        let binding: WorkspaceBinding = serde_json::from_str(&raw).unwrap();
        assert_eq!(binding.workspace_uuid, ws_uuid);
        assert_eq!(binding.identity_uuid.to_string(), identity_uuid);
        assert_eq!(binding.db_password_enc, "dGVzdA==");

        // identity_settings.json must no longer have workspaces key
        let raw_settings = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
        assert!(!raw_settings.contains("workspaces"),
            "workspaces key must be absent after migration");
    }

    #[test]
    fn migration_pass2_drops_stale_entry_for_missing_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(config_dir.join("identities")).unwrap();

        // Stale binding — workspace folder does not exist
        let settings_json = serde_json::json!({
            "identities": [],
            "workspaces": {
                "dead-ws-uuid": {
                    "db_path": "/nonexistent/workspace/notes.db",
                    "identity_uuid": "00000000-0000-0000-0000-000000000001",
                    "db_password_enc": "dGVzdA=="
                }
            }
        });
        std::fs::write(
            config_dir.join("identity_settings.json"),
            serde_json::to_string(&settings_json).unwrap()
        ).unwrap();

        // Must not panic
        let _mgr = IdentityManager::new(config_dir.clone()).unwrap();

        // identity_settings.json cleaned up
        let raw = std::fs::read_to_string(config_dir.join("identity_settings.json")).unwrap();
        assert!(!raw.contains("workspaces"));
    }
}
