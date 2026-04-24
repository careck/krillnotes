// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Attachment crypto primitives and metadata types.

use crate::{KrillnotesError, Result};
use hkdf::Hkdf;
use sha2::Sha256;
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use super::timestamp::UnixSecs;

/// Metadata for a single file attachment stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub id: String,
    pub note_id: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub hash_sha256: String,
    /// 32-byte HKDF per-file salt (hex-encoded for Tauri serialisation).
    pub salt: String,
    pub created_at: UnixSecs,
}

/// Derives a 32-byte workspace attachment key from the master password and a
/// workspace-unique UUID string (used as the HKDF salt).
pub fn derive_attachment_key(password: &str, workspace_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(
        Some(workspace_id.as_bytes()),
        password.as_bytes(),
    );
    let mut key = [0u8; 32];
    hk.expand(b"krillnotes-attachment-v1", &mut key)
        .expect("HKDF expand cannot fail for 32-byte output");
    key
}

/// Derives a 32-byte per-file key from the workspace attachment key and a
/// random per-file salt.
fn derive_file_key(attachment_key: &[u8; 32], file_salt: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(file_salt), attachment_key);
    let mut key = [0u8; 32];
    hk.expand(b"krillnotes-file-v1", &mut key)
        .expect("HKDF expand cannot fail for 32-byte output");
    key
}

/// Encrypts `plaintext` using ChaCha20-Poly1305.
///
/// If `key` is `None` (unencrypted workspace), bytes are returned unchanged.
/// Otherwise the output format is: `[12-byte nonce][ciphertext+16-byte tag]`.
/// Returns `(encrypted_bytes, file_salt)`.
pub fn encrypt_attachment(plaintext: &[u8], key: Option<&[u8; 32]>) -> Result<(Vec<u8>, [u8; 32])> {
    let Some(attachment_key) = key else {
        // Unencrypted workspace — store plaintext, return zero salt
        return Ok((plaintext.to_vec(), [0u8; 32]));
    };

    let mut nonce_bytes = [0u8; 12];
    let mut file_salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    rand::thread_rng().fill_bytes(&mut file_salt);

    let file_key = derive_file_key(attachment_key, &file_salt);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&file_key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| KrillnotesError::AttachmentEncryption(e.to_string()))?;

    // Format: [12-byte nonce][ciphertext+16-byte tag]
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok((output, file_salt))
}

/// Decrypts bytes previously encrypted by `encrypt_attachment`.
///
/// If `key` is `None`, bytes are returned unchanged (unencrypted workspace).
pub fn decrypt_attachment(data: &[u8], key: Option<&[u8; 32]>, salt: &[u8]) -> Result<Vec<u8>> {
    let Some(attachment_key) = key else {
        return Ok(data.to_vec());
    };

    if data.len() < 12 {
        return Err(KrillnotesError::AttachmentEncryption(
            "File too short to contain nonce".to_string(),
        ));
    }

    let nonce_bytes = &data[..12];
    let ciphertext = &data[12..];

    let salt_array: [u8; 32] = salt
        .try_into()
        .map_err(|_| KrillnotesError::AttachmentEncryption("Invalid salt length".to_string()))?;

    let file_key = derive_file_key(attachment_key, &salt_array);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&file_key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| KrillnotesError::AttachmentEncryption(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_attachment_key_is_deterministic() {
        let k1 = derive_attachment_key("hunter2", "workspace-uuid-abc");
        let k2 = derive_attachment_key("hunter2", "workspace-uuid-abc");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_attachment_key_differs_by_password() {
        let k1 = derive_attachment_key("pass1", "same-uuid");
        let k2 = derive_attachment_key("pass2", "same-uuid");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_derive_attachment_key_differs_by_workspace() {
        let k1 = derive_attachment_key("pass", "uuid-a");
        let k2 = derive_attachment_key("pass", "uuid-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let key = derive_attachment_key("testpass", "test-uuid");
        let plaintext = b"Hello, attachments!";
        let (ciphertext, salt) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        let recovered = decrypt_attachment(&ciphertext, Some(&key), &salt).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_output_each_time() {
        let key = derive_attachment_key("testpass", "test-uuid");
        let plaintext = b"same content";
        let (ct1, _) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        let (ct2, _) = encrypt_attachment(plaintext, Some(&key)).unwrap();
        // Due to random nonce, ciphertexts must differ
        assert_ne!(ct1, ct2);
    }

    #[test]
    fn test_unencrypted_workspace_passthrough() {
        let plaintext = b"unencrypted content";
        let (stored, _salt) = encrypt_attachment(plaintext, None).unwrap();
        let recovered = decrypt_attachment(&stored, None, &[]).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let key = derive_attachment_key("correct", "uuid");
        let plaintext = b"secret data";
        let (ciphertext, salt) = encrypt_attachment(plaintext, Some(&key)).unwrap();

        let wrong_key = derive_attachment_key("wrong", "uuid");
        let result = decrypt_attachment(&ciphertext, Some(&wrong_key), &salt);
        assert!(result.is_err());
    }
}
