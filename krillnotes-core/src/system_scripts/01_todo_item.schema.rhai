// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Todo Item
// @description: A note with a checkbox — ideal for tasks, checklists, and to-do lists
// @category: schema

schema("TodoItem", #{
    version: 1,
    show_checkbox: true,
    is_leaf: true,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});

register_view("TodoItem", "Content", #{ display_first: true }, |note| {
    let body = note.fields["body"] ?? "";
    if note.is_checked {
        text("✓ Done")
    } else if body != "" {
        markdown(body)
    } else {
        text("(empty)")
    }
});
