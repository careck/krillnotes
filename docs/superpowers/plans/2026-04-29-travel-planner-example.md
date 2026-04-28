# Travel Planner Example Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a travel planning example script with 8 schemas, views, hovers, menus, and a sample .krillnotes archive.

**Architecture:** Pure Rhai example — no Rust/TS changes. One schema file defines all 8 schemas (Folder, Location, Stay, Transport, Trip, DayPlan, Activity, Task). One presentation file defines all views, hovers, and menu actions. One .krillnotes archive provides a Japan trip sample workspace.

**Tech Stack:** Rhai scripting, zip (for .krillnotes archive)

**Spec:** `docs/superpowers/specs/2026-04-29-travel-planner-example-design.md`

---

## File Structure

| File | Purpose |
|------|---------|
| Create: `example-scripts/travel-planner/travel-planner.schema.rhai` | 8 schemas with fields, on_save hooks, constraints |
| Create: `example-scripts/travel-planner/travel-planner.rhai` | Views, hovers, context menu actions |
| Create: `example-scripts/travel-planner/travel-planner.krillnotes` | Sample Japan trip archive |

---

## Task 1: Schema file — Folder, Location, Stay

**Files:**
- Create: `example-scripts/travel-planner/travel-planner.schema.rhai`

- [ ] **Step 1: Create schema file with Folder, Location, and Stay schemas**

```rhai
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Travel Planner
// @description: Plan trips with locations, stays, transport, day-by-day itineraries, and a planning checklist.
//
// Usage: Create a Resources folder with sub-folders (Hotels, Restaurants, Attractions)
// to build a location library. Create a Trip, add DayPlans, and link Activities to locations.

// ---------------------------------------------------------------------------
// Folder — generic container for organising the tree
// ---------------------------------------------------------------------------
schema("Folder", #{
    version: 1,
    fields: [],
});

// ---------------------------------------------------------------------------
// Location — a place (hotel, restaurant, attraction, etc.)
// ---------------------------------------------------------------------------
schema("Location", #{
    version: 1,
    title_can_edit: false,
    is_leaf: true,
    fields: [
        #{ name: "name",     type: "text",     required: true                          },
        #{ name: "city",     type: "text",     required: true                          },
        #{ name: "country",  type: "text",     required: true                          },
        #{ name: "category", type: "select",   required: true,
           options: ["Hotel", "Restaurant", "Attraction", "Museum", "Temple",
                     "Park", "Market", "Bar", "Cafe", "Beach", "Other"]                },
        #{ name: "address",  type: "text",     required: false                         },
        #{ name: "rating",   type: "rating",   required: false, max: 5                 },
        #{ name: "notes",    type: "textarea", required: false                         },
        #{ name: "photo",    type: "file",     required: false,
           allowed_types: ["image/jpeg", "image/png", "image/webp"]                    },
    ],
    on_save: |note| {
        let name = note.fields["name"] ?? "";
        let city = note.fields["city"] ?? "";
        let title = if city != "" { name + " (" + city + ")" } else { name };
        if title == "" { title = "Untitled Location"; }
        set_title(note.id, title);
        commit();
    }
});

// ---------------------------------------------------------------------------
// Stay — a hotel booking, links to a Location
// ---------------------------------------------------------------------------
schema("Stay", #{
    version: 1,
    title_can_edit: false,
    is_leaf: true,
    fields: [
        #{ name: "hotel",                type: "note_link", required: false,
           target_schema: "Location"                                                   },
        #{ name: "check_in",             type: "date",      required: false            },
        #{ name: "check_out",            type: "date",      required: false            },
        #{ name: "price_per_night",      type: "number",    required: false            },
        #{ name: "currency",             type: "select",    required: false,
           options: ["USD", "EUR", "GBP", "JPY", "AUD", "Other"]                      },
        #{ name: "booking_ref",          type: "text",      required: false            },
        #{ name: "notes",                type: "textarea",  required: false            },
        #{ name: "booking_confirmation", type: "file",      required: false,
           allowed_types: ["application/pdf", "image/jpeg", "image/png"]               },
    ],
    on_save: |note| {
        let hotel_id = note.fields["hotel"] ?? ();
        let hotel_name = if hotel_id != () {
            let h = get_note(hotel_id);
            if h != () { h.title } else { "Unknown Hotel" }
        } else {
            "No Hotel"
        };

        let ci = note.fields["check_in"]  ?? ();
        let co = note.fields["check_out"] ?? ();
        let dates = if type_of(ci) == "string" && ci != ""
                    && type_of(co) == "string" && co != "" {
            " (" + ci + " – " + co + ")"
        } else if type_of(ci) == "string" && ci != "" {
            " (from " + ci + ")"
        } else {
            ""
        };

        set_title(note.id, hotel_name + dates);
        commit();
    }
});
```

- [ ] **Step 2: Verify the file was created correctly**

Run: `head -90 example-scripts/travel-planner/travel-planner.schema.rhai`
Expected: License header, Folder schema, start of Location schema visible.

- [ ] **Step 3: Commit**

```bash
git add example-scripts/travel-planner/travel-planner.schema.rhai
git commit -m "feat(example): travel-planner schemas — Folder, Location, Stay"
```

---

## Task 2: Schema file — Transport, Trip, DayPlan, Activity, Task

**Files:**
- Modify: `example-scripts/travel-planner/travel-planner.schema.rhai` (append after Stay schema)

- [ ] **Step 1: Append Transport, Trip, DayPlan, Activity, and Task schemas**

Append the following after the Stay schema closing `});`:

