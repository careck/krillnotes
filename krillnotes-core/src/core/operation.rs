// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! CRDT-style operation types for the Krillnotes operation log.

use crate::core::hlc::HlcTimestamp;
use crate::FieldValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single document mutation recorded in the workspace operation log.
///
/// Operations capture the full intent of each change so they can be
/// replayed, merged, or synced across devices in a future sync phase.
/// Every variant carries a stable `operation_id`, an HLC `timestamp`,
/// and the `device_id` of the originating machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Operation {
    /// A new note was inserted into the workspace hierarchy.
    CreateNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID assigned to the new note.
        note_id: String,
        /// Parent note ID, or `None` for a root note.
        parent_id: Option<String>,
        /// Fractional position among siblings.
        position: f64,
        /// Schema type of the new note.
        node_type: String,
        /// Initial title of the new note.
        title: String,
        /// Initial field values of the new note.
        fields: BTreeMap<String, FieldValue>,
        /// Public key (base64) of the identity that created this note.
        created_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// The title of an existing note was updated.
    UpdateNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose title was updated.
        note_id: String,
        /// New title for the note.
        title: String,
        /// Public key (base64) of the identity that modified this note.
        modified_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// A single schema field on an existing note was updated.
    UpdateField {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose field was updated.
        note_id: String,
        /// Name of the field that changed.
        field: String,
        /// New value for the field.
        value: FieldValue,
        /// Public key (base64) of the identity that modified this note.
        modified_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// A note (and all its descendants) was deleted.
    DeleteNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the deleted note.
        note_id: String,
        /// Public key (base64) of the identity that deleted this note.
        deleted_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// A note was relocated to a new parent or position.
    MoveNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note that was moved.
        note_id: String,
        /// New parent note ID, or `None` to move to root level.
        new_parent_id: Option<String>,
        /// New fractional position among siblings.
        new_position: f64,
        /// Public key (base64) of the identity that moved this note.
        moved_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// The tags on an existing note were replaced.
    SetTags {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose tags were updated.
        note_id: String,
        /// Full replacement tag list (normalised).
        tags: Vec<String>,
        /// Public key (base64) of the identity that modified this note.
        modified_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// A new user script was created.
    CreateUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID assigned to the new script.
        script_id: String,
        /// Script name (from front matter).
        name: String,
        /// Script description (from front matter).
        description: String,
        /// Full Rhai source code.
        source_code: String,
        /// Position in load order.
        load_order: i32,
        /// Whether the script is active.
        enabled: bool,
        /// Public key (base64) of the identity that created this script.
        created_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// An existing user script was modified (source, enabled state, or load order).
    UpdateUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the script that was modified.
        script_id: String,
        /// Updated script name.
        name: String,
        /// Updated script description.
        description: String,
        /// Updated full source code.
        source_code: String,
        /// Updated load order.
        load_order: i32,
        /// Updated enabled state.
        enabled: bool,
        /// Public key (base64) of the identity that modified this script.
        modified_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// A user script was deleted.
    DeleteUserScript {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the deleted script.
        script_id: String,
        /// Public key (base64) of the identity that deleted this script.
        deleted_by: String,
        /// Ed25519 signature over the canonical JSON payload (base64).
        signature: String,
    },
    /// Reverses one or more previously logged operations (undo).
    ///
    /// `retracted_ids` lists all operation IDs this retract covers.
    /// A note save emits title + N field ops; one retract covers all of them.
    RetractOperation {
        /// Stable UUID for this operation.
        operation_id: String,
        /// HLC timestamp when the operation was created.
        timestamp: HlcTimestamp,
        /// ID of the device that performed this operation.
        device_id: String,
        /// Operation IDs that this retract reverses.
        retracted_ids: Vec<String>,
        /// The inverse data needed to restore the previous state.
        inverse: crate::RetractInverse,
        /// `false` for textarea (CRDT) field retracts — excluded from `.swarm` diffs.
        propagate: bool,
    },
}

impl Operation {
    /// Returns the stable identifier for this operation.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        match self {
            Self::CreateNote { operation_id, .. }
            | Self::UpdateNote { operation_id, .. }
            | Self::UpdateField { operation_id, .. }
            | Self::DeleteNote { operation_id, .. }
            | Self::MoveNote { operation_id, .. }
            | Self::SetTags { operation_id, .. }
            | Self::CreateUserScript { operation_id, .. }
            | Self::UpdateUserScript { operation_id, .. }
            | Self::DeleteUserScript { operation_id, .. }
            | Self::RetractOperation { operation_id, .. } => operation_id,
        }
    }

    /// Returns the HLC timestamp when this operation was created.
    #[must_use]
    pub fn timestamp(&self) -> HlcTimestamp {
        match self {
            Self::CreateNote { timestamp, .. }
            | Self::UpdateNote { timestamp, .. }
            | Self::UpdateField { timestamp, .. }
            | Self::DeleteNote { timestamp, .. }
            | Self::MoveNote { timestamp, .. }
            | Self::SetTags { timestamp, .. }
            | Self::CreateUserScript { timestamp, .. }
            | Self::UpdateUserScript { timestamp, .. }
            | Self::DeleteUserScript { timestamp, .. }
            | Self::RetractOperation { timestamp, .. } => *timestamp,
        }
    }

    /// Returns the device identifier of the machine that created this operation.
    #[must_use]
    pub fn device_id(&self) -> &str {
        match self {
            Self::CreateNote { device_id, .. }
            | Self::UpdateNote { device_id, .. }
            | Self::UpdateField { device_id, .. }
            | Self::DeleteNote { device_id, .. }
            | Self::MoveNote { device_id, .. }
            | Self::SetTags { device_id, .. }
            | Self::CreateUserScript { device_id, .. }
            | Self::UpdateUserScript { device_id, .. }
            | Self::DeleteUserScript { device_id, .. }
            | Self::RetractOperation { device_id, .. } => device_id,
        }
    }

    /// Returns the base64-encoded public key of the author of this operation.
    ///
    /// Returns an empty string for `RetractOperation`, which is a local-only undo marker.
    #[must_use]
    pub fn author_key(&self) -> &str {
        match self {
            Self::CreateNote { created_by, .. } => created_by,
            Self::UpdateNote { modified_by, .. } => modified_by,
            Self::UpdateField { modified_by, .. } => modified_by,
            Self::DeleteNote { deleted_by, .. } => deleted_by,
            Self::MoveNote { moved_by, .. } => moved_by,
            Self::SetTags { modified_by, .. } => modified_by,
            Self::CreateUserScript { created_by, .. } => created_by,
            Self::UpdateUserScript { modified_by, .. } => modified_by,
            Self::DeleteUserScript { deleted_by, .. } => deleted_by,
            Self::RetractOperation { .. } => "",
        }
    }

    // ── Private helpers for sign/verify ────────────────────────────────────

    fn set_author_key(&mut self, key: String) {
        match self {
            Self::CreateNote { created_by, .. } => *created_by = key,
            Self::UpdateNote { modified_by, .. } => *modified_by = key,
            Self::UpdateField { modified_by, .. } => *modified_by = key,
            Self::DeleteNote { deleted_by, .. } => *deleted_by = key,
            Self::MoveNote { moved_by, .. } => *moved_by = key,
            Self::SetTags { modified_by, .. } => *modified_by = key,
            Self::CreateUserScript { created_by, .. } => *created_by = key,
            Self::UpdateUserScript { modified_by, .. } => *modified_by = key,
            Self::DeleteUserScript { deleted_by, .. } => *deleted_by = key,
            Self::RetractOperation { .. } => {}
        }
    }

    fn set_signature(&mut self, sig: String) {
        match self {
            Self::CreateNote { signature, .. }
            | Self::UpdateNote { signature, .. }
            | Self::UpdateField { signature, .. }
            | Self::DeleteNote { signature, .. }
            | Self::MoveNote { signature, .. }
            | Self::SetTags { signature, .. }
            | Self::CreateUserScript { signature, .. }
            | Self::UpdateUserScript { signature, .. }
            | Self::DeleteUserScript { signature, .. } => *signature = sig,
            Self::RetractOperation { .. } => {}
        }
    }

    fn get_signature(&self) -> &str {
        match self {
            Self::CreateNote { signature, .. }
            | Self::UpdateNote { signature, .. }
            | Self::UpdateField { signature, .. }
            | Self::DeleteNote { signature, .. }
            | Self::MoveNote { signature, .. }
            | Self::SetTags { signature, .. }
            | Self::CreateUserScript { signature, .. }
            | Self::UpdateUserScript { signature, .. }
            | Self::DeleteUserScript { signature, .. } => signature,
            Self::RetractOperation { .. } => "",
        }
    }

    // ── Cryptographic signing ───────────────────────────────────────────────

    /// Sign this operation in place. Sets the author key and signature.
    ///
    /// The canonical payload is the operation serialised to JSON with
    /// `signature = ""` so that the signature field itself is not part of
    /// what is signed.
    pub fn sign(&mut self, key: &ed25519_dalek::SigningKey) {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        use ed25519_dalek::Signer;

        // 1. Set the author key to the verifying key (base64).
        let pubkey_bytes = key.verifying_key().to_bytes();
        let pubkey_b64 = STANDARD.encode(pubkey_bytes);
        self.set_author_key(pubkey_b64);

        // 2. Set signature to "" for canonical payload.
        self.set_signature(String::new());

        // 3. Serialise and sign.
        let payload = serde_json::to_string(self).expect("Operation must be serializable");
        let sig = key.sign(payload.as_bytes());

        // 4. Store the signature.
        self.set_signature(STANDARD.encode(sig.to_bytes()));
    }

    /// Verify the Ed25519 signature on this operation against the provided public key.
    ///
    /// Returns `false` if the signature is missing, malformed, or invalid.
    pub fn verify(&self, pubkey: &ed25519_dalek::VerifyingKey) -> bool {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        use ed25519_dalek::Verifier;

        let sig_b64 = self.get_signature();
        let Ok(sig_bytes) = STANDARD.decode(sig_b64) else {
            return false;
        };
        let Ok(sig_bytes_arr) = <[u8; 64]>::try_from(sig_bytes) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes_arr);

        // Build canonical payload with signature = "".
        let mut clone = self.clone();
        clone.set_signature(String::new());
        let payload = serde_json::to_string(&clone).expect("Operation must be serializable");

        pubkey.verify(payload.as_bytes(), &sig).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_timestamp() -> HlcTimestamp {
        HlcTimestamp {
            wall_ms: 1_000_000,
            counter: 0,
            node_id: 0,
        }
    }

    #[test]
    fn test_retract_operation_serialization() {
        use crate::RetractInverse;
        let op = Operation::RetractOperation {
            operation_id: "ret-1".into(),
            timestamp: HlcTimestamp {
                wall_ms: 9_999_000,
                counter: 0,
                node_id: 0,
            },
            device_id: "dev-1".into(),
            retracted_ids: vec!["op-1".into(), "op-2".into()],
            inverse: RetractInverse::DeleteNote { note_id: "n-1".into() },
            propagate: true,
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.operation_id(), "ret-1");
        assert_eq!(back.timestamp().wall_ms, 9_999_000);
    }

    #[test]
    fn test_operation_serialization() {
        let op = Operation::CreateNote {
            operation_id: "op-123".to_string(),
            timestamp: dummy_timestamp(),
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            parent_id: None,
            position: 0.0,
            node_type: "TextNote".to_string(),
            title: "Test".to_string(),
            fields: BTreeMap::new(),
            created_by: String::new(),
            signature: String::new(),
        };

        let json = serde_json::to_string(&op).unwrap();
        let deserialized: Operation = serde_json::from_str(&json).unwrap();

        assert_eq!(op.operation_id(), deserialized.operation_id());
    }

    #[test]
    fn test_sign_and_verify() {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let mut op = Operation::UpdateField {
            operation_id: "op-sign-1".to_string(),
            timestamp: dummy_timestamp(),
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            field: "body".to_string(),
            value: crate::FieldValue::Text("hello".to_string()),
            modified_by: String::new(),
            signature: String::new(),
        };

        op.sign(&signing_key);

        // Signature and author key must be set after signing.
        assert!(!op.get_signature().is_empty());
        assert!(!op.author_key().is_empty());

        // Verification must pass with the correct key.
        assert!(op.verify(&verifying_key), "signature should verify");

        // Tamper with the operation — verification must fail.
        if let Operation::UpdateField { ref mut field, .. } = op {
            *field = "tampered".to_string();
        }
        assert!(!op.verify(&verifying_key), "tampered operation should not verify");
    }

    #[test]
    fn test_create_note_sign_and_verify_multi_field() {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Build a CreateNote with multiple fields — order must be deterministic.
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), crate::FieldValue::Text("hello world".to_string()));
        fields.insert("rating".to_string(), crate::FieldValue::Text("5".to_string()));
        fields.insert("author".to_string(), crate::FieldValue::Text("Alice".to_string()));

        let mut op = Operation::CreateNote {
            operation_id: "op-cn-sign-1".to_string(),
            timestamp: dummy_timestamp(),
            device_id: "dev-1".to_string(),
            note_id: "note-multi-1".to_string(),
            parent_id: None,
            position: 0.0,
            node_type: "TextNote".to_string(),
            title: "Multi-field note".to_string(),
            fields,
            created_by: String::new(),
            signature: String::new(),
        };

        op.sign(&signing_key);

        assert!(!op.get_signature().is_empty());
        assert!(!op.author_key().is_empty());

        // Verification must succeed with the correct key.
        assert!(op.verify(&verifying_key), "CreateNote multi-field signature should verify");

        // Tamper with a field value — verification must fail.
        if let Operation::CreateNote { ref mut title, .. } = op {
            *title = "tampered".to_string();
        }
        assert!(!op.verify(&verifying_key), "tampered CreateNote should not verify");
    }

    #[test]
    fn test_update_note_variant() {
        let op = Operation::UpdateNote {
            operation_id: "op-upd-1".to_string(),
            timestamp: dummy_timestamp(),
            device_id: "dev-2".to_string(),
            note_id: "note-42".to_string(),
            title: "New Title".to_string(),
            modified_by: String::new(),
            signature: String::new(),
        };

        let json = serde_json::to_string(&op).unwrap();
        let back: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.operation_id(), "op-upd-1");
        assert_eq!(back.author_key(), "");
    }

    #[test]
    fn test_set_tags_variant() {
        let op = Operation::SetTags {
            operation_id: "op-tags-1".to_string(),
            timestamp: dummy_timestamp(),
            device_id: "dev-3".to_string(),
            note_id: "note-7".to_string(),
            tags: vec!["rust".to_string(), "crdt".to_string()],
            modified_by: String::new(),
            signature: String::new(),
        };

        let json = serde_json::to_string(&op).unwrap();
        let back: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.operation_id(), "op-tags-1");
        if let Operation::SetTags { tags, .. } = back {
            assert_eq!(tags, vec!["rust", "crdt"]);
        } else {
            panic!("wrong variant after round-trip");
        }
    }
}
