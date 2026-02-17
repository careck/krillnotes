use crate::{KrillnotesError, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Get a stable device ID based on hardware identifiers (MAC address)
pub fn get_device_id() -> Result<String> {
    // Try to get MAC address
    match mac_address::get_mac_address() {
        Ok(Some(mac)) => {
            // Hash the MAC address to create a stable device ID
            let mut hasher = DefaultHasher::new();
            mac.bytes().hash(&mut hasher);
            let hash = hasher.finish();
            Ok(format!("device-{:016x}", hash))
        }
        Ok(None) => {
            // No MAC address found, fall back to error
            Err(KrillnotesError::InvalidWorkspace(
                "Could not determine device MAC address".to_string(),
            ))
        }
        Err(e) => Err(KrillnotesError::InvalidWorkspace(format!(
            "Failed to get MAC address: {}",
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_is_stable() {
        // Device ID should be the same across multiple calls
        let id1 = get_device_id();
        let id2 = get_device_id();

        // Both should succeed or both should fail
        match (id1, id2) {
            (Ok(id1), Ok(id2)) => {
                assert_eq!(id1, id2, "Device ID should be stable");
                assert!(id1.starts_with("device-"), "Device ID should have correct format");
            }
            (Err(_), Err(_)) => {
                // Both failed, this is acceptable in test environments
                // without network interfaces
            }
            _ => panic!("Device ID generation is inconsistent"),
        }
    }
}
