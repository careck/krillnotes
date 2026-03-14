// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Encrypted relay credential storage.
//!
//! Credentials are AES-256-GCM encrypted and stored as a JSON envelope
//! (base64 nonce + ciphertext) at `<relay_dir>/<identity_uuid>.json`.

use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::core::error::KrillnotesError;

/// Relay session credentials for a given identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCredentials {
    pub relay_url: String,
    pub email: String,
    pub session_token: String,
    pub session_expires_at: DateTime<Utc>,
    pub device_public_key: String,
}

/// On-disk format: AES-256-GCM encrypted JSON envelope.
#[derive(Serialize, Deserialize)]
struct EncryptedRelayFile {
    /// base64-encoded 12-byte nonce.
    nonce: String,
    /// base64-encoded AES-256-GCM ciphertext (includes 16-byte auth tag).
    ciphertext: String,
}

/// Save relay credentials to `<relay_dir>/<identity_uuid>.json`, encrypted
/// with `encryption_key` using AES-256-GCM.
pub fn save_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
    creds: &RelayCredentials,
    encryption_key: &[u8; 32],
) -> Result<(), KrillnotesError> {
    std::fs::create_dir_all(relay_dir)?;

    let plaintext = serde_json::to_vec(creds)?;

    let key = Key::<Aes256Gcm>::from_slice(encryption_key);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| KrillnotesError::ContactEncryption(format!("relay credential encryption failed: {e}")))?;

    let envelope = EncryptedRelayFile {
        nonce: BASE64.encode(nonce_bytes),
        ciphertext: BASE64.encode(&ciphertext),
    };

    let path = relay_dir.join(format!("{identity_uuid}.json"));
    let json = serde_json::to_string(&envelope)?;
    std::fs::write(&path, json)?;

    Ok(())
}

/// Load relay credentials from `<relay_dir>/<identity_uuid>.json`.
///
/// Returns `None` if the file does not exist.
pub fn load_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<RelayCredentials>, KrillnotesError> {
    let path = relay_dir.join(format!("{identity_uuid}.json"));

    if !path.exists() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(&path)?;

    let envelope: EncryptedRelayFile = serde_json::from_str(&json)?;

    let nonce_bytes = BASE64.decode(&envelope.nonce).map_err(|e| {
        KrillnotesError::ContactEncryption(format!("invalid relay nonce base64: {e}"))
    })?;
    if nonce_bytes.len() != 12 {
        return Err(KrillnotesError::ContactEncryption(format!(
            "invalid relay nonce length: {} bytes",
            nonce_bytes.len()
        )));
    }
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = BASE64.decode(&envelope.ciphertext).map_err(|e| {
        KrillnotesError::ContactEncryption(format!("invalid relay ciphertext base64: {e}"))
    })?;

    let key = Key::<Aes256Gcm>::from_slice(encryption_key);
    let cipher = Aes256Gcm::new(key);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| KrillnotesError::ContactEncryption(format!("relay credential decryption failed: {e}")))?;

    let creds: RelayCredentials = serde_json::from_slice(&plaintext)?;

    Ok(Some(creds))
}

