# Frontend Cleanup Phase A — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract custom hooks from WorkspaceView (945L), InfoPanel (929L), and App (690L) to reduce component complexity, and move inline utilities to shared `utils/` files.

**Architecture:** Pure extraction refactoring — no behavior changes, no new dependencies. Each hook encapsulates a concern (state + effects + handlers) and exposes a typed return object. Components call the hooks and wire the return values into their JSX.

**Tech Stack:** React 19, TypeScript, Tauri v2 IPC (`invoke`)

**Spec:** `docs/plans/2026-03-14-frontend-cleanup-design.md`

**Important implementation notes:**
- `loadNotes` in WorkspaceView is a plain `async` function (not `useCallback`-wrapped), recreated every render. Before passing it to any hook, wrap it in a ref pattern: `const loadNotesRef = useRef(loadNotes); loadNotesRef.current = loadNotes;` and pass `loadNotesRef` instead. Hooks should call `loadNotesRef.current()`. This avoids cascading callback instability.
- All handlers in extracted hooks that are passed to child components as props (e.g., `handleSelectNote` → TreeView's `onSelect`) must use `useCallback` for stability.
- The design spec's `useDragAndDrop` hook is **deferred** — the drag-drop state in WorkspaceView is only 3 state variables + 1 memo, with the actual logic living in TreeView. Not worth a separate hook file. The design doc should be updated to reflect this.

---

## Chunk 1: Setup & Utility Extraction

### Task 1: Create worktree and hooks directory

**Files:**
- Create: `krillnotes-desktop/src/hooks/` (directory)

- [ ] **Step 1: Create worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/frontend-cleanup -b feat/frontend-cleanup
```

- [ ] **Step 2: Create hooks directory**

```bash
mkdir -p /Users/careck/Source/Krillnotes/.worktrees/feat/frontend-cleanup/krillnotes-desktop/src/hooks
```

- [ ] **Step 3: Verify directory exists**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/frontend-cleanup/krillnotes-desktop/src/hooks/
```

---

### Task 2: Extract fieldValue utilities from InfoPanel

**Files:**
- Create: `krillnotes-desktop/src/utils/fieldValue.ts`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx:32-52` (remove inline functions)

- [ ] **Step 1: Create `utils/fieldValue.ts`**

Extract the two module-level functions from InfoPanel.tsx (lines 32–52):

```typescript
import type { FieldValue } from '../types';

/** Return a sensible empty/default value for a given field type string. */
export function defaultValueForFieldType(fieldType: string): FieldValue {
  // ... exact body from InfoPanel.tsx lines 33-43
}

/** Check whether a FieldValue is effectively "empty" (blank text, null date, etc.). */
export function isEmptyFieldValue(value: FieldValue): boolean {
  // ... exact body from InfoPanel.tsx lines 45-52
}
```

- [ ] **Step 2: Update InfoPanel.tsx imports**

Remove the two function definitions (lines 32–52) and add:

```typescript
import { defaultValueForFieldType, isEmptyFieldValue } from '../utils/fieldValue';
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/frontend-cleanup/krillnotes-desktop && npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/utils/fieldValue.ts src/components/InfoPanel.tsx
git commit -m "refactor(frontend): extract fieldValue utils from InfoPanel"
```

---

### Task 3: Extract scriptHelpers from ScriptManagerDialog

**Files:**
- Create: `krillnotes-desktop/src/utils/scriptHelpers.ts`
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx:45-58` (remove inline function)

- [ ] **Step 1: Create `utils/scriptHelpers.ts`**

Extract `parseFrontMatterName` from ScriptManagerDialog.tsx (lines 45–58):

```typescript
/**
 * Parse the `// @name: <value>` front-matter from a Rhai script source.
 * Returns the name string, or '' if not found.
 */
export function parseFrontMatterName(source: string): string {
  // ... exact body from ScriptManagerDialog.tsx lines 46-58
}
```

- [ ] **Step 2: Update ScriptManagerDialog.tsx imports**

Remove the function definition (lines 45–58) and add:

```typescript
import { parseFrontMatterName } from '../utils/scriptHelpers';
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/utils/scriptHelpers.ts src/components/ScriptManagerDialog.tsx
git commit -m "refactor(frontend): extract scriptHelpers from ScriptManagerDialog"
```

---

### Task 4: Fix duplicate slugify in App.tsx

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx:30-35` (remove inline slugify, add import)

- [ ] **Step 1: Replace inline slugify with import**

Remove the inline `slugify` function (lines 30–35) and add to imports:

```typescript
import { slugify } from './utils/slugify';
```

- [ ] **Step 2: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add src/App.tsx
git commit -m "refactor(frontend): remove duplicate slugify, import from utils"
```

---

## Chunk 2: WorkspaceView Hook Extractions (Independent Hooks)

These three hooks have no cross-dependencies and can be extracted in any order. They are completely self-contained or take minimal params from the parent.

### Task 5: Extract useResizablePanels from WorkspaceView

**Files:**
- Create: `krillnotes-desktop/src/hooks/useResizablePanels.ts`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

This is the simplest extraction — all state and effects are fully isolated.

- [ ] **Step 1: Create `hooks/useResizablePanels.ts`**

**State to move** (from WorkspaceView):
- `treeWidth` (line 89) — `useState<number>(300)`
- `isDragging` ref (line 90)
- `dragStartX` ref (line 91)
- `dragStartWidth` ref (line 92)

**Tag cloud resize state to move:**
- `tagCloudHeight` (line 104) — `useState<number>(120)`
- `isTagDragging` ref (line 106)
- `tagDragStartY` ref (line 107)
- `tagDragStartHeight` ref (line 108)

**Handlers to move:**
- `handleDividerMouseDown` (line 110)
- `handleTagDividerMouseDown` (line 132)

**Effects to move:**
- Tree divider mouse move/up listener (lines 117–130)
- Tag divider mouse move/up listener (lines 139–152)

**Hook signature:**

```typescript
export function useResizablePanels(initialTreeWidth = 300, initialTagCloudHeight = 120) {
  // ... state, refs, handlers, effects from above
  return {
    treeWidth,
    tagCloudHeight,
    handleDividerMouseDown,
    handleTagDividerMouseDown,
  };
}
```

- [ ] **Step 2: Update WorkspaceView to use the hook**

Replace the moved state/refs/handlers/effects with:

```typescript
const { treeWidth, tagCloudHeight, handleDividerMouseDown, handleTagDividerMouseDown } =
  useResizablePanels();
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useResizablePanels.ts src/components/WorkspaceView.tsx
git commit -m "refactor(frontend): extract useResizablePanels from WorkspaceView"
```

---

### Task 6: Extract useHoverTooltip from WorkspaceView

**Files:**
- Create: `krillnotes-desktop/src/hooks/useHoverTooltip.ts`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

- [ ] **Step 1: Create `hooks/useHoverTooltip.ts`**

**State to move:**
- `hoveredNoteId` (line 83)
- `tooltipAnchorY` (line 84)
- `hoverHtml` (line 85)

**Refs to move:**
- `hoverTimer` (line 86)

**Handlers to move:**
- `handleHoverEnd` (line 406) — clears timer, resets state
- `handleHoverStart` (line 413) — 600ms delay, fetches `get_note_hover` HTML

**Effects to move:**
- Dismiss hover on drag start (line 435) — watches `draggedNoteId`

**Hook signature:**

```typescript
import type { Note, SchemaInfo } from '../types';

export function useHoverTooltip(
  draggedNoteId: string | null,
  notes: Note[],
  schemas: Record<string, SchemaInfo>,
) {
  // ... state, refs, handlers, effects
  return {
    hoveredNoteId,
    tooltipAnchorY,
    hoverHtml,
    handleHoverStart,
    handleHoverEnd,
  };
}
```

- [ ] **Step 2: Update WorkspaceView to use the hook**

Replace moved code with:

```typescript
const { hoveredNoteId, tooltipAnchorY, hoverHtml, handleHoverStart, handleHoverEnd } =
  useHoverTooltip(draggedNoteId, notes, schemas);
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useHoverTooltip.ts src/components/WorkspaceView.tsx
git commit -m "refactor(frontend): extract useHoverTooltip from WorkspaceView"
```

---

### Task 7: Extract useUndoRedo from WorkspaceView

**Files:**
- Create: `krillnotes-desktop/src/hooks/useUndoRedo.ts`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

- [ ] **Step 1: Create `hooks/useUndoRedo.ts`**

**State to move:**
- `canUndo` (line 95)
- `canRedo` (line 96)
- `noteRefreshSignal` (line 97)

**Refs to move:**
- `pendingUndoGroupRef` (line 100)

**Handlers to move:**
- `refreshUndoState` (line 156) — queries `can_undo`/`can_redo`
- `performUndo` (line 165) — invokes `undo`, reloads notes, updates selection
- `performRedo` (line 180) — invokes `redo`, reloads notes, updates selection
- `closePendingUndoGroup` (line 197) — ends undo group if `pendingUndoGroupRef` is set

**Callbacks the hook needs:**
- `loadNotes: () => Promise<void>` — to refresh tree after undo/redo
- `setSelectedNoteId: (id: string | null) => void` — to update selection after undo/redo
- `selectedNoteIdRef: React.MutableRefObject<string | null>` — to check current selection

**Hook signature:**

```typescript
export function useUndoRedo(
  loadNotes: () => Promise<void>,
  setSelectedNoteId: (id: string | null) => void,
  selectedNoteIdRef: React.MutableRefObject<string | null>,
) {
  // ... state, refs, handlers
  return {
    canUndo,
    canRedo,
    noteRefreshSignal,
    refreshUndoState,
    performUndo,
    performRedo,
    closePendingUndoGroup,
    pendingUndoGroupRef,
  };
}
```

**Important:** `performUndo` and `performRedo` call `loadNotes()` internally and then call `refreshUndoState()`. They also update `selectedNoteId` via `setSelectedNoteId` after the undo/redo resolves (using the returned selected note ID from the backend).

- [ ] **Step 2: Update WorkspaceView to use the hook**

Replace moved code. Pass `loadNotesRef` (see implementation notes at top), `setSelectedNoteId`, and `selectedNoteIdRef`. The hook should call `loadNotesRef.current()` internally.

```typescript
const { canUndo, canRedo, noteRefreshSignal, refreshUndoState, performUndo, performRedo, closePendingUndoGroup, pendingUndoGroupRef } =
  useUndoRedo(loadNotes, setSelectedNoteId, selectedNoteIdRef);
```

**Note:** Several other handlers in WorkspaceView call `refreshUndoState()` and `closePendingUndoGroup()`. These continue to work because the hook returns them and they're used in the component scope.

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useUndoRedo.ts src/components/WorkspaceView.tsx
git commit -m "refactor(frontend): extract useUndoRedo from WorkspaceView"
```

---

## Chunk 3: WorkspaceView Hook Extractions (Dependent Hooks)

### Task 8: Extract useTagCloud from WorkspaceView

**Files:**
- Create: `krillnotes-desktop/src/hooks/useTagCloud.ts`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

- [ ] **Step 1: Create `hooks/useTagCloud.ts`**

**State to move:**
- `workspaceTags` (line 103)
- `tagFilterQuery` (line 105)

**Note:** `tagCloudHeight` already moved to `useResizablePanels` in Task 5.

**The hook manages:**
- Tag list state (set externally when notes load — the tag list comes from `loadNotes`)
- Tag filter selection (click a tag → set filter query → passed to SearchBar)

**Hook signature:**

```typescript
export function useTagCloud() {
  const [workspaceTags, setWorkspaceTags] = useState<string[]>([]);
  const [tagFilterQuery, setTagFilterQuery] = useState<string | undefined>(undefined);

  const handleTagClick = useCallback((tag: string) => {
    setTagFilterQuery(prev => prev === `tag:${tag}` ? undefined : `tag:${tag}`);
  }, []);

  const clearTagFilter = useCallback(() => {
    setTagFilterQuery(undefined);
  }, []);

  return {
    workspaceTags,
    setWorkspaceTags,
    tagFilterQuery,
    handleTagClick,
    clearTagFilter,
  };
}
```

**Note:** `setWorkspaceTags` is exposed so that `loadNotes()` in WorkspaceView can update it after fetching. The tags are derived from the notes list in `loadNotes`.

- [ ] **Step 2: Update WorkspaceView to use the hook**

Replace moved state with:

```typescript
const { workspaceTags, setWorkspaceTags, tagFilterQuery, handleTagClick, clearTagFilter } =
  useTagCloud();
```

Update `loadNotes()` to call `setWorkspaceTags(tags)` (it already does — just ensure the reference is to the hook's setter).

Update the tag pill click handler in JSX (around line 798) to use `handleTagClick(tag)`.

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useTagCloud.ts src/components/WorkspaceView.tsx
git commit -m "refactor(frontend): extract useTagCloud from WorkspaceView"
```

---

### Task 9: Extract useTreeState from WorkspaceView

**Files:**
- Create: `krillnotes-desktop/src/hooks/useTreeState.ts`
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

This is the most complex extraction. The hook manages selection, expansion, keyboard navigation, and link navigation.

- [ ] **Step 1: Create `hooks/useTreeState.ts`**

**State to move:**
- `selectedNoteId` (line 38)
- `viewHistory` (line 40)

**Refs to move:**
- `selectedNoteIdRef` (line 41)
- `selectionInitialized` (line 47)

**Handlers to move:**
- `handleSelectNote` (line 439) — selects note, closes pending undo group, invokes `set_selected_note`
- `handleToggleExpand` (line 488) — invokes `toggle_note_expansion`, reloads
- `handleLinkNavigate` (line 451) — pushes to viewHistory, expands ancestors, selects
- `handleBack` (line 478) — pops viewHistory
- `handleSearchSelect` (line 507) — expands ancestors, selects
- `handleTreeKeyDown` (line 530) — arrow keys, enter (requests edit mode)

**Callbacks the hook needs:**
- `notes: Note[]` — for ancestor lookups in link/search nav
- `tree: TreeNode[]` — for keyboard navigation (flattenVisibleTree)
- `schemas: Record<string, SchemaInfo>` — for keyboard nav (expandable check)
- `closePendingUndoGroup: () => Promise<void>` — called on selection change
- `loadNotes: () => Promise<void>` — called after toggle expand
- `setRequestEditMode: React.Dispatch<React.SetStateAction<number>>` — Enter key triggers edit

**Hook signature:**

```typescript
import type { Note, TreeNode as TreeNodeType, SchemaInfo } from '../types';

export function useTreeState(
  notes: Note[],
  tree: TreeNodeType[],
  schemas: Record<string, SchemaInfo>,
  closePendingUndoGroup: () => Promise<void>,
  loadNotes: () => Promise<void>,
  setRequestEditMode: React.Dispatch<React.SetStateAction<number>>,
) {
  // ... state, refs, handlers
  return {
    selectedNoteId,
    setSelectedNoteId,
    selectedNoteIdRef,
    viewHistory,
    handleSelectNote,
    handleToggleExpand,
    handleLinkNavigate,
    handleBack,
    handleSearchSelect,
    handleTreeKeyDown,
    selectionInitialized,
  };
}
```

**Important constraints:**
- `handleSelectNote` must call `closePendingUndoGroup()` before selecting (preserves undo group behavior)
- `handleToggleExpand` invokes `toggle_note_expansion` on the backend — expansion is server-persisted
- `handleLinkNavigate` and `handleSearchSelect` use `getAncestorIds()` from `utils/tree.ts` to expand ancestor chain
- `handleTreeKeyDown` uses `flattenVisibleTree()` from `utils/tree.ts` for up/down navigation
- `setSelectedNoteId` and `selectedNoteIdRef` must stay in sync (ref updated in the hook whenever state changes)

- [ ] **Step 2: Update WorkspaceView to use the hook**

Replace moved state/refs/handlers with:

```typescript
const {
  selectedNoteId, setSelectedNoteId, selectedNoteIdRef, viewHistory,
  handleSelectNote, handleToggleExpand, handleLinkNavigate, handleBack,
  handleSearchSelect, handleTreeKeyDown, selectionInitialized,
} = useTreeState(notes, tree, schemas, closePendingUndoGroup, loadNotes, setRequestEditMode);
```

**Note:** `selectedNoteIdRef` is also used by `useUndoRedo` (Task 7). Since `useTreeState` creates and owns this ref, the call order must be: define `useTreeState` first (to get `selectedNoteIdRef`), then pass it to `useUndoRedo`. This means Task 7's hook call in WorkspaceView needs to be reordered.

**Reorder hooks in WorkspaceView:**
1. `useTreeState(...)` — creates `selectedNoteIdRef`
2. `useUndoRedo(loadNotes, setSelectedNoteId, selectedNoteIdRef)` — receives it

The `closePendingUndoGroup` from useUndoRedo is needed by useTreeState, creating a circular dependency. **Resolution:** useTreeState takes `closePendingUndoGroup` as a stable callback ref or the two hooks share it via a ref pattern:

```typescript
// In WorkspaceView:
const closePendingUndoGroupRef = useRef<() => Promise<void>>();

const { selectedNoteId, setSelectedNoteId, selectedNoteIdRef, ... } =
  useTreeState(notes, tree, schemas, closePendingUndoGroupRef, loadNotes, setRequestEditMode);

const { closePendingUndoGroup, ... } =
  useUndoRedo(loadNotes, setSelectedNoteId, selectedNoteIdRef);

// Keep ref in sync:
closePendingUndoGroupRef.current = closePendingUndoGroup;
```

Inside `useTreeState`, `handleSelectNote` calls `closePendingUndoGroupRef.current?.()` instead of `closePendingUndoGroup()` directly.

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useTreeState.ts src/components/WorkspaceView.tsx
git commit -m "refactor(frontend): extract useTreeState from WorkspaceView"
```

---

## Chunk 4: InfoPanel Hook Extractions

### Task 10: Extract useSchema from InfoPanel

**Files:**
- Create: `krillnotes-desktop/src/hooks/useSchema.ts`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

- [ ] **Step 1: Create `hooks/useSchema.ts`**

**State to move:**
- `schemaInfo` (line 56) — the full `SchemaInfo` for current note's schema
- `views` (line 70) — `ViewInfo[]` registered views
- `activeTab` (line 71) — current tab ("fields" or view label)
- `viewHtml` (line 72) — cached rendered HTML per view label
- `previousTab` (line 73) — tab to restore after edit mode exits

**Refs to move:**
- `schemaLoadedRef` (lines 92–95) — tracks if schema fetch has resolved

**Effects to move:**
- Effect 1: Schema & views fetch (lines 103–160) — fetches schema fields and views when `selectedNote?.id` changes
- Effect 4: Render view HTML (lines 199–210) — renders view HTML when tab changes

**Note:** The schema effect (lines 128–131) checks `pendingEditModeRef` — this ref belongs to the form concern. The hook should accept a callback `onSchemaLoaded` that the form hook can use to resolve the pending edit mode.

**Hook signature:**

```typescript
import type { Note, SchemaInfo, ViewInfo, FieldValue } from '../types';

export function useSchema(
  selectedNote: Note | null,
  isEditing: boolean,
  onSchemaLoaded: (schema: SchemaInfo) => void,
) {
  // ... state, refs, effects
  return {
    schemaInfo,
    views,
    activeTab,
    setActiveTab,
    viewHtml,
    setViewHtml,
    previousTab,
    setPreviousTab,
    schemaLoadedRef,
  };
}
```

**Key behavior in the schema fetch effect:**
1. Reset `schemaLoadedRef.current = false`
2. Fetch schema via `invoke('get_schema_fields', ...)`
3. Set `schemaLoadedRef.current = true`
4. Call `onSchemaLoaded(schema)` — this replaces the inline `pendingEditModeRef` check
5. Fetch views via `invoke('get_views_for_type', ...)`
6. Select default tab

- [ ] **Step 2: Update InfoPanel to use the hook**

```typescript
const { schemaInfo, views, activeTab, setActiveTab, viewHtml, setViewHtml, previousTab, setPreviousTab, schemaLoadedRef } =
  useSchema(selectedNote, isEditing, handleSchemaLoaded);
```

Where `handleSchemaLoaded` is defined in the form concern (Task 11) or inline:

```typescript
const handleSchemaLoaded = useCallback((schema: SchemaInfo) => {
  if (pendingEditModeRef.current) {
    pendingEditModeRef.current = false;
    setIsEditing(true);
  }
}, []);
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useSchema.ts src/components/InfoPanel.tsx
git commit -m "refactor(frontend): extract useSchema from InfoPanel"
```

---

### Task 11: Extract useNoteForm from InfoPanel

**Files:**
- Create: `krillnotes-desktop/src/hooks/useNoteForm.ts`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

- [ ] **Step 1: Create `hooks/useNoteForm.ts`**

**State to move:**
- `isEditing` (line 74)
- `editedTitle` (line 75)
- `editedFields` (line 76)
- `isDirty` (line 77)
- `editedTags` (line 78)
- `allTags` (line 79)
- `tagInput` (line 80)
- `tagSuggestions` (line 81)
- `groupCollapsed` (line 83)
- `groupVisible` (line 84)
- `fieldErrors` (line 85)
- `noteErrors` (line 86)

**Refs to move:**
- `titleInputRef` (line 88)
- `pendingEditModeRef` (line 91)

**Effects to move:**
- Effect 2: Reset form state (lines 162–176) — resets all edit state when note changes
- Effect 3: Request edit mode race handler (lines 186–194) — handles `requestEditMode` prop
- Effect 7: Auto-focus on edit mode (lines 290–300) — focuses title input when editing starts

**Handlers to move:**
- `handleFormKeyDown` (line 314) — Escape/Enter
- `handleEdit` (line 325) — enters edit mode, fetches all tags
- `addTag` (line 332)
- `removeTag` (line 341)
- `handleTagInputChange` (line 346)
- `handleFieldBlur` (line 371) — validates single field
- `handleCancel` (line 389) — discards changes
- `handleSave` (line 411) — validates all, saves
- `handleFieldChange` (line 456) — updates field, evaluates group visibility

**Callbacks the hook needs:**
- `selectedNote: Note | null`
- `requestEditMode: number` — prop from parent
- `schemaInfo: SchemaInfo` — from useSchema
- `schemaLoadedRef: React.MutableRefObject<boolean>` — from useSchema
- `activeTab: string` — to save/restore previous tab
- `setActiveTab: (tab: string) => void`
- `previousTab: string | null`
- `setPreviousTab: (tab: string | null) => void`
- `setViewHtml: React.Dispatch<...>` — to clear view cache on save
- `onNoteUpdated: () => void` — prop callback
- `onEditDone: () => void` — prop callback

**Hook signature:**

```typescript
export function useNoteForm(
  selectedNote: Note | null,
  requestEditMode: number,
  schemaInfo: SchemaInfo,
  schemaLoadedRef: React.MutableRefObject<boolean>,
  schemaCallbacks: {
    activeTab: string;
    setActiveTab: (tab: string) => void;
    previousTab: string | null;
    setPreviousTab: (tab: string | null) => void;
    setViewHtml: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  },
  onNoteUpdated: () => void,
  onEditDone: () => void,
) {
  // ... all state, refs, handlers, effects
  return {
    isEditing,
    editedTitle, setEditedTitle,
    editedFields,
    isDirty, setIsDirty,
    editedTags,
    allTags,
    tagInput,
    tagSuggestions,
    groupCollapsed, setGroupCollapsed,
    groupVisible,
    fieldErrors,
    noteErrors,
    titleInputRef,
    pendingEditModeRef,
    handleFormKeyDown,
    handleEdit,
    handleCancel,
    handleSave,
    handleFieldChange,
    handleFieldBlur,
    addTag,
    removeTag,
    handleTagInputChange,
  };
}
```

**Critical constraint — React.memo compatibility:**
InfoPanel is wrapped in `React.memo` with a custom comparator that ignores `onNoteUpdated`, `onEditDone`, `onLinkNavigate`, `onBack` callback identity. Since these callbacks are passed into the hook (not returned from it), and the hook only calls them inside event handlers (not during render), the memo guard remains effective. The hook's own returned callbacks (`handleSave`, etc.) are only used inside InfoPanel's JSX, so their identity doesn't affect the memo comparator.

- [ ] **Step 2: Update InfoPanel to use the hook**

```typescript
const {
  isEditing, editedTitle, setEditedTitle, editedFields, isDirty, setIsDirty,
  editedTags, allTags, tagInput, tagSuggestions, groupCollapsed, setGroupCollapsed,
  groupVisible, fieldErrors, noteErrors, titleInputRef, pendingEditModeRef,
  handleFormKeyDown, handleEdit, handleCancel, handleSave,
  handleFieldChange, handleFieldBlur, addTag, removeTag, handleTagInputChange,
} = useNoteForm(
  selectedNote, requestEditMode, schemaInfo, schemaLoadedRef,
  { activeTab, setActiveTab, previousTab, setPreviousTab, setViewHtml },
  onNoteUpdated, onEditDone,
);
```

The `handleSchemaLoaded` callback from Task 10 should use `pendingEditModeRef` from this hook:

```typescript
const handleSchemaLoaded = useCallback(() => {
  if (pendingEditModeRef.current) {
    pendingEditModeRef.current = false;
    setIsEditing(true);
  }
}, []);
```

**Required approach:** Keep the `pendingEditModeRef` + `requestEditMode` effect (lines 186–194) in InfoPanel as residual code. It's only ~10 lines and avoids a circular dependency between `useSchema` (needs `onSchemaLoaded` callback) and `useNoteForm` (needs `schemaInfo` from `useSchema` but also owns `setIsEditing`). Keeping this race condition logic in the component where both hooks' return values are available is the cleanest solution. Remove `pendingEditModeRef` from `useNoteForm`'s scope — it stays as a standalone `useRef(false)` in InfoPanel.

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useNoteForm.ts src/components/InfoPanel.tsx
git commit -m "refactor(frontend): extract useNoteForm from InfoPanel"
```

---

## Chunk 5: App.tsx Hook Extractions

### Task 12: Extract useMenuEvents from App.tsx

**Files:**
- Create: `krillnotes-desktop/src/hooks/useMenuEvents.ts`
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Create `hooks/useMenuEvents.ts`**

Move the `createMenuHandlers` factory (lines 43–118) and the menu event listener effect (lines 262–283) into this hook.

**The hook encapsulates:**
- The `createMenuHandlers` factory function
- Effect 7: Menu event listener setup (re-registers when workspace changes)

**Callbacks the hook needs:**
- All the dialog state setters and action callbacks currently passed to `createMenuHandlers`
- `workspace` — dependency for re-registering the listener
- `proceedWithImport` — for the import menu handler
- `openSwarmFile` — for the swarm file menu handler

**Hook signature:**

```typescript
interface MenuEventCallbacks {
  setShowNewWorkspace: (show: boolean) => void;
  setShowOpenWorkspace: (show: boolean) => void;
  setShowSettings: (show: boolean) => void;
  setShowExportPasswordDialog: (show: boolean) => void;
  setShowIdentityManager: (show: boolean) => void;
  setShowSwarmInvite: (show: boolean) => void;
  setShowWorkspacePeers: (show: boolean) => void;
  setShowCreateDeltaDialog: (show: boolean) => void;
  statusSetter: (msg: string, isError?: boolean) => void;
  proceedWithImport: (zipPath: string, password: string | null) => Promise<void>;
  openSwarmFile: (path: string) => void;
}

export function useMenuEvents(
  workspace: WorkspaceInfoType | null,
  callbacks: MenuEventCallbacks,
) {
  // createMenuHandlers factory + effect
}
```

- [ ] **Step 2: Update App.tsx**

Remove `createMenuHandlers` and Effect 7. Replace with:

```typescript
useMenuEvents(workspace, {
  setShowNewWorkspace, setShowOpenWorkspace, setShowSettings,
  setShowExportPasswordDialog, setShowIdentityManager, setShowSwarmInvite,
  setShowWorkspacePeers, setShowCreateDeltaDialog,
  statusSetter, proceedWithImport, openSwarmFile,
});
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useMenuEvents.ts src/App.tsx
git commit -m "refactor(frontend): extract useMenuEvents from App"
```

---

### Task 13: Extract useWorkspaceLifecycle from App.tsx

**Files:**
- Create: `krillnotes-desktop/src/hooks/useWorkspaceLifecycle.ts`
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Create `hooks/useWorkspaceLifecycle.ts`**

**State to move:**
- `workspace` (line 122)

**Functions to move:**
- `refreshUnlockedIdentity` (lines 153–157)
- `openSwarmFile` (lines 161–172)

**Effects to move:**
- Effect 1: Initial mount (lines 174–198) — workspace fetch, first-launch identity check
- Effect 2: Swarm dialog lifecycle (lines 202–204)
- Effect 3: Cold-start file open (lines 208–217)
- Effect 4: Warm-start file open (lines 221–232)
- Effect 5: Warm-start swarm file (lines 236–243)
- Effect 6: Load settings language (lines 246–254)

**Hook signature:**

```typescript
export function useWorkspaceLifecycle(
  callbacks: {
    setShowCreateFirstIdentity: (show: boolean) => void;
    setShowSwarmOpen: (show: boolean) => void;
    showSwarmInvite: boolean;
    showSwarmOpen: boolean;
    proceedWithImport: (zipPath: string, password: string | null) => Promise<void>;
    setPendingInvitePath: (path: string | null) => void;
    setPendingInviteData: (data: InviteFileData | null) => void;
    setSwarmFilePath: (path: string | null) => void;
  }
) {
  // ... state, functions, effects
  return {
    workspace,
    unlockedIdentityUuid,
    refreshUnlockedIdentity,
    openSwarmFile,
  };
}
```

- [ ] **Step 2: Update App.tsx**

Replace moved state/functions/effects with:

```typescript
const { workspace, unlockedIdentityUuid, refreshUnlockedIdentity, openSwarmFile } =
  useWorkspaceLifecycle({ ... });
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useWorkspaceLifecycle.ts src/App.tsx
git commit -m "refactor(frontend): extract useWorkspaceLifecycle from App"
```

---

### Task 14: Extract useDialogState from App.tsx

**Files:**
- Create: `krillnotes-desktop/src/hooks/useDialogState.ts`
- Modify: `krillnotes-desktop/src/App.tsx`

- [ ] **Step 1: Create `hooks/useDialogState.ts`**

**State to move** — all dialog visibility booleans:
- `showNewWorkspace` (line 125)
- `showOpenWorkspace` (line 126)
- `showSettings` (line 127)
- `showCreateFirstIdentity` (line 142)
- `showIdentityManager` (line 143)
- `showSwarmInvite` (line 144)
- `showSwarmOpen` (line 145)
- `showWorkspacePeers` (line 150)
- `showCreateDeltaDialog` (line 151)
- `showExportPasswordDialog` (line 139)
- `showImportPasswordDialog` (line 134)

**Also move the associated import workflow state** that drives the inline dialogs:
- `importState` (line 128), `importName` (line 129), `importError` (line 130), `importing` (line 131)
- `importIdentities` (line 132), `importSelectedIdentity` (line 133)
- `importPassword` (line 135), `importPasswordError` (line 136)
- `pendingImportZipPath` (line 137), `pendingImportPassword` (line 138)
- `exportPassword` (line 140), `exportPasswordConfirm` (line 141)
- `swarmFilePath` (line 146), `pendingInvitePath` (line 147), `pendingInviteData` (line 148)
- `status` (line 123), `isError` (line 124)
- `statusSetter` helper (lines 256–260)

**Hook signature:**

```typescript
export function useDialogState() {
  // All useState declarations for dialog visibility and form state
  // statusSetter helper

  return {
    // Dialog visibility (show + setShow pairs)
    showNewWorkspace, setShowNewWorkspace,
    showOpenWorkspace, setShowOpenWorkspace,
    showSettings, setShowSettings,
    // ... all dialog state
    // Import workflow state
    importState, setImportState,
    importName, setImportName,
    // ... all import state
    // Export state
    exportPassword, setExportPassword,
    exportPasswordConfirm, setExportPasswordConfirm,
    showExportPasswordDialog, setShowExportPasswordDialog,
    // Status
    status, isError, statusSetter,
    // Swarm/invite
    swarmFilePath, setSwarmFilePath,
    pendingInvitePath, setPendingInvitePath,
    pendingInviteData, setPendingInviteData,
  };
}
```

This is a large return surface but it's purely mechanical — just state declarations grouped together. It reduces App.tsx by ~30 lines of `useState` declarations and the `statusSetter` helper.

- [ ] **Step 2: Update App.tsx**

Replace all the useState declarations with:

```typescript
const {
  showNewWorkspace, setShowNewWorkspace,
  showOpenWorkspace, setShowOpenWorkspace,
  // ... destructure all needed state
} = useDialogState();
```

- [ ] **Step 3: Verify compilation**

```bash
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useDialogState.ts src/App.tsx
git commit -m "refactor(frontend): extract useDialogState from App"
```

---

## Chunk 6: Final Verification

### Task 15: Full build verification and smoke test

- [ ] **Step 1: TypeScript compilation check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/frontend-cleanup/krillnotes-desktop && npx tsc --noEmit
```

Expected: zero errors.

- [ ] **Step 2: Dev build check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/frontend-cleanup/krillnotes-desktop && npm update && npm run tauri dev
```

Expected: app launches, workspace loads.

- [ ] **Step 3: Smoke test checklist**

Manual verification (all in the running app):

- [ ] Select/expand/collapse notes in tree
- [ ] Keyboard navigation (arrow keys, Enter to edit)
- [ ] Edit a note field, save, cancel, verify dirty state
- [ ] Drag-drop a note to reorder
- [ ] Undo/redo (Cmd+Z / Cmd+Shift+Z)
- [ ] Hover tooltip on tree nodes
- [ ] Right-click context menu (add child, add sibling, delete)
- [ ] Delete a note with children (strategy dialog)
- [ ] Copy/paste a note (Cmd+C / Cmd+V)
- [ ] Tag cloud: click tag to filter, click again to clear
- [ ] Resize panels (tree width, tag cloud height)
- [ ] Open workspace from menu
- [ ] Export workspace (password dialog)
- [ ] Import workspace (multi-phase dialog)
- [ ] Settings dialog
- [ ] Script manager

- [ ] **Step 4: Verify line count reduction**

```bash
wc -l src/components/WorkspaceView.tsx src/components/InfoPanel.tsx src/App.tsx
```

Expected approximate targets:
- WorkspaceView: 350–450 lines (from 945)
- InfoPanel: 400–500 lines (from 929)
- App: 350–400 lines (from 690)

- [ ] **Step 5: Create PR**

```bash
git push -u github-https feat/frontend-cleanup
gh pr create --base master --title "refactor(frontend): extract hooks from large components" --body "..."
```

---

## Parallelization Guide

For subagent-driven execution, these tasks can be parallelized:

**Independent (can run in parallel):**
- Tasks 2, 3, 4 (utility extractions — different files, no conflicts)

**Sequential within WorkspaceView:**
- Task 5 (useResizablePanels) → Task 6 (useHoverTooltip) → Task 7 (useUndoRedo) → Task 8 (useTagCloud) → Task 9 (useTreeState)
- Reason: each modifies WorkspaceView.tsx, so they must be sequential to avoid merge conflicts

**Sequential within InfoPanel:**
- Task 10 (useSchema) → Task 11 (useNoteForm)
- Reason: useNoteForm depends on useSchema's return values

**Sequential within App:**
- Task 12 (useMenuEvents) → Task 13 (useWorkspaceLifecycle) → Task 14 (useDialogState)
- Reason: each modifies App.tsx

**Cross-file parallelization:**
- After Task 4, WorkspaceView tasks (5–9), InfoPanel tasks (10–11), and App tasks (12–14) can run in parallel on separate branches, merged sequentially at the end.
- However, since all hooks go into `src/hooks/`, file creation won't conflict — only the component modifications will.

**Recommended execution order:**
1. Tasks 1–4 (setup + utils) — sequential
2. Tasks 5–9 (WorkspaceView) in parallel with Tasks 10–11 (InfoPanel) in parallel with Tasks 12–14 (App)
3. Task 15 (verification) — after all above complete
