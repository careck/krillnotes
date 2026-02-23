// @name: Task
// @description: A trackable to-do item with status, priority, and due date.
//
// on_save hook:
//   - Computes the note title as a status symbol + task name, e.g. "[✓] Buy groceries"
//   - Derives `priority_label` (view-only) from the priority field
//
// Note: real-time urgency labels (e.g. "Due in 3 days") would require an on_view() hook,
// which is not yet implemented. Use the due_date field to track deadlines for now.

schema("Task", #{
    title_can_edit: false,
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

        note.title = "[" + symbol + "] " + name;

        let priority = note.fields["priority"];
        note.fields["priority_label"] =
            if priority == "high"        { "🔴 High" }
            else if priority == "medium" { "🟡 Medium" }
            else if priority == "low"    { "🟢 Low" }
            else                         { "" };

        note
    }
});
