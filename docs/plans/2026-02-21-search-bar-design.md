# Search Bar Design

## Goal

Add a search bar above the tree in the left sidebar that filters notes by title and text-like field values, showing results in a dropdown overlay. Selecting a result reveals the note in the tree by expanding collapsed ancestors.

## Decisions

- **Search layer:** Frontend in-memory filtering over the `notes[]` array already loaded in WorkspaceView. No new backend commands. Can be swapped for a backend SQL query later if performance requires it.
- **Search scope:** Note `title` + all field values stored as `Text` (covers text, textarea, and select fields) + `Email` field values. Case-insensitive substring matching.
- **Selection UX:** Dropdown stays open after clicking a result so the user can quickly jump between matches. Clears on Escape or when the input is emptied.
- **Result detail:** Each result shows the note title plus a match snippet (field name + surrounding text) so the user can see why it matched.
- **Approach:** Standalone `SearchBar` component above `TreeView` in the left panel. Dropdown is absolutely positioned over the tree.

## Architecture

### Layout

```
┌──────────────────┐
│  🔍 Search...    │  ← SearchBar (fixed, doesn't scroll)
├──────────────────┤
│  ┌────────────┐  │  ← dropdown overlay (absolute, shown when query non-empty)
│  │ Result 1   │  │
│  │ Result 2   │  │
│  └────────────┘  │
│                  │
│  TreeView        │  ← existing tree (scrollable, unchanged)
│                  │
└──────────────────┘
```

### Components

**`SearchBar` component** (new file: `SearchBar.tsx`):

- Text input with `Search` icon from lucide-react
- Debounced filtering (150ms) over `notes: Note[]` prop
- Matches query against: `note.title`, and any field value where the variant is `Text` or `Email`
- Each dropdown result row shows: note title (bold) + match snippet (field name + context text, muted)
- Keyboard navigation: arrow keys move selection, Enter selects, Escape clears/closes
- Calls `onSelect(noteId)` on result click or Enter

### Reveal in Tree (Ancestor Expansion)

When a result is selected and the note is hidden (parent collapsed):

1. Walk up the `parentId` chain in the flat `notes[]` array to collect ancestor IDs
2. For each ancestor where `isExpanded === false`, call `toggle_note_expansion`
3. Call `loadNotes()` once after all expansions to refresh the tree
4. Set the selected note ID

Uses existing `toggle_note_expansion` Tauri command — no new backend method needed.

### Data Flow

```
User types → debounce 150ms → filter notes[] in memory → show dropdown

User clicks result → onSelect(noteId) to WorkspaceView
  → collect collapsed ancestors via parentId chain
  → toggle_note_expansion for each collapsed ancestor
  → loadNotes() (single reload)
  → setSelectedNoteId(noteId)
```

## No Backend Changes

Everything runs client-side. The `notes[]` array already contains all fields and parent IDs needed for search and ancestor walking.
