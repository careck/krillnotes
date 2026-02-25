// @name: Book Collection
// @description: A personal library. Includes a Book note type and a Library folder
// with grouped on_view (Currently Reading / To Read / Read) and sort tree actions.
//
// Usage: Create a Library note, then add Book notes as children.

schema("Book", #{
    title_can_edit: false,
    allowed_parent_types: ["Library"],
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

        note.title = if author != "" && title != "" {
            author + ": " + title
        } else if title != "" {
            title
        } else {
            "Untitled Book"
        };

        let started  = note.fields["started"];
        let finished = note.fields["finished"];
        // Date fields are () (unit/null) when unset — type_of check is needed
        // because the ?? operator only handles unit, not an absent string.
        note.fields["read_duration"] = if type_of(started) == "string" && started != ""
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
            // diff > 0: show "N days". Same-day or reversed dates produce blank.
            if diff > 0 { diff.to_string() + " days" } else { "" }
        } else {
            ""
        };

        note
    }
});

schema("Library", #{
    allowed_children_types: ["Book"],
    fields: [],
    on_view: |note| {
        let books = get_children(note.id);
        if books.len() == 0 {
            return text("No books yet. Right-click to add a Book.");
        }

        let reading = books.filter(|b| (b.fields["status"] ?? "") == "Reading");
        let to_read = books.filter(|b| (b.fields["status"] ?? "") == "To Read");
        let read    = books.filter(|b| (b.fields["status"] ?? "") == "Read");
        let unsorted = books.filter(|b| {
            let s = b.fields["status"] ?? "";
            s != "Reading" && s != "To Read" && s != "Read"
        });

        let sections = [];

        if reading.len() > 0 {
            let rows = reading.map(|b| [
                b.title,
                b.fields["author"] ?? "-",
                b.fields["started"] ?? "-"
            ]);
            sections += [section(
                "Currently Reading (" + reading.len() + ")",
                table(["Title", "Author", "Started"], rows)
            )];
        }

        if to_read.len() > 0 {
            let rows = to_read.map(|b| [
                b.title,
                b.fields["author"] ?? "-",
                b.fields["genre"] ?? "-"
            ]);
            sections += [section(
                "To Read (" + to_read.len() + ")",
                table(["Title", "Author", "Genre"], rows)
            )];
        }

        if read.len() > 0 {
            let rows = read.map(|b| {
                let r = b.fields["rating"] ?? 0;
                let stars = "";
                let i = 0;
                while i < r    { stars += "★"; i += 1; }
                while i < 5    { stars += "☆"; i += 1; }
                [
                    b.title,
                    b.fields["author"]   ?? "-",
                    b.fields["finished"] ?? "-",
                    if r > 0 { stars } else { "—" }
                ]
            });
            sections += [section(
                "Read (" + read.len() + ")",
                table(["Title", "Author", "Finished", "Rating"], rows)
            )];
        }

        if unsorted.len() > 0 {
            let rows = unsorted.map(|b| [
                b.title,
                b.fields["author"] ?? "-"
            ]);
            sections += [section(
                "Unsorted (" + unsorted.len() + ")",
                table(["Title", "Author"], rows)
            )];
        }

        stack(sections)
    }
});

add_tree_action("Sort by Title (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

add_tree_action("Sort by Author (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["author"] ?? "") <= (b.fields["author"] ?? "")
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Genre (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["genre"] ?? "") <= (b.fields["genre"] ?? "")
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Rating (High→Low)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["rating"] ?? 0) >= (b.fields["rating"] ?? 0)
    );
    children.map(|c| c.id)
});

add_tree_action("Sort by Date Read (Newest First)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["finished"] ?? "") >= (b.fields["finished"] ?? "")
    );
    children.map(|c| c.id)
});
