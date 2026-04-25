use super::*;
use crate::core::timestamp::UnixSecs;

/// Helper: loads the bundled TextNote starter script into a registry.
fn load_text_note(registry: &mut ScriptRegistry) {
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/system_scripts/00_text_note.schema.rhai"
            )),
            "Text Note",
        )
        .expect("TextNote schema script should load");
    registry.resolve_bindings();
}

// ── hooks-inside-schema (new style) ─────────────────────────────────────

#[test]
fn test_on_save_inside_schema_sets_title() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Person", #{ version: 1,
                fields: [
                    #{ name: "first", type: "text", required: false },
                    #{ name: "last",  type: "text", required: false },
                ],
                on_save: |note| {
                    set_title(note.id, note.fields["last"] + ", " + note.fields["first"]);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
    fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

    let tx = registry
        .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
        .unwrap()
        .unwrap();

    assert!(tx.committed);
    let pn = tx.pending_notes.get("id-1").unwrap();
    assert_eq!(pn.effective_title(), "Doe, John");
}

#[test]
fn test_on_view_inside_schema_returns_html() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("Folder", "Default", |note| {
                text("hello from view")
            });
        "#,
            "test_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    use crate::Note;
    let note = Note {
        id: "n1".to_string(),
        schema: "Folder".to_string(),
        title: "F".to_string(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields: std::collections::BTreeMap::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: std::collections::HashMap::new(),
        children_by_id: std::collections::HashMap::new(),
        notes_by_type: std::collections::HashMap::new(),
        notes_by_tag: std::collections::HashMap::new(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let html = registry.run_on_view_hook(&note, ctx).unwrap();
    assert!(html.is_some());
    assert!(html.unwrap().contains("hello from view"));
}

// ── has_hook() ─────────────────────────────────────────────────────────

#[test]
fn test_has_hook_after_schema_with_on_save() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
                schema("Widget", #{ version: 1,
                    fields: [ #{ name: "label", type: "text", required: false } ],
                    on_save: |note| { note }
                });
            "#,
            "test",
        )
        .unwrap();
    assert!(registry.has_hook("Widget"));
    assert!(!registry.has_hook("Missing"));
}

// ── Schema registration ─────────────────────────────────────────────────

#[test]
fn test_schema_registration() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
                schema("TestNote", #{ version: 1,
                    fields: [
                        #{ name: "body", type: "text", required: false },
                        #{ name: "count", type: "number", required: false },
                    ]
                });
            "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TestNote").unwrap();
    assert_eq!(schema.name, "TestNote");
    assert_eq!(schema.fields.len(), 2);
    assert_eq!(schema.fields[0].name, "body");
    assert_eq!(schema.fields[0].field_type, "text");
}

// ── Schema collision detection ───────────────────────────────────────────

#[test]
fn test_schema_collision_returns_error() {
    let mut registry = ScriptRegistry::new().unwrap();

    // First script registers "Contact" — should succeed.
    registry
        .load_script(
            r#"
            schema("Contact", #{ version: 1, fields: [] });
        "#,
            "script_a",
        )
        .expect("first registration should succeed");

    // Second script tries to register "Contact" — should fail.
    let err = registry
        .load_script(
            r#"
            schema("Contact", #{ version: 1, fields: [] });
        "#,
            "script_b",
        )
        .expect_err("second registration should fail");

    let msg = err.to_string();
    assert!(
        msg.contains("Contact"),
        "error should mention the schema name"
    );
    assert!(
        msg.contains("script_a"),
        "error should name the owning script"
    );
}

#[test]
fn test_first_schema_wins_after_collision() {
    let mut registry = ScriptRegistry::new().unwrap();

    registry
        .load_script(
            r#"
            schema("Widget", #{ version: 1,
                fields: [ #{ name: "color", type: "text", required: false } ],
            });
        "#,
            "owner_script",
        )
        .unwrap();

    // Collision attempt — should fail.
    let _ = registry.load_script(
        r#"
            schema("Widget", #{ version: 1,
                fields: [ #{ name: "size", type: "number", required: false } ],
            });
        "#,
        "intruder_script",
    );

    // The schema registered by the first script must still be intact.
    let schema = registry.get_schema("Widget").unwrap();
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(
        schema.fields[0].name, "color",
        "first script's field definition should win"
    );
}

#[test]
fn test_clear_all_resets_owners_for_reload() {
    let mut registry = ScriptRegistry::new().unwrap();

    registry
        .load_script(
            r#"
            schema("Reloadable", #{ version: 1, fields: [] });
        "#,
            "script_one",
        )
        .unwrap();

    // After clear_all, the owner record is gone — so the same name can be registered again.
    registry.clear_all();

    registry
        .load_script(
            r#"
            schema("Reloadable", #{ version: 1, fields: [] });
        "#,
            "script_one",
        )
        .expect("re-registration after clear_all should succeed");
}

#[test]
fn test_default_fields() {
    let schema = Schema {
        name: "TestNote".to_string(),
        fields: vec![
            FieldDefinition {
                name: "body".to_string(),
                field_type: "text".to_string(),
                required: false,
                can_view: true,
                can_edit: true,
                options: vec![],
                max: 0,
                target_schema: None,
                show_on_hover: false,
                allowed_types: vec![],
                validate: None,
            },
            FieldDefinition {
                name: "count".to_string(),
                field_type: "number".to_string(),
                required: false,
                can_view: true,
                can_edit: true,
                options: vec![],
                max: 0,
                target_schema: None,
                show_on_hover: false,
                allowed_types: vec![],
                validate: None,
            },
        ],
        title_can_view: true,
        title_can_edit: true,
        children_sort: "none".to_string(),
        allowed_parent_schemas: vec![],
        allowed_children_schemas: vec![],
        allow_attachments: false,
        attachment_types: vec![],
        field_groups: vec![],
        ast: None,
        version: 1,
        migrations: std::collections::BTreeMap::new(),
        is_leaf: false,
        show_checkbox: false,
    };
    let defaults = schema.default_fields();
    assert_eq!(defaults.len(), 2);
    assert!(matches!(defaults.get("body"), Some(FieldValue::Text(_))));
    assert!(matches!(defaults.get("count"), Some(FieldValue::Number(_))));
}

#[test]
fn test_text_note_schema_loaded_from_starter() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    let schema = registry.get_schema("TextNote").unwrap();
    assert_eq!(schema.name, "TextNote");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "body");
    assert_eq!(schema.fields[0].field_type, "textarea");
}

#[test]
fn test_new_registry_is_empty() {
    let registry = ScriptRegistry::new().unwrap();
    assert!(
        registry.get_schema("TextNote").is_err(),
        "New registry should have no schemas"
    );
    assert!(registry.list_types().unwrap().is_empty());
}

#[test]
fn test_date_field_default() {
    let schema = Schema {
        name: "Test".to_string(),
        fields: vec![FieldDefinition {
            name: "birthday".to_string(),
            field_type: "date".to_string(),
            required: false,
            can_view: true,
            can_edit: true,
            options: vec![],
            max: 0,
            target_schema: None,
            show_on_hover: false,
            allowed_types: vec![],
            validate: None,
        }],
        title_can_view: true,
        title_can_edit: true,
        children_sort: "none".to_string(),
        allowed_parent_schemas: vec![],
        allowed_children_schemas: vec![],
        allow_attachments: false,
        attachment_types: vec![],
        field_groups: vec![],
        ast: None,
        version: 1,
        migrations: std::collections::BTreeMap::new(),
        is_leaf: false,
        show_checkbox: false,
    };
    let defaults = schema.default_fields();
    assert!(matches!(
        defaults.get("birthday"),
        Some(FieldValue::Date(None))
    ));
}

#[test]
fn test_email_field_default() {
    let schema = Schema {
        name: "Test".to_string(),
        fields: vec![FieldDefinition {
            name: "email_addr".to_string(),
            field_type: "email".to_string(),
            required: false,
            can_view: true,
            can_edit: true,
            options: vec![],
            max: 0,
            target_schema: None,
            show_on_hover: false,
            allowed_types: vec![],
            validate: None,
        }],
        title_can_view: true,
        title_can_edit: true,
        children_sort: "none".to_string(),
        allowed_parent_schemas: vec![],
        allowed_children_schemas: vec![],
        allow_attachments: false,
        attachment_types: vec![],
        field_groups: vec![],
        ast: None,
        version: 1,
        migrations: std::collections::BTreeMap::new(),
        is_leaf: false,
        show_checkbox: false,
    };
    let defaults = schema.default_fields();
    assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(s)) if s.is_empty()));
}