```rhai

// ---------------------------------------------------------------------------
// Transport — a travel leg (flight, train, bus, etc.)
// ---------------------------------------------------------------------------
schema("Transport", #{
    version: 1,
    title_can_edit: false,
    fields: [
        #{ name: "type",           type: "select",   required: false,
           options: ["Flight", "Train", "Bus", "Ferry", "Car", "Taxi", "Walk", "Other"] },
        #{ name: "from_city",      type: "text",     required: false            },
        #{ name: "to_city",        type: "text",     required: false            },
        #{ name: "departure_date", type: "date",     required: false            },
        #{ name: "departure_time", type: "text",     required: false            },
        #{ name: "arrival_date",   type: "date",     required: false            },
        #{ name: "arrival_time",   type: "text",     required: false            },
        #{ name: "carrier",        type: "text",     required: false            },
        #{ name: "booking_ref",    type: "text",     required: false            },
        #{ name: "price",          type: "number",   required: false            },
        #{ name: "currency",       type: "select",   required: false,
           options: ["USD", "EUR", "GBP", "JPY", "AUD", "Other"]               },
        #{ name: "notes",          type: "textarea", required: false            },
        #{ name: "ticket",         type: "file",     required: false,
           allowed_types: ["application/pdf", "image/jpeg", "image/png"]        },
    ],
    on_save: |note| {
        let t    = note.fields["type"]      ?? "";
        let from = note.fields["from_city"] ?? "";
        let to   = note.fields["to_city"]   ?? "";
        let title = if from != "" && to != "" {
            t + " " + from + " → " + to
        } else if to != "" {
            t + " → " + to
        } else if from != "" {
            t + " from " + from
        } else if t != "" {
            t
        } else {
            "Untitled Transport"
        };
        set_title(note.id, title.trim());
        commit();
    }
});

// ---------------------------------------------------------------------------
// Trip — top-level trip container
// ---------------------------------------------------------------------------
schema("Trip", #{
    version: 1,
    allowed_children_schemas: ["DayPlan", "Stay", "Transport", "Folder"],
    fields: [
        #{ name: "start_date", type: "date",     required: false },
        #{ name: "end_date",   type: "date",     required: false },
        #{ name: "notes",      type: "textarea", required: false },
    ],
});

// ---------------------------------------------------------------------------
// DayPlan — a single day in a trip
// ---------------------------------------------------------------------------
schema("DayPlan", #{
    version: 1,
    title_can_edit: false,
    allowed_children_schemas: ["Activity"],
    fields: [
        #{ name: "date",  type: "date",     required: true  },
        #{ name: "notes", type: "textarea", required: false },
    ],
    on_save: |note| {
        let d = note.fields["date"] ?? ();
        let title = if type_of(d) == "string" && d != "" {
            d
        } else {
            "Unscheduled Day"
        };
        set_title(note.id, title);
        commit();
    }
});

// ---------------------------------------------------------------------------
// Activity — a single item on a day, links to Location, Transport, or Stay
// ---------------------------------------------------------------------------
schema("Activity", #{
    version: 1,
    title_can_edit: false,
    is_leaf: true,
    fields: [
        #{ name: "time",        type: "text",      required: false            },
        #{ name: "description", type: "text",      required: false            },
        #{ name: "notes",       type: "textarea",  required: false            },
        #{ name: "location",    type: "note_link", required: false,
           target_schema: "Location"                                          },
        #{ name: "transport",   type: "note_link", required: false,
           target_schema: "Transport"                                         },
        #{ name: "stay",        type: "note_link", required: false,
           target_schema: "Stay"                                              },
    ],
    on_save: |note| {
        let time = note.fields["time"]        ?? "";
        let desc = note.fields["description"] ?? "";

        let resolved = "";
        let loc_id = note.fields["location"]  ?? ();
        let trn_id = note.fields["transport"] ?? ();
        let sty_id = note.fields["stay"]      ?? ();

        if loc_id != () {
            let n = get_note(loc_id);
            if n != () { resolved = n.fields["name"] ?? n.title; }
        } else if trn_id != () {
            let n = get_note(trn_id);
            if n != () { resolved = n.title; }
        } else if sty_id != () {
            let n = get_note(sty_id);
            if n != () { resolved = n.title; }
        }

        let label = if resolved != "" {
            resolved
        } else if desc != "" {
            desc
        } else {
            "Untitled Activity"
        };

        let title = if time != "" { time + " — " + label } else { label };
        set_title(note.id, title);
        commit();
    }
});

// ---------------------------------------------------------------------------
// Task — a planning checklist item
// ---------------------------------------------------------------------------
schema("Task", #{
    version: 1,
    show_checkbox: true,
    is_leaf: true,
    fields: [
        #{ name: "due_date",  type: "date",      required: false            },
        #{ name: "category",  type: "select",    required: false,
           options: ["Booking", "Visa", "Insurance", "Packing", "Other"]    },
        #{ name: "cost",      type: "number",    required: false            },
        #{ name: "currency",  type: "select",    required: false,
           options: ["USD", "EUR", "GBP", "JPY", "AUD", "Other"]           },
        #{ name: "notes",     type: "textarea",  required: false            },
        #{ name: "location",  type: "note_link", required: false,
           target_schema: "Location"                                        },
        #{ name: "stay",      type: "note_link", required: false,
           target_schema: "Stay"                                            },
        #{ name: "transport", type: "note_link", required: false,
           target_schema: "Transport"                                       },
    ],
});
```

- [ ] **Step 2: Verify the complete schema file**

Run: `grep -c "^schema(" example-scripts/travel-planner/travel-planner.schema.rhai`
Expected: `8` (Folder, Location, Stay, Transport, Trip, DayPlan, Activity, Task)

- [ ] **Step 3: Commit**

```bash
git add example-scripts/travel-planner/travel-planner.schema.rhai
git commit -m "feat(example): travel-planner schemas — Transport, Trip, DayPlan, Activity, Task"
```

---

## Task 3: Presentation file — Views

**Files:**
- Create: `example-scripts/travel-planner/travel-planner.rhai`

- [ ] **Step 1: Create presentation file with all views**

