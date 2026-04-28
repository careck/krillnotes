# Travel Planner Example Script — Design Spec

**Date:** 2026-04-29
**Type:** Example script (no core changes)
**Output:** `example-scripts/travel-planner/` with `.schema.rhai`, `.rhai`, and `.krillnotes` archive

---

## Purpose

A travel planning example that showcases features no other example covers: `note_link` fields, multi-schema hierarchies, `show_checkbox`, `is_leaf`, `file` fields with `allowed_types`, tags for cross-cutting classification, and rich views that resolve linked notes. Demonstrates a realistic use case: planning trips with locations, stays, transport, day-by-day itineraries, and a planning task checklist.

---

## Data Model

### Schemas

#### Folder
Pure container for organising the tree. No fields.

```
version: 1
fields: []
```

#### Location
A place — hotel, restaurant, attraction, museum, etc. Self-contained with all descriptive data.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| name | text | yes | |
| city | text | yes | |
| country | text | yes | |
| category | select | yes | Hotel, Restaurant, Attraction, Museum, Temple, Park, Market, Bar, Cafe, Beach, Other |
| address | text | no | |
| rating | rating (max 5) | no | |
| notes | textarea | no | |
| photo | file | no | `allowed_types: ["image/jpeg", "image/png", "image/webp"]` |

- `title_can_edit: false` — title derived by `on_save` as `"name (city)"` or just `"name"` if city is empty.
- `is_leaf: true`

#### Stay
A hotel booking — links to a Location of type Hotel.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| check_in | date | no | |
| check_out | date | no | |
| price_per_night | number | no | |
| currency | select | no | USD, EUR, GBP, JPY, AUD, Other |
| booking_ref | text | no | |
| notes | textarea | no | |
| hotel | note_link | no | `target_schema: "Location"` |
| booking_confirmation | file | no | `allowed_types: ["application/pdf", "image/jpeg", "image/png"]` |

- `title_can_edit: false` — title derived by `on_save`: resolves linked hotel name + date range, e.g. `"Park Hyatt Tokyo (May 11–14)"`.
- `is_leaf: true`

#### Transport
A travel leg — flight, train, bus, etc.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| type | select | no | Flight, Train, Bus, Ferry, Car, Taxi, Walk, Other |
| from_city | text | no | |
| to_city | text | no | |
| departure_date | date | no | |
| departure_time | text | no | e.g. "14:30" |
| arrival_date | date | no | |
| arrival_time | text | no | |
| carrier | text | no | |
| booking_ref | text | no | |
| price | number | no | |
| currency | select | no | USD, EUR, GBP, JPY, AUD, Other |
| notes | textarea | no | |
| ticket | file | no | `allowed_types: ["application/pdf", "image/jpeg", "image/png"]` |

- `title_can_edit: false` — title derived by `on_save`: `"type from_city → to_city"` or `"type to_city"` if from_city is empty.

#### Trip
Top-level trip container.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| start_date | date | no | |
| end_date | date | no | |
| notes | textarea | no | |

- `allowed_children_schemas: ["DayPlan", "Stay", "Transport", "Folder"]` — DayPlans, Stays, Transport legs, and Folders (for Planning, Accommodation, Transport grouping).

#### DayPlan
A single day in a trip.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| date | date | yes | |
| notes | textarea | no | |

- `title_can_edit: false` — title derived by `on_save` from the date field, e.g. `"2026-05-10"`.
- `allowed_children_schemas: ["Activity"]`

#### Activity
A single item on a day — links to a Location, Transport, or Stay.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| time | text | no | e.g. "09:00" |
| description | text | no | optional label, e.g. "Lunch", "Check in", "Depart" |
| notes | textarea | no | |
| location | note_link | no | `target_schema: "Location"` |
| transport | note_link | no | `target_schema: "Transport"` |
| stay | note_link | no | `target_schema: "Stay"` |

- `title_can_edit: false` — title derived by `on_save`: `"time — resolved_name"` or `"time — description"`.
- `is_leaf: true`

#### Task
A planning checklist item.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| due_date | date | no | |
| category | select | no | Booking, Visa, Insurance, Packing, Other |
| cost | number | no | |
| currency | select | no | USD, EUR, GBP, JPY, AUD, Other |
| notes | textarea | no | |
| location | note_link | no | `target_schema: "Location"` |
| stay | note_link | no | `target_schema: "Stay"` |
| transport | note_link | no | `target_schema: "Transport"` |

- `show_checkbox: true`
- `is_leaf: true`

---

## Tree Structure

