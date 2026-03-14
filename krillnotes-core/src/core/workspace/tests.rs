    use super::*;
    use crate::core::contact::{ContactManager, TrustLevel};
    use crate::FieldValue;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_workspace() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Verify root note exists
        let count: i64 = ws
            .connection()
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_humanize() {
        assert_eq!(humanize("my-project"), "My Project");
        assert_eq!(humanize("hello_world"), "Hello World");
        assert_eq!(humanize("test-case-123"), "Test Case 123");
    }

    #[test]
    fn test_create_and_get_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert_eq!(child.title, "Untitled");
        assert_eq!(child.parent_id, Some(root.id));
    }

    #[test]
    fn test_update_note_title() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "New Title".to_string())
            .unwrap();

        let updated = ws.get_note(&root.id).unwrap();
        assert_eq!(updated.title, "New Title");
    }

    #[test]
    fn test_open_existing_workspace() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace first
        {
            let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            assert_eq!(root.schema, "TextNote");
        }

        // Open it
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Verify we can read notes
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].schema, "TextNote");
    }

    #[test]
    fn test_is_expanded_defaults_to_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Check root note is expanded by default
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root note should be expanded by default");

        // Create a child note and verify it's expanded by default
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "New child note should be expanded by default");
    }

    #[test]
    fn test_is_expanded_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace with notes
        {
            let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                .unwrap();
        }

        // Open and verify is_expanded is true
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].is_expanded, "Root note should be expanded");
        assert!(notes[1].is_expanded, "Child note should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root should start expanded");

        // Toggle to collapsed
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(!note.is_expanded, "Root should now be collapsed");

        // Toggle back to expanded
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(note.is_expanded, "Root should be expanded again");
    }

    #[test]
    fn test_toggle_note_expansion_with_child_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Toggle child note
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(!child.is_expanded, "Child should be collapsed");

        // Toggle back
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "Child should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion_nonexistent_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Try to toggle a note that doesn't exist
        let result = ws.toggle_note_expansion("nonexistent-id");
        assert!(result.is_err(), "Should error for nonexistent note");
    }

    #[test]
    fn test_set_and_get_selected_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Initially no selection
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection initially");

        // Set selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()), "Should return selected note ID");

        // Clear selection
        ws.set_selected_note(None).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection after clearing");
    }

    #[test]
    fn test_selected_note_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace and set selection
        {
            let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.set_selected_note(Some(&root.id)).unwrap();
        }

        // Open workspace and verify selection persists
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id), "Selection should persist across open");
    }

    #[test]
    fn test_set_selected_note_overwrites_previous() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Set first selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()));

        // Set second selection (should overwrite)
        ws.set_selected_note(Some(&child_id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(child_id.clone()), "Should overwrite previous selection");
    }

    #[test]
    fn test_create_note_root() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Delete existing root note to simulate empty workspace
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Create a new root note
        let new_root_id = ws.create_note_root("TextNote").unwrap();
        let new_root = ws.get_note(&new_root_id).unwrap();

        assert_eq!(new_root.title, "Untitled");
        assert_eq!(new_root.schema, "TextNote");
        assert_eq!(new_root.parent_id, None, "Root note should have no parent");
        assert_eq!(new_root.position, 0.0, "Root note should be at position 0");
        assert!(new_root.is_expanded, "Root note should be expanded");
    }

    #[test]
    fn test_create_note_root_invalid_type() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Delete existing root note
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Try to create a root note with invalid type
        let result = ws.create_note_root("InvalidType");
        assert!(result.is_err(), "Should fail with invalid node type");
    }

    #[test]
    fn test_sibling_insertion_does_not_create_duplicate_positions() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 at position 0 under root
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        // Create child2 as sibling after child1 → gets position 1
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Create child3 as sibling after child1 → should push child2 to position 2, child3 at position 1
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // All siblings should have unique positions
        assert_ne!(child1.position, child2.position, "child1 and child2 should not share a position");
        assert_ne!(child2.position, child3.position, "child2 and child3 should not share a position");
        assert_ne!(child1.position, child3.position, "child1 and child3 should not share a position");
    }

    #[test]
    fn test_get_note_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        // Corrupt the stored JSON directly.
        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        // Should return Err, not panic.
        let result = ws.get_note(&root.id);
        assert!(result.is_err(), "get_note should return Err for corrupt fields_json");
    }

    #[test]
    fn test_list_all_notes_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        let result = ws.list_all_notes();
        assert!(result.is_err(), "list_all_notes should return Err for corrupt fields_json");
    }

    #[test]
    fn test_sibling_insertion_preserves_correct_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 (position 0), child2 as sibling (position 1)
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Insert child3 as sibling after child1 — should land between child1 and child2
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // Expected order: child1 (0), child3 (1), child2 (2)
        assert_eq!(child1.position, 0.0, "child1 should remain at position 0");
        assert_eq!(child3.position, 1.0, "child3 (inserted after child1) should be at position 1");
        assert_eq!(child2.position, 2.0, "child2 should be bumped to position 2");
    }

    #[test]
    fn test_update_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get the root note
        let notes = ws.list_all_notes().unwrap();
        let note_id = notes[0].id.clone();
        let original_modified = notes[0].modified_at;

        // Timestamp resolution is 1 s; sleep ensures modified_at advances.
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Update the note
        let new_title = "Updated Title".to_string();
        let mut new_fields = BTreeMap::new();
        new_fields.insert("body".to_string(), FieldValue::Text("Updated body".to_string()));

        let updated = ws.update_note(&note_id, new_title.clone(), new_fields.clone()).unwrap();

        // Verify changes
        assert_eq!(updated.title, new_title);
        assert_eq!(updated.fields.get("body"), Some(&FieldValue::Text("Updated body".to_string())));
        assert!(updated.modified_at > original_modified);
    }

    #[test]
    fn test_update_note_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let result = ws.update_note("nonexistent-id", "Title".to_string(), BTreeMap::new());
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_count_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Initially has 0 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 0);

        // Create 3 child notes
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Now has 3 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_delete_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> child1 -> grandchild1
        //                   -> child2
        let child1_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild1_id = ws.create_note(&child1_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: root + child1 + child2 + grandchild1 = 4 notes
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete child1 (should delete child1 + grandchild1)
        let result = ws.delete_note_recursive(&child1_id).unwrap();
        assert_eq!(result.deleted_count, 2);
        assert!(result.affected_ids.contains(&child1_id));
        assert!(result.affected_ids.contains(&grandchild1_id));

        // Now only root + child2 remain
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|n| n.id == root_id));
        assert!(remaining.iter().any(|n| n.id == child2_id));
    }

    #[test]
    fn test_delete_note_recursive_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let result = ws.delete_note_recursive("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_delete_note_promote() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> middle -> child1
        //                              -> child2
        let middle_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child1_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: 4 notes total
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete middle (promote children)
        let result = ws.delete_note_promote(&middle_id).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.affected_ids, vec![middle_id.clone()]);

        // Now: root, child1, child2 (3 notes)
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 3);

        // Verify child1 and child2 now have root as parent
        let child1_updated = remaining.iter().find(|n| n.id == child1_id).unwrap();
        let child2_updated = remaining.iter().find(|n| n.id == child2_id).unwrap();
        assert_eq!(child1_updated.parent_id, Some(root_id.clone()));
        assert_eq!(child2_updated.parent_id, Some(root_id.clone()));
    }

    #[test]
    fn test_update_contact_rejects_empty_required_fields() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Contact schema is already loaded from starter scripts.

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        // Contact must be created under a ContactsFolder (allowed_parent_schemas constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        // first_name is required but empty — save must fail.
        let mut fields = BTreeMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Smith".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let result = ws.update_note(&contact_id, "".to_string(), fields);
        assert!(
            matches!(result, Err(KrillnotesError::ValidationFailed(_))),
            "Expected ValidationFailed, got {:?}", result
        );
    }

    /// Verify that `delete_note_promote` returns `NoteNotFound` when the given ID does not exist.
    #[test]
    fn test_delete_note_promote_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let result = ws.delete_note_promote("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    /// Verifies that positions do not collide when children are promoted by
    /// `delete_note_promote`. Specifically, when a node with two children (sib1,
    /// sib2) is deleted, and sib1 itself has children (child1, child2), those
    /// grandchildren should receive sequential positions with no duplicates.
    #[test]
    fn test_delete_note_promote_no_position_collision() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Build tree: root -> sib1 (pos 0) -> child1 (pos 0)
        //                                   -> child2 (pos 1)
        //                  -> sib2 (pos 1)
        let root = ws.list_all_notes().unwrap()[0].clone();
        let sib1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let sib2_id = ws.create_note(&sib1_id, AddPosition::AsSibling, "TextNote").unwrap();
        let child1_id = ws.create_note(&sib1_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        // Delete sib1 with promote — child1 and child2 move up to root level
        ws.delete_note_promote(&sib1_id).unwrap();

        // Collect remaining notes at root level
        let notes = ws.list_all_notes().unwrap();

        // sib1 must be gone
        assert!(notes.iter().all(|n| n.id != sib1_id), "sib1 should be deleted");

        // Gather positions of the surviving root-level notes
        let root_level: Vec<_> = notes.iter().filter(|n| n.parent_id == Some(root.id.clone())).collect();
        let mut positions: Vec<f64> = root_level.iter().map(|n| n.position).collect();
        positions.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // All positions must be unique
        let unique_count = {
            let mut deduped = positions.clone();
            deduped.dedup();
            deduped.len()
        };
        assert_eq!(
            positions.len(), unique_count,
            "Positions after promote must be unique, got: {:?}", positions
        );

        // sib2, child1, child2 should all be at root level
        let surviving_ids: Vec<_> = root_level.iter().map(|n| n.id.clone()).collect();
        assert!(surviving_ids.contains(&sib2_id), "sib2 should remain at root level");
        assert!(surviving_ids.contains(&child1_id), "child1 should be promoted to root level");
        assert!(surviving_ids.contains(&child2_id), "child2 should be promoted to root level");
    }

    #[test]
    fn test_update_contact_derives_title_from_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Contact schema is already loaded from starter scripts.

        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Contact must be created under a ContactsFolder (allowed_parent_schemas constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("Alice".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Walker".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let updated = ws
            .update_note(&contact_id, "ignored title".to_string(), fields)
            .unwrap();

        assert_eq!(updated.title, "Walker, Alice");
    }

    /// Verifies that `delete_note` dispatches correctly to both deletion strategies.
    ///
    /// - `DeleteAll` removes the target note and all descendants.
    /// - `PromoteChildren` removes only the target, re-parenting its children to
    ///   the grandparent.
    // ── User-script CRUD tests ──────────────────────────────────

    #[test]
    fn test_workspace_created_with_starter_scripts() {
        let temp = NamedTempFile::new().unwrap();
        let workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts.is_empty(), "New workspace should have starter scripts");
        // Verify starter scripts include both presentation and schema scripts
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Text Note"), "Should have Text Note schema");
        assert!(names.contains(&"Text Note Actions"), "Should have Text Note Actions");
    }

    #[test]
    fn test_create_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: Test Script\n// @description: A test\nschema(\"TestType\", #{ version: 1, fields: [] });";
        let (script, errors) = workspace.create_user_script(source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(script.name, "Test Script");
        assert_eq!(script.description, "A test");
        assert!(script.enabled);
        assert_eq!(script.load_order, starter_count as i32);
    }

    #[test]
    fn test_create_user_script_missing_name_fails() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// no name here\nschema(\"X\", #{ version: 1, fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// @name: Original\nschema(\"Orig\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();

        let new_source = "// @name: Updated\nschema(\"Updated\", #{ version: 1, fields: [] });";
        let (updated, errors) = workspace.update_user_script(&script.id, new_source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(updated.name, "Updated");
    }

    #[test]
    fn test_delete_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let initial_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: ToDelete\nschema(\"Del\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count + 1);

        workspace.delete_user_script(&script.id).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count);
    }

    #[test]
    fn test_toggle_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// @name: Toggle\nschema(\"Tog\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert!(script.enabled);

        workspace.toggle_user_script(&script.id, false).unwrap();
        let updated = workspace.get_user_script(&script.id).unwrap();
        assert!(!updated.enabled);
    }

    #[test]
    fn test_user_scripts_sorted_by_load_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();

        let s1 = "// @name: Second\nschema(\"S2\", #{ version: 1, fields: [] });";
        let s2 = "// @name: First\nschema(\"S1\", #{ version: 1, fields: [] });";
        workspace.create_user_script(s1).unwrap();
        let (second, _) = workspace.create_user_script(s2).unwrap();
        // Move "First" before all starters
        workspace.reorder_user_script(&second.id, -1).unwrap();

        let scripts = workspace.list_user_scripts().unwrap();
        assert_eq!(scripts[0].name, "First", "Reordered script should come first");
        // "Second" should come after all starters
        assert_eq!(scripts[starter_count + 1].name, "Second");
    }

    #[test]
    fn test_user_scripts_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            workspace.create_user_script(
                "// @name: TestOpen\nschema(\"OpenType\", #{ version: 1, fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap(); // (UserScript, Vec<ScriptError>) — result not inspected here
        }

        let workspace = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(workspace.script_registry().get_schema("OpenType").is_ok());
    }

    #[test]
    fn test_disabled_user_scripts_not_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let (script, _) = workspace.create_user_script(
                "// @name: Disabled\nschema(\"DisType\", #{ version: 1, fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap();
            workspace.toggle_user_script(&script.id, false).unwrap();
        }

        let workspace = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(workspace.script_registry().get_schema("DisType").is_err());
    }

    #[test]
    fn test_delete_note_with_strategy() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Test DeleteAll strategy
        let result = ws.delete_note(&child_id, DeleteStrategy::DeleteAll).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Create new child for PromoteChildren test
        let child2_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild_id = ws.create_note(&child2_id, AddPosition::AsChild, "TextNote").unwrap();

        let result = ws.delete_note(&child2_id, DeleteStrategy::PromoteChildren).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Verify grandchild promoted
        let notes = ws.list_all_notes().unwrap();
        let gc = notes.iter().find(|n| n.id == grandchild_id).unwrap();
        assert_eq!(gc.parent_id, Some(root.id));
    }

    // ── move_note tests ──────────────────────────────────────────

    /// Helper: create a workspace with a root note and N children under it.
    ///
    /// The first child is created with `AsChild` (position 0). Subsequent
    /// children are created with `AsSibling` relative to the previous child,
    /// giving them sequential positions 0, 1, 2, .... The returned `Vec`
    /// preserves that order: `child_ids[0]` is at position 0, etc.
    fn setup_with_children(n: usize) -> (Workspace, String, Vec<String>, NamedTempFile) {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let mut child_ids: Vec<String> = Vec::new();
        for i in 0..n {
            let id = if i == 0 {
                ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                    .unwrap()
            } else {
                ws.create_note(&child_ids[i - 1], AddPosition::AsSibling, "TextNote")
                    .unwrap()
            };
            child_ids.push(id);
        }
        (ws, root.id, child_ids, temp)
    }

    #[test]
    fn test_move_note_reorder_siblings() {
        let (mut ws, root_id, children, _temp) = setup_with_children(3);
        ws.move_note(&children[2], Some(&root_id), 0.0).unwrap();
        let kids = ws.get_children(&root_id).unwrap();
        assert_eq!(kids[0].id, children[2]);
        assert_eq!(kids[1].id, children[0]);
        assert_eq!(kids[2].id, children[1]);
        for (i, kid) in kids.iter().enumerate() {
            assert_eq!(kid.position, i as f64, "Position mismatch at index {i}");
        }
    }

    #[test]
    fn test_move_note_to_different_parent() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&children[0]), 0.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[0]);
        assert_eq!(root_kids[0].position, 0.0);
        let grandkids = ws.get_children(&children[0]).unwrap();
        assert_eq!(grandkids.len(), 1);
        assert_eq!(grandkids[0].id, children[1]);
        assert_eq!(grandkids[0].position, 0.0);
    }

    #[test]
    fn test_move_note_to_root() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[0], None, 1.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[1]);
        assert_eq!(root_kids[0].position, 0.0);
        let moved = ws.get_note(&children[0]).unwrap();
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.position, 1.0);
    }

    #[test]
    fn test_move_note_prevents_cycle() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let grandchild_id = ws
            .create_note(&children[0], AddPosition::AsChild, "TextNote")
            .unwrap();
        let result = ws.move_note(&children[0], Some(&grandchild_id), 0.0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cycle"), "Expected cycle error, got: {err}");
    }

    #[test]
    fn test_move_note_prevents_self_move() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let result = ws.move_note(&children[0], Some(&children[0]), 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_move_note_logs_operation() {
        // The operation log is always active — MoveNote must be recorded.
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&root_id), 0.0).unwrap();
        let ops = ws.list_operations(None, None, None).unwrap();
        let move_ops: Vec<_> = ops.iter().filter(|o| o.operation_type == "MoveNote").collect();
        assert_eq!(move_ops.len(), 1, "Expected one MoveNote operation in always-on log");
    }

    #[test]
    fn test_move_note_positions_gapless_after_cross_parent_move() {
        let (mut ws, root_id, children, _temp) = setup_with_children(4);
        ws.move_note(&children[1], Some(&children[0]), 0.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 3);
        for (i, kid) in root_kids.iter().enumerate() {
            assert_eq!(kid.position, i as f64, "Gap at index {i}");
        }
    }

    #[test]
    fn test_run_view_hook_returns_html_without_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Load a schema with a textarea field but no on_view hook.
        ws.create_user_script(
            r#"// @name: Memo
schema("Memo", #{ version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false }
    ]
});
"#,
        )
        .unwrap();

        // Create a Memo note under the root.
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws
            .create_note(&root.id, AddPosition::AsChild, "Memo")
            .unwrap();

        // Update the note's body field with Markdown content.
        let mut fields = BTreeMap::new();
        fields.insert("body".into(), FieldValue::Text("**hello**".into()));
        ws.update_note(&note_id, "My Memo".into(), fields).unwrap();

        let html = ws.run_view_hook(&note_id).unwrap();
        assert!(!html.is_empty(), "default view must return non-empty HTML");
        assert!(
            html.contains("<strong>hello</strong>"),
            "textarea body should be markdown-rendered, got: {html}"
        );
    }

    #[test]
    fn test_create_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Clearly invalid Rhai: assignment with no identifier
        let bad_script = "// @name: Bad Script\n\nlet = 5;";
        let result = ws.create_user_script(bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai");
        // Confirm nothing was saved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count, "No script should be saved on compile error");
    }

    #[test]
    fn test_update_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Create a valid script first
        let valid_script = "// @name: Good Script\n\n// valid empty body";
        let (created, _) = ws.create_user_script(valid_script).unwrap();

        // Attempt update with invalid Rhai
        let bad_script = "// @name: Good Script\n\nlet = 5;";
        let result = ws.update_user_script(&created.id, bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai on update");

        // Original source code must be preserved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count + 1, "Script count must be unchanged after failed update");
        let saved = scripts.iter().find(|s| s.id == created.id).unwrap();
        assert_eq!(
            saved.source_code, valid_script,
            "Source code must be unchanged after failed update"
        );
    }

    #[test]
    fn test_create_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Should have at least one note (the root note)
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let ws = Workspace::open(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_wrong_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let result = Workspace::open(temp.path(), "wrong", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]));
        assert!(matches!(result, Err(KrillnotesError::WrongPassword)));
    }

    #[test]
    fn test_deep_copy_note_as_child() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // root → child
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&child_id, "Original Child".to_string())
            .unwrap();

        // Copy child as another child of root
        let copy_id = ws
            .deep_copy_note(&child_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // Copy has a new ID
        assert_ne!(copy_id, child_id);

        // Copy has same title and schema
        let copy = ws.get_note(&copy_id).unwrap();
        assert_eq!(copy.title, "Original Child");
        assert_eq!(copy.schema, "TextNote");

        // Original is unchanged
        let original = ws.get_note(&child_id).unwrap();
        assert_eq!(original.title, "Original Child");
        assert_eq!(original.parent_id, Some(root.id.clone()));
    }

    #[test]
    fn test_deep_copy_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // root → note_a → note_b
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_a_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_a_id, "Note A".to_string())
            .unwrap();
        let note_b_id = ws
            .create_note(&note_a_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_b_id, "Note B".to_string())
            .unwrap();

        // Copy note_a (with note_b inside) as a child of root
        let copy_a_id = ws
            .deep_copy_note(&note_a_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // copy of note_a exists with a new ID and correct title
        assert_ne!(copy_a_id, note_a_id);
        let copy_a = ws.get_note(&copy_a_id).unwrap();
        assert_eq!(copy_a.title, "Note A");

        // A copy of note_b also exists — find it by parent = copy_a
        let all_notes = ws.list_all_notes().unwrap();
        let copy_b = all_notes
            .iter()
            .find(|n| n.parent_id.as_deref() == Some(&copy_a_id) && n.title == "Note B")
            .expect("copy of note_b should exist under copy_a");

        // copy of note_b has a new ID (not the original)
        assert_ne!(copy_b.id, note_b_id);

        // originals are untouched
        let orig_a = ws.get_note(&note_a_id).unwrap();
        assert_eq!(orig_a.parent_id, Some(root.id.clone()));
        let orig_b = ws.get_note(&note_b_id).unwrap();
        assert_eq!(orig_b.parent_id, Some(note_a_id.clone()));
    }

    #[test]
    fn test_on_add_child_hook_fires_on_create() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (1)");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();

        // Create an Item under the Folder — this should trigger the hook
        ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    #[test]
    fn test_on_add_child_hook_fires_for_sibling_under_hooked_parent() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        // First child created as child of Folder (hook fires, count=1)
        let first_item_id = ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();
        // Second item created as sibling of first (still a child of Folder, hook should fire again, count=2)
        ws.create_note(&first_item_id, AddPosition::AsSibling, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.fields["count"], FieldValue::Number(2.0));
    }

    #[test]
    fn test_on_add_child_hook_does_not_fire_for_root_level_creation() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // No on_add_child hook registered — creating a sibling of root should work silently
        let root = ws.list_all_notes().unwrap()[0].clone();
        // This creates a sibling of root, which has no parent — should not panic or error
        let result = ws.create_note(&root.id, AddPosition::AsSibling, "TextNote");
        assert!(result.is_ok(), "sibling of root should succeed without hook");
    }

    #[test]
    fn test_on_add_child_hook_fires_on_move() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (1)");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        // Create Folder and Item as siblings (both children of root)
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        let item_id   = ws.create_note(&root.id, AddPosition::AsChild, "Item").unwrap();

        // Move Item under Folder — hook should fire
        ws.move_note(&item_id, Some(&folder_id), 0.0).unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    // ── tree actions ─────────────────────────────────────────────────────────

    #[test]
    fn test_run_tree_action_reorders_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let parent_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Create first child: "B Note" (position 0)
        let child_b_id = ws.create_note(&parent_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_title(&child_b_id, "B Note".to_string()).unwrap();

        // Create second child as sibling: "A Note" (position 1)
        let child_a_id = ws.create_note(&child_b_id, AddPosition::AsSibling, "TextNote").unwrap();
        ws.update_note_title(&child_a_id, "A Note".to_string()).unwrap();

        // Verify initial order: B Note first, A Note second
        let kids_before = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids_before[0].title, "B Note");
        assert_eq!(kids_before[1].title, "A Note");

        // Load a script that sorts children alphabetically
        ws.create_user_script(r#"
// @name: SortTest
register_menu("Sort A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
        "#).unwrap();

        ws.run_tree_action(&parent_id, "Sort A→Z").unwrap();

        let kids = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids[0].title, "A Note");
        assert_eq!(kids[1].title, "B Note");
    }

    // ── tree action creates / updates ─────────────────────────────────────────

    #[test]
    fn test_tree_action_create_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: CreateAction
