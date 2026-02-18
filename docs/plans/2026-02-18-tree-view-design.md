# Phase 3: Tree View Design

**Date:** 2026-02-18
**Status:** Approved
**Follows:** Phase 2 (Workspace Integration)
**Next:** Phase 4 (Detail View & Editing)

## Overview

Add hierarchical tree view for browsing notes with selection and basic info display. Users can view the note hierarchy, expand/collapse nodes, select notes to view details, and create new notes.

## Goals

- Display notes in a collapsible tree structure (left sidebar)
- Show selected note details in an info panel (right side)
- Persist tree expansion and selection state in the workspace database
- Support adding new notes as children or siblings with type selection
- State travels with workspace file (not localStorage)

## User Experience

### Layout
- **Split view:** Tree on left (300px fixed width), info panel on right (flex-1)
- **Tree sidebar:** Shows all notes in hierarchy, title-only display
- **Info panel:** Shows selected note details or empty state

### Interactions
1. **Viewing:** Click a note in tree → highlights and shows details in info panel
2. **Expanding:** Click expand/collapse icon → toggles children visibility
3. **Adding notes:** Menu "Edit > Add Note" → dialog with position choice (child/sibling) and node type dropdown
4. **Persistence:** Expansion and selection state saved to database, restored on reopen

### State Persistence
- Tree expansion state: stored in `is_expanded` column on each note
- Selected note: stored in `workspace_meta` as `selected_note_id`
- State persists across sessions and machines (travels with .db file)

## Architecture

### Approach: Client-Side Tree Building

**Chosen approach:** Fetch flat note list, build tree in React, render with custom components.

**Rationale:**
- Backend already provides `list_notes` returning flat array
- Tree building from flat list is straightforward (~50 lines)
- Full control over rendering and state management
- No external dependencies needed

### Component Structure

**New Components:**

1. **`WorkspaceView.tsx`** - Main container (replaces `WorkspaceInfo`)
   - Fetches notes via `list_notes` on mount
   - Builds tree structure from flat array
   - Manages selection state
   - Renders split layout: TreeView (left) + InfoPanel (right)

2. **`TreeView.tsx`** - Left sidebar tree container
   - Receives: notes array, tree structure, selected note ID
   - Renders recursive list of TreeNode components
   - Handles scroll container styling

3. **`TreeNode.tsx`** - Individual tree node (recursive component)
   - Displays: expand/collapse button (if children), note title
   - Indentation: 20px per level
   - Selection highlighting
   - Click handlers: toggle expansion, select note
   - Recursively renders children when `is_expanded` is true

4. **`InfoPanel.tsx`** - Right side info display
   - Empty state: "Select a note to view details" (only if workspace empty)
   - When note selected:
     - Title (large, prominent)
     - Node type
     - Created/modified timestamps (formatted)
     - Note ID (monospace, small)
   - Clean card-based layout

5. **`AddNoteDialog.tsx`** - Modal for creating notes
   - **Normal case (notes exist):**
     - Radio buttons: "As child of selected" / "As sibling of selected"
     - Dropdown: Node type selection (from schema registry)
   - **Empty workspace case:**
     - Message: "Creating first note"
     - No position radio (creates root)
     - Dropdown: Node type selection
   - Cancel/Create buttons
   - Triggered by "Edit > Add Note" menu event

### Data Flow

```
1. Load workspace
   ↓
2. Fetch notes via list_notes (includes is_expanded for each note)
   ↓
3. Fetch selected_note_id from workspace_meta (or get with workspace info)
   ↓
4. Build tree structure in React (flat array → parent-child hierarchy)
   ↓
5. If no selection, auto-select first root node
   ↓
6. Render TreeView + InfoPanel
   ↓
7. User interactions:
   - Click note → call set_selected_note → update backend → refresh UI
   - Click expand → call toggle_note_expansion → update backend → refresh UI
   - Add note → show dialog → call create_note_with_type → refresh notes
```

### Tree Building Algorithm