#[test]
fn test_contact_schema_loaded() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.rhai"
            )),
            "Contacts View",
        )
        .unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.schema.rhai"
            )),
            "Contacts",
        )
        .unwrap();
    registry.resolve_bindings();
    let schema = registry.get_schema("Contact").unwrap();
    assert_eq!(schema.name, "Contact");
    assert_eq!(schema.fields.len(), 12);
    let is_family_field = schema
        .fields
        .iter()
        .find(|f| f.name == "is_family")
        .unwrap();
    assert_eq!(is_family_field.field_type, "boolean");
    assert!(
        !is_family_field.required,
        "is_family should not be required"
    );
    let email_field = schema.fields.iter().find(|f| f.name == "email").unwrap();
    assert_eq!(email_field.field_type, "email");
    let birthdate_field = schema
        .fields
        .iter()
        .find(|f| f.name == "birthdate")
        .unwrap();
    assert_eq!(birthdate_field.field_type, "date");
    let first_name_field = schema
        .fields
        .iter()
        .find(|f| f.name == "first_name")
        .unwrap();
    assert!(first_name_field.required, "first_name should be required");
    let last_name_field = schema
        .fields
        .iter()
        .find(|f| f.name == "last_name")
        .unwrap();
    assert!(last_name_field.required, "last_name should be required");
}

// ── on_save hooks ───────────────────────────────────────────────────────

#[test]
fn test_run_on_save_hook_sets_title() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
                schema("Person", #{ version: 1,
                    fields: [
                        #{ name: "first", type: "text", required: false },
                        #{ name: "last",  type: "text", required: false },
                    ],
                    on_save: |note| {
                        set_title(note.id, note.fields["last"] + ", " + note.fields["first"]);
                        commit();
                    }
                });
            "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
    fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

    let tx = registry
        .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
        .unwrap()
        .unwrap();

    assert!(tx.committed);
    let pn = tx.pending_notes.get("id-1").unwrap();
    assert_eq!(pn.effective_title(), "Doe, John");
    assert_eq!(
        pn.effective_fields().get("first"),
        Some(&FieldValue::Text("John".to_string()))
    );
}

#[test]
fn test_run_on_save_hook_no_hook_returns_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    let fields = BTreeMap::new();
    let result = registry
        .run_on_save_hook("TextNote", "id-1", "TextNote", "title", &fields)
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn test_contact_on_save_hook_derives_title() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.rhai"
            )),
            "Contacts View",
        )
        .unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.schema.rhai"
            )),
            "Contacts",
        )
        .unwrap();
    registry.resolve_bindings();
    assert!(
        registry.has_hook("Contact"),
        "Contact schema should have an on_save hook"
    );

    let mut fields = BTreeMap::new();
    fields.insert(
        "first_name".to_string(),
        FieldValue::Text("Jane".to_string()),
    );
    fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
    fields.insert(
        "last_name".to_string(),
        FieldValue::Text("Smith".to_string()),
    );
    fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
    fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
    fields.insert("email".to_string(), FieldValue::Email("".to_string()));
    fields.insert("birthdate".to_string(), FieldValue::Date(None));
    fields.insert(
        "address_street".to_string(),
        FieldValue::Text("".to_string()),
    );
    fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
    fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
    fields.insert(
        "address_country".to_string(),
        FieldValue::Text("".to_string()),
    );
    fields.insert("is_family".to_string(), FieldValue::Boolean(false));

    let tx = registry
        .run_on_save_hook("Contact", "id-1", "Contact", "", &fields)
        .unwrap()
        .unwrap();

    assert!(tx.committed);
    let pn = tx.pending_notes.get("id-1").unwrap();
    assert_eq!(pn.effective_title(), "Smith, Jane");
}

// ── Field flags ─────────────────────────────────────────────────────────

#[test]
fn test_field_can_view_can_edit_defaults_to_true() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TestVis", #{ version: 1,
                fields: [
                    #{ name: "f1", type: "text" },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TestVis").unwrap();
    assert!(schema.fields[0].can_view, "can_view should default to true");
    assert!(schema.fields[0].can_edit, "can_edit should default to true");
}

#[test]
fn test_field_can_view_can_edit_explicit_false() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TestVis2", #{ version: 1,
                fields: [
                    #{ name: "view_only", type: "text", can_edit: false },
                    #{ name: "edit_only", type: "text", can_view: false },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TestVis2").unwrap();
    assert!(schema.fields[0].can_view);
    assert!(!schema.fields[0].can_edit);
    assert!(!schema.fields[1].can_view);
    assert!(schema.fields[1].can_edit);
}

#[test]
fn test_field_can_view_can_edit_explicit_true() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TestVisExplicit", #{ version: 1,
                fields: [
                    #{ name: "both_true",  type: "text", can_view: true,  can_edit: true  },
                    #{ name: "both_false", type: "text", can_view: false, can_edit: false },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TestVisExplicit").unwrap();
    assert!(
        schema.fields[0].can_view,
        "explicit can_view: true should parse as true"
    );
    assert!(
        schema.fields[0].can_edit,
        "explicit can_edit: true should parse as true"
    );
    assert!(
        !schema.fields[1].can_view,
        "explicit can_view: false should parse as false"
    );
    assert!(
        !schema.fields[1].can_edit,
        "explicit can_edit: false should parse as false"
    );
}

// ── Title flags ─────────────────────────────────────────────────────────

#[test]
fn test_schema_title_flags_default_to_true() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TitleTest", #{ version: 1,
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TitleTest").unwrap();
    assert!(
        schema.title_can_view,
        "title_can_view should default to true"
    );
    assert!(
        schema.title_can_edit,
        "title_can_edit should default to true"
    );
}

#[test]
fn test_schema_title_can_edit_false() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TitleHidden", #{ version: 1,
                title_can_edit: false,
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TitleHidden").unwrap();
    assert!(schema.title_can_view);
    assert!(!schema.title_can_edit);
}

#[test]
fn test_schema_title_flags_explicit_true() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TitleExplicit", #{ version: 1,
                title_can_view: true,
                title_can_edit: true,
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("TitleExplicit").unwrap();
    assert!(
        schema.title_can_view,
        "explicit title_can_view: true should parse as true"
    );
    assert!(
        schema.title_can_edit,
        "explicit title_can_edit: true should parse as true"
    );
}

#[test]
fn test_schema_allow_attachments_defaults_to_false() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("AttachTest", #{ version: 1,
                fields: [#{ name: "name", type: "text" }]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("AttachTest").unwrap();
    assert!(
        !schema.allow_attachments,
        "allow_attachments should default to false"
    );
    assert!(
        schema.attachment_types.is_empty(),
        "attachment_types should default to empty"
    );
}

#[test]
fn test_schema_allow_attachments_explicit_with_types() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("PhotoNote", #{ version: 1,
                allow_attachments: true,
                attachment_types: ["image/jpeg", "image/png"],
                fields: [#{ name: "caption", type: "text" }]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("PhotoNote").unwrap();
    assert!(schema.allow_attachments);
    assert_eq!(schema.attachment_types, vec!["image/jpeg", "image/png"]);
}

#[test]
fn test_contact_title_can_edit_false() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.rhai"
            )),
            "Contacts View",
        )
        .unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/contacts/contacts.schema.rhai"
            )),
            "Contacts",
        )
        .unwrap();
    registry.resolve_bindings();
    let schema = registry.get_schema("Contact").unwrap();
    assert!(
        !schema.title_can_edit,
        "Contact title_can_edit should be false"
    );
    assert!(
        schema.title_can_view,
        "Contact title_can_view should still be true"
    );
}

// ── Boolean / default value edge cases ──────────────────────────────────