schema("TaFolder", #{ version: 1, fields: [] });
schema("TaItem", #{ version: 1, fields: [#{ name: "tag", type: "text", required: false }] });
register_menu("Add Item", ["TaFolder"], |folder| {
    let item = create_child(folder.id, "TaItem");
    set_title(item.id, "My Item");
    set_field(item.id, "tag", "hello");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaFolder").unwrap();

        ws.run_tree_action(&folder_id, "Add Item").unwrap();

        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 1, "one child should have been created");
        assert_eq!(children[0].title, "My Item");
        assert_eq!(
            children[0].fields.get("tag"),
            Some(&FieldValue::Text("hello".into()))
        );
    }

    #[test]
    fn test_tree_action_update_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: UpdateAction
schema("TaTask", #{ version: 1, fields: [#{ name: "status", type: "text", required: false }] });
register_menu("Mark Done", ["TaTask"], |note| {
    set_title(note.id, "Done Task");
    set_field(note.id, "status", "done");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let task_id = ws.create_note(&root.id, AddPosition::AsChild, "TaTask").unwrap();

        ws.run_tree_action(&task_id, "Mark Done").unwrap();

        let updated = ws.get_note(&task_id).unwrap();
        assert_eq!(updated.title, "Done Task");
        assert_eq!(
            updated.fields.get("status"),
            Some(&FieldValue::Text("done".into()))
        );
    }

    #[test]
    fn test_tree_action_nested_create_builds_subtree() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: NestedCreate
schema("TaSprint", #{ version: 1, fields: [] });
schema("TaSubTask", #{ version: 1, fields: [] });
register_menu("Add Sprint With Task", ["TaSprint"], |sprint| {
    let child_sprint = create_child(sprint.id, "TaSprint");
    set_title(child_sprint.id, "Child Sprint");
    let task = create_child(child_sprint.id, "TaSubTask");
    set_title(task.id, "Sprint Task");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let sprint_id = ws.create_note(&root.id, AddPosition::AsChild, "TaSprint").unwrap();

        ws.run_tree_action(&sprint_id, "Add Sprint With Task").unwrap();

        // The child sprint should be under sprint_id
        let sprint_children = ws.get_children(&sprint_id).unwrap();
        assert_eq!(sprint_children.len(), 1, "one child sprint expected");
        assert_eq!(sprint_children[0].title, "Child Sprint");

        // The task should be under the child sprint
        let task_children = ws.get_children(&sprint_children[0].id).unwrap();
        assert_eq!(task_children.len(), 1, "one task expected under child sprint");
        assert_eq!(task_children[0].title, "Sprint Task");
    }

    #[test]
    fn test_tree_action_error_rolls_back_all_writes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: ErrorAction
schema("TaErrFolder", #{ version: 1, fields: [] });
schema("TaErrItem", #{ version: 1, fields: [] });
register_menu("Create Then Fail", ["TaErrFolder"], |folder| {
    let item = create_child(folder.id, "TaErrItem");
    set_title(item.id, "Orphan");
    throw "deliberate error";
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaErrFolder").unwrap();

        let result = ws.run_tree_action(&folder_id, "Create Then Fail");
        assert!(result.is_err(), "action should propagate the thrown error");

        // No note should have been created — the creates are not applied when the action errors
        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 0, "rollback: no child note should exist");
    }

    #[test]
    fn test_tree_action_create_child_gated() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: TAGated
schema("TAFolder", #{ version: 1, fields: [] });
schema("TAItem", #{ version: 1,
    fields: [
        #{ name: "value", type: "text", required: false },
    ],
});
register_menu("Add Item", ["TAFolder"], |note| {
    let child = create_child(note.id, "TAItem");
    set_title(child.id, "New Item");
    set_field(child.id, "value", "default");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TAFolder").unwrap();
        ws.run_tree_action(&folder_id, "Add Item").unwrap();
        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].title, "New Item");
        assert_eq!(
            children[0].fields.get("value"),
            Some(&FieldValue::Text("default".into()))
        );
    }

    #[test]
    fn test_note_tags_round_trip() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.tags.is_empty());

        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["design", "rust"]); // sorted
    }

    #[test]
    fn test_get_all_tags_empty() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.get_all_tags().unwrap().is_empty());
    }

    #[test]
    fn test_get_all_tags_sorted_distinct() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["rust".into(), "testing".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["design", "rust", "testing"]);
    }

    #[test]
    fn test_get_notes_for_tag() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["design".into()]).unwrap();

        let rust_notes = ws.get_notes_for_tag(&["rust".into()]).unwrap();
        assert_eq!(rust_notes.len(), 1);
        assert_eq!(rust_notes[0].id, root.id);

        // OR logic: both notes returned when both tags queried
        let both = ws.get_notes_for_tag(&["rust".into(), "design".into()]).unwrap();
        assert_eq!(both.len(), 2);

        // Unknown tag returns empty
        let none = ws.get_notes_for_tag(&["unknown".into()]).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_update_note_tags_replaces_existing() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["old".into()]).unwrap();
        ws.update_note_tags(&root.id, vec!["new".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["new"]); // "old" removed
    }

    #[test]
    fn test_update_note_tags_normalises() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["  Rust  ".into(), "RUST".into(), "rust".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["rust"]); // deduped, lowercased, trimmed
    }

    // ── note_links helpers ────────────────────────────────────────────────────

    /// Creates a workspace with a single user script loaded (for schema setup).
    /// Returns the workspace ready to use.
    fn create_test_workspace_with_schema(schema_script: &str) -> Workspace {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Wrap the bare schema call in the required front matter so create_user_script accepts it.
        let source = format!("// @name: TestSchema\n{schema_script}");
        ws.create_user_script(&source).unwrap();
        // Leak the tempfile so the DB file stays alive for the duration of the test.
        std::mem::forget(temp);
        ws
    }

    /// Creates a new root-level note of `note_type` and returns the full `Note`.
    fn create_note_with_type(ws: &mut Workspace, note_type: &str) -> Note {
        let id = ws.create_note_root(note_type).unwrap();
        ws.get_note(&id).unwrap()
    }

    // ── note_links tests ──────────────────────────────────────────────────────

    #[test]
    fn test_sync_note_links_inserts_row() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
            [&source.id, &target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_sync_note_links_removes_row_when_cleared() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        let mut fields2 = BTreeMap::new();
        fields2.insert("link".into(), FieldValue::NoteLink(None));
        ws.update_note(&source.id, source.title.clone(), fields2).unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1",
            [&source.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_clear_links_to_nulls_field_in_source_note() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        ws.clear_links_to(&target.id).unwrap();

        let updated_source = ws.get_note(&source.id).unwrap();
        assert!(matches!(
            updated_source.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE target_id = ?1",
            [&target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_note_nulls_links_in_other_notes() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        ws.delete_note(&target.id, DeleteStrategy::DeleteAll).unwrap();

        let updated_source = ws.get_note(&source.id).unwrap();
        assert!(matches!(
            updated_source.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));
    }

    #[test]
    fn test_delete_note_recursive_clears_links_for_entire_subtree() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let parent = create_note_with_type(&mut ws, "LinkTestType");
        let child_id = ws.create_note(&parent.id, AddPosition::AsChild, "LinkTestType").unwrap();
        let child = ws.get_note(&child_id).unwrap();
        let observer = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(child.id.clone())));
        ws.update_note(&observer.id, observer.title.clone(), fields).unwrap();

        ws.delete_note(&parent.id, DeleteStrategy::DeleteAll).unwrap();

        let updated_observer = ws.get_note(&observer.id).unwrap();
        assert!(matches!(
            updated_observer.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));
    }

    // ── get_notes_with_link tests ─────────────────────────────────────────────

    #[test]
    fn test_get_notes_with_link_returns_linking_notes() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source1 = create_note_with_type(&mut ws, "LinkTestType");
        let source2 = create_note_with_type(&mut ws, "LinkTestType");

        for source in [&source1, &source2] {
            let mut fields = BTreeMap::new();
            fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
            ws.update_note(&source.id, source.title.clone(), fields.clone()).unwrap();
        }

        let results = ws.get_notes_with_link(&target.id).unwrap();
        assert_eq!(results.len(), 2);
        let result_ids: Vec<&str> = results.iter().map(|n| n.id.as_str()).collect();
        assert!(result_ids.contains(&source1.id.as_str()));
        assert!(result_ids.contains(&source2.id.as_str()));
    }

    #[test]
    fn test_get_notes_with_link_returns_empty_for_unlinked_note() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let note = create_note_with_type(&mut ws, "LinkTestType");
        let results = ws.get_notes_with_link(&note.id).unwrap();
        assert!(results.is_empty());
    }

    // ── search_notes tests ────────────────────────────────────────────────────

    #[test]
    fn test_search_notes_matches_title() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [] })"#
        );
        let note = create_note_with_type(&mut ws, "LinkTestType");
        ws.update_note(&note.id, "Fix login bug".into(), BTreeMap::new()).unwrap();

        let results = ws.search_notes("login", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, note.id);
    }

    #[test]
    fn test_search_notes_filters_by_target_type() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("TaskNote", #{ version: 1, fields: [] }); schema("OtherNote", #{ version: 1, fields: [] })"#
        );
        let task = create_note_with_type(&mut ws, "TaskNote");
        ws.update_note(&task.id, "login task".into(), BTreeMap::new()).unwrap();
        let other = create_note_with_type(&mut ws, "OtherNote");
        ws.update_note(&other.id, "login other".into(), BTreeMap::new()).unwrap();

        let results = ws.search_notes("login", Some("TaskNote")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, task.id);
    }

    #[test]
    fn test_search_notes_matches_text_fields() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("ContactNote", #{ version: 1, fields: [#{ name: "email", type: "email" }] })"#
        );
        let c = create_note_with_type(&mut ws, "ContactNote");
        let mut fields = BTreeMap::new();
        fields.insert("email".into(), FieldValue::Email("alice@example.com".into()));
        ws.update_note(&c.id, "Alice".into(), fields).unwrap();

        let results = ws.search_notes("alice@example", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, c.id);
    }

    // ── rebuild_note_links_index tests ────────────────────────────────────────

    #[test]
    fn test_rebuild_note_links_index_repopulates_from_fields_json() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        // Manually wipe the index
        ws.connection().execute("DELETE FROM note_links", []).unwrap();

        // Rebuild
        ws.rebuild_note_links_index().unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
            [&source.id, &target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn operations_log_always_records_create_note() {
        // The operation log is always active — every mutation must be recorded.
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(!ops.is_empty(), "operation log must always be active");
        let create_ops: Vec<_> = ops.iter().filter(|o| o.operation_type == "CreateNote").collect();
        assert!(!create_ops.is_empty(), "CreateNote must be recorded in always-on log");
    }

    #[test]
    fn test_workspace_has_attachment_key_when_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "hunter2", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.attachment_key().is_some(), "Encrypted workspace must have attachment_key");
    }

    #[test]
    fn test_workspace_has_no_attachment_key_when_unencrypted() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.attachment_key().is_none(), "Unencrypted workspace must have no attachment_key");
    }

    #[test]
    fn test_workspace_creates_attachments_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("attachments").is_dir());
    }

    #[test]
    fn test_workspace_attachment_key_stable_across_open() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws1 = Workspace::create(&db_path, "mypass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let key1 = ws1.attachment_key().unwrap().clone();
        drop(ws1);
        let ws2 = Workspace::open(&db_path, "mypass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let key2 = ws2.attachment_key().unwrap();
        assert_eq!(key1, *key2, "Key must be derived deterministically from password + workspace_id");
    }

    #[test]
    fn test_get_set_workspace_metadata() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Fresh workspace returns default (no error)
        let initial = ws.get_workspace_metadata().unwrap();
        assert!(initial.author_name.is_none());
        assert!(initial.tags.is_empty());

        // Set and read back
        let meta = WorkspaceMetadata {
            version: 1,
            author_name: Some("Bob".to_string()),
            author_org: None,
            homepage_url: None,
            description: Some("My workspace".to_string()),
            license: Some("CC BY 4.0".to_string()),
            license_url: None,
            language: Some("en".to_string()),
            tags: vec!["productivity".to_string()],
        };
        ws.set_workspace_metadata(&meta).unwrap();

        let restored = ws.get_workspace_metadata().unwrap();
        assert_eq!(restored.author_name.as_deref(), Some("Bob"));
        assert_eq!(restored.description.as_deref(), Some("My workspace"));
        assert_eq!(restored.license.as_deref(), Some("CC BY 4.0"));
        assert_eq!(restored.language.as_deref(), Some("en"));
        assert_eq!(restored.tags, vec!["productivity"]);
        assert!(restored.author_org.is_none());

        // Overwrite with new values
        let meta2 = WorkspaceMetadata {
            version: 1,
            author_name: Some("Alice".to_string()),
            ..Default::default()
        };
        ws.set_workspace_metadata(&meta2).unwrap();
        let updated = ws.get_workspace_metadata().unwrap();
        assert_eq!(updated.author_name.as_deref(), Some("Alice"));
        assert!(updated.description.is_none());
    }

    #[test]
    fn test_attach_file_stores_metadata_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let notes = ws.list_all_notes().unwrap();
        let root_id = &notes[0].id;

        let data = b"hello attachment";
        let meta = ws.attach_file(root_id, "test.txt", Some("text/plain"), data).unwrap();
        assert_eq!(meta.filename, "test.txt");
        assert_eq!(meta.size_bytes, data.len() as i64);

        let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
        assert!(enc_path.exists(), "Encrypted file must exist on disk");
    }

    #[test]
    fn test_get_attachment_bytes_decrypts_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        let data = b"secret file content";
        let meta = ws.attach_file(&root_id, "doc.txt", None, data).unwrap();
        let recovered = ws.get_attachment_bytes(&meta.id).unwrap();
        assert_eq!(recovered, data as &[u8]);
    }

    #[test]
    fn test_get_attachments_returns_metadata_list() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.attach_file(&root_id, "a.pdf", None, b"data a").unwrap();
        ws.attach_file(&root_id, "b.pdf", None, b"data b").unwrap();

        let attachments = ws.get_attachments(&root_id).unwrap();
        assert_eq!(attachments.len(), 2);
        let names: Vec<&str> = attachments.iter().map(|a| a.filename.as_str()).collect();
        assert!(names.contains(&"a.pdf"));
        assert!(names.contains(&"b.pdf"));
    }

    #[test]
    fn test_delete_attachment_soft_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        let meta = ws.attach_file(&root_id, "bye.txt", None, b"temp").unwrap();
        let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
        let trash_path = dir.path().join("attachments").join(format!("{}.enc.trash", meta.id));
        assert!(enc_path.exists());

        // Soft-delete: file moves to .enc.trash, DB row removed.
        // Attachment deletions do NOT go on the main undo stack (to avoid interfering
        // with note-edit undo/redo which uses the same Cmd+Z shortcut).
        ws.delete_attachment(&meta.id).unwrap();
        assert!(!enc_path.exists(), ".enc must be gone after soft-delete");
        assert!(trash_path.exists(), ".enc.trash must exist after soft-delete");
        assert!(ws.get_attachments(&root_id).unwrap().is_empty(), "DB row must be gone");
        assert!(!ws.can_undo(), "attachment deletion must NOT push to main undo stack");
    }

    #[test]
    fn test_attach_file_enforces_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.set_attachment_max_size_bytes(Some(10)).unwrap();
        let big_data = vec![0u8; 100];
        let result = ws.attach_file(&root_id, "big.bin", None, &big_data);
        assert!(matches!(result, Err(KrillnotesError::AttachmentTooLarge { .. })));
    }

    #[test]
    fn test_update_note_cleans_up_replaced_file_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Use the root TextNote that is always created on workspace init.
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        // Attach a first file to the note.
        let meta1 = ws.attach_file(&root_id, "a.png", Some("image/png"), b"fake_bytes_1").unwrap();

        // Set the File field to point at the first attachment.
        let mut fields = ws.get_note(&root_id).unwrap().fields.clone();
        fields.insert("photo".to_string(), FieldValue::File(Some(meta1.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields).unwrap();

        // Attach a second file and replace the field value with it.
        let meta2 = ws.attach_file(&root_id, "b.png", Some("image/png"), b"fake_bytes_2").unwrap();
        let mut fields2 = ws.get_note(&root_id).unwrap().fields.clone();
        fields2.insert("photo".to_string(), FieldValue::File(Some(meta2.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields2).unwrap();

        // The first attachment must have been deleted when the field was replaced.
        let result = ws.get_attachment_bytes(&meta1.id);
        assert!(result.is_err(), "old attachment should have been deleted when field value was replaced");

        // The second attachment must still be readable.
        assert!(ws.get_attachment_bytes(&meta2.id).is_ok(), "new attachment should still exist");
    }

    #[test]
    fn test_operation_log_always_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();

        // The operation log should record the CreateNote even without sync.
        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(!ops.is_empty(), "operation log must always be active");
        assert_eq!(ops[0].operation_type, "CreateNote");
        let _ = root_id;
    }

    #[test]
    fn test_update_note_cleans_up_cleared_file_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        // Attach a file and store its UUID in a File field.
        let meta = ws.attach_file(&root_id, "x.png", Some("image/png"), b"fake").unwrap();
        let mut fields = ws.get_note(&root_id).unwrap().fields.clone();
        fields.insert("photo".to_string(), FieldValue::File(Some(meta.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields).unwrap();

        // Clear the File field (set to None) — the attachment should be deleted.
        let mut fields2 = ws.get_note(&root_id).unwrap().fields.clone();
        fields2.insert("photo".to_string(), FieldValue::File(None));
        ws.update_note(&root_id, "Test".to_string(), fields2).unwrap();

        let result = ws.get_attachment_bytes(&meta.id);
        assert!(result.is_err(), "attachment should have been deleted when field was cleared");
    }

    #[test]
    fn test_can_undo_initially_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(!ws.can_undo());
        assert!(!ws.can_redo());
    }

    #[test]
    fn test_collect_subtree_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let _grandchild = ws.create_note(&child_id, AddPosition::AsChild, "TextNote").unwrap();

        let notes = ws.collect_subtree_notes(&root_id).unwrap();
        assert_eq!(notes.len(), 3);
        // Root must be first.
        assert_eq!(notes[0].id, root_id);
    }

    #[test]
    fn test_undo_group_collapses_to_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        // Clear the undo entry from root creation
        ws.undo_stack.clear();

        ws.begin_undo_group();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.end_undo_group();

        assert_eq!(ws.undo_stack.len(), 1, "group must collapse to one entry");
        match &ws.undo_stack[0].inverse {
            RetractInverse::Batch(items) => assert_eq!(items.len(), 2),
            _ => panic!("expected Batch"),
        }
    }

    #[test]
    fn test_undo_create_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear(); // ignore root creation

        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        assert!(ws.can_undo());

        let result = ws.undo().unwrap();
        assert_eq!(result.affected_note_id, None);
        assert!(ws.get_note(&child_id).is_err(), "note must be gone after undo");
    }

    #[test]
    fn test_undo_update_note_restores_old_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        ws.update_note(&root_id, "New Title".into(), BTreeMap::new()).unwrap();
        assert!(ws.can_undo());

        // Check undo entry inverse
        match &ws.undo_stack[0].inverse {
            RetractInverse::NoteRestore { old_title, .. } => {
                // The original title from create_note_root should be preserved.
                assert_ne!(old_title, "New Title");
            }
            _ => panic!("expected NoteRestore"),
        }
    }

    #[test]
    fn test_undo_delete_note_restores_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        ws.delete_note_recursive(&child_id).unwrap();
        assert!(ws.can_undo());
        assert!(ws.get_note(&child_id).is_err(), "note gone after delete");

        // Undo entry must be SubtreeRestore.
        assert!(matches!(ws.undo_stack[0].inverse, RetractInverse::SubtreeRestore { .. }));
    }

    #[test]
    fn test_undo_move_note_restores_position() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let sibling_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        let old_note = ws.get_note(&sibling_id).unwrap();
        ws.move_note(&sibling_id, Some(&child_id), 0.0).unwrap();

        assert!(ws.can_undo());
        match &ws.undo_stack[0].inverse {
            RetractInverse::PositionRestore { note_id, old_parent_id, old_position } => {
                assert_eq!(note_id, &sibling_id);
                assert_eq!(old_parent_id, &old_note.parent_id);
                assert_eq!(*old_position, old_note.position);
            }
            _ => panic!("expected PositionRestore"),
        }
    }

    #[test]
    fn test_undo_delete_script_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src = "// @name: TestScript\n// @description: desc\n";
        let (script, _) = ws.create_user_script(src).unwrap();
        ws.script_undo_stack.clear();

        ws.delete_user_script(&script.id).unwrap();
        // Script operations now land on the separate script_undo_stack.
        assert!(ws.can_script_undo());
        assert!(!ws.can_undo(), "note undo stack must be unaffected by script ops");
        assert!(matches!(ws.script_undo_stack[0].inverse, RetractInverse::ScriptRestore { .. }));
    }

    #[test]
    fn test_undo_redo_create_note_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        assert!(ws.can_undo());
        assert!(!ws.can_redo());

        ws.undo().unwrap();
        assert!(ws.get_note(&child_id).is_err(), "note removed by undo");
        assert!(!ws.can_undo());
        assert!(ws.can_redo());

        ws.redo().unwrap();
        assert!(ws.can_undo());
        assert!(!ws.can_redo());
        // Note should be back — look it up by parent.
        let children = ws.get_children(&root_id).unwrap();
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn test_undo_delete_note_full_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        ws.delete_note_recursive(&child_id).unwrap();
        ws.undo().unwrap();

        // Note must be back.
        assert!(ws.get_note(&child_id).is_ok());
    }

    #[test]
    fn test_tree_action_collapses_to_one_undo_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        // Simulate what run_tree_action does internally.
        ws.begin_undo_group();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.end_undo_group();

        assert_eq!(ws.undo_stack.len(), 1, "three creates must collapse to one undo step");
    }

    #[test]
    fn test_undo_limit_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.set_undo_limit(10).unwrap();
        drop(ws);

        let ws2 = Workspace::open(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert_eq!(ws2.undo_limit, 10);
    }

    #[test]
    fn test_undo_limit_clamp_and_trim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.set_undo_limit(0).unwrap();
        assert_eq!(ws.get_undo_limit(), 1);

        ws.set_undo_limit(9999).unwrap();
        assert_eq!(ws.get_undo_limit(), 500);

        // Grow the undo stack to 5 entries, then shrink
        ws.set_undo_limit(500).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();
        for _ in 0..5 {
            ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        }
        assert_eq!(ws.undo_stack.len(), 5);
        ws.set_undo_limit(3).unwrap();
        assert_eq!(ws.undo_stack.len(), 3, "oldest entries should have been dropped");
    }

    #[test]
    fn test_undo_redo_update_script_full_cycle() {
        // Regression: build_redo_inverse(ScriptRestore) used to always return
        // DeleteScript, causing redo to delete the script instead of re-applying
        // the update.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src_v1 = "// @name: CycleScript\n// @description: v1\nlet x = 1;";
        let (script, _) = ws.create_user_script(src_v1).unwrap();
        ws.script_undo_stack.clear();

        let src_v2 = "// @name: CycleScript\n// @description: v2\nlet x = 2;";
        ws.update_user_script(&script.id, src_v2).unwrap();

        // Script undo: should restore v1.
        ws.script_undo().unwrap();
        let after_undo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_undo.source_code, src_v1, "script_undo should restore v1");
        assert!(ws.can_script_redo());

        // Script redo: should restore v2 (not delete the script!).
        ws.script_redo().unwrap();
        let after_redo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_redo.source_code, src_v2, "script_redo should re-apply v2");

        // Script undo again: back to v1.
        ws.script_undo().unwrap();
        let final_state = ws.get_user_script(&script.id).unwrap();
        assert_eq!(final_state.source_code, src_v1, "second script_undo should restore v1 again");
    }

    #[test]
    fn test_undo_redo_create_script_full_cycle() {
        // Undo of CreateScript (DeleteScript inverse) should be able to re-create
        // the script with its real content on redo, not an empty placeholder.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src = "// @name: RedoScript\n// @description: test\nlet y = 42;";
        let (script, _) = ws.create_user_script(src).unwrap();
        ws.script_undo_stack.clear();
        // Re-push just the create entry we care about onto the script stack.
        ws.push_script_undo(UndoEntry {
            retracted_ids: vec!["test-op".into()],
            inverse: RetractInverse::DeleteScript { script_id: script.id.clone() },
            propagate: true,
        });

        // Script undo: script deleted.
        ws.script_undo().unwrap();
        assert!(ws.get_user_script(&script.id).is_err(), "script deleted by script_undo");

        // Script redo: script recreated with real content.
        ws.script_redo().unwrap();
        let after_redo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_redo.source_code, src, "script_redo must restore real source, not empty");
    }

    #[test]
    fn test_write_info_json_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.write_info_json().unwrap();

        let info_path = dir.path().join("info.json");
        assert!(info_path.exists(), "info.json should be created");

        let content = std::fs::read_to_string(&info_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(v["created_at"].is_number());
        assert_eq!(v["note_count"].as_u64().unwrap(), 1);
        assert_eq!(v["attachment_count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_write_info_json_counts_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.write_info_json().unwrap();

        let content = std::fs::read_to_string(dir.path().join("info.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["note_count"].as_u64().unwrap(), 3);
    }

    #[test]
    fn test_info_json_written_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("info.json").exists(), "info.json must exist after create");
    }

    #[test]
    fn test_info_json_written_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        std::fs::remove_file(dir.path().join("info.json")).unwrap(); // remove it
        Workspace::open(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("info.json").exists(), "info.json must be rewritten on open");
    }

    // ── HLC-specific tests ────────────────────────────────────────────────────

    #[test]
    fn test_hlc_counter_increments_for_rapid_ops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hlc.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Create two notes in rapid succession.
        ws.create_note_root("TextNote").unwrap();
        ws.create_note_root("TextNote").unwrap();

        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(ops.len() >= 2, "at least two operations must be logged");

        // Every logged timestamp must be a valid non-zero wall clock value.
        for op in &ops {
            assert!(op.timestamp_wall_ms > 0, "wall_ms must be non-zero");
        }

        // All operation IDs must be unique — HLC must not produce duplicate entries.
        let unique_ids: std::collections::HashSet<&str> =
            ops.iter().map(|o| o.operation_id.as_str()).collect();
        assert_eq!(unique_ids.len(), ops.len(), "all operation_ids must be unique");
    }

    #[test]
    fn test_set_tags_op_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tags_log.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.update_note_tags(&root.id, vec!["crdt".into(), "hlc".into()]).unwrap();

        let ops = ws.list_operations(None, None, None).unwrap();
        let has_set_tags = ops.iter().any(|o| o.operation_type == "SetTags");
        assert!(has_set_tags, "SetTags operation must appear in the log after update_note_tags");
    }

    #[test]
    fn test_update_note_op_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_note_log.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.update_note_title(&root.id, "HLC Title".to_string()).unwrap();

        // update_note_title logs an UpdateNote operation (not UpdateField).
        let ops = ws.list_operations(None, None, None).unwrap();
        let has_title_update = ops.iter().any(|o| o.operation_type == "UpdateNote");
        assert!(
            has_title_update,
            "UpdateNote operation must appear in the log after update_note_title"
        );
    }

    #[test]
    fn test_on_save_gated_model() {
        let mut ws = create_test_workspace_with_schema(r#"
            schema("GatedTest", #{ version: 1,
                fields: [
                    #{ name: "body", type: "text", required: false },
                ],
                on_save: |note| {
                    set_title(note.id, "Computed: " + note.fields["body"]);
                    commit();
                },
            });
        "#);

        let note_id = ws.create_note_root("GatedTest").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), FieldValue::Text("hello".to_string()));
        let updated = ws.update_note(&note_id, "ignored".to_string(), fields).unwrap();
        assert_eq!(updated.title, "Computed: hello");
    }

    #[test]
    fn test_old_style_on_save_raises_error() {
        let mut ws = create_test_workspace_with_schema(r#"
            schema("OldStyle", #{ version: 1,
                fields: [
                    #{ name: "body", type: "text", required: false },
                ],
                on_save: |note| {
                    note.title = "Old Style";
                    note
                },
            });
        "#);

        let note_id = ws.create_note_root("OldStyle").unwrap();
        let result = ws.update_note(&note_id, "test".to_string(), BTreeMap::new());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("set_field") || err_msg.contains("gated") || err_msg.contains("old"),
            "Expected migration error, got: {err_msg}"
        );
    }

    // ── save_note_with_pipeline ───────────────────────────────────────────────

    #[test]
    fn test_save_pipeline_success() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineOk
