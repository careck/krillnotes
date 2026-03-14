# Frontend Cleanup — Design

**Date:** 2026-03-14
**Status:** Phase A approved, B and C planned for later

## Problem

The React frontend has 11,560 lines across 42 components. The three largest files — WorkspaceView.tsx (945L), InfoPanel.tsx (929L), and App.tsx (690L) — each mix 4-5 concerns and contain 15-30+ `useState` hooks with no extraction. Inline utility functions are duplicated or trapped inside components. All 42 components live in a flat `components/` directory with no grouping.

## Phased Approach

### Phase A: Extract Hooks & Inline Utilities (NOW)

Goal: Split the big 3 components into focused hooks and move reusable helpers to `utils/`. Pure extraction, no behavior changes, no new dependencies.

#### New files

```
src/hooks/
├── useTreeState.ts         ← from WorkspaceView
├── useDragAndDrop.ts       ← from WorkspaceView
├── useResizablePanels.ts   ← from WorkspaceView
├── useUndoRedo.ts          ← from WorkspaceView
├── useTagCloud.ts          ← from WorkspaceView
├── useHoverTooltip.ts      ← from WorkspaceView
├── useNoteForm.ts          ← from InfoPanel
├── useSchema.ts            ← from InfoPanel
├── useMenuEvents.ts        ← from App
├── useWorkspaceLifecycle.ts ← from App
└── useDialogState.ts       ← from App

src/utils/
├── fieldValue.ts           ← defaultValueForFieldType, isEmptyFieldValue from InfoPanel
└── scriptHelpers.ts        ← parseFrontMatterName from ScriptManagerDialog
```

#### Extraction plan per file

**WorkspaceView.tsx (945L → ~350-450L)**

| Hook | Responsibility | State it owns | Callbacks it receives |
|------|---------------|---------------|----------------------|
| `useTreeState` | Expansion set, selection, keyboard navigation, tree building from flat note list | `selectedId`, `selectionHistory`, tree memo | `onSelectNote` (to coordinate with undo close), `loadNotes` (refresh after changes). Note: expansion is persisted server-side via `toggle_note_expansion` — the hook coordinates with the backend, it does not own expansion as local-only state. |
| ~~`useDragAndDrop`~~ | **Deferred** — only 3 state vars + 1 memo in WorkspaceView; actual DnD logic lives in TreeView. Not worth a separate hook. | `draggedNoteId`, `dropIndicator`, `dragDescendants` memo | — |
| `useResizablePanels` | Divider positions, mouse tracking, min/max constraints | `leftWidth`, `rightWidth`, divider refs | (self-contained) |
| `useUndoRedo` | Undo/redo handlers, event listeners, state refresh after undo | `canUndo`, `canRedo` | `refreshNotes` (reload after undo/redo) |
| `useTagCloud` | Tag aggregation across notes, selection filtering, resize observer | `allTags`, `selectedTag`, container ref | (derives from notes list) |
| `useHoverTooltip` | Hover timer, tooltip position, HTML content fetch | `hoveredNoteId`, `tooltipAnchorY`, `hoverHtml`, `hoverTimer` | (self-contained) |

**Residual in WorkspaceView (~350-450L):** JSX layout, copy/paste handlers (`copiedNoteId`, `copyNote`, `pasteNote`), context menu state + handlers, delete dialog state + handlers, schema migration toasts, dialog triggers, composition of hooks. These are small and tightly coupled to the JSX — not worth extracting into hooks.

**InfoPanel.tsx (929L → ~400-500L)**

| Hook | Responsibility | State it owns | Callbacks it receives |
|------|---------------|---------------|----------------------|
| `useNoteForm` | Field editing state, dirty tracking, save/cancel, validation. Encapsulates the race condition guard between `requestEditMode`, `schemaLoadedRef`, and `pendingEditModeRef`. | `editingFields`, `isDirty`, `isEditing`, `pendingEditModeRef`, `schemaLoadedRef` | `onSave` (invoke update_field) |
| `useSchema` | Schema fetch + cache by schema name, field definition lookups | `schemas` cache map, loading state | (self-contained) |

Inline utils `defaultValueForFieldType()` and `isEmptyFieldValue()` move to `utils/fieldValue.ts`.

**Constraint:** InfoPanel is wrapped in `React.memo` with a custom comparator that ignores callback identity changes (`onNoteUpdated`, `onDeleteRequest`, `onEditDone`, `onLinkNavigate`, `onBack`). Hooks that return callbacks consumed by InfoPanel must use `useCallback` with stable deps to avoid defeating this memo guard. Breaking callback stability would cause performance regressions (re-renders kill DOM hydration for image attachments in view HTML).

What remains in InfoPanel: JSX rendering (field display, attachments, tags, view HTML), memo wrapper, calls into the hooks above.

**App.tsx (690L → ~350-400L)**

| Hook | Responsibility | State it owns | Callbacks it receives |
|------|---------------|---------------|----------------------|
| `useMenuEvents` | Tauri menu event listeners, dispatch to handler functions | Event listener cleanup | Handler map |
| `useWorkspaceLifecycle` | Open/close/switch workspace, window management | `currentWorkspace`, loading state | (self-contained) |
| `useDialogState` | Which dialog is open, open/close callbacks | `openDialog` enum/string, dialog props | (self-contained) |

**Note:** App.tsx contains ~135L of inline dialog JSX (export password, import password, import name dialogs). These stay inline for Phase A — extracting them into separate dialog components is optional cleanup but not required for the hook extraction to work. The line count target reflects this.

**Misc cleanup:** App.tsx has a duplicate `slugify()` function (lines 30-35) that is identical to `utils/slugify.ts`. Replace with an import.

#### Rules