```typescript
interface TreeNode {
  note: Note;
  children: TreeNode[];
}

function buildTree(notes: Note[]): TreeNode[] {
  // 1. Group children by parent_id
  const childrenMap = new Map<string | null, Note[]>();
  notes.forEach(note => {
    const parentId = note.parentId;
    if (!childrenMap.has(parentId)) {
      childrenMap.set(parentId, []);
    }
    childrenMap.get(parentId)!.push(note);
  });

  // 2. Sort siblings by position
  childrenMap.forEach(children =>
    children.sort((a, b) => a.position - b.position)
  );

  // 3. Recursive builder
  function buildNode(note: Note): TreeNode {
    const children = childrenMap.get(note.id) || [];
    return {
      note,
      children: children.map(buildNode)
    };
  }

  // 4. Return root-level nodes (parentId = null)
  const roots = childrenMap.get(null) || [];
  return roots.map(buildNode);
}
```

## Backend Changes

### Database Schema

**Modify `notes` table:**
```sql
ALTER TABLE notes ADD COLUMN is_expanded INTEGER DEFAULT 1;
```
- Stores expansion state per note (1 = expanded, 0 = collapsed)
- Default to expanded for new notes

**Use `workspace_meta` table:**
- Key: `selected_note_id` → Value: note ID string
- Stores currently selected note

### New Backend Commands

1. **`get_node_types() -> Vec<String>`**
   - Returns list of available node types from SchemaRegistry
   - Used to populate dropdown in AddNoteDialog

2. **`create_note_with_type(parent_id: String | null, position: AddPosition, node_type: String) -> Note`**
   - Creates note with specified type and position
   - If parent_id is null: creates root node (for empty workspace case)
   - If position is AsChild: creates as child of parent
   - If position is AsSibling: creates as sibling of parent
   - Returns the created note

3. **`toggle_note_expansion(note_id: String) -> ()`**
   - Flips `is_expanded` boolean for specified note
   - Called when user clicks expand/collapse button

4. **`set_selected_note(note_id: String | null) -> ()`**
   - Updates `selected_note_id` in workspace_meta
   - Called when user selects a note
   - If null: clears selection (edge case)

### Modified Backend Commands

**`list_notes` - No changes needed**
- Already returns all fields including parent_id, position
- Need to ensure is_expanded is included in Note struct and returned

**`get_workspace_info` - Add selected_note_id**
- Extend WorkspaceInfo to include `selectedNoteId?: string`
- Fetch from workspace_meta when returning workspace info
- Or: create separate command `get_selected_note_id` if preferred

## Frontend Changes

### Type Definitions (types.ts)

```typescript
// Extend existing Note interface
export interface Note {
  id: string;
  title: string;
  nodeType: string;
  parentId: string | null;
  position: number;
  createdAt: number;
  modifiedAt: number;
  createdBy: number;
  modifiedBy: number;
  fields: Record<string, FieldValue>;
  isExpanded: boolean; // NEW
}

// Extend WorkspaceInfo
export interface WorkspaceInfo {
  filename: string;
  path: string;
  noteCount: number;
  selectedNoteId?: string; // NEW - optional for backward compat
}

// Tree structure
export interface TreeNode {
  note: Note;
  children: TreeNode[];
}
```

### App.tsx Changes

Replace:
```typescript
{workspace ? <WorkspaceInfo info={workspace} /> : <EmptyState />}
```

With:
```typescript
{workspace ? <WorkspaceView workspaceInfo={workspace} /> : <EmptyState />}
```

### Menu Integration

Add handler for "Edit > Add Note" in `createMenuHandlers`:
```typescript
'Edit > Add Note clicked': async () => {
  // Trigger AddNoteDialog via event or state
  // Dialog will handle the actual creation
}
```

## Selection & Add Note Behavior

### Default Selection
- On workspace load: if `selectedNoteId` is null/missing, auto-select first root node
- If selected note is deleted (future phase): auto-select next available note
- This ensures a note is always selected unless workspace is completely empty

### Add Note Cases

**Normal case (workspace has notes):**
- Dialog shows position radio buttons relative to selected note
- Dialog shows node type dropdown
- Creates note at specified position and type

