//! This script is used to create a contact.

schema("Contact", #{
    title_can_edit: false,
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
