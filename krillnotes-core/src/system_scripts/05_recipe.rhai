// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Recipe
// @description: A cooking recipe with ingredients, steps, and derived total time.
//
// on_save hook:
//   - Derives `total_time` (view-only) from prep_time + cook_time
//     (formatted as "45 min" or "1h 15min")

schema("Recipe", #{
    title_can_edit: true,
    fields: [
        #{ name: "servings",    type: "number",   required: false                  },
        #{ name: "prep_time",   type: "number",   required: false                  },
        #{ name: "cook_time",   type: "number",   required: false                  },
        #{ name: "difficulty",  type: "select",   required: false,
           options: ["Easy", "Medium", "Hard"]                                     },
        #{ name: "ingredients", type: "textarea", required: false                  },
        #{ name: "steps",       type: "textarea", required: false                  },
        #{ name: "total_time",  type: "text",     required: false, can_edit: false  },
    ],
    on_save: |note| {
        let prep  = note.fields["prep_time"];
        let cook  = note.fields["cook_time"];
        let total = (prep + cook).to_int();

        set_field(note.id, "total_time",
            if total <= 0 {
                ""
            } else if total < 60 {
                total.to_string() + " min"
            } else {
                let h = total / 60;
                let m = total % 60;
                if m == 0 { h.to_string() + "h" }
                else      { h.to_string() + "h " + m.to_string() + "min" }
            });

        commit();
    }
});