#[test]
fn test_boolean_field_not_set_by_hook_is_absent_from_effective_fields() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
                schema("FlagNote", #{ version: 1,
                    fields: [
                        #{ name: "flag", type: "boolean", required: false },
                    ],
                    on_save: |note| {
                        // intentionally does NOT set flag
                        commit();
                    }
                });
            "#,
            "test",
        )
        .unwrap();

    // Do NOT include "flag" in the submitted fields.
    // In the gated model, effective_fields() = original_fields merged with pending_fields.
    // Since neither contain "flag", it is absent from the output.
    let fields = BTreeMap::new();

    let tx = registry
        .run_on_save_hook("FlagNote", "id-1", "FlagNote", "title", &fields)
        .unwrap()
        .unwrap();

    assert!(tx.committed);
    let pn = tx.pending_notes.get("id-1").unwrap();
    // Field was not in submitted fields and hook did not set it — absent from output.
    assert_eq!(
            pn.effective_fields().get("flag"),
            None,
            "boolean field absent from input and not set by hook should be absent from effective_fields"
        );
}

// ── clear_all ───────────────────────────────────────────────────────────

#[test]
fn test_load_script_and_clear_all() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("MyType", #{ version: 1, fields: [#{ name: "x", type: "text" }] });
        "#,
            "test",
        )
        .unwrap();

    assert!(registry.get_schema("MyType").is_ok());

    registry.clear_all();

    assert!(registry.get_schema("MyType").is_err());
}

#[test]
fn test_clear_all_removes_everything() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    registry
        .load_script(
            r#"
            schema("Custom", #{ version: 1, fields: [#{ name: "a", type: "text" }] });
        "#,
            "test",
        )
        .unwrap();

    registry.clear_all();

    let types = registry.list_types().unwrap();
    assert!(types.is_empty(), "clear_all should remove all schemas");
}

/// Regression: library scripts (library category) define functions that schema scripts
/// should be able to call.  Before the fix, function definitions from a separately-eval'd
/// library AST were not visible when the schema AST was compiled and executed.
#[test]
fn test_schema_script_can_call_library_script_functions() {
    let mut registry = ScriptRegistry::new().unwrap();

    // Load a library script that defines a helper function.
    registry.set_loading_category(Some("library".to_string()));
    registry
        .load_script(
            r#"
            fn format_greeting(name) {
                "Hello, " + name
            }
        "#,
            "my_library",
        )
        .unwrap();

    // Load a schema script that calls the library function inside on_save.
    registry.set_loading_category(Some("schema".to_string()));
    registry
        .load_script(
            r#"
            schema("Greeted", #{
                version: 1,
                fields: [#{ name: "name", type: "text" }],
                on_save: |note| {
                    let greeting = format_greeting(note.fields["name"]);
                    set_title(note.id, greeting);
                    commit();
                }
            });
        "#,
            "my_schema",
        )
        .unwrap();

    let fields = {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "name".to_string(),
            crate::core::note::FieldValue::Text("World".to_string()),
        );
        m
    };
    let tx = registry
        .run_on_save_hook("Greeted", "id-1", "Greeted", "", &fields)
        .unwrap()
        .unwrap();

    let pn = tx.pending_notes.get("id-1").unwrap();
    assert_eq!(pn.pending_title, Some("Hello, World".to_string()));
}

/// Regression: clear_all must reset library_sources so a reload doesn't carry over
/// functions from a previous load cycle.
#[test]
fn test_clear_all_resets_library_sources() {
    let mut registry = ScriptRegistry::new().unwrap();

    registry.set_loading_category(Some("library".to_string()));
    registry.load_script("fn lib_fn() { 42 }", "lib").unwrap();

    registry.clear_all();

    // After clear_all, loading a schema that calls lib_fn should fail.
    registry.set_loading_category(Some("schema".to_string()));
    let result = registry.load_script(
        r#"
            schema("X", #{
                version: 1,
                fields: [],
                on_save: |note| { lib_fn(); commit(); }
            });
        "#,
        "schema_using_lib",
    );
    // on_save is a closure — the error surfaces at call time, not load time, so schema loads OK.
    // What matters is that library_sources is empty after clear_all.
    let _ = result;
    assert!(
        registry.library_sources.lock().unwrap().is_empty(),
        "library_sources should be empty after clear_all"
    );
}

// ── Host functions ──────────────────────────────────────────────────────

#[test]
fn test_schema_exists_host_function() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    assert!(registry.schema_exists("TextNote"));
    assert!(!registry.schema_exists("NonExistent"));

    // Test via script execution
    registry
        .load_script(
            r#"
            let exists = schema_exists("TextNote");
            if !exists { throw "TextNote should exist"; }
            let missing = schema_exists("Missing");
            if missing { throw "Missing should not exist"; }
        "#,
            "test",
        )
        .unwrap();
}

#[test]
fn test_get_schema_fields_host_function() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    registry.load_script(r#"
            let fields = get_schema_fields("TextNote");
            if fields.len() != 1 { throw "Expected 1 field, got " + fields.len(); }
            if fields[0].name != "body" { throw "Expected 'body', got " + fields[0].name; }
            if fields[0].type != "textarea" { throw "Expected 'textarea', got " + fields[0].type; }
            if fields[0].options.len() != 0 { throw "Expected options length 0, got " + fields[0].options.len(); }
            if fields[0].max != 0 { throw "Expected max 0, got " + fields[0].max; }
        "#, "test").unwrap();
}

#[test]
fn test_hooks_cleared_on_clear_all() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Hooked", #{ version: 1,
                fields: [#{ name: "x", type: "text" }],
                on_save: |note| { note }
            });
        "#,
            "test",
        )
        .unwrap();
    assert!(registry.has_hook("Hooked"));

    registry.clear_all();
    assert!(!registry.has_hook("Hooked"));
}

// ── Select / rating fields ──────────────────────────────────────────────

#[test]
fn test_select_field_parses_options() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
            schema("Ticket", #{ version: 1,
                fields: [
                    #{ name: "status", type: "select", options: ["TODO", "WIP", "DONE"], required: true }
                ]
            });
        "#, "test").unwrap();
    let fields = get_schema_fields_for_test(&registry, "Ticket");
    assert_eq!(fields[0].options, vec!["TODO", "WIP", "DONE"]);
}

#[test]
fn test_rating_field_parses_max() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Review", #{ version: 1,
                fields: [
                    #{ name: "stars", type: "rating", max: 5 }
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let fields = get_schema_fields_for_test(&registry, "Review");
    assert_eq!(fields[0].max, 5);
}

#[test]
fn test_regular_fields_have_empty_options_and_zero_max() {
    let mut registry = ScriptRegistry::new().unwrap();
    load_text_note(&mut registry);
    let fields = get_schema_fields_for_test(&registry, "TextNote");
    assert!(fields[0].options.is_empty());
    assert_eq!(fields[0].max, 0);
}

fn get_schema_fields_for_test(registry: &ScriptRegistry, name: &str) -> Vec<FieldDefinition> {
    registry.get_schema(name).unwrap().fields
}

#[test]
fn test_options_with_non_string_item_returns_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    let result = registry.load_script(
        r#"
            schema("Bad", #{ version: 1,
                fields: [
                    #{ name: "status", type: "select", options: ["OK", 42] }
                ]
            });
        "#,
        "test",
    );
    assert!(
        result.is_err(),
        "non-string item in options should return a Scripting error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("strings"),
        "error should mention 'strings', got: {msg}"
    );
}

// ── Starter scripts ─────────────────────────────────────────────────────

#[test]
fn test_starter_scripts_load_without_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    let starters = ScriptRegistry::starter_scripts();
    assert!(!starters.is_empty(), "Should have bundled starter scripts");

    for starter in &starters {
        registry
            .load_script(&starter.source_code, &starter.filename)
            .unwrap_or_else(|e| panic!("{} should load: {e}", starter.filename));
    }

    assert!(registry.schema_exists("TextNote"));
}