```rhai
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Travel Planner Views
// @description: Views, hover previews, and sort actions for the Travel Planner schemas.

// ===== Views =====

// --- Trip — Overview ---
register_view("Trip", "Overview", #{ display_first: true }, |note| {
    let start = note.fields["start_date"] ?? ();
    let end   = note.fields["end_date"]   ?? ();
    let parts = [];

    // Date range
    if type_of(start) == "string" && start != "" {
        let range = if type_of(end) == "string" && end != "" {
            start + " → " + end
        } else {
            "From " + start
        };
        parts += [field("Dates", range)];
    }

    // Planning progress — find Task notes under children (including inside Folder children)
    let children = get_children(note.id);
    let tasks = [];
    for child in children {
        if child.schema == "Task" {
            tasks += [child];
        } else if child.schema == "Folder" {
            let grandchildren = get_children(child.id);
            for gc in grandchildren {
                if gc.schema == "Task" { tasks += [gc]; }
            }
        }
    }
    if tasks.len() > 0 {
        let done = tasks.filter(|t| t.is_checked).len();
        parts += [field("Planning", done.to_string() + "/" + tasks.len().to_string() + " tasks done")];
    }

    // Day-by-day summary
    let days = children.filter(|c| c.schema == "DayPlan");
    days.sort_by(|a, b| (a.fields["date"] ?? "") <= (b.fields["date"] ?? ""));

    if days.len() > 0 {
        parts += [divider()];
        for day in days {
            let day_children = get_children(day.id);
            day_children.sort_by(|a, b| (a.fields["time"] ?? a.fields["departure_time"] ?? "") <= (b.fields["time"] ?? b.fields["departure_time"] ?? ""));
            let bullets = day_children.map(|c| "• " + c.title);
            let body = if bullets.len() > 0 { bullets.reduce(|a, b| a + "\n" + b) } else { "(no activities)" };
            parts += [section(day.title, text(body))];
        }
    }

    if parts.len() == 0 { return text("Empty trip. Add DayPlans and Activities to get started."); }
    stack(parts)
});

// --- DayPlan — Timeline ---
register_view("DayPlan", "Timeline", #{ display_first: true }, |note| {
    let children = get_children(note.id);
    if children.len() == 0 {
        return text("No activities yet. Right-click to add an Activity.");
    }

    children.sort_by(|a, b| (a.fields["time"] ?? "") <= (b.fields["time"] ?? ""));

    let rows = children.map(|c| {
        let time = c.fields["time"] ?? "";
        let desc = c.fields["description"] ?? "";

        let destination = "";
        let loc_id = c.fields["location"]  ?? ();
        let trn_id = c.fields["transport"] ?? ();
        let sty_id = c.fields["stay"]      ?? ();
        if loc_id != () {
            let n = get_note(loc_id);
            if n != () { destination = n.fields["name"] ?? n.title; }
        } else if trn_id != () {
            let n = get_note(trn_id);
            if n != () { destination = n.title; }
        } else if sty_id != () {
            let n = get_note(sty_id);
            if n != () { destination = n.title; }
        }

        let label = if destination != "" && desc != "" {
            destination + " — " + desc
        } else if destination != "" {
            destination
        } else {
            desc
        };

        [time, label]
    });

    table(["Time", "Activity"], rows)
});

// --- Location — Details ---
register_view("Location", "Details", #{ display_first: true }, |note| {
    let parts = [];
    let photo = note.fields["photo"] ?? ();
    if photo != () { parts += [display_image(photo, 480, note.fields["name"] ?? "")]; }

    let category = note.fields["category"] ?? "";
    if category != "" { parts += [field("Category", category)]; }

    let address = note.fields["address"] ?? "";
    if address != "" { parts += [field("Address", address)]; }

    let rating = note.fields["rating"] ?? 0;
    if rating > 0 { parts += [field("Rating", stars(rating))]; }

    let notes = note.fields["notes"] ?? "";
    if notes != "" { parts += [divider(), markdown(notes)]; }

    if parts.len() == 0 { return text("No details yet."); }
    stack(parts)
});

// --- Stay — Booking ---
register_view("Stay", "Booking", #{ display_first: true }, |note| {
    let parts = [];

    let hotel_id = note.fields["hotel"] ?? ();
    if hotel_id != () {
        let h = get_note(hotel_id);
        if h != () {
            let photo = h.fields["photo"] ?? ();
            if photo != () { parts += [display_image(photo, 400, h.fields["name"] ?? "")]; }
            parts += [field("Hotel", link_to(h))];
        }
    }

    let ci = note.fields["check_in"]  ?? ();
    let co = note.fields["check_out"] ?? ();
    if type_of(ci) == "string" && ci != "" {
        let dates = if type_of(co) == "string" && co != "" { ci + " → " + co } else { ci };
        parts += [field("Dates", dates)];
    }

    let ppn = note.fields["price_per_night"] ?? 0;
    let cur = note.fields["currency"] ?? "";
    if ppn > 0 {
        let price_str = ppn.to_string() + if cur != "" { " " + cur } else { "" } + " / night";
        parts += [field("Price", price_str)];
    }

    let ref_val = note.fields["booking_ref"] ?? "";
    if ref_val != "" { parts += [field("Booking Ref", ref_val)]; }

    let notes = note.fields["notes"] ?? "";
    if notes != "" { parts += [divider(), markdown(notes)]; }

    if parts.len() == 0 { return text("No booking details yet."); }
    stack(parts)
});

// --- Transport — Details ---
register_view("Transport", "Details", #{ display_first: true }, |note| {
    let parts = [];

    let t    = note.fields["type"]      ?? "";
    let from = note.fields["from_city"] ?? "";
    let to   = note.fields["to_city"]   ?? "";
    if t != "" { parts += [field("Type", t)]; }
    if from != "" && to != "" {
        parts += [field("Route", from + " → " + to)];
    } else if to != "" {
        parts += [field("To", to)];
    } else if from != "" {
        parts += [field("From", from)];
    }

    let dep_d = note.fields["departure_date"] ?? ();
    let dep_t = note.fields["departure_time"] ?? "";
    let arr_d = note.fields["arrival_date"]   ?? ();
    let arr_t = note.fields["arrival_time"]   ?? "";
    if type_of(dep_d) == "string" && dep_d != "" {
        let dep = dep_d + if dep_t != "" { " " + dep_t } else { "" };
        parts += [field("Departure", dep)];
    }
    if type_of(arr_d) == "string" && arr_d != "" {
        let arr = arr_d + if arr_t != "" { " " + arr_t } else { "" };
        parts += [field("Arrival", arr)];
    }

    let carrier = note.fields["carrier"] ?? "";
    if carrier != "" { parts += [field("Carrier", carrier)]; }

    let ref_val = note.fields["booking_ref"] ?? "";
    if ref_val != "" { parts += [field("Booking Ref", ref_val)]; }

    let price = note.fields["price"]    ?? 0;
    let cur   = note.fields["currency"] ?? "";
    if price > 0 {
        parts += [field("Price", price.to_string() + if cur != "" { " " + cur } else { "" })];
    }

    let notes = note.fields["notes"] ?? "";
    if notes != "" { parts += [divider(), markdown(notes)]; }

    if parts.len() == 0 { return text("No transport details yet."); }
    stack(parts)
});

// --- Folder — Contents ---
register_view("Folder", "Contents", #{ display_first: true }, |note| {
    let children = get_children(note.id);
    if children.len() == 0 {
        return text("Empty folder.");
    }
    let rows = children.map(|c| [c.title, c.schema]);
    table(["Name", "Type"], rows)
});
```

