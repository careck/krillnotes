// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Book Collection Views
// @description: Views and sort actions for the Library note type.

register_view("Library", "Collection", #{ display_first: true }, |note| {
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
        let rows = read.map(|b| [
            b.title,
            b.fields["author"]   ?? "-",
            b.fields["finished"] ?? "-",
            stars(b.fields["rating"] ?? 0)
        ]);
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
});

register_menu("Sort by Title (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

register_menu("Sort by Author (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["author"] ?? "") <= (b.fields["author"] ?? "")
    );
    children.map(|c| c.id)
});

register_menu("Sort by Genre (A→Z)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["genre"] ?? "") <= (b.fields["genre"] ?? "")
    );
    children.map(|c| c.id)
});

register_menu("Sort by Rating (High→Low)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["rating"] ?? 0) >= (b.fields["rating"] ?? 0)
    );
    children.map(|c| c.id)
});

register_menu("Sort by Date Read (Newest First)", ["Library"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b|
        (a.fields["finished"] ?? "") >= (b.fields["finished"] ?? "")
    );
    children.map(|c| c.id)
});
