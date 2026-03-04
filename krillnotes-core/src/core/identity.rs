//! Per-workspace cryptographic identity management.
//!
//! Manages Ed25519 keypairs protected by Argon2id-derived passphrases.
//! Each identity is stored as an encrypted JSON file. A separate settings
//! file binds workspaces to identities with encrypted DB passwords.

use chrono::{DateTime, Utc};
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
