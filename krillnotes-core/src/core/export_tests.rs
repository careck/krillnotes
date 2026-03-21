    use super::*;

    #[test]
    fn test_export_notes_serialization() {
        let export = ExportNotes {
            version: 1,
            app_version: "0.1.0".to_string(),
            notes: vec![],
        };
        let json = serde_json::to_string(&export).unwrap();
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"appVersion\":\"0.1.0\""));
        assert!(json.contains("\"notes\":[]"));

        let parsed: ExportNotes = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.app_version, "0.1.0");
    }

    #[test]
    fn test_script_manifest_serialization() {
        let manifest = ScriptManifest {
            scripts: vec![ScriptManifestEntry {
                filename: "contacts.rhai".to_string(),
                load_order: 0,
                enabled: true,
                category: Some("schema".to_string()),
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"loadOrder\":0"));
        assert!(json.contains("\"filename\":\"contacts.rhai\""));
    }

    #[test]
    fn test_slugify_script_name() {
        assert_eq!(slugify_script_name("Contacts"), "contacts");
        assert_eq!(slugify_script_name("My Tasks"), "my-tasks");
        assert_eq!(slugify_script_name("Hello World!"), "hello-world");
        assert_eq!(slugify_script_name("  Spaced  Out  "), "spaced-out");
        assert_eq!(slugify_script_name(""), "script");
        assert_eq!(slugify_script_name("---"), "script");
    }

    use crate::{AddPosition, Workspace};
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    #[test]
    fn test_export_workspace_creates_valid_zip() {
        // Create a workspace with a note and a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        // Add a user script (unique name to avoid collision with starters)
        let script_source =
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ version: 1, fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        // Read back the zip and verify structure
        let reader = Cursor::new(&buf);
        let mut archive = zip::ZipArchive::new(reader).unwrap();

        // Must contain notes.json
        let notes_file = archive.by_name("notes.json").unwrap();
        let notes_data: ExportNotes = serde_json::from_reader(notes_file).unwrap();
        assert_eq!(notes_data.version, 1);
        assert!(!notes_data.app_version.is_empty());
        assert!(!notes_data.notes.is_empty()); // at least the root note

        // Must contain scripts/scripts.json
        let manifest_file = archive.by_name("scripts/scripts.json").unwrap();
        let manifest: ScriptManifest = serde_json::from_reader(manifest_file).unwrap();
        // Starter scripts + the user-created Widget script
        assert!(manifest.scripts.len() >= 2, "Should have starter scripts plus user script");
        let widget_entry = manifest.scripts.iter().find(|s| s.filename == "custom-widget.rhai");
        assert!(widget_entry.is_some(), "Should contain custom-widget.rhai in manifest");

        // Must contain the .rhai file
        let mut rhai_file = archive.by_name("scripts/custom-widget.rhai").unwrap();
        let mut source = String::new();
        std::io::Read::read_to_string(&mut rhai_file, &mut source).unwrap();
        assert!(source.contains("@name: Custom Widget"));
    }

    #[test]
    fn test_peek_import_reads_metadata() {
        // Create a workspace with a script
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let script_source =
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ version: 1, fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export to a buffer
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        // Peek at the export
        let result = peek_import(Cursor::new(&buf), None).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 1); // root note
        // Starter scripts + the user-created Widget script
        assert!(result.script_count >= 2, "Should have starters + user script, got {}", result.script_count);
    }

    /// NOTE: This test works because export currently reads through the Workspace API
    /// and import writes through bulk SQL inserts. If export ever reads directly from
    /// SQLite (e.g. for streaming large workspaces), this in-memory round-trip approach
    /// will need to be revised to use actual database files for both sides.
    #[test]
    fn test_round_trip_export_import() {
        // Create a workspace with nested notes and a script
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "Root Note".to_string()).unwrap();

        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&child_id, "Child Note".to_string()).unwrap();

        let grandchild_id = ws
            .create_note(&child_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&grandchild_id, "Grandchild Note".to_string())
            .unwrap();

        let script_source =
            "// @name: Custom Widget\n// @description: Widget cards\nschema(\"Widget\", #{ version: 1, fields: [] });";
        ws.create_user_script(script_source).unwrap();

        // Export
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        // Import into a new workspace file
        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        assert_eq!(result.app_version, APP_VERSION);
        assert_eq!(result.note_count, 3);
        // Starter scripts + the user-created Widget script
        assert!(result.script_count >= 2, "Should have starters + user script, got {}", result.script_count);

        // Open the imported workspace and verify contents
        let imported_ws = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let notes = imported_ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 3);

        // Verify note titles
        let titles: Vec<&str> = notes.iter().map(|n| n.title.as_str()).collect();
        assert!(titles.contains(&"Root Note"));
        assert!(titles.contains(&"Child Note"));
        assert!(titles.contains(&"Grandchild Note"));

        // Verify parent-child relationships are preserved
        let root_note = notes.iter().find(|n| n.title == "Root Note").unwrap();
        let child_note = notes.iter().find(|n| n.title == "Child Note").unwrap();
        let grandchild_note = notes.iter().find(|n| n.title == "Grandchild Note").unwrap();

        assert_eq!(root_note.parent_id, None);
        assert_eq!(child_note.parent_id, Some(root_note.id.clone()));
        assert_eq!(grandchild_note.parent_id, Some(child_note.id.clone()));

        // Verify scripts
        let scripts = imported_ws.list_user_scripts().unwrap();
        assert!(scripts.len() >= 2, "Should have starters + user script");
        let widget = scripts.iter().find(|s| s.name == "Custom Widget").unwrap();
        assert_eq!(widget.description, "Widget cards");
        assert!(widget.source_code.contains("@name: Custom Widget"));
    }

    #[test]
    fn test_round_trip_preserves_script_category() {
        // Regression test: imported scripts must retain their original category.
        // Previously all scripts were hardcoded to "presentation" on import, which
        // caused schema() scripts to fail with "can only be called from schema-category scripts".
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let schema_src = "// @name: My Schema\n// @description: A schema script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let lib_src = "// @name: My Library\n// @description: A presentation script\nfn my_helper() { \"hello\" }";

        ws.create_user_script_with_category(schema_src, "schema").unwrap();
        ws.create_user_script_with_category(lib_src, "presentation").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let temp_dst = NamedTempFile::new().unwrap();
        import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let imported_ws = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let scripts = imported_ws.list_user_scripts().unwrap();

        let schema_script = scripts.iter().find(|s| s.name == "My Schema").unwrap();
        let lib_script = scripts.iter().find(|s| s.name == "My Library").unwrap();

        assert_eq!(schema_script.category, "schema", "schema script category must survive export/import round-trip");
        assert_eq!(lib_script.category, "presentation", "presentation script category must survive export/import round-trip");
    }

    #[test]
    fn test_export_includes_workspace_json() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let mut archive = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();
        let ws_file = archive.by_name("workspace.json").unwrap();
        let ws_meta: WorkspaceMetadata = serde_json::from_reader(ws_file).unwrap();
        assert_eq!(ws_meta.version, 1);
    }

    #[test]
    fn test_round_trip_preserves_tags() {
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["rust".into()]).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let temp_dst = NamedTempFile::new().unwrap();
        import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let imported = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let tags = imported.get_all_tags().unwrap();
        assert_eq!(tags, vec!["rust"]);

        // Tags are also on the note itself
        let notes = imported.list_all_notes().unwrap();
        let root_imported = notes.iter().find(|n| n.parent_id.is_none()).unwrap();
        assert_eq!(root_imported.tags, vec!["rust"]);
    }

    #[test]
    fn test_import_invalid_zip() {
        let garbage = b"this is not a zip file at all";
        let result = import_workspace(Cursor::new(garbage), Path::new("/tmp/invalid.db"), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]));
        assert!(result.is_err());
    }

    #[test]
    fn test_import_missing_notes_json() {
        // Create a valid zip that has no notes.json
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("readme.txt", options).unwrap();
            zip.write_all(b"no notes here").unwrap();
            zip.finish().unwrap();
        }

        let result = import_workspace(Cursor::new(&buf), Path::new("/tmp/missing_notes.db"), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]));
        assert!(matches!(result, Err(ExportError::InvalidFormat(_))));
    }

    #[test]
    fn test_export_with_password_creates_encrypted_zip() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("hunter2")).unwrap();

        // notes.json should be marked as encrypted.
        // Use by_index_raw to read metadata without decrypting.
        let reader = Cursor::new(&buf);
        let mut archive = ZipArchive::new(reader).unwrap();
        let index = archive.index_for_name("notes.json").unwrap();
        let notes_file = archive.by_index_raw(index).unwrap();
        assert!(notes_file.encrypted(), "notes.json must be encrypted when password is provided");
    }

    #[test]
    fn test_export_without_password_creates_plain_zip() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let reader = Cursor::new(&buf);
        let mut archive = ZipArchive::new(reader).unwrap();
        let notes_file = archive.by_name("notes.json").unwrap();
        assert!(!notes_file.encrypted(), "notes.json must be plain when no password given");
    }

    #[test]
    fn test_read_entry_wrong_password_returns_invalid_password() {
        // Export with a password
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("correct")).unwrap();

        // Try to read an entry with the wrong password
        let mut archive = ZipArchive::new(Cursor::new(&buf)).unwrap();
        let err = read_entry(&mut archive, "notes.json", Some("wrong")).unwrap_err();
        assert!(matches!(err, ExportError::InvalidPassword), "got: {err:?}");
    }
    #[test]
    fn test_peek_import_returns_encrypted_archive_error_when_no_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let err = peek_import(Cursor::new(&buf), None).unwrap_err();
        assert!(matches!(err, ExportError::EncryptedArchive), "got: {err:?}");
    }

    #[test]
    fn test_peek_import_with_correct_password_succeeds() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let result = peek_import(Cursor::new(&buf), Some("s3cr3t")).unwrap();
        assert_eq!(result.app_version, APP_VERSION);
        assert!(result.note_count >= 1);
    }

    #[test]
    fn test_peek_import_with_wrong_password_returns_invalid_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("s3cr3t")).unwrap();

        let err = peek_import(Cursor::new(&buf), Some("wrong-password")).unwrap_err();
        assert!(matches!(err, ExportError::InvalidPassword), "got: {err:?}");
    }

    #[test]
    fn test_encrypted_round_trip_import() {
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "Encrypted Root".to_string()).unwrap();

        // Export with password
        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), Some("mypass")).unwrap();

        // Import with correct password → should succeed
        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), Some("mypass"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert_eq!(result.note_count, 1);

        // Verify imported note title
        let imported_ws = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let notes = imported_ws.list_all_notes().unwrap();
        assert!(notes.iter().any(|n| n.title == "Encrypted Root"));
    }

    /// Older archives (pre-tags feature) serialize notes without a `"tags"` key.
    /// Import must succeed and produce notes with empty tag lists.
    #[test]
    fn test_import_notes_without_tags_field() {
        let notes_json = serde_json::json!({
            "version": 1,
            "appVersion": "0.1.0",
            "notes": [{
                "id": "root-id",
                "title": "Root",
                "nodeType": "TextNote",
                "parentId": null,
                "position": 0,
                "createdAt": 0,
                "modifiedAt": 0,
                "createdBy": 0,
                "modifiedBy": 0,
                "fields": {},
                "isExpanded": true
                // no "tags" key — simulates a pre-tags-feature archive
            }]
        });

        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("notes.json", options).unwrap();
            serde_json::to_writer(&mut zip, &notes_json).unwrap();
            zip.start_file("scripts/scripts.json", options).unwrap();
            zip.write_all(b"{\"scripts\":[]}").unwrap();
            zip.finish().unwrap();
        }

        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert_eq!(result.note_count, 1);

        let imported_ws = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let notes = imported_ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].tags.is_empty(), "imported note from old archive should have no tags");
    }

    /// Older archives (pre-workspace.json) don't include that file.
    /// Import must succeed — the file is never read during import.
    #[test]
    fn test_import_archive_without_workspace_json() {
        let notes_json = serde_json::json!({
            "version": 1,
            "appVersion": "0.1.0",
            "notes": [{
                "id": "root-id",
                "title": "Root",
                "nodeType": "TextNote",
                "parentId": null,
                "position": 0,
                "createdAt": 0,
                "modifiedAt": 0,
                "createdBy": 0,
                "modifiedBy": 0,
                "fields": {},
                "isExpanded": true,
                "tags": []
            }]
        });

        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("notes.json", options).unwrap();
            serde_json::to_writer(&mut zip, &notes_json).unwrap();
            // intentionally no workspace.json and no scripts/scripts.json
            zip.finish().unwrap();
        }

        let temp_dst = NamedTempFile::new().unwrap();
        let result = import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert_eq!(result.note_count, 1);
        assert_eq!(result.script_count, 0);
    }

    #[test]
    fn test_workspace_metadata_roundtrip() {
        let temp_src = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp_src.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();

        let meta = WorkspaceMetadata {
            version: 1,
            author_name: Some("Alice".to_string()),
            author_org: Some("ACME".to_string()),
            homepage_url: Some("https://example.com".to_string()),
            description: Some("A test workspace".to_string()),
            license: Some("MIT".to_string()),
            license_url: Some("https://mit-license.org".to_string()),
            language: Some("en".to_string()),
            tags: vec!["notes".to_string(), "template".to_string()],
        };
        ws.set_workspace_metadata(&meta).unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let temp_dst = NamedTempFile::new().unwrap();
        import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let imported = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let restored = imported.get_workspace_metadata().unwrap();

        assert_eq!(restored.author_name.as_deref(), Some("Alice"));
        assert_eq!(restored.author_org.as_deref(), Some("ACME"));
        assert_eq!(restored.homepage_url.as_deref(), Some("https://example.com"));
        assert_eq!(restored.description.as_deref(), Some("A test workspace"));
        assert_eq!(restored.license.as_deref(), Some("MIT"));
        assert_eq!(restored.license_url.as_deref(), Some("https://mit-license.org"));
        assert_eq!(restored.language.as_deref(), Some("en"));
        assert_eq!(restored.tags, vec!["notes", "template"]);
    }

    #[test]
    fn test_export_includes_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.attach_file(&root_id, "hello.txt", Some("text/plain"), b"hello world").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let mut archive = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();
        assert!(archive.by_name("attachments.json").is_ok(), "Must have attachments.json");
        let found = (0..archive.len()).any(|i| {
            archive.by_index(i).ok()
                .map(|f| f.name().ends_with("hello.txt"))
                .unwrap_or(false)
        });
        assert!(found, "Attachment file must be in the zip");
    }

    #[test]
    fn test_import_restores_attachments() {
        let dir_src = tempfile::tempdir().unwrap();
        let db_src = dir_src.path().join("notes.db");
        let mut ws = Workspace::create(&db_src, "pass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.attach_file(&root_id, "data.txt", None, b"attachment content").unwrap();

        let mut buf = Vec::new();
        export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

        let dir_dst = tempfile::tempdir().unwrap();
        let db_dst = dir_dst.path().join("notes.db");
        import_workspace(Cursor::new(&buf), &db_dst, None, "newpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let ws2 = Workspace::open(&db_dst, "newpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let notes = ws2.list_all_notes().unwrap();
        let root = notes.iter().find(|n| n.parent_id.is_none()).unwrap();
        let attachments = ws2.get_attachments(&root.id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename, "data.txt");

        let recovered = ws2.get_attachment_bytes(&attachments[0].id).unwrap();
        assert_eq!(recovered, b"attachment content" as &[u8]);
    }

    #[test]
    fn test_workspace_metadata_absent_in_old_archive() {
        // Old archive: workspace.json only has version + tags (old format), no metadata fields.
        let notes_json = serde_json::json!({
            "version": 1, "appVersion": "0.1.0",
            "notes": [{ "id": "root", "title": "Root", "nodeType": "TextNote",
                        "parentId": null, "position": 0, "createdAt": 0,
                        "modifiedAt": 0, "createdBy": 0, "modifiedBy": 0,
                        "fields": {}, "isExpanded": true, "tags": [] }]
        });
        let old_ws_json = serde_json::json!({ "version": 1, "tags": [] });

        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("notes.json", opts).unwrap();
            serde_json::to_writer(&mut zip, &notes_json).unwrap();
            zip.start_file("workspace.json", opts).unwrap();
            serde_json::to_writer(&mut zip, &old_ws_json).unwrap();
            zip.start_file("scripts/scripts.json", opts).unwrap();
            zip.write_all(b"{\"scripts\":[]}").unwrap();
            zip.finish().unwrap();
        }

        let temp_dst = NamedTempFile::new().unwrap();
        import_workspace(Cursor::new(&buf), temp_dst.path(), None, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let imported = Workspace::open(temp_dst.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]), None).unwrap();
        let meta = imported.get_workspace_metadata().unwrap();
        assert!(meta.author_name.is_none());
        assert!(meta.tags.is_empty());
    }