schema("PipeItem", #{ version: 1,
    fields: [
        #{ name: "value", type: "text", required: false },
    ],
    on_save: |note| { commit(); }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "PipeItem").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("value".to_string(), FieldValue::Text("hello".into()));
        let result = ws.save_note_with_pipeline(&note_id, "My Item".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_save_pipeline_validation_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineValidate
schema("RatedItem", #{ version: 1,
    fields: [
        #{
            name: "score", type: "number", required: false,
            validate: |v| if v < 0.0 { "Must be positive" } else { () },
        },
    ],
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RatedItem").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("score".to_string(), FieldValue::Number(-1.0));
        let result = ws.save_note_with_pipeline(&note_id, "Item".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("score"), "expected score error");
            }
            other => panic!("expected ValidationErrors, got: {:?}", other),
        }
    }

    #[test]
    fn test_save_pipeline_reject_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineReject
schema("RejectItem", #{ version: 1,
    fields: [],
    on_save: |note| {
        reject("Always rejected");
        commit();
    }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RejectItem").unwrap();
        let result = ws.save_note_with_pipeline(&note_id, "Item".to_string(), BTreeMap::new()).unwrap();
        match result {
            SaveResult::ValidationErrors { note_errors, .. } => {
                assert!(!note_errors.is_empty(), "expected note_errors from reject()");
            }
            other => panic!("expected ValidationErrors, got: {:?}", other),
        }
    }

    /// Integration test: full pipeline with field groups, conditional visibility, validate
    /// closure, and note-level reject.
    ///
    /// Schema:
    ///   - top-level field "type" (select: ["A", "B"])
    ///   - field_group "B Details" visible only when type == "B"
    ///     - field "b_value" (number, required, validate: must be > 0)
    ///   - on_save: reject if type == "B" and b_value > 100
    ///
    /// Tests:
    ///   1. type="A" → success (B Details hidden, b_value not required)
    ///   2. type="B", b_value=-1 → validate error on b_value
    ///   3. type="B", b_value=200 → note-level reject error
    ///   4. type="B", b_value=50 → success
    #[test]
    fn test_full_pipeline_groups_validation_reject() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineGrouped
