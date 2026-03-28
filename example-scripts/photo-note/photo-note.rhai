// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Photo Note Views
// @description: Views for PhotoNote: detail view and hover thumbnail.

register_view("PhotoNote", "Photo", #{ display_first: true }, |note| {
    let caption = note.fields["caption"] ?? "";
    let body    = note.fields["body"]    ?? "";

    let img = display_image(note.fields["photo"], 480, caption);

    let parts = [img];
    if caption != "" { parts += [text(caption)]; }
    if body != "" { parts += [section("Notes", markdown(body))]; }
    stack(parts)
});

register_hover("PhotoNote", |note| {
    display_image(note.fields["photo"], 200, note.title)
});