#[test]
fn test_starter_scripts_sorted_by_filename() {
    let starters = ScriptRegistry::starter_scripts();
    let filenames: Vec<&str> = starters.iter().map(|s| s.filename.as_str()).collect();
    let mut sorted = filenames.clone();
    sorted.sort();
    assert_eq!(
        filenames, sorted,
        "Starter scripts should be sorted by filename"
    );
}

#[test]
fn test_negative_max_returns_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    let result = registry.load_script(
        r#"
            schema("Bad", #{ version: 1,
                fields: [
                    #{ name: "stars", type: "rating", max: -1 }
                ]
            });
        "#,
        "test",
    );
    assert!(
        result.is_err(),
        "negative max should return a Scripting error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("max"),
        "error should mention 'max', got: {msg}"
    );
}

#[test]
fn test_select_and_rating_default_fields() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Widget", #{ version: 1,
                fields: [
                    #{ name: "status", type: "select", options: ["A", "B"] },
                    #{ name: "stars",  type: "rating",  max: 5 }
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("Widget").unwrap();
    let defaults = schema.default_fields();
    assert_eq!(defaults["status"], crate::FieldValue::Text(String::new()));
    assert_eq!(defaults["stars"], crate::FieldValue::Number(0.0));
}

#[test]
fn test_select_field_round_trips_through_hook() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("S", #{ version: 1,
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    set_field(note.id, "status", "B");
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert("status".to_string(), FieldValue::Text("A".to_string()));

    let tx = registry
        .run_on_save_hook("S", "id1", "S", "title", &fields)
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let pn = tx.pending_notes.get("id1").unwrap();
    assert_eq!(
        pn.effective_fields()["status"],
        FieldValue::Text("B".to_string())
    );
}

#[test]
fn test_rating_field_round_trips_through_hook() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("R", #{ version: 1,
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    set_field(note.id, "stars", 4.0);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert("stars".to_string(), FieldValue::Number(0.0));

    let tx = registry
        .run_on_save_hook("R", "id1", "R", "title", &fields)
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let pn = tx.pending_notes.get("id1").unwrap();
    assert_eq!(pn.effective_fields()["stars"], FieldValue::Number(4.0));
}

#[test]
fn test_select_field_not_set_by_hook_is_absent_from_effective_fields() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("S2", #{ version: 1,
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    // deliberately do NOT set status
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    // In the gated model, effective_fields() = original_fields merged with pending_fields.
    // Since neither contain "status", it is absent from the output.
    let fields = BTreeMap::new(); // no status field
    let tx = registry
        .run_on_save_hook("S2", "id1", "S2", "title", &fields)
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let pn = tx.pending_notes.get("id1").unwrap();
    // Field was not in submitted fields and hook did not set it — absent from output.
    assert_eq!(pn.effective_fields().get("status"), None);
}

#[test]
fn test_rating_field_not_set_by_hook_is_absent_from_effective_fields() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("R2", #{ version: 1,
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    // deliberately do NOT set stars
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    // In the gated model, effective_fields() = original_fields merged with pending_fields.
    // Since neither contain "stars", it is absent from the output.
    let fields = BTreeMap::new(); // no stars field
    let tx = registry
        .run_on_save_hook("R2", "id1", "R2", "title", &fields)
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let pn = tx.pending_notes.get("id1").unwrap();
    // Field was not in submitted fields and hook did not set it — absent from output.
    assert_eq!(pn.effective_fields().get("stars"), None);
}

// ── children_sort ───────────────────────────────────────────────────────

#[test]
fn test_children_sort_defaults_to_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("SortTest", #{ version: 1,
                fields: [#{ name: "x", type: "text" }]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("SortTest").unwrap();
    assert_eq!(
        schema.children_sort, "none",
        "children_sort should default to 'none'"
    );
}

#[test]
fn test_children_sort_explicit_asc() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("SortAsc", #{ version: 1,
                children_sort: "asc",
                fields: [#{ name: "x", type: "text" }]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("SortAsc").unwrap();
    assert_eq!(schema.children_sort, "asc");
}

#[test]
fn test_children_sort_explicit_desc() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("SortDesc", #{ version: 1,
                children_sort: "desc",
                fields: [#{ name: "x", type: "text" }]
            });
        "#,
            "test",
        )
        .unwrap();
    let schema = registry.get_schema("SortDesc").unwrap();
    assert_eq!(schema.children_sort, "desc");
}

// ── Book hook edge case ─────────────────────────────────────────────────

#[test]
fn test_book_hook_with_unset_dates_does_not_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/book-collection/book-collection.rhai"
            )),
            "Book Collection Views",
        )
        .expect("Book library script should load");
    registry
        .load_script(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../example-scripts/book-collection/book-collection.schema.rhai"
            )),
            "Book Collection",
        )
        .expect("Book schema script should load");
    registry.resolve_bindings();

    let mut fields = BTreeMap::new();
    fields.insert(
        "book_title".to_string(),
        crate::FieldValue::Text("Dune".to_string()),
    );
    fields.insert(
        "author".to_string(),
        crate::FieldValue::Text("Herbert".to_string()),
    );
    fields.insert("genre".to_string(), crate::FieldValue::Text(String::new()));
    fields.insert("status".to_string(), crate::FieldValue::Text(String::new()));
    fields.insert("rating".to_string(), crate::FieldValue::Number(0.0));
    fields.insert("started".to_string(), crate::FieldValue::Date(None));
    fields.insert("finished".to_string(), crate::FieldValue::Date(None));
    fields.insert(
        "read_duration".to_string(),
        crate::FieldValue::Text(String::new()),
    );
    fields.insert("notes".to_string(), crate::FieldValue::Text(String::new()));

    let result = registry.run_on_save_hook("Book", "id1", "Book", "Dune", &fields);
    assert!(
        result.is_ok(),
        "book hook should not error with unset dates: {:?}",
        result
    );
    let tx = result.unwrap().unwrap();
    assert!(tx.committed);
    let pn = tx.pending_notes.get("id1").unwrap();
    assert_eq!(pn.effective_title(), "Herbert: Dune");
    assert_eq!(
        pn.effective_fields()["read_duration"],
        crate::FieldValue::Text(String::new())
    );
}

// ── render_default_view on ScriptRegistry ───────────────────────────────

#[test]
fn test_script_registry_render_default_view_textarea_markdown() {
    use crate::{FieldValue, Note};
    use std::collections::BTreeMap;

    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Memo", #{ version: 1,
                fields: [
                    #{ name: "body", type: "textarea", required: false }
                ]
            });
        "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert("body".into(), FieldValue::Text("**important**".into()));
    let note = Note {
        id: "n1".into(),
        title: "Test".into(),
        schema: "Memo".into(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields,
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };

    let html = registry.render_default_view(&note, &Default::default(), &[]);
    assert!(html.contains("<strong>important</strong>"), "got: {html}");
}

// ── markdown() Rhai host function ───────────────────────────────────────

#[test]
fn test_markdown_rhai_function_renders_bold() {
    let registry = ScriptRegistry::new().unwrap();
    let script = r#"
            let result = markdown("**hello**");
            result
        "#;
    let result = registry.engine.eval::<String>(script).unwrap();
    assert!(result.contains("<strong>hello</strong>"), "got: {result}");
}

// ── link_to integration ─────────────────────────────────────────────────

#[test]
fn test_link_to_is_callable_from_on_view_script() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
            register_view("LinkTest", "Default", |note| {
                let target = #{ id: "target-id-123", title: "Target Note", fields: #{}, schema: "TextNote" };
                link_to(target)
            });
        "#, "test_views.rhai").unwrap();
    registry
        .load_script(
            r#"
            schema("LinkTest", #{ version: 1,
                fields: [#{ name: "ref_id", type: "text" }],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = Note {
        id: "note-1".to_string(),
        schema: "LinkTest".to_string(),
        title: "Test".to_string(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields: BTreeMap::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };

    let context = QueryContext {
        notes_by_id: HashMap::new(),
        children_by_id: HashMap::new(),
        notes_by_type: HashMap::new(),
        notes_by_tag: HashMap::new(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };

    let result = registry.run_on_view_hook(&note, context).unwrap();
    assert!(result.is_some());
    let html = result.unwrap();
    assert!(
        html.contains("kn-view-link"),
        "html should contain kn-view-link class"
    );
    assert!(
        html.contains("target-id-123"),
        "html should contain the target note id"
    );
    assert!(
        html.contains("Target Note"),
        "html should contain the target note title"
    );
}

#[test]
fn test_on_save_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Boom", #{ version: 1,
                fields: [ #{ name: "x", type: "text" } ],
                on_save: |note| {
                    throw "intentional runtime error";
                    note
                }
            });
            "#,
            "My Exploding Script",
        )
        .unwrap();

    let fields = BTreeMap::new();
    let err = registry
        .run_on_save_hook("Boom", "id-1", "Boom", "title", &fields)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("My Exploding Script"),
        "error should include script name, got: {msg}"
    );
}

