// @name: Contacts
// @description: A contact folder and contact card with name, address, and communication details.

schema("ContactsFolder", #{
    children_sort: "asc",
    allowed_children_types: ["Contact"],
    fields: [
        #{ name: "notes", type: "textarea", required: false },
    ]
});

schema("Contact", #{
    title_can_edit: false,
    allowed_parent_types: ["ContactsFolder"],
    fields: [
        #{ name: "first_name",      type: "text",  required: true  },
        #{ name: "middle_name",     type: "text",  required: false },
        #{ name: "last_name",       type: "text",  required: true  },
        #{ name: "phone",           type: "text",  required: false },
        #{ name: "mobile",          type: "text",  required: false },
        #{ name: "email",           type: "email", required: false },
        #{ name: "birthdate",       type: "date",  required: false },
        #{ name: "address_street",  type: "text",  required: false },
        #{ name: "address_city",    type: "text",  required: false },
        #{ name: "address_zip",     type: "text",  required: false },
        #{ name: "address_country", type: "text",  required: false },
        #{ name: "is_family",       type: "boolean",  required: false },
    ]
});

on_save("Contact", |note| {
    let last  = note.fields["last_name"];
    let first = note.fields["first_name"];
    if last != "" || first != "" {
        note.title = last + ", " + first;
    }
    note
});

on_view("ContactsFolder", |note| {
    let contacts = get_children(note.id);
    if contacts.len() == 0 {
        return text("No contacts yet. Add a contact using the context menu.");
    }
    let rows = contacts.map(|c| [
        link_to(c),
        c.fields.email  ?? "-",
        c.fields.phone  ?? "-",
        c.fields.mobile ?? "-"
    ]);
    let contacts_section = section(
        "Contacts (" + contacts.len() + ")",
        table(["Name", "Email", "Phone", "Mobile"], rows)
    );
    let notes_val = note.fields["notes"] ?? "";
    if notes_val == "" {
        contacts_section
    } else {
        stack([contacts_section, section("Notes", text(notes_val))])
    }
});
