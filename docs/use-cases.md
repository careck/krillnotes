# Krillnotes Use Cases & Ideas

A living list of schema/hook ideas to explore. Add, refine, and cross off as we go.

---

## `on_view` showcases

- **Journal** — date + body fields; `on_view` renders a timeline of the last N entries pulled via `get_notes_of_type()`
- **Contact relations** — Contact `on_view` shows all Tasks where `assignee` matches the contact's name, fetched inline

## Tree action mutation showcases

- **Meeting Notes scaffold** — right-click a Project, creates a dated TextNote child with an agenda template
- **Promote to Project** — converts a Task into a full Project note, copies relevant fields, creates default sub-tasks beneath it

## `on_add_child` + `on_save` showcases

- **Kanban Board** — parent note that auto-stamps `status = "TODO"` on every new Task added to it via `on_add_child`

## Cross-note / dashboard showcases

- **Dashboard** — no fields; `on_view` aggregates stats across all Projects (count by status, overdue tasks, etc.)