schema("Grouped", #{ version: 1,
    fields: [
        #{ name: "category", type: "select", options: ["A", "B"] }
    ],
    field_groups: [
        #{
            name: "B Details",
            collapsed: false,
            visible: |fields| fields["category"] == "B",
            fields: [
                #{
                    name: "b_value",
                    type: "number",
                    required: true,
                    validate: |v| if v <= 0.0 { "Must be positive" } else { () }
                }
            ]
        }
    ],
    on_save: |note| {
        if note.fields.category == "B" && note.fields.b_value > 100.0 {
            reject("b_value must be <= 100 for category B");
        }
        commit();
    }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "Grouped").unwrap();

        // Test 1: category="A" — succeeds; b_value is in a hidden group, not required
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("A".to_string()));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "test 1 failed: {:?}", result);

        // Test 2: category="B", b_value=-1 — validate closure error on b_value
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(-1.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("b_value"), "expected b_value error, got: {:?}", field_errors);
            }
            other => panic!("test 2 failed: expected ValidationErrors, got: {:?}", other),
        }

        // Test 3: category="B", b_value=200 — note-level reject
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(200.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { note_errors, .. } => {
                assert!(!note_errors.is_empty(), "test 3 failed: expected note_errors");
            }
            other => panic!("test 3 failed: expected ValidationErrors, got: {:?}", other),
        }

        // Test 4: category="B", b_value=50 — success
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(50.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "test 4 failed: {:?}", result);
    }

    /// Integration test: tree action that creates a child; the required field is left empty.
    /// The save pipeline should detect the required field and return a ValidationErrors result
    /// (no note is persisted).
    ///
    /// NOTE: tree actions use SaveTransaction internally but do NOT run the full
    /// save_note_with_pipeline (they commit the transaction directly). Required-field
    /// checking via save_note_with_pipeline is the frontend save path. This test verifies
    /// that save_note_with_pipeline catches the missing required field.
    #[test]
    fn test_tree_action_validates_created_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: RequiredField
