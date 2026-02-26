// @name: Zettelkasten
// @description: An atomic note-taking system. Zettel notes are auto-titled with
// today's date and first words of the body. The Kasten folder shows recent notes
// and related-note links are surfaced via shared tags.
//
// Usage: Create a Kasten note, then add Zettel notes as children.
// Tip: assign native tags to each Zettel to enable related-note discovery.

schema("Zettel", #{
    title_can_edit: false,
    allowed_parent_types: ["Kasten"],
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ],
    on_save: |note| {
        let body = note.fields["body"] ?? "";
        let words = body.split(" ").filter(|w| w != "");
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
        note.title = today() + " — " + snippet;
        note
    },
    on_view: |note| {
        let body = note.fields["body"] ?? "";

        let body_block = if body != "" {
            text(body)
        } else {
            text("(no content)")
        };

        // Related notes via shared tags
        let tags = note.tags;
        if tags.len() == 0 {
            return body_block;
        }

        let related = get_notes_for_tag(tags).filter(|n| n.id != note.id);
        if related.len() == 0 {
            return body_block;
        }

        let rows = related.map(|n| [
            n.title,
            if n.tags.len() == 0 { "" } else if n.tags.len() == 1 { n.tags[0] } else { n.tags.reduce(|a, b| a + ", " + b) }
        ]);
        let related_section = section(
            "Related Notes (" + related.len() + ")",
            table(["Note", "Tags"], rows)
        );

        stack([body_block, related_section])
    }
});

schema("Kasten", #{
    allowed_children_types: ["Zettel"],
    fields: [],
    on_view: |note| {
        let zettel = get_children(note.id);
        let count = zettel.len();

        if count == 0 {
            return text("No notes yet. Right-click to add a Zettel.");
        }

        // Collect all unique tags across all Zettel
        let all_tags = [];
        for z in zettel { for t in z.tags { all_tags += [t]; } }
        all_tags.sort();
        let unique_tags = [];
        for t in all_tags {
            if unique_tags.len() == 0 || unique_tags[unique_tags.len() - 1] != t {
                unique_tags += [t];
            }
        }

        let stats = count.to_string() + " Zettel · " + unique_tags.len().to_string() + " unique tags";

        // Sort children by title descending (YYYY-MM-DD prefix makes newest first)
        zettel.sort_by(|a, b| a.title >= b.title);

        let recent = if zettel.len() > 10 { zettel.extract(0, 10) } else { zettel };
        let rows = recent.map(|z| [
            z.title,
            if z.tags.len() == 0 { "—" } else if z.tags.len() == 1 { z.tags[0] } else { z.tags.reduce(|a, b| a + ", " + b) }
        ]);

        stack([
            text(stats),
            section("Recent Notes", table(["Note", "Tags"], rows))
        ])
    }
});

add_tree_action("Sort by Date (Newest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title >= b.title);
    children.map(|c| c.id)
});

add_tree_action("Sort by Date (Oldest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
