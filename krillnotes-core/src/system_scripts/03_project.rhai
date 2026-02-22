// @name: Project
// @description: A piece of work with status, priority, and optional dates.
//
// on_save hook:
//   - Derives `health` (view-only) from the status field, e.g. "🚧 Active"

schema("Project", #{
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
    ]
});

on_save("Project", |note| {
    let status = note.fields["status"];

    note.fields["health"] =
        if status == "Done"          { "✅ Done" }
        else if status == "Active"   { "🚧 Active" }
        else if status == "On Hold"  { "⏸ On Hold" }
        else                         { "📋 Planning" };

    note
});
