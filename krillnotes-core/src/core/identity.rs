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
    #[serde(default)]
    pub workspaces: std::collections::HashMap<String, WorkspaceBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRef {
    pub uuid: Uuid,
    pub display_name: String,
    pub file: String,
    pub last_used: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBinding {
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

impl IdentityManager {
    /// Create a new `IdentityManager`.
    ///
    /// Ensures the `identities/` subdirectory exists under `config_dir`.
    pub fn new(config_dir: PathBuf) -> Result<Self> {
        let identities_dir = config_dir.join("identities");
        std::fs::create_dir_all(&identities_dir)?;
        Ok(Self { config_dir })
    }

    fn identities_dir(&self) -> PathBuf {
        self.config_dir.join("identities")
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

        // Write identity file
        let file_path = self.identities_dir().join(format!("{identity_uuid}.json"));
        let json = serde_json::to_string_pretty(&identity_file)?;
        std::fs::write(&file_path, json)?;

        // Register in settings
        let mut settings = self.load_settings()?;
        settings.identities.push(IdentityRef {
            uuid: identity_uuid,
            display_name: display_name.to_string(),
            file: format!("identities/{identity_uuid}.json"),
            last_used: Utc::now(),
        });
        self.save_settings(&settings)?;

        Ok(identity_file)
    }

    /// Unlock an identity by decrypting its Ed25519 seed with the given passphrase.
    pub fn unlock_identity(&self, identity_uuid: &Uuid, passphrase: &str) -> Result<UnlockedIdentity> {
        // Load identity file
        let file_path = self.identities_dir().join(format!("{identity_uuid}.json"));
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

    /// Delete an identity. Fails if any workspaces are still bound to it.
    pub fn delete_identity(&self, identity_uuid: &Uuid) -> Result<()> {
        let mut settings = self.load_settings()?;

        // Check for bound workspaces
        let bound: Vec<_> = settings.workspaces.values()
            .filter(|b| b.identity_uuid == *identity_uuid)
            .collect();
        if !bound.is_empty() {
            return Err(crate::KrillnotesError::IdentityHasBoundWorkspaces(
                identity_uuid.to_string(),
            ));
        }

        // Remove from settings
        settings.identities.retain(|i| i.uuid != *identity_uuid);
        self.save_settings(&settings)?;

        // Delete file
        let file_path = self.identities_dir().join(format!("{identity_uuid}.json"));
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
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
        let file_path = self.identities_dir().join(format!("{identity_uuid}.json"));
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

    /// Bind a workspace to an identity, encrypting the DB password with a key
    /// derived from the Ed25519 seed.
    pub fn bind_workspace(
        &self,
        identity_uuid: &Uuid,
        workspace_uuid: &str,
        db_path: &str,
        db_password: &str,
        seed: &[u8; 32],
    ) -> Result<()> {
        let encrypted = self.encrypt_db_password(seed, workspace_uuid, db_password)?;

        let mut settings = self.load_settings()?;
        settings.workspaces.insert(
            workspace_uuid.to_string(),
            WorkspaceBinding {
                db_path: db_path.to_string(),
                identity_uuid: *identity_uuid,
                db_password_enc: encrypted,
            },
        );
        self.save_settings(&settings)?;
        Ok(())
    }

    /// Remove a workspace binding from the identity settings.
    pub fn unbind_workspace(&self, workspace_uuid: &str) -> Result<()> {
        let mut settings = self.load_settings()?;
        if settings.workspaces.remove(workspace_uuid).is_none() {
            return Err(crate::KrillnotesError::WorkspaceNotBound(workspace_uuid.to_string()));
        }
        self.save_settings(&settings)?;
        Ok(())
    }

    /// Decrypt a workspace's DB password using the Ed25519 seed.
    pub fn decrypt_db_password(
        &self,
        workspace_uuid: &str,
        seed: &[u8; 32],
    ) -> Result<String> {
        let settings = self.load_settings()?;
        let binding = settings.workspaces.get(workspace_uuid)
            .ok_or_else(|| crate::KrillnotesError::WorkspaceNotBound(workspace_uuid.to_string()))?;

        let blob = BASE64.decode(&binding.db_password_enc)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("db_password_enc decode: {e}")))?;

        let key = self.derive_db_password_key(seed, workspace_uuid)?;

        // AES-256-GCM decrypt: first 12 bytes are nonce, rest is ciphertext+tag
        if blob.len() < 13 {
            return Err(crate::KrillnotesError::IdentityCorrupt("db password blob too short".to_string()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("AES key: {e}")))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| crate::KrillnotesError::IdentityWrongPassphrase)?;

        String::from_utf8(plaintext)
            .map_err(|e| crate::KrillnotesError::IdentityCorrupt(format!("db password not UTF-8: {e}")))
    }

    /// Get all workspace bindings for a given identity.
    pub fn get_workspaces_for_identity(&self, identity_uuid: &Uuid) -> Result<Vec<(String, WorkspaceBinding)>> {
        let settings = self.load_settings()?;
        let result = settings.workspaces.into_iter()
            .filter(|(_, b)| b.identity_uuid == *identity_uuid)
            .collect();
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

        // File was written
        let file_path = dir.path().join("identities").join(format!("{}.json", identity_file.identity_uuid));
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

        let file_path = dir.path().join("identities").join(format!("{}.json", identity.identity_uuid));
        assert!(file_path.exists());

        mgr.delete_identity(&identity.identity_uuid).unwrap();

        assert!(!file_path.exists());
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
    fn test_bind_workspace_and_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Frank", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&identity.identity_uuid, "pass").unwrap();

        let db_password = "random-db-password-abc123";
        mgr.bind_workspace(
            &identity.identity_uuid,
            "workspace-uuid-1",
            "/path/to/ws.db",
            db_password,
            unlocked.signing_key.as_bytes(),
        ).unwrap();

        // Settings should have the binding
        let settings = mgr.load_settings().unwrap();
        assert!(settings.workspaces.contains_key("workspace-uuid-1"));
        assert_eq!(settings.workspaces["workspace-uuid-1"].identity_uuid, identity.identity_uuid);

        // Decrypt should return original password
        let decrypted = mgr.decrypt_db_password(
            "workspace-uuid-1",
            unlocked.signing_key.as_bytes(),
        ).unwrap();
        assert_eq!(decrypted, db_password);
    }

    #[test]
    fn test_db_password_roundtrip_multiple_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Grace", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&identity.identity_uuid, "pass").unwrap();

        let seed = unlocked.signing_key.as_bytes();
        mgr.bind_workspace(&identity.identity_uuid, "ws-1", "/a.db", "pw-alpha", seed).unwrap();
        mgr.bind_workspace(&identity.identity_uuid, "ws-2", "/b.db", "pw-beta", seed).unwrap();

        assert_eq!(mgr.decrypt_db_password("ws-1", seed).unwrap(), "pw-alpha");
        assert_eq!(mgr.decrypt_db_password("ws-2", seed).unwrap(), "pw-beta");
    }

    #[test]
    fn test_unbind_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Heidi", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&identity.identity_uuid, "pass").unwrap();

        mgr.bind_workspace(
            &identity.identity_uuid,
            "ws-unbind",
            "/db.db",
            "pw",
            unlocked.signing_key.as_bytes(),
        ).unwrap();

        mgr.unbind_workspace("ws-unbind").unwrap();

        let result = mgr.decrypt_db_password("ws-unbind", unlocked.signing_key.as_bytes());
        assert!(matches!(result.unwrap_err(), crate::KrillnotesError::WorkspaceNotBound(_)));
    }

    #[test]
    fn test_multiple_identities_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        let id_a = mgr.create_identity("IdentA", "passA").unwrap();
        let id_b = mgr.create_identity("IdentB", "passB").unwrap();
        let unlocked_a = mgr.unlock_identity(&id_a.identity_uuid, "passA").unwrap();
        let unlocked_b = mgr.unlock_identity(&id_b.identity_uuid, "passB").unwrap();

        mgr.bind_workspace(&id_a.identity_uuid, "ws-a", "/a.db", "pw-a", unlocked_a.signing_key.as_bytes()).unwrap();
        mgr.bind_workspace(&id_b.identity_uuid, "ws-b", "/b.db", "pw-b", unlocked_b.signing_key.as_bytes()).unwrap();

        // A can decrypt A's workspace
        assert_eq!(mgr.decrypt_db_password("ws-a", unlocked_a.signing_key.as_bytes()).unwrap(), "pw-a");

        // B can decrypt B's workspace
        assert_eq!(mgr.decrypt_db_password("ws-b", unlocked_b.signing_key.as_bytes()).unwrap(), "pw-b");

        // A cannot decrypt B's workspace (wrong key, AES-GCM will fail)
        let result = mgr.decrypt_db_password("ws-b", unlocked_a.signing_key.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_identity_with_bound_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Bound", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&identity.identity_uuid, "pass").unwrap();

        // Bind a workspace
        mgr.bind_workspace(
            &identity.identity_uuid,
            "ws-uuid-123",
            "/path/to/db",
            "db-password",
            unlocked.signing_key.as_bytes(),
        ).unwrap();

        // Delete should fail
        let result = mgr.delete_identity(&identity.identity_uuid);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::KrillnotesError::IdentityHasBoundWorkspaces(_)
        ));
    }

    #[test]
    fn test_get_workspaces_for_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();

        let id_a = mgr.create_identity("Multi", "pass").unwrap();
        let unlocked = mgr.unlock_identity(&id_a.identity_uuid, "pass").unwrap();
        let seed = unlocked.signing_key.as_bytes();

        mgr.bind_workspace(&id_a.identity_uuid, "ws-1", "/a.db", "pw1", seed).unwrap();
        mgr.bind_workspace(&id_a.identity_uuid, "ws-2", "/b.db", "pw2", seed).unwrap();

        let workspaces = mgr.get_workspaces_for_identity(&id_a.identity_uuid).unwrap();
        assert_eq!(workspaces.len(), 2);
    }

    #[test]
    fn test_identity_file_format_matches_spec() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = IdentityManager::new(dir.path().to_path_buf()).unwrap();
        let identity = mgr.create_identity("Spec Check", "pass").unwrap();

        // Read the raw JSON file
        let file_path = dir.path().join("identities").join(format!("{}.json", identity.identity_uuid));
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
}