// ── on_add_child hooks ──────────────────────────────────────────────────

#[test]
fn test_on_add_child_hook_modifies_parent_and_child() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (" + new_count.to_int().to_string() + ")");
                    set_title(child_note.id, "Child from hook");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [
                    #{ name: "name", type: "text", required: false },
                ],
            });
        "#,
            "test",
        )
        .unwrap();

    let mut parent_fields = BTreeMap::new();
    parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

    let mut child_fields = BTreeMap::new();
    child_fields.insert("name".to_string(), FieldValue::Text("".to_string()));

    let result = registry
        .run_on_add_child_hook(
            "Folder",
            "parent-id",
            "Folder",
            "Folder",
            &parent_fields,
            "child-id",
            "Item",
            "Untitled",
            &child_fields,
        )
        .unwrap();

    let result = result.expect("hook should return a result");
    let (p_title, p_fields) = result.parent.expect("should have parent update");
    assert_eq!(p_title, "Folder (1)");
    assert_eq!(p_fields["count"], FieldValue::Number(1.0));

    let (c_title, _) = result.child.expect("should have child update");
    assert_eq!(c_title, "Child from hook");
}

#[test]
fn test_on_add_child_hook_absent_returns_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Plain", #{ version: 1,
                fields: [],
            });
        "#,
            "test",
        )
        .unwrap();

    let result = registry
        .run_on_add_child_hook(
            "Plain",
            "p-id",
            "Plain",
            "Title",
            &std::collections::BTreeMap::new(),
            "c-id",
            "Plain",
            "Child",
            &std::collections::BTreeMap::new(),
        )
        .unwrap();

    assert!(result.is_none(), "no hook registered should return None");
}

#[test]
fn test_on_add_child_hook_returns_unit_gives_no_modifications() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    ()
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#,
            "test",
        )
        .unwrap();

    let result = registry
        .run_on_add_child_hook(
            "Folder",
            "p-id",
            "Folder",
            "Title",
            &std::collections::BTreeMap::new(),
            "c-id",
            "Item",
            "Child",
            &std::collections::BTreeMap::new(),
        )
        .unwrap();

    // Some(result) because hook exists, but both modifications are None
    let result = result.expect("hook present: should return Some");
    assert!(
        result.parent.is_none(),
        "unit return: parent should not be modified"
    );
    assert!(
        result.child.is_none(),
        "unit return: child should not be modified"
    );
}

#[test]
fn test_on_add_child_hook_parent_only_modification() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    set_field(parent_note.id, "count", 5.0);
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#,
            "test",
        )
        .unwrap();

    let mut parent_fields = BTreeMap::new();
    parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

    let result = registry
        .run_on_add_child_hook(
            "Folder",
            "p-id",
            "Folder",
            "Folder",
            &parent_fields,
            "c-id",
            "Item",
            "Untitled",
            &std::collections::BTreeMap::new(),
        )
        .unwrap();

    let result = result.expect("hook present: should return Some");
    let (_, p_fields) = result.parent.expect("parent modification expected");
    assert_eq!(p_fields["count"], FieldValue::Number(5.0));
    assert!(result.child.is_none(), "child should not be modified");
}

#[test]
fn test_on_add_child_hook_child_only_modification() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    set_title(child_note.id, "Initialized by hook");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#,
            "test",
        )
        .unwrap();

    let result = registry
        .run_on_add_child_hook(
            "Folder",
            "p-id",
            "Folder",
            "Folder",
            &std::collections::BTreeMap::new(),
            "c-id",
            "Item",
            "Untitled",
            &std::collections::BTreeMap::new(),
        )
        .unwrap();

    let result = result.expect("hook present: should return Some");
    assert!(result.parent.is_none(), "parent should not be modified");
    let (c_title, _) = result.child.expect("child modification expected");
    assert_eq!(c_title, "Initialized by hook");
}

#[test]
fn test_on_add_child_hook_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    throw "deliberate error";
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#,
            "my_test_script",
        )
        .unwrap();

    let err = registry
        .run_on_add_child_hook(
            "Folder",
            "p-id",
            "Folder",
            "Title",
            &std::collections::BTreeMap::new(),
            "c-id",
            "Item",
            "Child",
            &std::collections::BTreeMap::new(),
        )
        .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("my_test_script"),
        "error should include script name, got: {msg}"
    );
    assert!(
        msg.contains("on_add_child"),
        "error should mention hook name, got: {msg}"
    );
}

#[test]
fn test_on_add_child_hook_old_style_returns_helpful_error() {
    // A hook that returns a map (old-style) should be rejected with a migration message.
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = 1.0;
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#,
            "my_script",
        )
        .unwrap();

    let mut parent_fields = BTreeMap::new();
    parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

    let err = registry
        .run_on_add_child_hook(
            "Folder",
            "p-id",
            "Folder",
            "Title",
            &parent_fields,
            "c-id",
            "Item",
            "Child",
            &std::collections::BTreeMap::new(),
        )
        .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("my_script"),
        "error should include script name, got: {msg}"
    );
    assert!(
        msg.contains("gated model"),
        "error should mention migration, got: {msg}"
    );
}

#[test]
fn test_on_view_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("BoomView", "Default", |note| {
                throw "intentional runtime error";
                text("x")
            });
            "#,
            "My View Script",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("BoomView", #{ version: 1,
                fields: [],
            });
            "#,
            "BoomView Schema",
        )
        .unwrap();
    registry.resolve_bindings();

    use crate::Note;
    let note = Note {
        id: "n1".to_string(),
        schema: "BoomView".to_string(),
        title: "T".to_string(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields: BTreeMap::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: HashMap::new(),
        children_by_id: HashMap::new(),
        notes_by_type: HashMap::new(),
        notes_by_tag: HashMap::new(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let err = registry.run_on_view_hook(&note, ctx).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("My View Script"),
        "error should include script name, got: {msg}"
    );
}

// ── tree actions ─────────────────────────────────────────────────────────

#[test]
fn test_add_tree_action_registers_entry() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Sort Children", ["TextNote"], |note| { () });
        "#,
            "test_script",
        )
        .unwrap();
    registry.resolve_bindings();
    let map = registry.tree_action_map();
    assert_eq!(
        map.get("TextNote"),
        Some(&vec!["Sort Children".to_string()])
    );
}

#[test]
fn test_tree_action_map_empty_before_load() {
    let registry = ScriptRegistry::new().unwrap();
    assert!(registry.tree_action_map().is_empty());
}

#[test]
fn test_clear_all_removes_tree_actions() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Do Thing", ["TextNote"], |note| { () });
        "#,
            "test_script",
        )
        .unwrap();
    registry.resolve_bindings();
    registry.clear_all();
    assert!(registry.tree_action_map().is_empty());
}

#[test]
fn test_invoke_tree_action_hook_calls_callback() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Noop", ["TextNote"], |note| { () });
        "#,
            "test_script",
        )
        .unwrap();
    registry.resolve_bindings();
    let note = crate::Note {
        id: "n1".into(),
        title: "Hello".into(),
        schema: "TextNote".into(),
        parent_id: None,
        fields: std::collections::BTreeMap::new(),
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let result = registry
        .invoke_tree_action_hook("Noop", &note, ctx)
        .unwrap();
    assert!(
        result.reorder.is_none(),
        "callback returning () should yield no reorder"
    );
}