/// Delete relay credentials for the given identity.
///
/// Returns `Ok(())` if the file is absent (idempotent).
pub fn delete_relay_credentials(
    relay_dir: &Path,
    identity_uuid: &str,
) -> Result<(), KrillnotesError> {
    let path = relay_dir.join(format!("{identity_uuid}.json"));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Decrypt a proof-of-possession challenge nonce sent by the relay server.
///
/// The relay server encrypts a nonce using NaCl `crypto_box` (X25519 + XSalsa20-Poly1305)
/// addressed to the client's X25519 public key (derived from their Ed25519 key).
///
/// The `encrypted_nonce_hex` is a hex-encoded byte string with a 24-byte nonce
/// prepended to the ciphertext.
///
/// Returns the plaintext nonce bytes on success.
#[cfg(feature = "relay")]
pub fn decrypt_pop_challenge(
    client_signing_key: &ed25519_dalek::SigningKey,
    encrypted_nonce_hex: &str,
    server_public_key_hex: &str,
) -> Result<Vec<u8>, KrillnotesError> {
    use crate::core::swarm::crypto::ed25519_sk_to_x25519;
    use crypto_box::{aead::Aead, PublicKey, SalsaBox, SecretKey};

    // 1. Convert Ed25519 signing key to X25519 secret key.
    let x25519_sk = ed25519_sk_to_x25519(client_signing_key);
    let client_sk = SecretKey::from(x25519_sk.to_bytes());

    // 2. Decode server's ephemeral public key.
    let server_pk_bytes: [u8; 32] = hex::decode(server_public_key_hex)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid server pubkey hex: {}", e)))?
        .try_into()
        .map_err(|_| KrillnotesError::Crypto("Server public key must be 32 bytes".to_string()))?;
    let server_pk = PublicKey::from(server_pk_bytes);

    // 3. Decode encrypted nonce (24-byte nonce prefix + ciphertext).
    let encrypted_bytes = hex::decode(encrypted_nonce_hex)
        .map_err(|e| KrillnotesError::Crypto(format!("Invalid encrypted nonce hex: {}", e)))?;
    if encrypted_bytes.len() < 24 {
        return Err(KrillnotesError::Crypto(
            "Encrypted nonce too short".to_string(),
        ));
    }
    let (nonce_bytes, ciphertext) = encrypted_bytes.split_at(24);
    let nonce = crypto_box::Nonce::from_slice(nonce_bytes);

    // 4. Decrypt using SalsaBox (X25519 + XSalsa20-Poly1305, NaCl-compatible).
    let salsa_box = SalsaBox::new(&server_pk, &client_sk);
    salsa_box
        .decrypt(nonce, ciphertext)
        .map_err(|_| KrillnotesError::Crypto("PoP challenge decryption failed".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_credentials_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");

        let identity_uuid = "test-identity-uuid";
        let encryption_key = [0x42u8; 32];

        let creds = RelayCredentials {
            relay_url: "https://relay.example.com".to_string(),
            email: "test@example.com".to_string(),
            session_token: "tok_abc123".to_string(),
            session_expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            device_public_key: "deadbeef".to_string(),
        };

        save_relay_credentials(&relay_dir, identity_uuid, &creds, &encryption_key).unwrap();
        let loaded = load_relay_credentials(&relay_dir, identity_uuid, &encryption_key).unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.relay_url, creds.relay_url);
        assert_eq!(loaded.email, creds.email);
        assert_eq!(loaded.session_token, creds.session_token);
        assert_eq!(loaded.device_public_key, creds.device_public_key);
    }

    #[test]
    fn test_relay_credentials_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");
        let encryption_key = [0x42u8; 32];

        let loaded = load_relay_credentials(&relay_dir, "nonexistent", &encryption_key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_relay_credentials_delete() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");
        let identity_uuid = "delete-test";
        let encryption_key = [0x11u8; 32];

        let creds = RelayCredentials {
            relay_url: "https://relay.example.com".to_string(),
            email: "del@example.com".to_string(),
            session_token: "tok_del".to_string(),
            session_expires_at: chrono::Utc::now(),
            device_public_key: "aabbcc".to_string(),
        };

        save_relay_credentials(&relay_dir, identity_uuid, &creds, &encryption_key).unwrap();
        delete_relay_credentials(&relay_dir, identity_uuid).unwrap();
        let loaded = load_relay_credentials(&relay_dir, identity_uuid, &encryption_key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_relay_credentials_delete_nonexistent_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let relay_dir = dir.path().join("relay");
        // Should not error if file doesn't exist
        delete_relay_credentials(&relay_dir, "never-existed").unwrap();
    }
}

#[cfg(all(test, feature = "relay"))]
mod pop_tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    /// Simulate what the relay server does to create a PoP challenge.
    /// Returns (encrypted_nonce_hex, server_public_key_hex).
    fn simulate_server_challenge(
        client_ed25519_vk: &ed25519_dalek::VerifyingKey,
        nonce_plaintext: &[u8],
    ) -> (String, String) {
        use crypto_box::{aead::{Aead, AeadCore}, PublicKey, SalsaBox, SecretKey};
        use rand::rngs::OsRng;

        // 1. Convert client's Ed25519 verifying key to X25519 public key.
        let client_x25519_pk_bytes = ed25519_vk_to_x25519_pk_bytes(client_ed25519_vk);
        let client_pk = PublicKey::from(client_x25519_pk_bytes);

        // 2. Generate server ephemeral keypair.
        let server_sk = SecretKey::generate(&mut OsRng);
        let server_pk = server_sk.public_key();

        // 3. Encrypt nonce using SalsaBox (NaCl crypto_box).
        let salsa_box = SalsaBox::new(&client_pk, &server_sk);
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let ciphertext = salsa_box.encrypt(&nonce, nonce_plaintext).unwrap();

        // 4. Return 24-byte nonce prefix + ciphertext as hex, server pubkey as hex.
        let mut encrypted = nonce.to_vec();
        encrypted.extend_from_slice(&ciphertext);
        (hex::encode(encrypted), hex::encode(server_pk.as_bytes()))
    }

    fn ed25519_vk_to_x25519_pk_bytes(vk: &ed25519_dalek::VerifyingKey) -> [u8; 32] {
        // Convert Ed25519 verifying key to X25519 public key (Montgomery form).
        vk.to_montgomery().to_bytes()
    }

    #[test]
    fn test_pop_challenge_decrypt() {
        let client_signing_key = SigningKey::generate(&mut OsRng);
        let client_verifying_key = client_signing_key.verifying_key();

        let nonce_plaintext = b"test-challenge-nonce-1234567890ab";
        let (encrypted_nonce, server_public_key) =
            simulate_server_challenge(&client_verifying_key, nonce_plaintext);

        let decrypted = decrypt_pop_challenge(
            &client_signing_key,
            &encrypted_nonce,
            &server_public_key,
        )
        .unwrap();

        assert_eq!(decrypted, nonce_plaintext);
    }
}
