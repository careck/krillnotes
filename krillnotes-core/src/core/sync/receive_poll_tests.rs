use super::*;

#[test]
fn test_workspace_poll_result_serializes_camel_case() {
    let result = WorkspacePollResult {
        applied_bundles: vec![AppliedBundle {
            peer_device_id: "device-1".to_string(),
            mode: "delta".to_string(),
            op_count: 5,
        }],
        new_responses: vec![],
        errors: vec![],
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("peerDeviceId"));
    assert!(json.contains("opCount"));
}

#[test]
fn test_identity_poll_result_serializes_camel_case() {
    let result = IdentityPollResult {
        received_snapshots: vec![],
        errors: vec![PollError {
            bundle_id: Some("bundle-123".to_string()),
            error: "timeout".to_string(),
        }],
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("bundleId"));
}
