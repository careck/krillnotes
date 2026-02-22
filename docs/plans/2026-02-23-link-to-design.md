# Design: `link_to` View Function + Note Navigation History

**Date:** 2026-02-23
**Feature:** `link_to(note)` Rhai display helper with full navigation history and back button

---

## Summary

Add a `link_to(note: Map) → str` display helper to the `on_view` hook DSL. Clicking a rendered link navigates the view panel to that note. A full navigation stack tracks link-based navigation, with a back button shown above the view content whenever history is non-empty. History resets on manual tree/search navigation.

---

## Architecture

### Backend — `link_to` display helper

Add `link_to(note: Map) → str` to `krillnotes-core/src/core/scripting/display_helpers.rs`.

```rust
pub fn link_to(note: Map) -> String {
    let id    = note.get("id").and_then(|v| v.clone().into_string().ok()).unwrap_or_default();
    let title = note.get("title").and_then(|v| v.clone().into_string().ok()).unwrap_or_default();
    format!(
        r#"<a class="kn-view-link" data-note-id="{}">{}</a>"#,
        html_escape(&id),
        html_escape(&title),
    )
}
```

Register in `mod.rs` alongside the other display helpers. Rhai signature: `link_to(note: Map) → str`.

### Frontend — Navigation state (WorkspaceView)

Add `viewHistory: string[]` state alongside the existing `selectedNoteId`.

- **`onLinkNavigate(noteId)`**: pushes `selectedNoteId` onto the history stack, then navigates to `noteId`.
- **`onBack()`**: pops the last entry from history, navigates to it.
- **`onSelectNote(noteId)`** (existing, tree/search navigation): clears `viewHistory`, sets `selectedNoteId`. This is the reset point.

Tree selection stays in sync with the currently viewed note when following links.

### Frontend — InfoPanel

New props: `onLinkNavigate`, `onBack`, `viewHistory`, `notes` (for resolving back-button title).

**Back button** — rendered at the top of the view area, outside both the custom-view and default field-layout branches. Visible whenever `!isEditing && viewHistory.length > 0`, so it works regardless of whether the previous or current note has a custom view:

```tsx
{!isEditing && viewHistory.length > 0 && (
    <div className="kn-view-back">
        <button onClick={onBack}>
            ← Back to "{notes.find(n => n.id === viewHistory.at(-1))?.title ?? '…'}"
        </button>
    </div>
)}
```

**Click interception** — event delegation on the custom HTML wrapper div. After DOMPurify sanitizes the HTML (stripping `onclick` attributes), we intercept clicks at the React level:

```tsx
<div
    dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(customViewHtml) }}
    onClick={(e) => {
        const link = (e.target as Element).closest('.kn-view-link');
        if (link) {
            e.preventDefault();
            const noteId = link.getAttribute('data-note-id');
            if (noteId) onLinkNavigate(noteId);
        }
    }}
/>
```

### CSS

Two new CSS classes added to the existing stylesheet:

- `.kn-view-link` — accent-coloured underlined link cursor, for generated note links
- `.kn-view-back` + `.kn-view-back button` — unstyled button in accent colour, small font, for the back navigation control

---

## Navigation Behaviour

| Action | History effect |
|--------|---------------|
| Click a `link_to` link | Push current note onto stack, navigate to linked note |
| Click back button | Pop stack, navigate to popped note |
| Click tree node / search | Clear entire stack, navigate to selected note |

Following A → B → C via links: back goes C → B → A.
Clicking the tree at any point resets history.

---

## Key Files

| File | Change |
|------|--------|
| `krillnotes-core/src/core/scripting/display_helpers.rs` | Add `link_to` fn |
| `krillnotes-core/src/core/scripting/mod.rs` | Register `link_to` |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Add `viewHistory` state + callbacks |
| `krillnotes-desktop/src/components/InfoPanel.tsx` | Back button + click delegation |
| `krillnotes-desktop/src/index.css` (or globals) | `.kn-view-link`, `.kn-view-back` styles |

---

## Out of Scope

- Persisting history across app restarts (session-only)
- Forward navigation
- History when navigating via search or keyboard
