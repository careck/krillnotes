//! Product — an inventory item with auto-formatted title and stock status.
//!
//! on_save hook:
//!   - Computes title as "Product Name (SKU)" (or just name if SKU is blank)
//!   - Derives `stock_status` (view-only) from the stock count

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
    ]
});

on_save("Product", |note| {
    let name = note.fields["product_name"];
    let sku  = note.fields["sku"];

    note.title = if sku != "" {
        name + " (" + sku + ")"
    } else {
        name
    };

    let stock = note.fields["stock"];
    note.fields["stock_status"] =
        if stock <= 0.0        { "❌ Out of Stock" }
        else if stock < 5.0    { "⚠️ Low Stock" }
        else                   { "✅ In Stock" };

    note
});