- [ ] **Step 2: Verify all 7 views registered**

Run: `grep -c "register_view" example-scripts/travel-planner/travel-planner.rhai`
Expected: `6` (Trip, DayPlan, Location, Stay, Transport, Folder)

- [ ] **Step 3: Commit**

```bash
git add example-scripts/travel-planner/travel-planner.rhai
git commit -m "feat(example): travel-planner views — Trip, DayPlan, Location, Stay, Transport, Folder"
```

---

## Task 4: Presentation file — Hovers and Context Menus

**Files:**
- Modify: `example-scripts/travel-planner/travel-planner.rhai` (append after views)

- [ ] **Step 1: Append hover tooltips and context menu actions**

Append the following after the Folder view:

```rhai

// ===== Hover Tooltips =====

register_hover("Location", |note| {
    let category = note.fields["category"] ?? "";
    let city     = note.fields["city"]     ?? "";
    let country  = note.fields["country"]  ?? "";
    let rating   = note.fields["rating"]   ?? 0;
    let parts = [];
    if category != "" { parts += [field("Category", category)]; }
    let loc = if city != "" && country != "" { city + ", " + country }
              else if city != "" { city }
              else { country };
    if loc != "" { parts += [field("Location", loc)]; }
    if rating > 0 { parts += [field("Rating", stars(rating))]; }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});

register_hover("Stay", |note| {
    let hotel_id = note.fields["hotel"] ?? ();
    let hotel_name = if hotel_id != () {
        let h = get_note(hotel_id);
        if h != () { h.fields["name"] ?? h.title } else { "-" }
    } else { "-" };
    let ci = note.fields["check_in"]  ?? ();
    let co = note.fields["check_out"] ?? ();
    let dates = if type_of(ci) == "string" && ci != "" && type_of(co) == "string" && co != "" {
        ci + " → " + co
    } else if type_of(ci) == "string" && ci != "" {
        "from " + ci
    } else { "-" };
    stack([field("Hotel", hotel_name), field("Dates", dates)])
});

register_hover("Transport", |note| {
    let t    = note.fields["type"]      ?? "";
    let from = note.fields["from_city"] ?? "";
    let to   = note.fields["to_city"]   ?? "";
    let dep_d = note.fields["departure_date"] ?? ();
    let dep_t = note.fields["departure_time"] ?? "";
    let parts = [];
    if t != "" { parts += [field("Type", t)]; }
    let route = if from != "" && to != "" { from + " → " + to }
                else if to != "" { "→ " + to }
                else { "" };
    if route != "" { parts += [field("Route", route)]; }
    if type_of(dep_d) == "string" && dep_d != "" {
        let dep = dep_d + if dep_t != "" { " " + dep_t } else { "" };
        parts += [field("Departure", dep)];
    }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});

register_hover("Trip", |note| {
    let start = note.fields["start_date"] ?? ();
    let end   = note.fields["end_date"]   ?? ();
    let parts = [];
    if type_of(start) == "string" && start != "" {
        let range = if type_of(end) == "string" && end != "" { start + " → " + end } else { start };
        parts += [field("Dates", range)];
    }
    let children = get_children(note.id);
    let tasks = [];
    for child in children {
        if child.schema == "Task" { tasks += [child]; }
        else if child.schema == "Folder" {
            let gcs = get_children(child.id);
            for gc in gcs { if gc.schema == "Task" { tasks += [gc]; } }
        }
    }
    if tasks.len() > 0 {
        let done = tasks.filter(|t| t.is_checked).len();
        parts += [field("Planning", done.to_string() + "/" + tasks.len().to_string() + " done")];
    }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});

register_hover("DayPlan", |note| {
    let d = note.fields["date"] ?? "";
    let children = get_children(note.id);
    let parts = [];
    if d != "" { parts += [field("Date", d)]; }
    parts += [field("Activities", children.len().to_string())];
    stack(parts)
});

register_hover("Activity", |note| {
    let time = note.fields["time"] ?? "";
    let dest = "";
    let loc_id = note.fields["location"]  ?? ();
    let trn_id = note.fields["transport"] ?? ();
    let sty_id = note.fields["stay"]      ?? ();
    if loc_id != () {
        let n = get_note(loc_id);
        if n != () { dest = n.fields["name"] ?? n.title; }
    } else if trn_id != () {
        let n = get_note(trn_id);
        if n != () { dest = n.title; }
    } else if sty_id != () {
        let n = get_note(sty_id);
        if n != () { dest = n.title; }
    }
    let parts = [];
    if time != "" { parts += [field("Time", time)]; }
    if dest != "" { parts += [field("Destination", dest)]; }
    if parts.len() == 0 { return text(note.title); }
    stack(parts)
});

register_hover("Task", |note| {
    let due      = note.fields["due_date"]  ?? ();
    let category = note.fields["category"]  ?? "";
    let parts = [];
    if type_of(due) == "string" && due != "" { parts += [field("Due", due)]; }
    if category != "" { parts += [field("Category", category)]; }
    parts += [field("Status", if note.is_checked { "Done" } else { "Pending" })];
    stack(parts)
});

// ===== Context Menu Actions =====

register_menu("Sort by Time", ["DayPlan"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| (a.fields["time"] ?? "") <= (b.fields["time"] ?? ""));
    children.map(|c| c.id)
});

register_menu("Sort by Date", ["Trip"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| {
        let da = if a.schema == "DayPlan" { a.fields["date"] ?? "" } else { "" };
        let db = if b.schema == "DayPlan" { b.fields["date"] ?? "" } else { "" };
        da <= db
    });
    children.map(|c| c.id)
});
```

