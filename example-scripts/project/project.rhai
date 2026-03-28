// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Project Views
// @description: Views for Project: overdue tasks and active tasks.

register_view("Project", "Overdue Tasks", #{}, |note| {
    let now = today();
    let children = get_children(note.id);
    let overdue = children.filter(|c| {
        c.schema == "Task"
        && c.fields.status != "DONE"
        && type_of(c.fields.due_date) == "string"
        && c.fields.due_date != ""
        && c.fields.due_date < now
    });

    if overdue.len() == 0 {
        return text("No overdue tasks.");
    }

    overdue.sort_by(|a, b| a.fields.due_date >= b.fields.due_date);

    let rows = overdue.map(|t| [
        link_to(t),
        t.fields.status,
        t.fields.priority_label ?? "-",
        t.fields.due_date,
        t.fields.assignee ?? "-"
    ]);

    section(
        "Overdue (" + overdue.len() + ")",
        table(["Task", "Status", "Priority", "Due Date", "Assignee"], rows)
    )
});

register_view("Project", "Active Tasks", #{}, |note| {
    let children = get_children(note.id);
    let active = children.filter(|c| {
        c.schema == "Task"
        && c.fields.status != "DONE"
    });

    if active.len() == 0 {
        return text("No active tasks.");
    }

    // Tasks with a due date first (sorted descending), then tasks without.
    let with_date = active.filter(|t|
        type_of(t.fields.due_date) == "string" && t.fields.due_date != ""
    );
    let no_date = active.filter(|t|
        type_of(t.fields.due_date) != "string" || t.fields.due_date == ""
    );

    with_date.sort_by(|a, b| a.fields.due_date >= b.fields.due_date);

    let sorted = with_date + no_date;

    let rows = sorted.map(|t| [
        link_to(t),
        t.fields.status,
        t.fields.priority_label ?? "-",
        t.fields.due_date ?? "-",
        t.fields.assignee ?? "-"
    ]);

    section(
        "Active (" + active.len() + ")",
        table(["Task", "Status", "Priority", "Due Date", "Assignee"], rows)
    )
});
