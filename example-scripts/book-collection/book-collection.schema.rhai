// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Book Collection
// @description: A personal library. Includes a Book note type and a Library folder.
//
// Usage: Create a Library note, then add Book notes as children.

schema("Book", #{
    version: 1,
    title_can_edit: false,
    allowed_parent_schemas: ["Library"],
    fields: [
        #{ name: "book_title",    type: "text",     required: true                   },
        #{ name: "author",        type: "text",     required: true                   },
        #{ name: "genre",         type: "text",     required: false                  },
        #{ name: "status",        type: "select",   required: false,
           options: ["To Read", "Reading", "Read"]                                   },
        #{ name: "rating",        type: "rating",   required: false, max: 5          },
        #{ name: "started",       type: "date",     required: false                  },
        #{ name: "finished",      type: "date",     required: false                  },
        #{ name: "notes",         type: "textarea", required: false                  },
        #{ name: "read_duration", type: "text",     required: false, can_edit: false },
    ],
    on_save: |note| {
        let title  = note.fields["book_title"];
        let author = note.fields["author"];

        let new_title = if author != "" && title != "" {
            author + ": " + title
        } else if title != "" {
            title
        } else {
            "Untitled Book"
        };
        set_title(note.id, new_title);

        let started  = note.fields["started"];
        let finished = note.fields["finished"];
        let duration = if type_of(started) == "string" && started != ""
            && type_of(finished) == "string" && finished != "" {
            let s_parts = started.split("-");
            let f_parts = finished.split("-");
            let s_days = parse_int(s_parts[0]) * 365
                       + parse_int(s_parts[1]) * 30
                       + parse_int(s_parts[2]);
            let f_days = parse_int(f_parts[0]) * 365
                       + parse_int(f_parts[1]) * 30
                       + parse_int(f_parts[2]);
            let diff = f_days - s_days;
            if diff > 0 { diff.to_string() + " days" } else { "" }
        } else {
            ""
        };
        set_field(note.id, "read_duration", duration);

        commit();
    }
});

schema("Library", #{
    version: 1,
    allowed_children_schemas: ["Book"],
    fields: [],
});