- [ ] **Step 2: Verify hover and menu counts**

Run: `grep -c "register_hover" example-scripts/travel-planner/travel-planner.rhai`
Expected: `7` (Location, Stay, Transport, Trip, DayPlan, Activity, Task)

Run: `grep -c "register_menu" example-scripts/travel-planner/travel-planner.rhai`
Expected: `2` (Sort by Time, Sort by Date)

- [ ] **Step 3: Commit**

```bash
git add example-scripts/travel-planner/travel-planner.rhai
git commit -m "feat(example): travel-planner hovers and context menus"
```

---

## Task 5: Sample archive — notes.json

Build the .krillnotes archive with sample Japan trip data. This task creates all JSON files, the next task zips them.

**Files:**
- Create: `/tmp/travel-planner-archive/notes.json`
- Create: `/tmp/travel-planner-archive/workspace.json`
- Create: `/tmp/travel-planner-archive/scripts/scripts.json`
- Copy: schema and presentation scripts into `/tmp/travel-planner-archive/scripts/`

- [ ] **Step 1: Create workspace.json**

```bash
mkdir -p /tmp/travel-planner-archive/scripts
```

Write `/tmp/travel-planner-archive/workspace.json`:

```json
{
  "version": 1,
  "description": "Sample Japan trip — 5 locations, 1 stay, 2 transport legs, 3 day plans with activities, and a planning checklist.",
  "tags": [
    "travel",
    "japan",
    "example"
  ]
}
```

- [ ] **Step 2: Create scripts.json**

Write `/tmp/travel-planner-archive/scripts/scripts.json`:

```json
{
  "scripts": [
    {
      "filename": "text-note.schema.rhai",
      "loadOrder": -1,
      "enabled": true,
      "category": "schema"
    },
    {
      "filename": "travel-planner-schema.rhai",
      "loadOrder": 0,
      "enabled": true,
      "category": "schema"
    },
    {
      "filename": "travel-planner-views.rhai",
      "loadOrder": 1,
      "enabled": true,
      "category": "library"
    }
  ]
}
```

- [ ] **Step 3: Copy scripts into archive**

```bash
cp example-scripts/travel-planner/travel-planner.schema.rhai /tmp/travel-planner-archive/scripts/travel-planner-schema.rhai
cp example-scripts/travel-planner/travel-planner.rhai /tmp/travel-planner-archive/scripts/travel-planner-views.rhai
```

Also copy the system TextNote schema:

```bash
cp krillnotes-core/src/system_scripts/00_text_note.schema.rhai /tmp/travel-planner-archive/scripts/text-note.schema.rhai
```

- [ ] **Step 4: Create notes.json**

Write `/tmp/travel-planner-archive/notes.json`. UUIDs and timestamps are pre-generated. Timestamp `1778486400` = 2026-05-10T00:00:00Z (approximate trip era).

The note tree:

| ID (short prefix) | Title | Schema | Parent |
|---|---|---|---|
| `res-00000` | Resources | Folder | null |
| `htl-00000` | Hotels | Folder | res-00000 |
| `htl-jp000` | Japan | Folder | htl-00000 |
| `loc-hyatt` | Park Hyatt Tokyo (Tokyo) | Location | htl-jp000 |
| `rst-00000` | Restaurants | Folder | res-00000 |
| `rst-jp000` | Japan | Folder | rst-00000 |
| `loc-ichir` | Ichiran Shibuya (Tokyo) | Location | rst-jp000 |
| `att-00000` | Attractions | Folder | res-00000 |
| `att-jp000` | Japan | Folder | att-00000 |
| `loc-fushi` | Fushimi Inari (Kyoto) | Location | att-jp000 |
| `loc-meiji` | Meiji Shrine (Tokyo) | Location | att-jp000 |
| `loc-bambo` | Arashiyama Bamboo Grove (Kyoto) | Location | att-jp000 |
| `trp-00000` | Trips | Folder | null |
| `trip-jpn0` | Japan 2026 | Trip | trp-00000 |
| `plan-0000` | Planning | Folder | trip-jpn0 |
| `task-flt0` | Book flights SYD→NRT | Task | plan-0000 |
| `task-htl0` | Reserve Park Hyatt | Task | plan-0000 |
| `task-ghibli` | Buy Ghibli Museum tickets | Task | plan-0000 |
| `task-rail` | Buy JR Rail Pass | Task | plan-0000 |
| `acco-0000` | Accommodation | Folder | trip-jpn0 |
| `stay-hyat` | Park Hyatt Tokyo (2026-05-11 – 2026-05-14) | Stay | acco-0000 |
| `tran-0000` | Transport | Folder | trip-jpn0 |
| `tran-flt0` | Flight Sydney → Tokyo | Transport | tran-0000 |
| `tran-shin` | Train Kyoto → Tokyo | Transport | tran-0000 |
| `day1-0000` | 2026-05-10 | DayPlan | trip-jpn0 |
| `act-fushi` | 09:00 — Fushimi Inari (Kyoto) | Activity | day1-0000 |
| `act-bamboo` | 11:00 — Arashiyama Bamboo Grove (Kyoto) | Activity | day1-0000 |
| `act-ichir` | 12:30 — Ichiran Shibuya (Tokyo) | Activity | day1-0000 |
| `day2-0000` | 2026-05-11 | DayPlan | trip-jpn0 |
| `act-train` | 08:30 — Train Kyoto → Tokyo | Activity | day2-0000 |
| `act-check` | 15:00 — Check in Park Hyatt | Activity | day2-0000 |
| `day3-0000` | 2026-05-12 | DayPlan | trip-jpn0 |
| `act-meiji` | 10:00 — Meiji Shrine (Tokyo) | Activity | day3-0000 |

