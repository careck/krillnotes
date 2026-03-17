use super::*;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, ReceivedResponseManager) {
    let dir = TempDir::new().unwrap();
    let mgr = ReceivedResponseManager::new(dir.path().to_path_buf()).unwrap();
    (dir, mgr)
}

#[test]
fn test_save_and_get() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-123".to_string(),
        "Research Notes".to_string(),
        "invitee_key_base64".to_string(),
        "Carol Davis".to_string(),
    );
    let id = response.response_id;
    mgr.save(&response).unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.invitee_declared_name, "Carol Davis");
    assert_eq!(fetched.status, ReceivedResponseStatus::Pending);
}

#[test]
fn test_list_by_workspace() {
    let (_dir, mut mgr) = setup();
    let r1 = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key1".into(),
        "Carol".into(),
    );
    let r2 = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-2".into(),
        "Other".into(),
        "key2".into(),
        "Dave".into(),
    );
    let r3 = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key3".into(),
        "Eve".into(),
    );
    mgr.save(&r1).unwrap();
    mgr.save(&r2).unwrap();
    mgr.save(&r3).unwrap();

    assert_eq!(mgr.list_by_workspace("ws-1").unwrap().len(), 2);
    assert_eq!(mgr.list_by_workspace("ws-2").unwrap().len(), 1);
}

#[test]
fn test_update_status_progression() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key".into(),
        "Carol".into(),
    );
    let id = response.response_id;
    mgr.save(&response).unwrap();

    mgr.update_status(id, ReceivedResponseStatus::PeerAdded)
        .unwrap();
    assert_eq!(
        mgr.get(id).unwrap().unwrap().status,
        ReceivedResponseStatus::PeerAdded
    );

    mgr.update_status(id, ReceivedResponseStatus::SnapshotSent)
        .unwrap();
    assert_eq!(
        mgr.get(id).unwrap().unwrap().status,
        ReceivedResponseStatus::SnapshotSent
    );
}

#[test]
fn test_find_by_invite_and_invitee() {
    let (_dir, mut mgr) = setup();
    let invite_id = Uuid::new_v4();
    let r1 = ReceivedResponse::new(
        invite_id,
        "ws-1".into(),
        "Notes".into(),
        "key_carol".into(),
        "Carol".into(),
    );
    let r2 = ReceivedResponse::new(
        invite_id,
        "ws-1".into(),
        "Notes".into(),
        "key_dave".into(),
        "Dave".into(),
    );
    mgr.save(&r1).unwrap();
    mgr.save(&r2).unwrap();

    let found = mgr
        .find_by_invite_and_invitee(invite_id, "key_carol")
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().invitee_declared_name, "Carol");

    assert!(mgr
        .find_by_invite_and_invitee(invite_id, "key_unknown")
        .unwrap()
        .is_none());
}

#[test]
fn test_delete() {
    let (_dir, mut mgr) = setup();
    let response = ReceivedResponse::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key".into(),
        "Carol".into(),
    );
    let id = response.response_id;
    mgr.save(&response).unwrap();
    mgr.delete(id).unwrap();
    assert!(mgr.get(id).unwrap().is_none());
}
