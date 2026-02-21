//! Book — reading tracker with star rating and derived read duration.
//!
//! on_save hook:
//!   - Computes title as "Author: Book Title"
//!   - Derives `rating_stars` (view-only) from the rating field
//!   - Derives `read_duration` (view-only) from started + finished dates
//!     (shown as "N days"; blank if either date is missing,
//!      or blank if finished is on or before started)
//!
//! Note: date arithmetic uses a simplified approximation (year×365 + month×30 + day)
//! for display purposes — it is not calendar-accurate.

schema("Book", #{
    title_can_edit: false,
    fields: [
        #{ name: "book_title",    type: "text",     required: true                  },
        #{ name: "author",        type: "text",     required: true                  },
        #{ name: "genre",         type: "text",     required: false                 },
        #{ name: "status",        type: "select",   required: false,
           options: ["To Read", "Reading", "Read"]                                  },
        #{ name: "rating",        type: "rating",   required: false, max: 5         },
        #{ name: "started",       type: "date",     required: false                 },
        #{ name: "finished",      type: "date",     required: false                 },
        #{ name: "notes",         type: "textarea", required: false                 },
        #{ name: "rating_stars",  type: "text",     required: false, can_edit: false },
        #{ name: "read_duration", type: "text",     required: false, can_edit: false },
    ]
});

on_save("Book", |note| {
    let title  = note.fields["book_title"];
    let author = note.fields["author"];

    note.title = if author != "" && title != "" {
        author + ": " + title
    } else if title != "" {
        title
    } else {
        "Untitled Book"
    };

    // Star rating string
    // rating is stored as a float; 0.0 means "not rated"
    let r = note.fields["rating"];
    note.fields["rating_stars"] = if r <= 0.0 {
        ""
    } else {
        let filled = max(0, min(5, (r + 0.5).floor().to_int()));
        let empty  = 5 - filled;
        let stars  = "";
        for i in 0..filled { stars += "★"; }
        for i in 0..empty  { stars += "☆"; }
        stars
    };

    // Read duration from ISO date strings (YYYY-MM-DD)
    let started  = note.fields["started"];
    let finished = note.fields["finished"];
    note.fields["read_duration"] = if started != "" && finished != "" {
        let s_parts = started.split("-");
        let f_parts = finished.split("-");
        let s_days = parse_int(s_parts[0]) * 365 + parse_int(s_parts[1]) * 30 + parse_int(s_parts[2]);
        let f_days = parse_int(f_parts[0]) * 365 + parse_int(f_parts[1]) * 30 + parse_int(f_parts[2]);
        let diff = f_days - s_days;
        // diff > 0: show "N days". Same-day or reversed dates produce a blank.
        if diff > 0 { diff.to_string() + " days" } else { "" }
    } else {
        ""
    };

    note
});