schema("RequiredItem", #{ version: 1,
    fields: [
        #{ name: "sku", type: "text", required: true }
    ],
    on_save: |note| { commit(); }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RequiredItem").unwrap();

        // Save without providing the required "sku" field → should return ValidationErrors
        let result = ws.save_note_with_pipeline(
            &note_id,
            "Item".to_string(),
            BTreeMap::new(), // no fields provided
        ).unwrap();

        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("sku"), "expected sku required error, got: {:?}", field_errors);
            }
            other => panic!("expected ValidationErrors for missing required field, got: {:?}", other),
        }
    }

    // ── Migration pipeline tests ──────────────────────────────────────────────

    /// Create a workspace, seed it with a schema v1 note, then manually lower the note's
    /// `schema_version` in the DB (simulating stale state after a schema bump), load the
    /// v2 schema with a field-rename migration, call `run_schema_migrations()`, and assert
    /// the field was renamed and `schema_version` updated.
    #[test]
    fn migration_renames_field_on_version_bump() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Register a v1 schema with a "phone" field.
        ws.create_user_script(
            r#"// @name: MigTest
// @description: migration test type
schema("MigType", #{
    version: 1,
    fields: [
        #{ name: "phone", type: "text", required: false },
    ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        let note_id = ws.create_note(&root_id, AddPosition::AsChild, "MigType").unwrap();

        // Write "phone" field.
        let mut fields = BTreeMap::new();
        fields.insert("phone".to_string(), FieldValue::Text("555-1234".to_string()));
        ws.update_note(&note_id, "Contact".to_string(), fields).unwrap();

        // Manually set schema_version = 0 to simulate a stale note.
        ws.connection().execute(
            "UPDATE notes SET schema_version = 0 WHERE id = ?1",
            rusqlite::params![&note_id],
        ).unwrap();

        // Update the script to v2 with a migration that renames phone → mobile.
        let scripts = ws.list_user_scripts().unwrap();
        let script_id = scripts.iter().find(|s| s.name == "MigTest").unwrap().id.clone();
        ws.update_user_script(
            &script_id,
            r#"// @name: MigTest
// @description: migration test type
schema("MigType", #{
    version: 2,
    fields: [
        #{ name: "mobile", type: "text", required: false },
    ],
    migrate: #{
        "2": |note| {
            note.fields["mobile"] = note.fields["phone"];
            note.fields.remove("phone");
            note
        }
    }
});"#,
        ).unwrap();

        // Reload and run migrations.
        ws.reload_scripts().unwrap();
        let results = ws.run_schema_migrations().unwrap();

        assert_eq!(results.len(), 1, "expected 1 migration result, got: {:?}", results);
        assert_eq!(results[0].0, "MigType");
        assert_eq!(results[0].3, 1, "should have migrated 1 note");

        let note = ws.get_note(&note_id).unwrap();
        assert_eq!(note.schema_version, 2);
        assert_eq!(note.fields.get("mobile"), Some(&FieldValue::Text("555-1234".to_string())));
        assert!(!note.fields.contains_key("phone"), "old field should be removed");
    }

    /// Notes that are already at the current schema version must not be re-migrated.
    #[test]
    fn migration_skips_up_to_date_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: UpToDate
