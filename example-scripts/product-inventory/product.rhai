// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Product Views
// @description: Hover preview for the Product note type.

register_hover("Product", |note| {
    let price  = note.fields["price"]        ?? 0;
    let status = note.fields["stock_status"] ?? "";
    let cat    = note.fields["category"]     ?? "";
    let parts = [];
    if price > 0    { parts += [field("Price", "$" + price.to_string())]; }
    if status != "" { parts += [field("Stock", status)]; }
    if cat != ""    { parts += [field("Category", cat)]; }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});