#[test]
fn test_invoke_tree_action_returns_id_vec_when_callback_returns_array() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Sort", ["TextNote"], |note| { ["id-b", "id-a"] });
        "#,
            "test_script",
        )
        .unwrap();
    registry.resolve_bindings();
    let note = crate::Note {
        id: "p1".into(),
        title: "Parent".into(),
        schema: "TextNote".into(),
        parent_id: None,
        fields: std::collections::BTreeMap::new(),
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let result = registry
        .invoke_tree_action_hook("Sort", &note, ctx)
        .unwrap();
    assert_eq!(
        result.reorder,
        Some(vec!["id-b".to_string(), "id-a".to_string()])
    );
}

#[test]
fn test_invoke_tree_action_unknown_label_errors() {
    let registry = ScriptRegistry::new().unwrap();
    let note = crate::Note {
        id: "n1".into(),
        title: "T".into(),
        schema: "TextNote".into(),
        parent_id: None,
        fields: std::collections::BTreeMap::new(),
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let err = registry
        .invoke_tree_action_hook("No Such Action", &note, ctx)
        .unwrap_err();
    assert!(
        err.to_string().contains("unknown tree action"),
        "got: {err}"
    );
}

#[test]
fn test_invoke_tree_action_runtime_error_includes_script_name() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Boom", ["TextNote"], |note| { throw "intentional"; });
        "#,
            "my_script",
        )
        .unwrap();
    registry.resolve_bindings();
    let note = crate::Note {
        id: "n1".into(),
        title: "T".into(),
        schema: "TextNote".into(),
        parent_id: None,
        fields: std::collections::BTreeMap::new(),
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let err = registry
        .invoke_tree_action_hook("Boom", &note, ctx)
        .unwrap_err();
    assert!(
        err.to_string().contains("my_script"),
        "error should include script name, got: {err}"
    );
}

// ── create_child host function ────────────────────────────────────────────

fn make_test_note(id: &str, node_type: &str) -> crate::Note {
    crate::Note {
        id: id.into(),
        title: "Test".into(),
        schema: node_type.into(),
        parent_id: None,
        fields: Default::default(),
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    }
}

fn make_empty_ctx() -> QueryContext {
    QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    }
}

#[test]
fn test_create_note_returns_note_map_with_defaults() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "status", type: "text", required: false },
                ]
            });
            register_menu("Make Task", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                if t.schema != "Task" { throw "schema must be Task"; }
                if t.id == ""           { throw "id must not be empty"; }
                if t.fields.status != "" { throw "status must default to empty string"; }
                commit();
            });
        "#,
            "test",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = make_test_note("parent1", "Task");
    let ctx = make_empty_ctx();
    let result = registry
        .invoke_tree_action_hook("Make Task", &note, ctx)
        .unwrap();
    let new_notes: Vec<_> = result
        .transaction
        .pending_notes
        .values()
        .filter(|p| p.is_new)
        .collect();
    assert_eq!(new_notes.len(), 1, "one new pending note expected");
    assert_eq!(new_notes[0].schema, "Task");
    assert_eq!(new_notes[0].parent_id.as_deref(), Some("parent1"));
}

// ── set_title / set_field on existing notes ──────────────────────────────

#[test]
fn test_update_note_queues_update_for_existing_note() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "status", type: "text", required: false },
                ]
            });
            register_menu("Mark Done", ["Task"], |note| {
                set_title(note.id, "Completed");
                set_field(note.id, "status", "Done");
                commit();
            });
        "#,
            "test",
        )
        .unwrap();
    registry.resolve_bindings();

    let mut fields = BTreeMap::new();
    fields.insert("status".to_string(), FieldValue::Text("Open".to_string()));
    let mut note = make_test_note("n1", "Task");
    note.fields = fields;
    let result = registry
        .invoke_tree_action_hook("Mark Done", &note, make_empty_ctx())
        .unwrap();
    assert!(
        result.transaction.committed,
        "transaction should be committed"
    );
    let pending = result
        .transaction
        .pending_notes
        .get("n1")
        .expect("n1 should have a pending note");
    assert!(!pending.is_new, "should be an update to an existing note");
    assert_eq!(pending.effective_title(), "Completed");
    assert_eq!(
        pending.effective_fields().get("status"),
        Some(&crate::core::note::FieldValue::Text("Done".into())),
    );
}

#[test]
fn test_update_note_on_inflight_note_updates_create_spec() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1,
                fields: [#{ name: "status", type: "text", required: false }]
            });
            register_menu("New Task", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                set_title(t.id, "My Task");
                set_field(t.id, "status", "Open");
                commit();
            });
        "#,
            "test",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = make_test_note("parent1", "Task");
    let result = registry
        .invoke_tree_action_hook("New Task", &note, make_empty_ctx())
        .unwrap();

    let new_notes: Vec<_> = result
        .transaction
        .pending_notes
        .values()
        .filter(|p| p.is_new)
        .collect();
    assert_eq!(new_notes.len(), 1, "one new pending note expected");
    assert!(
        result.transaction.committed,
        "transaction should be committed"
    );
    let pending = new_notes[0];
    assert!(pending.is_new, "should be a new note create");
    assert_eq!(pending.effective_title(), "My Task");
    assert_eq!(
        pending.effective_fields().get("status"),
        Some(&crate::core::note::FieldValue::Text("Open".into())),
    );
}

#[test]
fn test_get_children_sees_inflight_creates() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1, fields: [] });
            register_menu("Verify Children", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                let children = get_children(note.id);
                let found = children.filter(|c| c.id == t.id);
                if found.len() != 1 { throw "inflight note not visible in get_children"; }
                commit();
            });
        "#,
            "test",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = make_test_note("parent1", "Task");
    registry
        .invoke_tree_action_hook("Verify Children", &note, make_empty_ctx())
        .unwrap();
}

#[test]
fn test_get_note_sees_inflight_create() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1, fields: [] });
            register_menu("Verify get_note", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                let fetched = get_note(t.id);
                if fetched == () { throw "inflight note not visible via get_note"; }
                if fetched.id != t.id { throw "wrong note returned"; }
                commit();
            });
        "#,
            "test",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = make_test_note("parent1", "Task");
    registry
        .invoke_tree_action_hook("Verify get_note", &note, make_empty_ctx())
        .unwrap();
}

