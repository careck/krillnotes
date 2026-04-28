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
