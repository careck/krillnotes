// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Recipe Views
// @description: Hover preview for the Recipe note type.

register_hover("Recipe", |note| {
    let difficulty = note.fields["difficulty"] ?? "";
    let total      = note.fields["total_time"] ?? "";
    let servings   = note.fields["servings"]   ?? 0;
    let parts = [];
    if difficulty != "" { parts += [field("Difficulty", difficulty)]; }
    if total != ""      { parts += [field("Time", total)]; }
    if servings > 0     { parts += [field("Servings", servings.to_int().to_string())]; }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});