```json
{
  "version": 1,
  "appVersion": "0.9.2",
  "notes": [
    {
      "id": "a0000001-0000-4000-8000-000000000001",
      "title": "Resources",
      "schema": "Folder",
      "parentId": null,
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000002",
      "title": "Hotels",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000001",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000003",
      "title": "Japan",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000002",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "b0000001-0000-4000-8000-000000000001",
      "title": "Park Hyatt Tokyo (Tokyo)",
      "schema": "Location",
      "parentId": "a0000001-0000-4000-8000-000000000003",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "name":     { "Text": "Park Hyatt Tokyo" },
        "city":     { "Text": "Tokyo" },
        "country":  { "Text": "Japan" },
        "category": { "Text": "Hotel" },
        "address":  { "Text": "3-7-1-2 Nishi-Shinjuku, Shinjuku-ku" },
        "rating":   { "Number": 5.0 },
        "notes":    { "Text": "Iconic hotel from Lost in Translation. Excellent views from the 52nd floor bar." }
      },
      "isExpanded": false,
      "tags": ["japan", "tokyo"],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000004",
      "title": "Restaurants",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000001",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000005",
      "title": "Japan",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000004",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "b0000001-0000-4000-8000-000000000002",
      "title": "Ichiran Shibuya (Tokyo)",
      "schema": "Location",
      "parentId": "a0000001-0000-4000-8000-000000000005",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "name":     { "Text": "Ichiran Shibuya" },
        "city":     { "Text": "Tokyo" },
        "country":  { "Text": "Japan" },
        "category": { "Text": "Restaurant" },
        "address":  { "Text": "1-22-7 Jinnan, Shibuya-ku" },
        "rating":   { "Number": 4.0 },
        "notes":    { "Text": "Famous tonkotsu ramen chain. Individual booths for focused eating." }
      },
      "isExpanded": false,
      "tags": ["japan", "tokyo"],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000006",
      "title": "Attractions",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000001",
      "position": 2.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000001-0000-4000-8000-000000000007",
      "title": "Japan",
      "schema": "Folder",
      "parentId": "a0000001-0000-4000-8000-000000000006",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "b0000001-0000-4000-8000-000000000003",
      "title": "Fushimi Inari (Kyoto)",
      "schema": "Location",
      "parentId": "a0000001-0000-4000-8000-000000000007",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "name":     { "Text": "Fushimi Inari" },
        "city":     { "Text": "Kyoto" },
        "country":  { "Text": "Japan" },
        "category": { "Text": "Temple" },
        "address":  { "Text": "68 Fukakusa Yabunouchicho, Fushimi-ku" },
        "rating":   { "Number": 5.0 },
        "notes":    { "Text": "Thousands of vermilion torii gates. Best visited early morning to avoid crowds." }
      },
      "isExpanded": false,
      "tags": ["japan", "kyoto"],
      "schemaVersion": 1
    },
    {
      "id": "b0000001-0000-4000-8000-000000000004",
      "title": "Meiji Shrine (Tokyo)",
      "schema": "Location",
      "parentId": "a0000001-0000-4000-8000-000000000007",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "name":     { "Text": "Meiji Shrine" },
        "city":     { "Text": "Tokyo" },
        "country":  { "Text": "Japan" },
        "category": { "Text": "Temple" },
        "address":  { "Text": "1-1 Yoyogikamizonocho, Shibuya-ku" },
        "rating":   { "Number": 4.0 },
        "notes":    { "Text": "Serene Shinto shrine in a forested park near Harajuku." }
      },
      "isExpanded": false,
      "tags": ["japan", "tokyo"],
      "schemaVersion": 1
    },
    {
      "id": "b0000001-0000-4000-8000-000000000005",
      "title": "Arashiyama Bamboo Grove (Kyoto)",
      "schema": "Location",
      "parentId": "a0000001-0000-4000-8000-000000000007",
      "position": 2.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "name":     { "Text": "Arashiyama Bamboo Grove" },
        "city":     { "Text": "Kyoto" },
        "country":  { "Text": "Japan" },
        "category": { "Text": "Park" },
        "address":  { "Text": "Sagaogurayama Tabuchiyamacho, Ukyo-ku" },
        "rating":   { "Number": 4.0 },
        "notes":    { "Text": "Towering bamboo stalks create an otherworldly atmosphere. Walk through to the monkey park." }
      },
      "isExpanded": false,
      "tags": ["japan", "kyoto"],
      "schemaVersion": 1
    },
    {
      "id": "a0000002-0000-4000-8000-000000000001",
      "title": "Trips",
      "schema": "Folder",
      "parentId": null,
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "c0000001-0000-4000-8000-000000000001",
      "title": "Japan 2026",
      "schema": "Trip",
      "parentId": "a0000002-0000-4000-8000-000000000001",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "start_date": { "Date": "2026-05-10" },
        "end_date":   { "Date": "2026-05-14" },
        "notes":      { "Text": "Golden Week trip — Kyoto then Tokyo." }
      },
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000003-0000-4000-8000-000000000001",
      "title": "Planning",
      "schema": "Folder",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "e0000001-0000-4000-8000-000000000001",
      "title": "Book flights SYD→NRT",
      "schema": "Task",
      "parentId": "a0000003-0000-4000-8000-000000000001",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "due_date": { "Date": "2026-03-01" },
        "category": { "Text": "Booking" },
        "cost":     { "Number": 1850.0 },
        "currency": { "Text": "AUD" },
        "notes":    { "Text": "" },
        "transport": { "Text": "d0000001-0000-4000-8000-000000000001" }
      },
      "isExpanded": false,
      "isChecked": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "e0000001-0000-4000-8000-000000000002",
      "title": "Reserve Park Hyatt",
      "schema": "Task",
      "parentId": "a0000003-0000-4000-8000-000000000001",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "due_date": { "Date": "2026-04-01" },
        "category": { "Text": "Booking" },
        "cost":     { "Number": 1200.0 },
        "currency": { "Text": "AUD" },
        "notes":    { "Text": "" },
        "stay":     { "Text": "d0000002-0000-4000-8000-000000000001" }
      },
      "isExpanded": false,
      "isChecked": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "e0000001-0000-4000-8000-000000000003",
      "title": "Buy Ghibli Museum tickets",
      "schema": "Task",
      "parentId": "a0000003-0000-4000-8000-000000000001",
      "position": 2.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "due_date": { "Date": "2026-04-10" },
        "category": { "Text": "Booking" },
        "cost":     { "Number": 1000.0 },
        "currency": { "Text": "JPY" },
        "notes":    { "Text": "Tickets sell out months in advance — book on the 10th of the prior month." }
      },
      "isExpanded": false,
      "isChecked": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "e0000001-0000-4000-8000-000000000004",
      "title": "Buy JR Rail Pass",
      "schema": "Task",
      "parentId": "a0000003-0000-4000-8000-000000000001",
      "position": 3.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "due_date": { "Date": "2026-04-15" },
        "category": { "Text": "Booking" },
        "cost":     { "Number": 50000.0 },
        "currency": { "Text": "JPY" },
        "notes":    { "Text": "7-day pass covers the Shinkansen between Kyoto and Tokyo." }
      },
      "isExpanded": false,
      "isChecked": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000003-0000-4000-8000-000000000002",
      "title": "Accommodation",
      "schema": "Folder",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "d0000002-0000-4000-8000-000000000001",
      "title": "Park Hyatt Tokyo (2026-05-11 – 2026-05-14)",
      "schema": "Stay",
      "parentId": "a0000003-0000-4000-8000-000000000002",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "hotel":           { "Text": "b0000001-0000-4000-8000-000000000001" },
        "check_in":        { "Date": "2026-05-11" },
        "check_out":       { "Date": "2026-05-14" },
        "price_per_night": { "Number": 65000.0 },
        "currency":        { "Text": "JPY" },
        "booking_ref":     { "Text": "" },
        "notes":           { "Text": "Request a room with Mt Fuji view." }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "a0000003-0000-4000-8000-000000000003",
      "title": "Transport",
      "schema": "Folder",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 2.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {},
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "d0000001-0000-4000-8000-000000000001",
      "title": "Flight Sydney → Tokyo",
      "schema": "Transport",
      "parentId": "a0000003-0000-4000-8000-000000000003",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "type":           { "Text": "Flight" },
        "from_city":      { "Text": "Sydney" },
        "to_city":        { "Text": "Tokyo" },
        "departure_date": { "Date": "2026-05-09" },
        "departure_time": { "Text": "21:30" },
        "arrival_date":   { "Date": "2026-05-10" },
        "arrival_time":   { "Text": "05:30" },
        "carrier":        { "Text": "Qantas QF21" },
        "booking_ref":    { "Text": "QF-ABC123" },
        "price":          { "Number": 1850.0 },
        "currency":       { "Text": "AUD" },
        "notes":          { "Text": "Direct flight, arrives Narita. Take Haruka Express to Kyoto Station." }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "d0000001-0000-4000-8000-000000000002",
      "title": "Train Kyoto → Tokyo",
      "schema": "Transport",
      "parentId": "a0000003-0000-4000-8000-000000000003",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "type":           { "Text": "Train" },
        "from_city":      { "Text": "Kyoto" },
        "to_city":        { "Text": "Tokyo" },
        "departure_date": { "Date": "2026-05-11" },
        "departure_time": { "Text": "08:30" },
        "arrival_date":   { "Date": "2026-05-11" },
        "arrival_time":   { "Text": "10:45" },
        "carrier":        { "Text": "Nozomi Shinkansen" },
        "booking_ref":    { "Text": "" },
        "price":          { "Number": 0.0 },
        "currency":       { "Text": "JPY" },
        "notes":          { "Text": "Covered by JR Rail Pass. Reserved seat car 7." }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000001-0000-4000-8000-000000000001",
      "title": "2026-05-10",
      "schema": "DayPlan",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 3.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "date":  { "Date": "2026-05-10" },
        "notes": { "Text": "First day — explore Kyoto." }
      },
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000001",
      "title": "09:00 — Fushimi Inari (Kyoto)",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000001",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "09:00" },
        "description": { "Text": "" },
        "notes":       { "Text": "" },
        "location":    { "Text": "b0000001-0000-4000-8000-000000000003" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000002",
      "title": "11:00 — Arashiyama Bamboo Grove (Kyoto)",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000001",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "11:00" },
        "description": { "Text": "" },
        "notes":       { "Text": "" },
        "location":    { "Text": "b0000001-0000-4000-8000-000000000005" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000003",
      "title": "12:30 — Ichiran Shibuya (Tokyo)",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000001",
      "position": 2.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "12:30" },
        "description": { "Text": "Lunch" },
        "notes":       { "Text": "" },
        "location":    { "Text": "b0000001-0000-4000-8000-000000000002" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000001-0000-4000-8000-000000000002",
      "title": "2026-05-11",
      "schema": "DayPlan",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 4.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "date":  { "Date": "2026-05-11" },
        "notes": { "Text": "Travel day — Shinkansen to Tokyo, check in." }
      },
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000004",
      "title": "08:30 — Train Kyoto → Tokyo",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000002",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "08:30" },
        "description": { "Text": "Depart" },
        "notes":       { "Text": "" },
        "transport":   { "Text": "d0000001-0000-4000-8000-000000000002" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000005",
      "title": "15:00 — Check in Park Hyatt",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000002",
      "position": 1.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "15:00" },
        "description": { "Text": "Check in" },
        "notes":       { "Text": "" },
        "stay":        { "Text": "d0000002-0000-4000-8000-000000000001" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000001-0000-4000-8000-000000000003",
      "title": "2026-05-12",
      "schema": "DayPlan",
      "parentId": "c0000001-0000-4000-8000-000000000001",
      "position": 5.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "date":  { "Date": "2026-05-12" },
        "notes": { "Text": "Explore Tokyo." }
      },
      "isExpanded": true,
      "tags": [],
      "schemaVersion": 1
    },
    {
      "id": "f0000002-0000-4000-8000-000000000006",
      "title": "10:00 — Meiji Shrine (Tokyo)",
      "schema": "Activity",
      "parentId": "f0000001-0000-4000-8000-000000000003",
      "position": 0.0,
      "createdAt": 1778486400,
      "modifiedAt": 1778486400,
      "createdBy": "",
      "modifiedBy": "",
      "fields": {
        "time":        { "Text": "10:00" },
        "description": { "Text": "" },
        "notes":       { "Text": "" },
        "location":    { "Text": "b0000001-0000-4000-8000-000000000004" }
      },
      "isExpanded": false,
      "tags": [],
      "schemaVersion": 1
    }
  ]
}
```