- Pure extraction — no behavior changes, no new features, no new dependencies
- Each hook receives the minimum props/state it needs; returns what the component consumes
- Hooks that need to coordinate share state via parameters, not new context providers
- Hooks returning callbacks consumed by `React.memo` components must use `useCallback` with stable deps
- Inline utilities move to `utils/` only if reusable; component-specific one-liners stay
- Smaller dialogs (< 500L) stay as-is unless an obvious extraction falls out naturally

#### Acceptance criteria

1. `npx tsc --noEmit` passes with zero errors
2. `npm run tauri dev` builds and launches successfully
3. Manual smoke test: select/expand/collapse notes in tree, keyboard navigation
4. Manual smoke test: edit a note field, save, cancel, dirty state works
5. Manual smoke test: drag-drop a note to reorder, undo/redo
6. Manual smoke test: open workspace, menu actions (export, import), dialog flows
7. Manual smoke test: hover tooltip on tree nodes, context menu, delete with children

---

### Phase B: Service Layer (FUTURE)

Goal: Wrap all Tauri `invoke()` calls in a typed service layer so components never call `invoke()` directly.

#### Planned structure

```
src/services/
├── noteService.ts        ← CRUD notes, move, reorder
├── workspaceService.ts   ← open, close, export, import, settings
├── scriptService.ts      ← user script CRUD, system scripts
├── schemaService.ts      ← schema queries, field validation, view rendering, tree actions, group visibility (maps to commands/scripting.rs)
├── identityService.ts    ← identity CRUD, unlock, sign
├── peerService.ts        ← peer management, invitations
├── contactService.ts     ← contact book CRUD
├── swarmService.ts       ← swarm bundles, snapshots, deltas
├── attachmentService.ts  ← file attach/detach/read
├── themeService.ts       ← theme CRUD, apply (note: theme commands live in workspace.rs on the Rust side; separate service here for frontend ergonomics)
└── operationService.ts   ← operation log, purge
```

#### What this buys

- **Type safety at the boundary** — each service function has typed params and return types, catching mismatches at compile time instead of runtime
- **Single point of change** — when a Tauri command signature changes, update one service function instead of grepping through components
- **Testability** — services can be mocked in tests without mocking `invoke()` globally
- **Discoverability** — new developers can see all available backend calls in one place

#### Approach

- One service file per domain, mirroring the Rust `commands/` modules from yesterday's refactor
- Each function is a thin typed wrapper: `export async function getNote(id: string): Promise<Note> { return invoke("get_note", { id }); }`
- Components import from services instead of calling `invoke()` directly
- Migrate one domain at a time (notes first, then workspace, etc.)
- `schemaService.ts` covers the 12+ scripting/schema invoke calls (get_schema_fields, get_all_schemas, get_views_for_type, render_view, validate_field, evaluate_group_visibility, get_tree_action_map, get_note_hover, invoke_tree_action) that currently map to `commands/scripting.rs`

---

### Phase C: Component Directory Reorganization (FUTURE)

Goal: Group the 42 flat components into domain subdirectories.

#### Planned structure

```
src/components/
├── workspace/
│   ├── WorkspaceView.tsx
│   ├── TreeView.tsx
│   ├── TreeNode.tsx
│   ├── SearchBar.tsx
│   ├── ContextMenu.tsx
│   └── TagPill.tsx
├── notes/
│   ├── InfoPanel.tsx
│   ├── AddNoteDialog.tsx
│   ├── DeleteConfirmDialog.tsx
│   └── FieldEditor.tsx / FieldDisplay.tsx / FileField.tsx / NoteLinkEditor.tsx
├── identity/
│   ├── IdentityManagerDialog.tsx
│   ├── CreateIdentityDialog.tsx
│   └── UnlockIdentityDialog.tsx
├── peers/
│   ├── WorkspacePeersDialog.tsx
│   ├── AcceptPeerDialog.tsx
│   └── AddPeerFromContactsDialog.tsx
├── contacts/
│   ├── ContactBookDialog.tsx
│   ├── AddContactDialog.tsx
│   └── EditContactDialog.tsx
├── invites/
│   ├── InviteManagerDialog.tsx
│   ├── CreateInviteDialog.tsx
│   └── ImportInviteDialog.tsx
├── swarm/
│   ├── SwarmOpenDialog.tsx
│   ├── SwarmInviteDialog.tsx
│   ├── SendSnapshotDialog.tsx
│   ├── CreateDeltaDialog.tsx
│   └── PostAcceptDialog.tsx
├── scripts/
│   ├── ScriptManagerDialog.tsx
│   └── ScriptEditor.tsx
├── operations/
│   └── OperationsLogDialog.tsx
├── settings/
│   ├── SettingsDialog.tsx
│   └── ManageThemesDialog.tsx
├── shared/
│   ├── EmptyState.tsx
│   ├── StatusMessage.tsx
│   ├── HoverTooltip.tsx
│   └── AttachmentsSection.tsx
└── manager/
    ├── WorkspaceManagerDialog.tsx
    └── WorkspacePropertiesDialog.tsx (if separate)
```

#### Approach

- Move files into subdirectories, update all imports
- Add `index.ts` barrel exports per subdirectory for clean imports — **caution:** barrel files should re-export only the component's default export, not internal types or hooks, to avoid circular import cycles (e.g., WorkspaceView imports InfoPanel, InfoPanel uses types from WorkspaceView's context)
- Do this in one commit per domain group to keep diffs reviewable
- Run `npx tsc --noEmit` after each move to catch broken imports

#### What this buys

- **Navigability** — find components by domain, not by scrolling a 42-item flat list
- **Ownership clarity** — related components are co-located
- **Scales better** — new features add files to the right subdirectory instead of further bloating the flat list
