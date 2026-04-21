// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Book Collection Views
// @description: Views, hover previews, and sort actions for Book and Library.

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

register_hover("Library", |note| {
    let books = get_children(note.id);
    let reading = books.filter(|b| (b.fields["status"] ?? "") == "Reading").len();
    let read    = books.filter(|b| (b.fields["status"] ?? "") == "Read").len();
    let parts = [field("Books", books.len().to_string())];
    if reading > 0 { parts += [field("Reading", reading.to_string())]; }
    if read > 0    { parts += [field("Read",    read.to_string())]; }
    stack(parts)
});

register_hover("Book", |note| {
    let author = note.fields["author"] ?? "";
    let status = note.fields["status"] ?? "";
    let rating = note.fields["rating"] ?? 0;
    let parts = [];
    if author != "" { parts += [field("Author", author)]; }
    if status != "" { parts += [field("Status", status)]; }
    if rating > 0   { parts += [field("Rating", stars(rating))]; }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
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
