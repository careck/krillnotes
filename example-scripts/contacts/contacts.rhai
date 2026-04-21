// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Contacts View
// @description: Views and hover previews for ContactsFolder and Contact.

register_view("ContactsFolder", "Contacts", #{ display_first: true }, |note| {
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

register_hover("ContactsFolder", |note| {
    let kids = get_children(note.id);
    field("Contacts", kids.len().to_string())
});

register_hover("Contact", |note| {
    let email  = note.fields["email"]  ?? "";
    let phone  = note.fields["phone"]  ?? "";
    let mobile = note.fields["mobile"] ?? "";
    let parts = [];
    if email  != "" { parts += [field("Email",  email)]; }
    if phone  != "" { parts += [field("Phone",  phone)]; }
    if mobile != "" { parts += [field("Mobile", mobile)]; }
    if parts.len() == 0 { return text("No contact details"); }
    stack(parts)
});