- [ ] **Step 5: Commit notes.json and support files**

No git commit for temp files — they'll be zipped in the next task.

---

## Task 6: Build .krillnotes archive

**Files:**
- Create: `example-scripts/travel-planner/travel-planner.krillnotes`

- [ ] **Step 1: Create the zip archive from temp directory**

```bash
cd /tmp/travel-planner-archive && zip -r /path/to/project/example-scripts/travel-planner/travel-planner.krillnotes notes.json workspace.json scripts/
```

- [ ] **Step 2: Verify archive contents**

```bash
unzip -l example-scripts/travel-planner/travel-planner.krillnotes
```

Expected: `notes.json`, `workspace.json`, `scripts/text-note.schema.rhai`, `scripts/travel-planner-schema.rhai`, `scripts/travel-planner-views.rhai`, `scripts/scripts.json`

- [ ] **Step 3: Verify note count and key schema references**

```bash
unzip -p example-scripts/travel-planner/travel-planner.krillnotes notes.json | grep -c '"id"'
```

Expected: `33` notes total (11 Folders + 5 Locations + 1 Stay + 2 Transports + 1 Trip + 3 DayPlans + 6 Activities + 4 Tasks)

```bash
unzip -p example-scripts/travel-planner/travel-planner.krillnotes notes.json | grep -c '"note_link"\|"target_schema"'
```