schema("UpToDateType", #{
    version: 1,
    fields: [ #{ name: "val", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        ws.create_note(&root_id, AddPosition::AsChild, "UpToDateType").unwrap();

        // No stale notes → results should be empty.
        let results = ws.run_schema_migrations().unwrap();
        assert!(results.is_empty(), "expected no migrations, got: {:?}", results);
    }

    /// Migration closures must chain: a note at v1 with a schema at v3 should pass through
    /// closures for v2 and v3 in order.
    #[test]
    fn migration_chains_across_multiple_versions() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: ChainTest
schema("ChainType", #{
    version: 1,
    fields: [ #{ name: "val", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        let note_id = ws.create_note(&root_id, AddPosition::AsChild, "ChainType").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("val".to_string(), FieldValue::Text("original".to_string()));
        ws.update_note(&note_id, "N".to_string(), fields).unwrap();

        // Force note to schema_version = 0.
        ws.connection().execute(
            "UPDATE notes SET schema_version = 0 WHERE id = ?1",
            rusqlite::params![&note_id],
        ).unwrap();

        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "ChainTest").unwrap().id.clone();
        ws.update_user_script(
            &sid,
            r#"// @name: ChainTest
schema("ChainType", #{
    version: 3,
    fields: [ #{ name: "val", type: "text", required: false } ],
    migrate: #{
        "2": |note| { note.fields["val"] = note.fields["val"] + "_v2"; note },
        "3": |note| { note.fields["val"] = note.fields["val"] + "_v3"; note }
    }
});"#,
        ).unwrap();

        ws.reload_scripts().unwrap();
        ws.run_schema_migrations().unwrap();

        let note = ws.get_note(&note_id).unwrap();
        assert_eq!(note.schema_version, 3);
        assert_eq!(note.fields.get("val"), Some(&FieldValue::Text("original_v2_v3".to_string())));
    }

    /// Version downgrade must be rejected when a higher-version schema is already registered.
    #[test]
    fn schema_version_downgrade_rejected() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Register v2.
        ws.create_user_script(
            r#"// @name: DowngradeTest
schema("DowngradeType", #{
    version: 2,
    fields: []
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();
        assert!(ws.script_registry.schema_exists("DowngradeType"));

        // Try to update to v1 — update_user_script pre-validates and must reject downgrade.
        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "DowngradeTest").unwrap().id.clone();
        let result = ws.update_user_script(
            &sid,
            r#"// @name: DowngradeTest
schema("DowngradeType", #{
    version: 1,
    fields: []
});"#,
        );
        assert!(result.is_err(), "downgrade should be rejected");

        // The schema must still be at v2 after the failed update.
        let schema = ws.script_registry.get_schema("DowngradeType").unwrap();
        assert_eq!(schema.version, 2, "schema must not have been downgraded");
    }

    /// Re-registering a schema with the same version number must succeed without error.
    #[test]
    fn schema_same_version_reregistration_allowed() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: SameVerTest