**Empty workspace case (zero notes):**
- Dialog shows: "Creating first note"
- No position radio buttons (forced to root)
- Dialog shows node type dropdown
- Creates root node: parent_id = null, position = 0

## Error Handling

### Backend Errors
- `list_notes` fails → Show error in StatusMessage, display empty tree with message
- `toggle_note_expansion` fails → Revert UI state (keep note in previous state), log error
- `set_selected_note` fails → Keep previous selection, log error (non-blocking)
- `get_node_types` fails → Show error in AddNoteDialog, disable Create button
- `create_note_with_type` fails → Show error in dialog, keep dialog open for retry

### Frontend Errors
- Tree building fails (corrupted data) → Catch error, show error message in WorkspaceView
- No notes loaded → Show empty state message with "Add Note" prompt

### Error Display
- Critical (can't load): Full error state in WorkspaceView
- Action errors: Error message in dialog or StatusMessage
- Non-critical: Console log, don't block UX

## Testing

### Manual Testing Scenarios

1. **Tree Display**
   - Open workspace → tree shows all notes in hierarchy
   - Root notes at top level, children indented (20px per level)
   - Note titles displayed clearly

2. **Expand/Collapse**
   - Click expand button → children appear, button changes to collapse
   - Click collapse button → children hidden
   - Close & reopen workspace → expansion state restored from database

3. **Selection & Info Panel**
   - Click note → highlights in tree, shows info in right panel
   - Info panel displays: title, node type, created/modified timestamps, note ID
   - Close & reopen workspace → selection restored

4. **Add Note - Normal Case**
   - Select a note → "Edit > Add Note" → dialog opens
   - Choose "As child" + select node type → creates child correctly
   - Choose "As sibling" + select node type → creates sibling correctly
   - Verify position ordering is correct

5. **Add Note - Empty Workspace**
   - Delete all notes (future phase will enable)
   - "Edit > Add Note" → dialog shows "Creating first note"
   - Select node type → creates root node
   - Tree displays new root note

6. **State Persistence Across Sessions**
   - Expand some nodes, collapse others
   - Select a specific note
   - Close workspace window
   - Reopen workspace → exact same UI state (expansion + selection)

7. **State Persistence Across Machines**
   - Set up tree state on machine A
   - Copy .db file to machine B
   - Open on machine B → same tree state

8. **Multi-Window Same Workspace**
   - Open same workspace in two windows
   - Expand node in window A
   - Refresh window B → sees expansion
   - Select different notes in each window → each tracks own selection

### Backend Unit Tests

- `toggle_note_expansion` correctly updates `is_expanded` in database
- `set_selected_note` correctly updates `workspace_meta`
- `get_node_types` returns schema registry types
- `create_note_with_type` creates note with correct parent, position, type
- `create_note_with_type` with null parent creates root node

## Implementation Notes

### Phase Boundaries
- **Phase 3 (this):** Tree view, selection, add notes, view info
- **Phase 4 (next):** Edit note details, delete notes, update fields

### Future Enhancements (Not Phase 3)
- Icons/badges for node types
- Keyboard navigation (arrow keys, enter to select)
- Drag-drop reordering
- Multi-select
- Search/filter tree
- Virtualization for large trees (1000+ notes)
- **Auto-expand parent when adding child to collapsed node**: Currently, if a selected note is hidden (parent collapsed), creating a new child/sibling creates it correctly but invisibly. Consider auto-expanding parent or scrolling to show the new note.

### Dependencies
- No new external dependencies required
- Uses existing: React, Tailwind CSS, Tauri commands

## Success Criteria

Phase 3 is complete when:
- ✅ Tree displays all notes in correct hierarchy
- ✅ Expand/collapse works and persists in database
- ✅ Selecting a note shows info in panel
- ✅ Selection persists in database
- ✅ Can add new notes as child or sibling with type choice
- ✅ State (expansion + selection) survives workspace close/reopen
- ✅ All manual testing scenarios pass
- ✅ Backend unit tests pass
