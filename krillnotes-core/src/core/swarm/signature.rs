// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Bundle-level Ed25519 signature over a BLAKE3 manifest hash.
//!
//! The manifest is computed by hashing all (filename, contents) pairs in
//! lexicographic filename order.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use crate::KrillnotesError;
use crate::Result;

/// Compute BLAKE3 manifest hash over `(filename, contents)` pairs.
///
/// Pairs **must** be in lexicographic filename order for determinism.
pub fn manifest_hash(files: &[(&str, &[u8])]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let mut sorted: Vec<(&str, &[u8])> = files.to_vec();
    sorted.sort_by_key(|(name, _)| *name);
    for (name, data) in &sorted {
        hasher.update(name.as_bytes());
        hasher.update(data);
    }
    *hasher.finalize().as_bytes()
}

/// Sign the manifest hash with the given signing key.
///
/// Returns raw 64-byte Ed25519 signature.
pub fn sign_manifest(files: &[(&str, &[u8])], key: &SigningKey) -> Vec<u8> {
    let hash = manifest_hash(files);
    key.sign(&hash).to_bytes().to_vec()
}

/// Verify a manifest signature against a known verifying key.
pub fn verify_manifest(
    files: &[(&str, &[u8])],
    signature_bytes: &[u8],
    key: &VerifyingKey,
) -> Result<()> {
    let hash = manifest_hash(files);
    let sig_array: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| KrillnotesError::Swarm("signature must be 64 bytes".to_string()))?;
    let sig = Signature::from_bytes(&sig_array);
    key.verify(&hash, &sig)
        .map_err(|e| KrillnotesError::Swarm(format!("signature verification failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    #[test]
    fn test_sign_and_verify_manifest() {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let verifying_key = signing_key.verifying_key();
        let files: Vec<(&str, &[u8])> = vec![
            ("header.json", b"{}"),
            ("payload.enc", b"data"),
        ];
        let signature = sign_manifest(&files, &signing_key);
        assert!(verify_manifest(&files, &signature, &verifying_key).is_ok());
    }

    #[test]
    fn test_tampered_content_fails_verify() {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let verifying_key = signing_key.verifying_key();
        let files: Vec<(&str, &[u8])> = vec![("header.json", b"{}")];
        let signature = sign_manifest(&files, &signing_key);
        let tampered: Vec<(&str, &[u8])> = vec![("header.json", b"{evil}")];
        assert!(verify_manifest(&tampered, &signature, &verifying_key).is_err());
    }
}