#[test]
fn test_on_view_note_has_tags() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("Tagged", "Default", |note| {
                let t = note.tags;
                text(t.len().to_string() + ":" + t[0])
            });
        "#,
            "test_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("Tagged", #{ version: 1,
                fields: [],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = Note {
        id: "n1".to_string(),
        schema: "Tagged".to_string(),
        title: "T".to_string(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields: std::collections::BTreeMap::new(),
        is_expanded: false,
        tags: vec!["rust".to_string(), "notes".to_string()],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: std::collections::HashMap::new(),
        children_by_id: std::collections::HashMap::new(),
        notes_by_type: std::collections::HashMap::new(),
        notes_by_tag: std::collections::HashMap::new(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
    assert!(
        html.contains("2:rust"),
        "expected '2:rust' in output, got: {html}"
    );
}

#[test]
fn test_today_returns_yyyy_mm_dd() {
    let mut registry = ScriptRegistry::new().unwrap();
    // Wrap today() in an on_save hook so we test it through the normal hook path
    registry
        .load_script(
            r#"
            schema("DateTest", #{ version: 1,
                fields: [#{ name: "dummy", type: "text", required: false }],
                on_save: |note| {
                    set_title(note.id, today());
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let tx = registry
        .run_on_save_hook("DateTest", "id1", "DateTest", "", &BTreeMap::new())
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let title = tx
        .pending_notes
        .get("id1")
        .unwrap()
        .effective_title()
        .to_string();
    // Must be exactly 10 chars: YYYY-MM-DD
    assert_eq!(
        title.len(),
        10,
        "expected YYYY-MM-DD (10 chars), got: {title}"
    );
    assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
    assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
}

#[test]
fn test_zettel_on_save_sets_date_title() {
    use crate::FieldValue;
    let mut registry = ScriptRegistry::new().unwrap();
    // Inline the exact on_save logic from templates/zettelkasten.rhai
    registry
        .load_script(
            r#"
            schema("ZettelTest", #{ version: 1,
                title_can_edit: false,
                fields: [#{ name: "body", type: "textarea", required: false }],
                on_save: |note| {
                    let body = note.fields["body"] ?? "";
                    let words = body.split(" ").filter(|w| w != "");
                    let snippet = if words.len() == 0 {
                        "Untitled"
                    } else {
                        let take = if words.len() > 6 { 6 } else { words.len() };
                        let s = ""; let i = 0;
                        while i < take { s += words[i] + " "; i += 1; }
                        s.trim();
                        if words.len() > 6 { s + " …" } else { s }
                    };
                    set_title(note.id, today() + " — " + snippet);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let mut fields = BTreeMap::new();
    fields.insert(
        "body".to_string(),
        FieldValue::Text("Emergence is when simple rules produce complex behaviour".to_string()),
    );

    let tx = registry
        .run_on_save_hook("ZettelTest", "id1", "ZettelTest", "", &fields)
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let title = tx
        .pending_notes
        .get("id1")
        .unwrap()
        .effective_title()
        .to_string();

    // Title must start with YYYY-MM-DD (10 chars, dashes at [4] and [7])
    assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
    assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
    // Must contain the first 6 words
    assert!(
        title.contains("Emergence is when simple rules produce"),
        "snippet missing: {title}"
    );
    // Body has 8 words — title must end with ellipsis
    assert!(
        title.ends_with('…'),
        "expected truncation ellipsis: {title}"
    );
}

#[test]
fn test_zettel_on_save_empty_body_uses_untitled() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("ZettelEmpty", #{ version: 1,
                title_can_edit: false,
                fields: [#{ name: "body", type: "textarea", required: false }],
                on_save: |note| {
                    let body = note.fields["body"] ?? "";
                    let words = body.split(" ").filter(|w| w != "");
                    let snippet = if words.len() == 0 {
                        "Untitled"
                    } else {
                        let take = if words.len() > 6 { 6 } else { words.len() };
                        let s = ""; let i = 0;
                        while i < take { s += words[i] + " "; i += 1; }
                        s.trim();
                        if words.len() > 6 { s + " …" } else { s }
                    };
                    set_title(note.id, today() + " — " + snippet);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let tx = registry
        .run_on_save_hook(
            "ZettelEmpty",
            "id2",
            "ZettelEmpty",
            "",
            &std::collections::BTreeMap::new(),
        )
        .unwrap()
        .unwrap();
    assert!(tx.committed);
    let title = tx
        .pending_notes
        .get("id2")
        .unwrap()
        .effective_title()
        .to_string();
    assert!(
        title.contains("Untitled"),
        "expected Untitled fallback: {title}"
    );
    // Must still have the date prefix
    assert_eq!(
        &title[4..5],
        "-",
        "missing date separator in untitled title: {title}"
    );
}

#[test]
fn test_default_field_for_note_link_is_none() {
    let schema = Schema {
        name: "Test".to_string(),
        fields: vec![FieldDefinition {
            name: "linked_note".to_string(),
            field_type: "note_link".to_string(),
            required: false,
            can_view: true,
            can_edit: true,
            options: vec![],
            max: 0,
            target_schema: None,
            show_on_hover: false,
            allowed_types: vec![],
            validate: None,
        }],
        title_can_view: true,
        title_can_edit: true,
        children_sort: "none".to_string(),
        allowed_parent_schemas: vec![],
        allowed_children_schemas: vec![],
        allow_attachments: false,
        attachment_types: vec![],
        field_groups: vec![],
        ast: None,
        version: 1,
        migrations: std::collections::BTreeMap::new(),
        is_leaf: false,
        show_checkbox: false,
    };
    let defaults = schema.default_fields();
    assert!(matches!(
        defaults.get("linked_note"),
        Some(FieldValue::NoteLink(None))
    ));
}

#[test]
fn test_parse_note_link_target_schema() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "project", type: "note_link", target_schema: "Project" }
                ]
            });
        "#,
            "test",
        )
        .unwrap();
    let fields = get_schema_fields_for_test(&registry, "Task");
    assert_eq!(fields[0].field_type, "note_link");
    assert_eq!(fields[0].target_schema, Some("Project".to_string()));
}

// ── on_hover hook ────────────────────────────────────────────────────────

#[test]
fn test_has_hover_hook_registered() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_hover("WithHover", |note| { "hover: " + note.title });
        "#,
            "HoverHook_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("WithHover", #{ version: 1,
                fields: [#{ name: "body", type: "text" }],
            });
        "#,
            "HoverHook.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();
    assert!(registry.has_hover("WithHover"));
    assert!(!registry.has_hover("Nonexistent"));
}

#[test]
fn test_run_on_hover_hook_returns_html() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_hover("HoverRun", |note| { "HOVER:" + note.title });
        "#,
            "HoverRun_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("HoverRun", #{ version: 1,
                fields: [#{ name: "body", type: "text" }],
            });
        "#,
            "HoverRun.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();
    let note = crate::Note {
        id: "id1".into(),
        title: "Test Note".into(),
        schema: "HoverRun".into(),
        parent_id: None,
        position: 0.0,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        created_by: String::new(),
        modified_by: String::new(),
        fields: std::collections::BTreeMap::new(),
        is_expanded: false,
        tags: vec![],
        schema_version: 1,
        is_checked: false,
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(),
        children_by_id: Default::default(),
        notes_by_type: Default::default(),
        notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
        attachments_by_note_id: Default::default(),
    };
    let html = registry.run_on_hover_hook(&note, ctx).unwrap();
    assert_eq!(html, Some("HOVER:Test Note".to_string()));
}

// ── show_on_hover ────────────────────────────────────────────────────────

#[test]
fn test_field_show_on_hover_parsed() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            // @name: HoverTest
            schema("HoverTest", #{ version: 1,
                fields: [
                    #{ name: "summary", type: "text", show_on_hover: true },
                    #{ name: "internal", type: "text" },
                ],
            });
        "#,
            "HoverTest",
        )
        .unwrap();
    let schema = registry.get_schema("HoverTest").unwrap();
    assert!(schema.fields[0].show_on_hover);
    assert!(!schema.fields[1].show_on_hover);
}

