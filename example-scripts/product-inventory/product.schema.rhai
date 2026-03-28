// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Product
// @description: An inventory item with auto-formatted title and stock status.

schema("Product", #{
    version: 1,
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
        set_field(note.id, "stock_status",
            if stock < 0.0         { "❌ Out of Stock" }
            else if stock == 0.0   { "" }
            else if stock < 5.0    { "⚠️ Low Stock" }
            else                   { "✅ In Stock" });

        commit();
    }
});
