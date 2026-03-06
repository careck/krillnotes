// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Zettelkasten Views
// @description: Views, hover, and sort actions for the Zettelkasten system.

fn tag_list(tags) {
    if tags.len() == 0 { return ""; }
    let s = tags[0];
    let i = 1;
    while i < tags.len() { s += ", " + tags[i]; i += 1; }
    s
}

register_view("Zettel", "Content", #{ display_first: true }, |note| {
    let body = note.fields["body"] ?? "";

    let body_block = if body != "" {
        markdown(body)
    } else {
        text("(no content)")
    };

    let tags = note.tags;
    if tags.len() == 0 {
        return body_block;
    }

    let related = get_notes_for_tag(tags).filter(|n| n.id != note.id);
    if related.len() == 0 {
        return body_block;
    }

    let rows = related.map(|n| [link_to(n), tag_list(n.tags)]);
    let related_section = section(
        "Related Notes (" + related.len() + ")",
        table(["Note", "Tags"], rows)
    );

    stack([body_block, related_section])
});

register_view("Kasten", "Overview", #{ display_first: true }, |note| {
    let zettel = get_children(note.id);
    let count = zettel.len();

    if count == 0 {
        return text("No notes yet. Right-click to add a Zettel.");
    }

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

    zettel.sort_by(|a, b| a.title >= b.title);

    let recent = if zettel.len() > 10 { zettel.extract(0, 10) } else { zettel };
    let rows = recent.map(|z| [link_to(z), tag_list(z.tags)]);

    stack([
        text(stats),
        section("Recent Notes", table(["Note", "Tags"], rows))
    ])
});

register_hover("Kasten", |note| {
    let kids = get_children(note.id);
    field("Notes", kids.len().to_string())
});

register_menu("Sort by Date (Newest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title >= b.title);
    children.map(|c| c.id)
});

register_menu("Sort by Date (Oldest First)", ["Kasten"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