#[test]
fn test_get_attachments_returns_array_of_maps() {
    use crate::core::attachment::AttachmentMeta;

    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("PhotoNote", "Default", |note| {
                let atts = get_attachments(note.id);
                if atts.len() == 0 { return text("none"); }
                let first = atts[0];
                text(first.id + "|" + first.filename)
            });
        "#,
            "test_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("PhotoNote", #{ version: 1,
                fields: [],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    let note = make_test_note("note-1", "PhotoNote");

    let mut ctx = make_empty_ctx();
    ctx.attachments_by_note_id.insert(
        "note-1".to_string(),
        vec![AttachmentMeta {
            id: "att-uuid-1".to_string(),
            note_id: "note-1".to_string(),
            filename: "photo.png".to_string(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 100,
            hash_sha256: "abc".to_string(),
            salt: "00".repeat(32),
            created_at: UnixSecs::ZERO,
        }],
    );

    let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
    assert!(html.contains("att-uuid-1"), "got: {html}");
    assert!(html.contains("photo.png"), "got: {html}");
}

#[test]
fn test_rhai_display_image_with_uuid_in_field() {
    use crate::core::note::{FieldValue, Note};

    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("PhotoNote", "Default", |note| {
                display_image(note.fields["photo"], 300, "My alt")
            });
        "#,
            "test_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("PhotoNote", #{ version: 1,
                fields: [#{ name: "photo", type: "file", required: false }],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    let mut fields = BTreeMap::new();
    fields.insert(
        "photo".to_string(),
        FieldValue::File(Some("abc-uuid-123".to_string())),
    );
    let note = Note {
        id: "n1".to_string(),
        schema: "PhotoNote".to_string(),
        title: "T".to_string(),
        parent_id: None,
        fields,
        tags: vec![],
        schema_version: 1,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        position: 0.0,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        is_checked: false,
    };

    let html = registry
        .run_on_view_hook(&note, make_empty_ctx())
        .unwrap()
        .unwrap();
    assert!(
        html.contains("data-kn-attach-id=\"abc-uuid-123\""),
        "got: {html}"
    );
    assert!(html.contains("data-kn-width=\"300\""), "got: {html}");
}

#[test]
fn test_rhai_display_image_unset_field_shows_error() {
    use crate::core::note::{FieldValue, Note};

    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            register_view("PhotoNote", "Default", |note| {
                display_image(note.fields["photo"], 0, "")
            });
        "#,
            "test_views.rhai",
        )
        .unwrap();
    registry
        .load_script(
            r#"
            schema("PhotoNote", #{ version: 1,
                fields: [#{ name: "photo", type: "file", required: false }],
            });
        "#,
            "test.schema.rhai",
        )
        .unwrap();
    registry.resolve_bindings();

    let mut fields = BTreeMap::new();
    fields.insert("photo".to_string(), FieldValue::File(None));
    let note = Note {
        id: "n2".to_string(),
        schema: "PhotoNote".to_string(),
        title: "T".to_string(),
        parent_id: None,
        fields,
        tags: vec![],
        schema_version: 1,
        created_at: UnixSecs::ZERO,
        modified_at: UnixSecs::ZERO,
        position: 0.0,
        created_by: String::new(),
        modified_by: String::new(),
        is_expanded: false,
        is_checked: false,
    };

    let html = registry
        .run_on_view_hook(&note, make_empty_ctx())
        .unwrap()
        .unwrap();
    assert!(html.contains("kn-image-error"), "got: {html}");
}

// ── embed_media() Rhai host function ────────────────────────────────────

#[test]
fn test_embed_media_rhai_function_youtube() {
    let registry = ScriptRegistry::new().unwrap();
    let result = registry
        .engine
        .eval::<String>(r#"embed_media("https://www.youtube.com/watch?v=dQw4w9WgXcQ")"#)
        .expect("eval failed");
    assert!(
        result.contains("data-kn-embed-type=\"youtube\""),
        "got: {result}"
    );
    assert!(
        result.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""),
        "got: {result}"
    );
}

#[test]
fn test_embed_media_rhai_function_unknown_returns_empty() {
    let registry = ScriptRegistry::new().unwrap();
    let result = registry
        .engine
        .eval::<String>(r#"embed_media("https://example.com")"#)
        .expect("eval failed");
    assert!(result.is_empty(), "got: {result}");
}

// ── validate_field ────────────────────────────────────────────────────────

#[test]
fn test_validate_field_returns_error_on_invalid() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Rated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Must be positive" } else { () },
                    }
                ]
            });
        "#,
            "validate_test",
        )
        .unwrap();

    let err = registry
        .validate_field(
            "Rated",
            "score",
            &crate::core::note::FieldValue::Number(-1.0),
        )
        .unwrap();
    assert_eq!(err, Some("Must be positive".into()));
}

#[test]
fn test_validate_field_returns_none_on_valid() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Rated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Must be positive" } else { () },
                    }
                ]
            });
        "#,
            "validate_test",
        )
        .unwrap();

    let err = registry
        .validate_field(
            "Rated",
            "score",
            &crate::core::note::FieldValue::Number(5.0),
        )
        .unwrap();
    assert_eq!(err, None);
}

#[test]
fn test_validate_field_no_closure_returns_none() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Plain", #{ version: 1,
                fields: [ #{ name: "title", type: "text", required: false } ]
            });
        "#,
            "validate_test",
        )
        .unwrap();

    let err = registry
        .validate_field(
            "Plain",
            "title",
            &crate::core::note::FieldValue::Text("anything".into()),
        )
        .unwrap();
    assert_eq!(err, None);
}

// ── evaluate_group_visibility ─────────────────────────────────────────────

#[test]
fn test_evaluate_group_visibility_with_closure() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Typed", #{ version: 1,
                fields: [],
                field_groups: [
                    #{
                        name: "Special Group",
                        fields: [],
                        visible: |fields| fields["kind"] == "special",
                    }
                ]
            });
        "#,
            "visibility_test",
        )
        .unwrap();

    let mut fields_special = std::collections::BTreeMap::new();
    fields_special.insert(
        "kind".into(),
        crate::core::note::FieldValue::Text("special".into()),
    );
    let vis = registry
        .evaluate_group_visibility("Typed", &fields_special)
        .unwrap();
    assert_eq!(vis.get("Special Group"), Some(&true));

    let mut fields_other = std::collections::BTreeMap::new();
    fields_other.insert(
        "kind".into(),
        crate::core::note::FieldValue::Text("other".into()),
    );
    let vis2 = registry
        .evaluate_group_visibility("Typed", &fields_other)
        .unwrap();
    assert_eq!(vis2.get("Special Group"), Some(&false));
}

#[test]
fn test_evaluate_group_visibility_no_closure_always_true() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Simple", #{ version: 1,
                fields: [],
                field_groups: [
                    #{ name: "Always Visible", fields: [] }
                ]
            });
        "#,
            "visibility_test",
        )
        .unwrap();

    let vis = registry
        .evaluate_group_visibility("Simple", &Default::default())
        .unwrap();
    assert_eq!(vis.get("Always Visible"), Some(&true));
}

// ── set_field validate hard error ─────────────────────────────────────────

#[test]
fn test_set_field_validate_hard_error() {
    // When set_field is called from an on_save hook with a value that fails
    // the field's validate closure, the hook should abort with a hard error
    // that contains the validation message.
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Validated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Negative!" } else { () },
                    }
                ],
                on_save: |note| {
                    set_field(note.id, "score", -1.0);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let result = registry.run_on_save_hook(
        "Validated",
        "n1",
        "Validated",
        "Test Note",
        &Default::default(),
    );

    assert!(
        result.is_err(),
        "Expected hard error from validate in set_field"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Negative!"),
        "Expected 'Negative!' in error, got: {msg}"
    );
}

#[test]
fn test_set_field_validate_passes_when_valid() {
    // When set_field is called with a value that passes validation, the hook
    // should succeed normally.
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("Validated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Negative!" } else { () },
                    }
                ],
                on_save: |note| {
                    set_field(note.id, "score", 5.0);
                    commit();
                }
            });
        "#,
            "test",
        )
        .unwrap();

    let result = registry.run_on_save_hook(
        "Validated",
        "n1",
        "Validated",
        "Test Note",
        &Default::default(),
    );

    assert!(
        result.is_ok(),
        "Expected success with valid value, got: {:?}",
        result.err()
    );
    let tx = result.unwrap().expect("hook should return a transaction");
    assert!(tx.committed, "Transaction should be committed");
    let pending = tx.pending_notes.get("n1").expect("note should be in tx");
    assert_eq!(
        pending.pending_fields.get("score"),
        Some(&FieldValue::Number(5.0)),
    );
}

// ── C4: Rhai engine limits ─────────────────────────────────────────

#[test]
fn infinite_loop_script_returns_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    let result = registry.load_script(
        r#"
            schema("Looper", #{
                version: 1,
                fields: [],
                on_save: |note| { loop {} }
            });
            "#,
        "loop-test",
    );
    // Schema registration itself succeeds (the loop is in a closure)
    assert!(result.is_ok());
    registry.resolve_bindings();

    let fields = BTreeMap::new();
    let result = registry.run_on_save_hook("Looper", "n1", "Looper", "Test", &fields);
    assert!(result.is_err(), "Infinite loop should hit operation limit");
}

#[test]
fn deeply_recursive_script_returns_error() {
    let mut registry = ScriptRegistry::new().unwrap();
    let result = registry.load_script(
        r#"
            fn recurse(n) { recurse(n + 1) }
            schema("Recurser", #{
                version: 1,
                fields: [],
                on_save: |note| { recurse(0); commit(); }
            });
            "#,
        "recurse-test",
    );
    assert!(result.is_ok());
    registry.resolve_bindings();

    let fields = BTreeMap::new();
    let result = registry.run_on_save_hook("Recurser", "n1", "Recurser", "Test", &fields);
    assert!(
        result.is_err(),
        "Deep recursion should hit call level limit"
    );
}
