// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Zettelkasten
// @description: An atomic note-taking system. Zettel notes are auto-titled with
// today's date and first words of the body.
//
// Usage: Create a Kasten note, then add Zettel notes as children.

// Strips markdown syntax characters so they don't leak into auto-generated titles.
fn strip_markdown(s) {
    s.replace("#", ""); s.replace("*", ""); s.replace("_", "");
    s.replace("`", ""); s.replace(">", ""); s.replace("[", "");
    s.replace("]", ""); s.replace("~", ""); s.replace("\\", "");
    s.trim();
    s
}

schema("Zettel", #{
    version: 1,
    title_can_edit: false,
    allowed_parent_types: ["Kasten"],
    fields: [
        #{ name: "body", type: "textarea", required: false, show_on_hover: true },
    ],
    on_save: |note| {
        let body = note.fields["body"] ?? "";
        let words = body.split(" ").map(|w| strip_markdown(w)).filter(|w| w != "");
        let snippet = if words.len() == 0 {
            "Untitled"
        } else {
            let take = if words.len() > 6 { 6 } else { words.len() };
            let s = "";
            let i = 0;
            while i < take { s += words[i] + " "; i += 1; }
            s.trim();
            if words.len() > 6 { s + " …" } else { s }
        };
        set_title(note.id, today() + " — " + snippet);
        commit();
    },
});

schema("Kasten", #{
    version: 1,
    allowed_children_types: ["Zettel"],
    fields: [],
});
