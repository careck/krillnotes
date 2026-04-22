// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Stable hardware-based device identity for Krillnotes.

use crate::{KrillnotesError, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Returns a stable device identifier derived from the machine's primary MAC address.
///
/// The MAC address bytes are hashed to produce an opaque identifier of the form
/// `device-<16 hex digits>`. The same hardware always yields the same identifier
/// across process restarts.
///
/// # Errors
///
/// Returns [`KrillnotesError::InvalidWorkspace`] if the system has no network
/// interfaces or the MAC address cannot be read.
pub fn get_device_id() -> Result<String> {
    match mac_address::get_mac_address() {
        Ok(Some(mac)) => {
            // DefaultHasher is not guaranteed stable across Rust versions, but this
            // derivation is battle-tested for multi-device sync.  Do NOT change
            // without extensive cross-device migration testing.
            let mut hasher = DefaultHasher::new();
            mac.bytes().hash(&mut hasher);
            let hash = hasher.finish();
            Ok(format!("device-{hash:016x}"))
        }
        Ok(None) => Err(KrillnotesError::InvalidWorkspace(
            "Could not determine device MAC address".to_string(),
        )),
        Err(e) => Err(KrillnotesError::InvalidWorkspace(format!(
            "Failed to get MAC address: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_is_stable() {
        let id1 = get_device_id();
        let id2 = get_device_id();

        match (id1, id2) {
            (Ok(id1), Ok(id2)) => {
                assert_eq!(id1, id2, "Device ID should be stable");
                assert!(id1.starts_with("device-"), "Device ID should have correct format");
            }
            (Err(_), Err(_)) => {
                // Both failed — acceptable in environments without network interfaces.
            }
            _ => panic!("Device ID generation is inconsistent"),
        }
    }
}