```
Resources/                              (Folder)
├── Hotels/                             (Folder)
│   └── Japan/                          (Folder)
│       └── Park Hyatt Tokyo            (Location, category: Hotel)
├── Restaurants/                        (Folder)
│   └── Japan/                          (Folder)
│       └── Ichiran Shibuya             (Location, category: Restaurant)
└── Attractions/                        (Folder)
    └── Japan/                          (Folder)
        ├── Fushimi Inari               (Location, category: Temple)
        └── Meiji Shrine                (Location, category: Temple)

Trips/                                  (Folder)
└── Japan 2026                          (Trip)
    ├── Planning/                       (Folder)
    │   ├── ☑ Book flights SYD→NRT      (Task → Transport)
    │   ├── ☐ Reserve Park Hyatt        (Task → Stay)
    │   └── ☐ Buy Ghibli Museum tickets (Task → Location)
    ├── Accommodation/                  (Folder)
    │   └── Park Hyatt Tokyo (May 11–14)(Stay → Location)
    ├── Transport/                      (Folder)
    │   ├── Flight SYD → NRT            (Transport)
    │   └── Train Kyoto → Tokyo         (Transport)
    ├── Day 1 — 2026-05-10              (DayPlan)
    │   ├── 09:00 — Fushimi Inari       (Activity → Location)
    │   └── 12:30 — Ichiran             (Activity → Location)
    ├── Day 2 — 2026-05-11              (DayPlan)
    │   ├── 14:30 — Train to Tokyo      (Activity → Transport)
    │   └── 17:00 — Check in Park Hyatt (Activity → Stay)
    └── Day 3 — 2026-05-12              (DayPlan)
        └── 10:00 — Meiji Shrine        (Activity → Location)
```

---

## Views

### Trip — "Overview"
`register_view("Trip", "Overview", ...)` with `display_first: true`.

Renders:
1. **Date range** — start_date to end_date
2. **Planning progress** — queries Task children (recursively under Planning folder), shows "N/M tasks done" with a count of checked vs total.
3. **Day-by-day summary** — iterates DayPlan children sorted by date, for each day shows the date and a bullet list of Activity/Transport titles.

### DayPlan — "Timeline"
`register_view("DayPlan", "Timeline", ...)` with `display_first: true`.

Renders a chronological list of the day's Activity children, sorted by time. For each Activity, shows time, resolved linked name (Location, Transport, or Stay via `get_note`), and description.

### Location — "Details"
`register_view("Location", "Details", ...)` with `display_first: true`.

Renders: photo (via `display_image`), category badge, address, rating (via `stars()`), and notes.

### Stay — "Booking"
`register_view("Stay", "Booking", ...)` with `display_first: true`.

Renders: resolved hotel name + photo (from linked Location), date range, price summary (nights × price_per_night), booking ref, and notes.

### Folder — "Contents"
`register_view("Folder", "Contents", ...)` with `display_first: true`.

Renders a simple table of direct children: title and schema type.

---

## Hover Tooltips

| Schema | Hover content |
|--------|--------------|
| Location | category, city, country, rating |
| Stay | hotel name (resolved), check_in — check_out |
| Transport | type, route (from → to), departure_date + time |
| Trip | date range, planning progress |
| DayPlan | date, number of activities |
| Activity | time, resolved destination name |
| Task | due_date, category, checked state |

---

## Context Menu Actions

| Action | Available on | Behaviour |
|--------|-------------|-----------|
| Sort by Time | DayPlan | Sorts Activity children by time field |
| Sort by Date | Trip | Sorts DayPlan children by date field |

---

## Sample Data (for .krillnotes archive)

A Japan trip example with:
- 4–5 Locations (mix of Hotel, Restaurant, Temple, Park)
- 1 Stay under Accommodation/ (linked to the Hotel location)
- 2 Transport legs under Transport/ (flight in, shinkansen)
- 1 Trip with 3 DayPlans, each with 1–3 Activities linking to Locations/Transport/Stay
- 3–4 Tasks under a Planning folder (mix of checked/unchecked)
- Tags on locations: `japan`, `tokyo`, `kyoto`

---

## File Structure

```
example-scripts/travel-planner/
├── travel-planner.schema.rhai      # All schemas: Folder, Location, Stay, Transport,
│                                   # Trip, DayPlan, Activity, Task
├── travel-planner.rhai             # All views, hovers, and menu actions
└── travel-planner.krillnotes       # Sample workspace archive
```

All schemas in one `.schema.rhai` file (consistent with other examples like book-collection). All presentation in one `.rhai` file.

---

## Scope Boundaries

- **No core changes** — this is purely an example script using existing features.
- **No new field types** — uses text for time-of-day (no time picker exists).
- **No recursive queries** — views only look one or two levels deep (DayPlan → children, Trip → DayPlan children).
- **Tags are user-applied** — no auto-tagging logic in hooks.
