# Design: Hover Info on Tree Nodes (Issue #33)

## Summary

Show a speech-bubble callout tooltip when hovering a tree node, previewing key note data without requiring a click. Schema authors control what appears via `show_on_hover` field flags (simple path) or a full `on_hover` Rhai hook (powerful path). The `on_hover` hook wins when defined, matching the existing `on_view` override precedent.

## Approach

Option C — dual-path rendering with hook override:
- **Simple path:** `show_on_hover: true` on field definitions renders fields from the already-loaded `Note` object, no extra backend call.
- **Power path:** `on_hover: |note| { ... }` Rhai hook returns an HTML string via a new `get_note_hover` Tauri command, using the same display helpers as `on_view`.

## Schema Layer

### `FieldDefinition` — new `show_on_hover` flag

```rhai
fields: [
    #{ name: "body",         type: "textarea", show_on_hover: true },
    #{ name: "status",       type: "select",   show_on_hover: true },
    #{ name: "internal_ref", type: "text" },   // not shown on hover
]
```

Parsed with `get_bool("show_on_hover", false)` default, same pattern as `required`, `can_view`, `can_edit`. Note title is never shown (already visible in the tree).

### `on_hover` hook — new optional schema hook

```rhai
schema("Zettel", #{
    on_hover: |note| {
        stack([
            field("Status", note.fields.status),
            markdown(note.fields.body),
        ])
    },
    on_view: |note| { ... }
});
```

Same contract as `on_view` — returns an HTML string using existing display helpers. If `on_hover` is defined, it wins and `show_on_hover` flags are ignored.

`SchemaInfo` gains `hasHoverHook: bool` (mirrors existing `hasViewHook`).

## Backend Layer

### New Tauri command: `get_note_hover`

Mirrors `get_note_view` exactly — same execution path through `Workspace -> ScriptRegistry -> SchemaRegistry`, calling a new `run_on_hover_hook()` method. Returns `Option<String>` (None when no hook defined).

### `FieldDefinition` in Rust

```rust
pub struct FieldDefinition {
    // existing fields...
    pub show_on_hover: bool,   // new
}
```

### `SchemaRegistry`

New `run_on_hover_hook()` method, stored as `Option<FnPtr>` alongside the existing `on_view_hook`.

## Frontend Layer

### `HoverTooltip.tsx` — new component

Rendered as a React portal at the `WorkspaceView` level (not inside `TreeNode`) to avoid z-index/overflow clipping from the tree panel.

**Visual style:**
- Rounded corners (`border-radius: 8px`)
- Themed card background + subtle shadow
- Left-pointing CSS spike (`::before` pseudo-element with border trick), vertically aligned to the hovered node's center
- Spike tip positioned at the right edge of the tree panel
- Max-width 280px

```
Tree panel          Note view panel
                   ╭──────────────╮
  [Hovered node] <─┤ Status: Done │
                   │              │
                   │ Body: First  │
                   │ few words... │
                   ╰──────────────╯
```

**Two render paths:**
1. `hasHoverHook: true` — call `get_note_hover(noteId)`, sanitize with DOMPurify, render HTML (same pipeline as InfoPanel)
2. `hasHoverHook: false` — render fields where `showOnHover: true` from already-loaded `Note` + schema (zero extra backend call)

Note title is never rendered in the tooltip.

### State at `WorkspaceView` level

```typescript
hoveredNoteId: string | null
tooltipAnchorY: number   // px from top of viewport, center of hovered row
```

`TreeNode` calls `onHoverStart(noteId, y)` / `onHoverEnd()` up to WorkspaceView. WorkspaceView owns the debounce timer and renders `<HoverTooltip>`.

## Mouse State Management

**Show delay — 600ms debounce**
`onMouseEnter` starts a 600ms timer. Mouse leaving before timeout cancels it silently. Prevents tooltip flash on fast mouse traversal through the tree.

**Drag suppression**
Before starting the timer, check `draggedNoteId !== null` — do nothing if a drag is in progress. Hide immediately if a drag starts while the tooltip is visible. `draggedNoteId` is already threaded through to every `TreeNode`.

**Click safety — dismiss on `mousedown`**
`onMouseDown` on the tree node cancels the pending timer and hides any visible tooltip before the click/select fires. Ensures the tooltip never interferes with left-click (select) or right-click (context menu).

**Mouse leave**
`onMouseLeave` cancels the pending timer and hides the tooltip immediately.

## Architecture Decisions

- Tooltip portal lives at WorkspaceView level to escape tree panel overflow/z-index constraints.
- Simple-path rendering uses already-loaded data — no IPC round-trip, no flicker.
- Power-path (`on_hover` hook) accepts the small Rhai execution cost (~5ms) in exchange for full scripting expressiveness.
- `on_hover` hook override semantics mirror `on_view` — schema authors already understand this contract.
- `show_on_hover` defaults to `false` — opt-in only, no accidental data exposure.
