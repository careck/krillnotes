// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Hybrid encryption for `.swarm` bundle payloads.
//!
//! - Payload: AES-256-GCM (random symmetric key per bundle).
//! - Per-recipient key wrapping: ephemeral X25519 ECDH + HKDF-SHA-256 + AES-256-GCM.
//! - Ed25519 keys are converted to X25519 for key agreement.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey, StaticSecret};

use crate::core::swarm::header::RecipientEntry;
use crate::{KrillnotesError, Result};

// ---------------------------------------------------------------------------
// Key conversion helpers
// ---------------------------------------------------------------------------

/// Convert an Ed25519 verifying key to X25519 public key (Montgomery form).
fn ed25519_pub_to_x25519(key: &VerifyingKey) -> X25519PublicKey {
    X25519PublicKey::from(key.to_montgomery().to_bytes())
}

/// Derive an X25519 static secret from an Ed25519 signing key.
///
/// Algorithm: SHA-512(seed)[0..32], clamped per RFC 7748.
fn ed25519_sk_to_x25519(key: &SigningKey) -> StaticSecret {
    use sha2::Digest;
    let mut h = sha2::Sha512::new();
    h.update(key.as_bytes());
    let hash = h.finalize();
    let mut scalar = [0u8; 32];
    scalar.copy_from_slice(&hash[..32]);
    // RFC 7748 clamping
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;
    StaticSecret::from(scalar)
}

// ---------------------------------------------------------------------------
// AES-256-GCM helpers
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` with AES-256-GCM. Returns `nonce || ciphertext`.
fn aes_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| KrillnotesError::Swarm(format!("AES encrypt error: {e}")))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt `nonce || ciphertext` with AES-256-GCM.
fn aes_decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(KrillnotesError::Swarm("ciphertext too short".to_string()));
    }
    let (nonce_bytes, ct) = data.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ct)
        .map_err(|e| KrillnotesError::Swarm(format!("AES decrypt error: {e}")))
}

// ---------------------------------------------------------------------------
// HKDF key derivation
// ---------------------------------------------------------------------------

fn hkdf_derive(shared_secret: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("HKDF expand failed");
    okm
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` for a list of Ed25519 recipient public keys.
///
/// Returns `(encrypted_payload, Vec<RecipientEntry>)` with one entry per recipient.
pub fn encrypt_for_recipients(
    plaintext: &[u8],
    recipients: &[&VerifyingKey],
) -> Result<(Vec<u8>, Vec<RecipientEntry>)> {
    // 1. Generate random AES-256-GCM payload key.
    let mut aes_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut aes_key);

    // 2. Encrypt the payload.
    let ciphertext = aes_encrypt(&aes_key, plaintext)?;

    // 3. Wrap the AES key for each recipient.
    let mut entries = Vec::with_capacity(recipients.len());
    for (i, &vk) in recipients.iter().enumerate() {
        let recipient_x25519 = ed25519_pub_to_x25519(vk);
        let ephemeral = EphemeralSecret::random_from_rng(rand::thread_rng());
        let ephemeral_pub = X25519PublicKey::from(&ephemeral);
        let shared = ephemeral.diffie_hellman(&recipient_x25519);
        let wrap_key = hkdf_derive(shared.as_bytes(), b"krillnotes-swarm-key-wrap");
        let wrapped = aes_encrypt(&wrap_key, &aes_key)?;

        let mut blob = Vec::with_capacity(32 + wrapped.len());
        blob.extend_from_slice(ephemeral_pub.as_bytes());
        blob.extend_from_slice(&wrapped);

        entries.push(RecipientEntry {
            peer_id: i.to_string(), // caller sets real peer_id after
            encrypted_key: blob,
        });
    }

    Ok((ciphertext, entries))
}

/// Decrypt a payload using the recipient's Ed25519 signing key and their
/// `RecipientEntry` from the bundle header.
pub fn decrypt_payload(
    ciphertext: &[u8],
    entry: &RecipientEntry,
    signing_key: &SigningKey,
) -> Result<Vec<u8>> {
    let blob = &entry.encrypted_key;
    if blob.len() < 32 {
        return Err(KrillnotesError::Swarm("recipient blob too short".to_string()));
    }
    let (ephemeral_pub_bytes, wrapped) = blob.split_at(32);
    let ephemeral_pub = X25519PublicKey::from(
        <[u8; 32]>::try_from(ephemeral_pub_bytes)
            .map_err(|_| KrillnotesError::Swarm("bad ephemeral key".to_string()))?,
    );
    let my_x25519 = ed25519_sk_to_x25519(signing_key);
    let shared = my_x25519.diffie_hellman(&ephemeral_pub);
    let wrap_key = hkdf_derive(shared.as_bytes(), b"krillnotes-swarm-key-wrap");
    let aes_key_bytes = aes_decrypt(&wrap_key, wrapped)?;
    let aes_key: [u8; 32] = aes_key_bytes
        .try_into()
        .map_err(|_| KrillnotesError::Swarm("wrapped key wrong length".to_string()))?;
    aes_decrypt(&aes_key, ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_encrypt_decrypt_single_recipient() {
        let recipient_key = SigningKey::generate(&mut OsRng);
        let verifying_key = recipient_key.verifying_key();
        let plaintext = b"hello swarm payload";

        let (ciphertext, entry): (Vec<u8>, Vec<RecipientEntry>) = encrypt_for_recipients(plaintext, &[&verifying_key]).unwrap();
        assert_eq!(entry.len(), 1);

        let recovered = decrypt_payload(&ciphertext, &entry[0], &recipient_key).unwrap();
        assert_eq!(&recovered, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_multi_recipient() {
        let k1 = SigningKey::generate(&mut OsRng);
        let k2 = SigningKey::generate(&mut OsRng);
        let plaintext = b"shared payload";

        let (ciphertext, entries): (Vec<u8>, Vec<RecipientEntry>) = encrypt_for_recipients(
            plaintext,
            &[&k1.verifying_key(), &k2.verifying_key()],
        ).unwrap();
        assert_eq!(entries.len(), 2);

        let r1 = decrypt_payload(&ciphertext, &entries[0], &k1).unwrap();
        let r2 = decrypt_payload(&ciphertext, &entries[1], &k2).unwrap();
        assert_eq!(&r1, plaintext);
        assert_eq!(&r2, plaintext);
    }

    #[test]
    fn test_wrong_key_fails_decrypt() {
        let recipient_key = SigningKey::generate(&mut OsRng);
        let wrong_key = SigningKey::generate(&mut OsRng);
        let plaintext = b"secret";

        let (ciphertext, entries): (Vec<u8>, Vec<RecipientEntry>) =
            encrypt_for_recipients(plaintext, &[&recipient_key.verifying_key()]).unwrap();
        assert!(decrypt_payload(&ciphertext, &entries[0], &wrong_key).is_err());
    }
}
