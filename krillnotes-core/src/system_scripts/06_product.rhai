// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Product
// @description: An inventory item with auto-formatted title and stock status.
//
// on_save hook:
//   - Computes title as "Product Name (SKU)" (or just name if SKU is blank)
//   - Derives `stock_status` (view-only) from the stock count

schema("Product", #{
    title_can_edit: false,
    fields: [
        #{ name: "product_name", type: "text",     required: true                   },
        #{ name: "sku",          type: "text",     required: false                  },
        #{ name: "price",        type: "number",   required: false                  },
        #{ name: "stock",        type: "number",   required: false                  },
        #{ name: "category",     type: "text",     required: false                  },
        #{ name: "description",  type: "textarea", required: false                  },
        #{ name: "stock_status", type: "text",     required: false, can_edit: false  },
    ],
    on_save: |note| {
        let name = note.fields["product_name"];
        let sku  = note.fields["sku"];

        set_title(note.id, if sku != "" {
            name + " (" + sku + ")"
        } else {
            name
        });

        let stock = note.fields["stock"];
        // Note: stock defaults to 0.0 when not set. We treat 0.0 as "not entered"
        // and only show a status badge when stock has been explicitly recorded.
        // If you always want to show "Out of Stock" for zero stock, remove this guard.
        set_field(note.id, "stock_status",
            if stock < 0.0         { "❌ Out of Stock" }
            else if stock == 0.0   { "" }
            else if stock < 5.0    { "⚠️ Low Stock" }
            else                   { "✅ In Stock" });

        commit();
    }
});
