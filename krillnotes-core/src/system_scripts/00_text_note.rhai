// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Text Note Actions
// @description: Tree actions for TextNote.

register_view("TextNote", "Content", #{ display_first: true }, |note| {
    let body = note.fields["body"] ?? "";
    if body != "" {
        markdown(body)
    } else {
        text("(empty)")
    }
});

register_menu("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

register_menu("Add Child Note", ["TextNote"], |note| {
    let child = create_note(note.id, "TextNote");
    child.title = "New note";
    update_note(child);
});