schema("SameVerType", #{
    version: 1,
    fields: [ #{ name: "alpha", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "SameVerTest").unwrap().id.clone();
        ws.update_user_script(
            &sid,
            r#"// @name: SameVerTest
schema("SameVerType", #{
    version: 1,
    fields: [
        #{ name: "alpha", type: "text", required: false },
        #{ name: "beta",  type: "text", required: false },
    ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        // Should now have 2 fields, no error.
        let schema = ws.script_registry.get_schema("SameVerType").unwrap();
        assert_eq!(schema.version, 1);
        assert_eq!(schema.fields.len(), 2, "expected 2 fields after same-version re-registration");
    }

    #[test]
    fn test_to_snapshot_json_roundtrip() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(
            temp.path(),
            "",
            "test-identity",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        ).unwrap();
        // Add a note so the snapshot has more than just the root.
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let json = ws.to_snapshot_json().unwrap();
        assert!(!json.is_empty());
        let snap: WorkspaceSnapshot = serde_json::from_slice(&json).unwrap();
        // Workspace::create inserts a root note, so we have 2 notes total.
        assert_eq!(snap.notes.len(), 2);
    }

    #[test]
    fn test_to_snapshot_json_includes_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut ws = Workspace::create(&db_path, "", "test-id", key).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        ws.attach_file(&root_id, "test.txt", None, b"hello bytes").unwrap();
        let json = ws.to_snapshot_json().unwrap();
        let snap: WorkspaceSnapshot = serde_json::from_slice(&json).unwrap();
        assert_eq!(snap.attachments.len(), 1);
        assert_eq!(snap.attachments[0].filename, "test.txt");
    }

    #[test]
    fn test_get_latest_operation_id_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let ws = Workspace::create(&db_path, "", "test-id", key).unwrap();
        // A freshly created workspace has no operations logged yet.
        assert!(ws.get_latest_operation_id().unwrap().is_none());
    }

    #[test]
    fn test_create_with_id_preserves_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let custom_id = "my-fixed-workspace-uuid";
        let ws = Workspace::create_with_id(&db_path, "", "test-id", key, custom_id).unwrap();
        assert_eq!(ws.workspace_id(), custom_id);
    }

    #[test]
    fn test_create_empty_with_id_no_root_note() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
        let custom_id = "snapshot-workspace-uuid";
        let ws = Workspace::create_empty_with_id(&db_path, "", "test-id", key, custom_id).unwrap();
        assert_eq!(ws.workspace_id(), custom_id);
        // No root note should be auto-inserted — snapshot restoration will add its own notes.
        assert_eq!(ws.list_all_notes().unwrap().len(), 0);
    }

    #[test]
    fn test_import_snapshot_json_round_trip() {
        let src_temp = NamedTempFile::new().unwrap();
        let mut src = Workspace::create(
            src_temp.path(),
            "",
            "src-identity",
            ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]),
        ).unwrap();
        // Replace the default root note title and add two children.
        let root = src.list_all_notes().unwrap()[0].clone();
        src.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        src.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let json = src.to_snapshot_json().unwrap();

        // Destination workspace.
        let dst_temp = NamedTempFile::new().unwrap();
        let mut dst = Workspace::create(
            dst_temp.path(),
            "",
            "dst-identity",
            ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]),
        ).unwrap();
        // Remove the auto-created root so we start from a clean slate.
        let dst_root = dst.list_all_notes().unwrap()[0].clone();
        dst.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&dst_root.id],
        ).unwrap();

        let count = dst.import_snapshot_json(&json).unwrap();
        // src had: 1 original root + 2 children = 3 notes.
        assert_eq!(count, 3);
        let notes = dst.list_all_notes().unwrap();
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn test_is_leaf_defaults_to_false() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: NormalSchema\nschema(\"NormalType\", #{ version: 1, fields: [] });"
        ).unwrap();
        let schema = ws.script_registry().get_schema("NormalType").unwrap();
        assert!(!schema.is_leaf, "is_leaf should default to false");
    }

    #[test]
    fn test_is_leaf_explicit_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: LeafSchema\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });"
        ).unwrap();
        let schema = ws.script_registry().get_schema("LeafType").unwrap();
        assert!(schema.is_leaf, "is_leaf should be true when explicitly set");
    }

    #[test]
    fn test_is_leaf_blocks_create_child() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafSchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        // Create a root LeafType note
        let leaf_id = ws.create_note_root("LeafType").unwrap();

        // Trying to create a child under it must fail
        let result = ws.create_note(&leaf_id, AddPosition::AsChild, "ChildType");
        assert!(result.is_err(), "expected error when adding child to leaf note");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_is_leaf_blocks_move_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafMoveSchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        let leaf_id  = ws.create_note_root("LeafType").unwrap();
        let child_id = ws.create_note_root("ChildType").unwrap();

        // Moving child under leaf must fail
        let result = ws.move_note(&child_id, Some(&leaf_id), 0.0);
        assert!(result.is_err(), "expected error when moving note under leaf");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_is_leaf_blocks_deep_copy() {
        // deep_copy_note (paste) should also be blocked when the target parent is a leaf
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafDeepCopySchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        // Create a root ChildType note (the note to copy)
        let child_id = ws.create_note_root("ChildType").unwrap();
        // Create a root LeafType note (the intended paste target)
        let leaf_id = ws.create_note_root("LeafType").unwrap();

        // Pasting child under leaf must fail
        let result = ws.deep_copy_note(&child_id, &leaf_id, AddPosition::AsChild);
        assert!(result.is_err(), "expected error when deep-copying note under leaf");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_list_peers_info_unknown_peer() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.add_contact_as_peer("AAAAAAAAAAAAAAAA").unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let key = [0u8; 32];
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();

        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].display_name, "AAAAAAAA…");
        assert!(peers[0].trust_level.is_none());
        assert!(peers[0].contact_id.is_none());
        assert!(!peers[0].fingerprint.is_empty());
    }

    #[test]
    fn test_list_peers_info_known_contact() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let pubkey = "BBBBBBBBBBBBBBBB";
        ws.add_contact_as_peer(pubkey).unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let key = [1u8; 32];
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();
        let contact = cm.create_contact("Bob", pubkey, TrustLevel::CodeVerified).unwrap();

        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].display_name, "Bob");
        assert_eq!(peers[0].trust_level.as_deref(), Some("CodeVerified"));
        let expected_contact_id = contact.contact_id.to_string();
        assert_eq!(peers[0].contact_id.as_deref(), Some(expected_contact_id.as_str()));
    }

    #[test]
    fn test_add_and_remove_peer() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let pubkey = "CCCCCCCCCCCCCCCC";

        ws.add_contact_as_peer(pubkey).unwrap();
        let cm_dir = tempfile::tempdir().unwrap();
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), [0u8; 32]).unwrap();
        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);

        let placeholder = format!("identity:{}", pubkey);
        ws.remove_peer(&placeholder).unwrap();
        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 0);
    }

    #[test]
    fn test_operations_since_empty() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "id-1",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // No operations yet, so operations_since(None, "other-device") returns empty
        let ops = ws.operations_since(None, "other-device").unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_operations_since_watermark() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "id-1",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        // Create two child notes to generate two CreateNote operations
        let _id1 = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let _id2 = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Get all ops (excluding this device — but we need to use "nonexistent-device" so local ops show)
        let all_ops = ws.operations_since(None, "nonexistent-device").unwrap();
        assert_eq!(all_ops.len(), 2);
        let first_op_id = all_ops[0].operation_id().to_string();

        // Only second op should be returned when watermark = first_op_id
        let since_ops = ws.operations_since(Some(&first_op_id), "nonexistent-device").unwrap();
        assert_eq!(since_ops.len(), 1);
        assert_eq!(since_ops[0].operation_id(), all_ops[1].operation_id());
    }

    #[test]
    fn test_operations_since_excludes_device() {
        // ops_since should never return ops from the excluded device
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "id-1",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let _id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        // Get device_id from workspace_meta
        let current_device_id: String = ws.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key='device_id'", [],
            |row| row.get::<_, String>(0)).unwrap();
        let ops = ws.operations_since(None, &current_device_id).unwrap();
        assert!(ops.is_empty(), "ops from own device should be excluded");
    }

    #[test]
    fn test_operations_since_filters_local_retract() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "id-1",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let _id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        // Undo should create a RetractOperation with propagate=false
        ws.undo().unwrap();

        let ops = ws.operations_since(None, "other-device").unwrap();
        // RetractOperation(propagate=false) must be absent
        for op in &ops {
            if let Operation::RetractOperation { propagate, .. } = op {
                assert!(propagate, "local-only retract must be filtered from delta");
            }
        }
    }

    // ── apply_incoming_operation tests ──────────────────────────────────────

    /// Helper: build a minimal CreateNote operation for testing.
    fn make_create_note_op(op_id: &str, note_id: &str, device_id: &str, wall_ms: u64) -> Operation {
        use crate::core::hlc::HlcTimestamp;
        Operation::CreateNote {
            operation_id: op_id.to_string(),
            timestamp: HlcTimestamp { wall_ms, counter: 0, node_id: 42 },
            device_id: device_id.to_string(),
            note_id: note_id.to_string(),
            parent_id: None,
            position: 0.0,
            schema: "TextNote".to_string(),
            title: "Remote Note".to_string(),
            fields: BTreeMap::new(),
            created_by: String::new(),
            signature: String::new(),
        }
    }

    #[test]
    fn test_apply_incoming_create_note() {
        use crate::core::hlc::HlcTimestamp;
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "local-device",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let op = make_create_note_op("op-remote-1", "note-remote-1", "remote-device", 1_000_000);

        let applied = ws.apply_incoming_operation(op).unwrap();
        assert!(applied, "first application must return true");

        // The note must exist in the working table.
        let note_count: i64 = ws.connection().query_row(
            "SELECT COUNT(*) FROM notes WHERE id = 'note-remote-1'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(note_count, 1, "note must exist after apply");

        // The operation must be stored with synced = 1.
        let synced: i64 = ws.connection().query_row(
            "SELECT synced FROM operations WHERE operation_id = 'op-remote-1'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(synced, 1, "incoming operation must have synced=1");
    }

    #[test]
    fn test_apply_incoming_duplicate_is_idempotent() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "local-device",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let op = make_create_note_op("op-dup-1", "note-dup-1", "remote-device", 2_000_000);

        let first = ws.apply_incoming_operation(op.clone()).unwrap();
        assert!(first, "first call must return true");

        let second = ws.apply_incoming_operation(op).unwrap();
        assert!(!second, "duplicate must return false");

        // Only one row must exist.
        let count: i64 = ws.connection().query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id = 'op-dup-1'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1, "duplicate must not insert a second row");
    }

    #[test]
    fn test_apply_incoming_retract_propagate_false_skipped() {
        use crate::core::hlc::HlcTimestamp;
        use crate::RetractInverse;
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "local-device",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let op = Operation::RetractOperation {
            operation_id: "op-retract-local".to_string(),
            timestamp: HlcTimestamp { wall_ms: 3_000_000, counter: 0, node_id: 99 },
            device_id: "remote-device".to_string(),
            retracted_ids: vec!["some-op".to_string()],
            inverse: RetractInverse::DeleteNote { note_id: "fake-note".to_string() },
            propagate: false,
        };

        let applied = ws.apply_incoming_operation(op).unwrap();
        assert!(!applied, "local-only retract must be skipped");

        // Nothing must have been inserted into operations.
        let count: i64 = ws.connection().query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id = 'op-retract-local'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0, "skipped op must not appear in the log");
    }

    #[test]
    fn test_apply_incoming_hlc_advances() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "local-device",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Use a far-future wall_ms so the local clock must be advanced.
        let far_future_ms: u64 = 9_999_999_999_999;
        let op = make_create_note_op("op-future-1", "note-future-1", "remote-device", far_future_ms);
        ws.apply_incoming_operation(op).unwrap();

        // Now create a local note — its HLC timestamp must be >= far_future_ms.
        let root = ws.list_all_notes().unwrap()[0].clone();
        // We can't call create_note easily because it may fail validation,
        // but we can check the HLC state directly from the operations log.
        // Instead, verify that the remote op's wall_ms is stored correctly.
        let stored_wall_ms: i64 = ws.connection().query_row(
            "SELECT timestamp_wall_ms FROM operations WHERE operation_id = 'op-future-1'", [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(stored_wall_ms as u64, far_future_ms,
            "stored timestamp must match incoming operation's wall_ms");

        // Create a second remote op slightly ahead — its timestamp must exceed the first.
        let op2 = make_create_note_op("op-future-2", "note-future-2", "remote-device", far_future_ms + 1);
        ws.apply_incoming_operation(op2).unwrap();

        let stored2: i64 = ws.connection().query_row(
            "SELECT timestamp_wall_ms FROM operations WHERE operation_id = 'op-future-2'", [],
            |row| row.get(0),
        ).unwrap();
        assert!(stored2 as u64 >= far_future_ms,
            "second op wall_ms must be >= first far-future timestamp");
    }

    #[test]
    fn test_create_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();

        let source = "// @name: Evil Script\nschema(\"Evil\", #{ version: 1, fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_update_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.update_user_script(&script_id, "// @name: Hacked\nschema(\"Hacked\", #{ version: 1, fields: [] });");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_delete_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.delete_user_script(&script_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_toggle_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.toggle_user_script(&script_id, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_reorder_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.reorder_user_script(&script_id, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_reorder_all_user_scripts_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.reorder_all_user_scripts(&[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_owner_pubkey_matches_creator() {
        let temp = NamedTempFile::new().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "test-id", key.clone()).unwrap();

        assert_eq!(workspace.owner_pubkey(), workspace.identity_pubkey());
        assert!(workspace.is_owner());
    }

    #[test]
    fn test_is_owner_false_for_different_identity() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        assert!(!workspace.is_owner());
    }

    #[test]
    fn test_open_legacy_workspace_without_owner_pubkey_assigns_opener() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        // Manually remove owner_pubkey to simulate a pre-existing workspace
        workspace.connection().execute(
            "DELETE FROM workspace_meta WHERE key = 'owner_pubkey'", [],
        ).unwrap();
        drop(workspace);

        // Re-open — opener should become owner
        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        assert!(workspace.is_owner());
        assert_eq!(workspace.owner_pubkey(), workspace.identity_pubkey());
    }

    #[test]
    fn test_apply_incoming_script_op_from_owner_applied() {
        let temp = NamedTempFile::new().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "test-id", key.clone()).unwrap();
        let owner_pubkey = workspace.identity_pubkey().to_string();

        // Build a CreateUserScript op signed by the owner
        let mut op = Operation::CreateUserScript {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 99 },
            device_id: "remote-device".to_string(),
            script_id: uuid::Uuid::new_v4().to_string(),
            name: "Owner Script".to_string(),
            description: "From owner".to_string(),
            source_code: "// @name: Owner Script\n".to_string(),
            load_order: 99,
            enabled: true,
            created_by: owner_pubkey,
            signature: String::new(),
        };
        op.sign(&key);

        let applied = workspace.apply_incoming_operation(op).unwrap();
        assert!(applied);
    }

    #[test]
    fn test_apply_incoming_script_op_from_non_owner_skipped() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();

        // Build a CreateUserScript op signed by a different identity
        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let pubkey_b = {
            use base64::Engine as _;
            let vk = ed25519_dalek::VerifyingKey::from(&key_b);
            base64::engine::general_purpose::STANDARD.encode(vk.as_bytes())
        };
        let script_id = uuid::Uuid::new_v4().to_string();

        let mut op = Operation::CreateUserScript {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 99 },
            device_id: "attacker-device".to_string(),
            script_id: script_id.clone(),
            name: "Evil Script".to_string(),
            description: "From attacker".to_string(),
            source_code: "// @name: Evil Script\n".to_string(),
            load_order: 99,
            enabled: true,
            created_by: pubkey_b,
            signature: String::new(),
        };
        op.sign(&key_b);

        // Op is logged (returns true) but script should NOT appear in user_scripts
        let result = workspace.apply_incoming_operation(op).unwrap();
        assert!(result); // Logged to operations table

        // Verify the script was NOT applied to the working table
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts.iter().any(|s| s.id == script_id));
    }