Expected: 0 (note_link fields are serialized as `{"Text": "uuid"}` in the JSON, not as structured objects — the schema definition handles the semantics)

- [ ] **Step 4: Commit the archive**

```bash
git add example-scripts/travel-planner/travel-planner.krillnotes
git commit -m "feat(example): travel-planner sample archive — Japan 2026 trip"
```

- [ ] **Step 5: Clean up temp directory**

```bash
rm -rf /tmp/travel-planner-archive
```

---

## Task 7: Final verification

- [ ] **Step 1: Verify complete file structure**

```bash
ls -la example-scripts/travel-planner/
```

Expected: 3 files — `travel-planner.schema.rhai`, `travel-planner.rhai`, `travel-planner.krillnotes`

- [ ] **Step 2: Verify schema count**

```bash
grep -c "^schema(" example-scripts/travel-planner/travel-planner.schema.rhai
```

Expected: `8`

- [ ] **Step 3: Verify presentation counts**

```bash
grep -c "register_view" example-scripts/travel-planner/travel-planner.rhai
grep -c "register_hover" example-scripts/travel-planner/travel-planner.rhai
grep -c "register_menu" example-scripts/travel-planner/travel-planner.rhai
```

Expected: `6`, `7`, `2`

- [ ] **Step 4: Verify key features are showcased**

```bash
grep -c "note_link" example-scripts/travel-planner/travel-planner.schema.rhai
grep -c "type: \"file\"" example-scripts/travel-planner/travel-planner.schema.rhai
grep "show_checkbox" example-scripts/travel-planner/travel-planner.schema.rhai
grep "is_leaf" example-scripts/travel-planner/travel-planner.schema.rhai
grep "allowed_children_schemas" example-scripts/travel-planner/travel-planner.schema.rhai
grep "get_note" example-scripts/travel-planner/travel-planner.rhai
grep "display_image" example-scripts/travel-planner/travel-planner.rhai
grep "link_to" example-scripts/travel-planner/travel-planner.rhai
grep "stars" example-scripts/travel-planner/travel-planner.rhai
```

Expected: All commands produce output, confirming the example showcases note_link, file fields, show_checkbox, is_leaf, allowed_children_schemas, get_note, display_image, link_to, and stars.

- [ ] **Step 5: Run the app to smoke-test (manual)**

```bash
cd krillnotes-desktop && npm run tauri dev
```

Import the `travel-planner.krillnotes` archive and verify:
- Tree structure matches the spec
- Location view shows category, address, rating
- Stay view shows linked hotel name and dates
- Transport view shows route and times
- Trip overview shows date range, planning progress, day summaries
- DayPlan timeline shows sorted activities with resolved names
- Task checkboxes work
- Hovers show correct summary info
- Sort by Time/Date context menus work
