// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Project Management
// @description: A project with tasks. Projects contain TextNote and Task children.

schema("Project", #{
    version: 1,
    title_can_edit: true,
    allowed_children_schemas: ["TextNote", "Task"],
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

schema("Task", #{
    version: 1,
    title_can_edit: false,
    allowed_parent_schemas: ["Project"],
    fields: [
        #{ name: "name",           type: "text",     required: true                     },
        #{ name: "status",         type: "select",   required: true,
           options: ["TODO", "WIP", "DONE"]                                             },
        #{ name: "priority",       type: "select",   required: false,
           options: ["low", "medium", "high"]                                           },
        #{ name: "due_date",       type: "date",     required: false                    },
        #{ name: "assignee",       type: "text",     required: false                    },
        #{ name: "notes",          type: "textarea", required: false                    },
        #{ name: "priority_label", type: "text",     required: false,
           can_edit: false                                                               },
    ],
    on_save: |note| {
        let name   = note.fields["name"];
        let status = note.fields["status"];

        let symbol = if status == "DONE" { "✓" }
                     else if status == "WIP" { "→" }
                     else { " " };

        set_title(note.id, "[" + symbol + "] " + name);

        let priority = note.fields["priority"];
        set_field(note.id, "priority_label",
            if priority == "high"        { "🔴 High" }
            else if priority == "medium" { "🟡 Medium" }
            else if priority == "low"    { "🟢 Low" }
            else                         { "" });

        commit();
    }
});
