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

    let rows = [];
    let category = note.fields["category"] ?? "";
    if category != "" { rows += [["Category", category]]; }

    let address = note.fields["address"] ?? "";
    if address != "" { rows += [["Address", address]]; }

    let rating = note.fields["rating"] ?? 0;
    if rating > 0 { rows += [["Rating", stars(rating)]]; }

    if rows.len() > 0 { parts += [table(["", ""], rows)]; }

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
            parts += [table(["Hotel"], [[link_to(h)]])];
        }
    }

    let rows = [];
    let ci = note.fields["check_in"]  ?? ();
    let co = note.fields["check_out"] ?? ();
    if type_of(ci) == "string" && ci != "" {
        let dates = if type_of(co) == "string" && co != "" { ci + " → " + co } else { ci };
        rows += [["Dates", dates]];
    }

    let ppn = note.fields["price_per_night"] ?? 0;
    let cur = note.fields["currency"] ?? "";
    if ppn > 0 {
        let price_str = ppn.to_string() + if cur != "" { " " + cur } else { "" } + " / night";
        rows += [["Price", price_str]];
    }

    let ref_val = note.fields["booking_ref"] ?? "";
    if ref_val != "" { rows += [["Booking Ref", ref_val]]; }

    if rows.len() > 0 { parts += [table(["", ""], rows)]; }

    let notes = note.fields["notes"] ?? "";
    if notes != "" { parts += [divider(), markdown(notes)]; }

    if parts.len() == 0 { return text("No booking details yet."); }
    stack(parts)
});

// --- Transport — Details ---
register_view("Transport", "Details", #{ display_first: true }, |note| {
    let rows = [];

    let t    = note.fields["type"]      ?? "";
    let from = note.fields["from_city"] ?? "";
    let to   = note.fields["to_city"]   ?? "";
    if t != "" { rows += [["Type", t]]; }
    if from != "" && to != "" {
        rows += [["Route", from + " → " + to]];
    } else if to != "" {
        rows += [["To", to]];
    } else if from != "" {
        rows += [["From", from]];
    }

    let dep_d = note.fields["departure_date"] ?? ();
    let dep_t = note.fields["departure_time"] ?? "";
    let arr_d = note.fields["arrival_date"]   ?? ();
    let arr_t = note.fields["arrival_time"]   ?? "";
    if type_of(dep_d) == "string" && dep_d != "" {
        let dep = dep_d + if dep_t != "" { " " + dep_t } else { "" };
        rows += [["Departure", dep]];
    }
    if type_of(arr_d) == "string" && arr_d != "" {
        let arr = arr_d + if arr_t != "" { " " + arr_t } else { "" };
        rows += [["Arrival", arr]];
    }

    let carrier = note.fields["carrier"] ?? "";
    if carrier != "" { rows += [["Carrier", carrier]]; }

    let ref_val = note.fields["booking_ref"] ?? "";
    if ref_val != "" { rows += [["Booking Ref", ref_val]]; }

    let price = note.fields["price"]    ?? 0;
    let cur   = note.fields["currency"] ?? "";
    if price > 0 {
        rows += [["Price", price.to_string() + if cur != "" { " " + cur } else { "" }]];
    }

    let parts = [];
    if rows.len() > 0 { parts += [table(["", ""], rows)]; }

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
