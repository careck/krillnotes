use super::*;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, AcceptedInviteManager) {
    let dir = TempDir::new().unwrap();
    let mgr = AcceptedInviteManager::new(dir.path().to_path_buf()).unwrap();
    (dir, mgr)
}

#[test]
fn test_save_and_get() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-123".to_string(),
        "Research Notes".to_string(),
        "base64key".to_string(),
        "Alice".to_string(),
        Some("https://relay.example.com/invites/abc".to_string()),
        "editor".to_string(),
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.workspace_name, "Research Notes");
    assert_eq!(fetched.status, AcceptedInviteStatus::WaitingSnapshot);
    assert!(fetched.workspace_path.is_none());
    assert_eq!(fetched.offered_role, "editor");
}

#[test]
fn test_list_returns_sorted_by_accepted_at_desc() {
    let (_dir, mut mgr) = setup();
    let invite1 = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "First".into(),
        "key1".into(),
        "Alice".into(),
        None,
        "viewer".into(),
    );
    let invite2 = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-2".into(),
        "Second".into(),
        "key2".into(),
        "Bob".into(),
        None,
        "editor".into(),
    );
    mgr.save(&invite1).unwrap();
    mgr.save(&invite2).unwrap();

    let list = mgr.list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].accepted_at >= list[1].accepted_at);
}

#[test]
fn test_update_status_to_workspace_created() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key".into(),
        "Alice".into(),
        None,
        "viewer".into(),
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();

    mgr.update_status(
        id,
        AcceptedInviteStatus::WorkspaceCreated,
        Some("/path/to/ws".to_string()),
    )
    .unwrap();

    let fetched = mgr.get(id).unwrap().unwrap();
    assert_eq!(fetched.status, AcceptedInviteStatus::WorkspaceCreated);
    assert_eq!(fetched.workspace_path.as_deref(), Some("/path/to/ws"));
}

#[test]
fn test_list_waiting_snapshot() {
    let (_dir, mut mgr) = setup();
    let invite1 = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "First".into(),
        "key1".into(),
        "Alice".into(),
        None,
        "viewer".into(),
    );
    let id1 = invite1.invite_id;
    mgr.save(&invite1).unwrap();

    let invite2 = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-2".into(),
        "Second".into(),
        "key2".into(),
        "Bob".into(),
        None,
        "editor".into(),
    );
    mgr.save(&invite2).unwrap();

    mgr.update_status(id1, AcceptedInviteStatus::WorkspaceCreated, Some("/path".to_string()))
        .unwrap();

    let waiting = mgr.list_waiting_snapshot().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].workspace_name, "Second");
}

#[test]
fn test_delete() {
    let (_dir, mut mgr) = setup();
    let invite = AcceptedInvite::new(
        Uuid::new_v4(),
        "ws-1".into(),
        "Notes".into(),
        "key".into(),
        "Alice".into(),
        None,
        "viewer".into(),
    );
    let id = invite.invite_id;
    mgr.save(&invite).unwrap();
    assert!(mgr.get(id).unwrap().is_some());

    mgr.delete(id).unwrap();
    assert!(mgr.get(id).unwrap().is_none());
}

#[test]
fn test_get_nonexistent_returns_none() {
    let (_dir, mgr) = setup();
    assert!(mgr.get(Uuid::new_v4()).unwrap().is_none());
}
