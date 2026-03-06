// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Project
// @description: A piece of work with status, priority, and optional dates.

schema("Project", #{
    version: 1,
    title_can_edit: true,
    fields: [
        #{ name: "status",      type: "select",   required: true,
           options: ["Planning", "Active", "On Hold", "Done"]                  },
        #{ name: "priority",    type: "select",   required: false,
           options: ["low", "medium", "high"]                                  },
        #{ name: "start_date",  type: "date",     required: false              },
        #{ name: "due_date",    type: "date",     required: false              },
        #{ name: "description", type: "textarea", required: false              },
        #{ name: "health",      type: "text",     required: false,
           can_edit: false                                                      },
    ],
    on_save: |note| {
        let status = note.fields["status"];

        set_field(note.id, "health",
            if status == "Done"          { "✅ Done" }
            else if status == "Active"   { "🚧 Active" }
            else if status == "On Hold"  { "⏸ On Hold" }
            else                         { "📋 Planning" });

        commit();
    }
});
