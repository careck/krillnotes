# RBAC Plan B: Permission Management UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add visual permission indicators to the tree, permission management controls to the Info panel, a Share dialog for granting access, a Cascade preview for safe demotion/revocation, and role-aware UI disabling across all interactive elements.

**Architecture:** Fetches batch effective roles and share anchor data on workspace load, distributes through the component tree. Individual permission queries are fetched per-selected-note in the Info panel. Name resolution uses the existing `resolve_identity_name` Tauri command (three-tier: local identity → contacts → truncated key fallback). New dialogs (ShareDialog, CascadePreviewDialog) are self-contained modal components wired from the Info panel and context menu.

**Tech Stack:** React 19, TypeScript, Tailwind v4, i18next, Tauri v2 IPC

**Spec:** `docs/plans/2026-03-22-rbac-ui-design.md` §1-3, §5

**Depends on:** Plan A (backend queries — merged), Plan C (invite/onboard — PR #109 merged)

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `krillnotes-desktop/src/components/ShareDialog.tsx` | Peer picker + role selector for granting subtree access |
| `krillnotes-desktop/src/components/CascadePreviewDialog.tsx` | Impact preview dialog for demotion/revocation with opt-in checkboxes |

### Modified files

| File | Change |
|------|--------|
| `krillnotes-core/src/core/workspace/permissions.rs` | Add `get_share_anchor_ids()` + `is_root_owner()` methods; modify `visible_note_ids()` to include ghost ancestors |
| `krillnotes-desktop/src-tauri/src/commands/permissions.rs` | Add `get_share_anchor_ids` + `is_root_owner` Tauri commands |
| `krillnotes-desktop/src-tauri/src/lib.rs` | Register new commands in `tauri::generate_handler!` |
| `krillnotes-desktop/src/components/TreeView.tsx` | Pass `effectiveRoles` + `shareAnchorIds` props to TreeNode |
| `krillnotes-desktop/src/components/TreeNode.tsx` | Add role dots, share anchor icons, ghost ancestor styling |
| `krillnotes-desktop/src/components/ContextMenu.tsx` | Role-aware action disabling, "Share subtree..." entry, root-only root creation |
| `krillnotes-desktop/src/components/InfoPanel.tsx` | "Your role" row, edit/delete gating, "Shared with" section |
| `krillnotes-desktop/src/components/WorkspaceView.tsx` | Fetch permission state, wire new dialogs, pass data down |
| `krillnotes-desktop/src/i18n/locales/en.json` | New i18n keys for all permission UI |

---

### Task 1: Backend — add `get_share_anchor_ids` + `is_root_owner` methods and commands

**Files:**
- Modify: `krillnotes-core/src/core/workspace/permissions.rs`
- Modify: `krillnotes-desktop/src-tauri/src/commands/permissions.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Add `get_share_anchor_ids()` to Workspace**

In `krillnotes-core/src/core/workspace/permissions.rs`, add after the `preview_cascade` method (after line ~470):

```rust
/// Returns note IDs that have at least one explicit permission grant anchored to them.
/// Used by the tree to show share anchor icons.
pub fn get_share_anchor_ids(&self) -> Result<Vec<String>> {
    let conn = self.connection();
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT note_id FROM note_permissions WHERE note_id IS NOT NULL"
    ) {
        Ok(s) => s,
        Err(_) => return Ok(vec![]),  // table may not exist if RBAC not enabled
    };
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}
```

Note: Check the exact method name for getting the DB connection — it may be `self.connection()`, `self.conn()`, or `&self.conn`. Match the pattern used by the existing `get_note_permissions` method in the same file.

- [ ] **Step 2: Add `is_root_owner()` to Workspace**

In the same file, add:

```rust
/// Returns true if the current actor is the workspace root owner.
pub fn is_root_owner(&self) -> Result<bool> {
    let actor = self.actor_public_key()?;
    let owner = self.owner_public_key()?;
    Ok(actor == owner)
}
```

Note: Check the exact method names for getting actor and owner public keys. The `get_effective_role` method (lines ~97-168) performs this same comparison — look at how it gets the actor key and owner key and use the same pattern. The method may be called `actor_pubkey()`, `get_actor_public_key()`, or the keys may be accessed as fields. Read the first ~20 lines of `get_effective_role` to find the exact pattern.

- [ ] **Step 2b: Modify `visible_note_ids()` to include ghost ancestors**

**Critical:** The existing `visible_note_ids()` method (lines ~477-500) returns only the notes the user can access. It does NOT include ancestor path nodes. This means `list_notes` filters out parent nodes of granted subtrees, and the tree would show orphaned nodes with no path context.

Fix: After collecting the visible note IDs from grant propagation, walk up the parent chain for each granted subtree root and add those ancestor IDs to the set. These ancestors will appear in the tree with `effectiveRole = "none"` (since they have no grant), which the frontend renders as ghost ancestors.

In `visible_note_ids()`, after building the `visible` HashSet, add:

```rust
// Include ghost ancestors — walk up parent chain for each granted subtree root
// so the tree can show structural breadcrumbs
let grant_anchors: Vec<String> = conn
    .prepare("SELECT DISTINCT note_id FROM note_permissions WHERE user_id = ?1 AND note_id IS NOT NULL")?
    .query_map(rusqlite::params![actor], |row| row.get::<_, String>(0))?
    .collect::<std::result::Result<Vec<_>, _>>()?;

for anchor_id in &grant_anchors {
    let mut current_id = anchor_id.clone();
    loop {
        let parent: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![current_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        match parent {
            Some(pid) => {
                if visible.contains(&pid) {
                    break; // already visible, no need to go higher
                }
                visible.insert(pid.clone());
                current_id = pid;
            }
            None => break, // reached root
        }
    }
}
```

This ensures ghost ancestor nodes are included in `list_notes` results. They will have no entry in `get_all_effective_roles` (or role "none"), which the frontend uses to apply ghost styling.

- [ ] **Step 3: Add Tauri commands**

In `krillnotes-desktop/src-tauri/src/commands/permissions.rs`, add after the existing commands:

```rust
#[tauri::command]
pub fn get_share_anchor_ids(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<String>, String> {
    let workspaces = state.workspaces.lock().unwrap();
    let ws = workspaces
        .get(window.label())
        .ok_or("No workspace for this window")?;
    ws.get_share_anchor_ids().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_root_owner(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<bool, String> {
    let workspaces = state.workspaces.lock().unwrap();
    let ws = workspaces
        .get(window.label())
        .ok_or("No workspace for this window")?;
    ws.is_root_owner().map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Register commands in handler**

In `krillnotes-desktop/src-tauri/src/lib.rs`, find the `tauri::generate_handler!` macro invocation. Add `get_share_anchor_ids` and `is_root_owner` to the list alongside the existing permission commands (around lines 445-451).

- [ ] **Step 5: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib`

Expected: Compiles without errors.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace/permissions.rs krillnotes-desktop/src-tauri/src/commands/permissions.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat(core): add get_share_anchor_ids and is_root_owner queries"
```

---

### Task 2: WorkspaceView — fetch and distribute permission state

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Context:** WorkspaceView is the root component that manages notes, schemas, and now permissions. It needs to:
1. Fetch `get_all_effective_roles()` → `Record<string, string>` (noteId → role)
2. Fetch `get_share_anchor_ids()` → `string[]` (noteIds with anchored grants)
3. Fetch `is_root_owner()` → `boolean`
4. Re-fetch after mutations
5. Pass data down to TreeView, ContextMenu, InfoPanel

- [ ] **Step 1: Add permission state declarations**

In `WorkspaceView.tsx`, add state variables near the existing state declarations (around line 58):

```typescript
const [effectiveRoles, setEffectiveRoles] = useState<Record<string, string>>({});
const [shareAnchorIds, setShareAnchorIds] = useState<Set<string>>(new Set());
const [isRootOwner, setIsRootOwner] = useState(false);
```

- [ ] **Step 2: Add `loadPermissionState` function**

Add after `loadNotes` (or nearby):

```typescript
const loadPermissionState = async () => {
  try {
    const [roles, anchors, rootOwner] = await Promise.all([
      invoke<Record<string, string>>('get_all_effective_roles'),
      invoke<string[]>('get_share_anchor_ids'),
      invoke<boolean>('is_root_owner'),
    ]);
    setEffectiveRoles(roles);
    setShareAnchorIds(new Set(anchors));
    setIsRootOwner(rootOwner);
  } catch {
    // RBAC not enabled for this workspace — default to permissive
    setEffectiveRoles({});
    setShareAnchorIds(new Set());
    setIsRootOwner(true);
  }
};
```

- [ ] **Step 3: Call `loadPermissionState` on mount and after mutations**

Find the existing `useEffect` that calls `loadNotes()` on mount. Add `loadPermissionState()` alongside it:

```typescript
useEffect(() => {
  loadNotes();
  loadPermissionState();
}, [/* existing deps */]);
```

Also find places where `loadNotes()` is called after mutations (note creation, deletion, move, etc.) and add `loadPermissionState()` after each. Search for `loadNotes()` calls and co-locate permission refresh.

- [ ] **Step 4: Pass permission data to TreeView**

Find where `<TreeView>` is rendered (search for `<TreeView`). Add props:

```tsx
<TreeView
  // ... existing props ...
  effectiveRoles={effectiveRoles}
  shareAnchorIds={shareAnchorIds}
/>
```

- [ ] **Step 5: Pass `isRootOwner` to background context menu**

Find the `handleBackgroundContextMenu` function (or where the background context menu is opened — when `noteId` is null). The `isRootOwner` state needs to reach the ContextMenu.

Update the `contextMenu` state type to include `isRootOwner`:

```typescript
const [contextMenu, setContextMenu] = useState<{
  x: number; y: number;
  noteId: string | null;
  noteType: string;
  effectiveRole: string | null;
  isRootOwner: boolean;  // ← NEW
} | null>(null);
```

In the background context menu handler, set `isRootOwner` from state:

```typescript
setContextMenu({ x: e.clientX, y: e.clientY, noteId: null, noteType: '', effectiveRole: null, isRootOwner });
```

In the note context menu handler (`handleContextMenu`, lines 411-422), also set `isRootOwner`:

```typescript
setContextMenu({ ...existing, isRootOwner });
```

Pass to ContextMenu:

```tsx
<ContextMenu
  // ... existing props ...
  isRootOwner={contextMenu.isRootOwner}
/>
```

- [ ] **Step 6: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: Type errors for TreeView and ContextMenu not yet accepting the new props. This is fine — they're added in Tasks 3 and 4.

To unblock type-checking, temporarily add the props as optional (`?`) to the existing interfaces in TreeView and ContextMenu, then commit. The subsequent tasks will make them required as needed.

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat(desktop): fetch and distribute permission state from WorkspaceView"
```

---

### Task 3: Tree indicators — role dots, share anchor icons, ghost ancestors

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx`

**Context:** The tree shows a colored dot before each note title indicating the user's effective role. Share anchor nodes show a small share icon. Nodes with "none" effective role are ghost ancestors — greyed out, non-interactive, only for structural context.

Role dot colors:
- Green (`text-green-500`) = Owner / Root Owner
- Orange (`text-orange-500`) = Writer
- Yellow (`text-yellow-500`) = Reader
- No dot = ghost ancestor (no access)

- [ ] **Step 1: Add props to TreeView**

In `TreeView.tsx`, update the `TreeViewProps` interface (lines 11-29) to add:

```typescript
effectiveRoles?: Record<string, string>;
shareAnchorIds?: Set<string>;
```

Pass them through to each `<TreeNode>` in the map (lines 99-119):

```tsx
<TreeNode
  key={node.note.id}
  // ... existing props ...
  effectiveRoles={effectiveRoles}
  shareAnchorIds={shareAnchorIds}
/>
```

- [ ] **Step 2: Add props to TreeNode**

In `TreeNode.tsx`, update the `TreeNodeProps` interface (lines 11-28) to add:

```typescript
effectiveRoles?: Record<string, string>;
shareAnchorIds?: Set<string>;
```

- [ ] **Step 3: Compute role state in TreeNode render**

At the top of the TreeNode component function, derive the role for this node:

```typescript
const noteId = node.note.id;
const role = effectiveRoles?.[noteId] ?? null;
const isGhost = role === 'none' || (effectiveRoles && Object.keys(effectiveRoles).length > 0 && !role);
const isShareAnchor = shareAnchorIds?.has(noteId) ?? false;
```

Note: `isGhost` is true when RBAC is active (roles map is populated) AND this node has no access. When RBAC is not active (empty map), no nodes are ghosts.

- [ ] **Step 4: Add role dot rendering**

Find the title span (line 243): `<span className="text-sm truncate flex-1 min-w-0">{node.note.title}</span>`

Add a role dot BEFORE the title span (inside the same flex container):

```tsx
{!isGhost && role && (
  <span className={`text-[10px] mr-1 flex-shrink-0 ${
    role === 'owner' || role === 'root_owner' ? 'text-green-500' :
    role === 'writer' ? 'text-orange-500' :
    role === 'reader' ? 'text-yellow-500' : ''
  }`}>●</span>
)}
```

- [ ] **Step 5: Add share anchor icon**

After the role dot, add the share anchor icon:

```tsx
{isShareAnchor && (role === 'owner' || role === 'root_owner') && (
  <span className="text-[10px] mr-1 flex-shrink-0 text-zinc-400" title={t('tree.sharedSubtree', 'Shared subtree')}>👥</span>
)}
```

Import `useTranslation` at the top of TreeNode if not already imported:

```typescript
import { useTranslation } from 'react-i18next';
```

And add at the top of the component:

```typescript
const { t } = useTranslation();
```

- [ ] **Step 6: Ghost ancestor styling**

Wrap the node's outermost container with conditional ghost styling. Find the main `<div>` that wraps the tree item (around line 199). Add a conditional class:

```tsx
<div className={`... existing classes ... ${isGhost ? 'opacity-40 pointer-events-none' : ''}`}>
```

Wait — `pointer-events-none` would prevent expanding ghost nodes, which we need. Instead, ghost nodes should:
- Be visually dimmed (opacity)
- Still expandable (click the expand arrow)
- NOT selectable for editing (suppress onSelect)
- NOT show context menu (suppress onContextMenu)

Better approach — keep pointer-events but suppress specific handlers:

```tsx
onContextMenu={isGhost ? undefined : (e) => onContextMenu(e, noteId)}
onClick={isGhost ? undefined : () => onSelect(noteId)}
```

And add the opacity class to the title area only (not the expand arrow):

```tsx
<span className={`text-sm truncate flex-1 min-w-0 ${isGhost ? 'text-zinc-400 italic' : ''}`}>
  {node.note.title}
</span>
```

The expand/collapse button should remain interactive for ghost nodes so users can navigate to child subtrees.

- [ ] **Step 7: Pass props to recursive children**

TreeNode renders its children recursively. Find where child `<TreeNode>` is rendered (search for `children.map` or recursive render). Pass `effectiveRoles` and `shareAnchorIds` to child nodes:

```tsx
{node.children.map(child => (
  <TreeNode
    key={child.note.id}
    // ... existing props ...
    effectiveRoles={effectiveRoles}
    shareAnchorIds={shareAnchorIds}
  />
))}
```

- [ ] **Step 8: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: Passes (or only pre-existing errors).

- [ ] **Step 9: Commit**

```bash
git add krillnotes-desktop/src/components/TreeView.tsx krillnotes-desktop/src/components/TreeNode.tsx
git commit -m "feat(desktop): add role dots, share anchor icons, and ghost ancestor styling to tree"
```

---

### Task 4: Role-aware context menu

**Files:**
- Modify: `krillnotes-desktop/src/components/ContextMenu.tsx`

**Context:** The context menu must:
- Grey out "Add Root" for non-root-owners (background menu)
- Grey out "Add Child", "Add Sibling", "Edit", "Delete" for Readers
- Grey out "Add Child", "Add Sibling" for notes where user is Reader (Writers can create)
- Add "Share subtree..." entry for Owner+ (similar to existing "Invite to subtree...")
- Hide all actions for ghost ancestors (effectiveRole is "none")

- [ ] **Step 1: Add new props to ContextMenuProps**

In `ContextMenu.tsx`, update the props interface (lines 11-30):

```typescript
interface ContextMenuProps {
  // ... existing props ...
  isRootOwner?: boolean;                                     // ← NEW
  onShareSubtree?: (noteId: string) => void;                 // ← NEW
}
```

- [ ] **Step 2: Gate "Add Root" in background menu**

Find the background menu section (lines 67-72) where "Add Root" is rendered:

```tsx
{!noteId && (
  <button ... onClick={onAddRoot} ...>
    {t('contextMenu.addRoot', 'Add Root Note')}
  </button>
)}
```

Make it disabled for non-root-owners:

```tsx
{!noteId && (
  <button
    onClick={onAddRoot}
    disabled={!isRootOwner}
    className={`w-full text-left px-3 py-1.5 text-sm ${
      isRootOwner !== false
        ? 'hover:bg-zinc-100 dark:hover:bg-zinc-700'
        : 'opacity-40 cursor-not-allowed'
    }`}
  >
    {t('contextMenu.addRoot', 'Add Root Note')}
  </button>
)}
```

- [ ] **Step 3: Derive permission flags for note menu**

At the top of the note menu section, add permission checks:

```typescript
const canWrite = !effectiveRole || effectiveRole === 'owner' || effectiveRole === 'root_owner' || effectiveRole === 'writer';
const canManage = !effectiveRole || effectiveRole === 'owner' || effectiveRole === 'root_owner';
const isGhost = effectiveRole === 'none';
```

Note: when `effectiveRole` is `null` or `undefined`, RBAC is not active — allow everything (permissive default).

- [ ] **Step 4: Hide all actions for ghost nodes**

If `isGhost`, render nothing (or a minimal "no access" message):

```tsx
{noteId && isGhost && (
  <p className="px-3 py-1.5 text-xs text-zinc-400 italic">
    {t('contextMenu.noAccess', 'No access')}
  </p>
)}
```

- [ ] **Step 5: Gate create/edit/delete actions for Readers**

For each action button in the note menu (lines 76-143), add `disabled` and style conditions:

**Add Child** (line ~76-81):
```tsx
<button
  onClick={onAddChild}
  disabled={!canWrite || isLeaf}
  className={`... ${!canWrite ? 'opacity-40 cursor-not-allowed' : 'hover:bg-zinc-100 dark:hover:bg-zinc-700'}`}
>
```

**Add Sibling** (line ~82-87): same pattern with `disabled={!canWrite}`

**Edit** (line ~88-93): same pattern with `disabled={!canWrite}`

**Delete** (line ~138-143): same pattern with `disabled={!canWrite}`

Keep Copy and Paste actions available for all roles (reading/copying is fine).

- [ ] **Step 6: Add "Share subtree..." entry for Owner+**

After the existing "Invite to subtree..." block (line ~136), add:

```tsx
{noteId && canManage && onShareSubtree && (
  <button
    onClick={() => { onShareSubtree(noteId); onClose(); }}
    className="w-full text-left px-3 py-1.5 text-sm hover:bg-zinc-100 dark:hover:bg-zinc-700"
  >
    {t('contextMenu.shareSubtree', 'Share subtree...')}
  </button>
)}
```

- [ ] **Step 7: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/components/ContextMenu.tsx
git commit -m "feat(desktop): add role-aware action disabling and share entry to context menu"
```

---

### Task 5: InfoPanel — role display + edit/delete gating

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Context:** The Info panel's metadata section (lines 627-651) currently shows type, created, modified, and ID. We add a "Your role" row and gate the edit/delete buttons based on effective role.

- [ ] **Step 1: Add `effectiveRole` prop to InfoPanel**

Update the `InfoPanelProps` interface (lines 22-32):

```typescript
interface InfoPanelProps {
  // ... existing props ...
  effectiveRole?: EffectiveRoleInfo | null;  // ← NEW
}
```

Import the type at the top:

```typescript
import type { EffectiveRoleInfo } from '../types';
```

- [ ] **Step 2: Add "Your role" row to metadata section**

In the metadata details section (lines 627-651), after the existing metadata rows (type, created, modified, id), add:

```tsx
{effectiveRole && effectiveRole.role !== 'none' && (
  <div className="flex items-start gap-2 text-xs">
    <span className="text-zinc-500 w-20 flex-shrink-0">{t('info.yourRole', 'Your role')}</span>
    <div className="flex flex-col">
      <span className="flex items-center gap-1">
        <span className={
          effectiveRole.role === 'owner' || effectiveRole.role === 'root_owner' ? 'text-green-500' :
          effectiveRole.role === 'writer' ? 'text-orange-500' :
          effectiveRole.role === 'reader' ? 'text-yellow-500' : ''
        }>●</span>
        <span className="capitalize">
          {effectiveRole.role === 'root_owner'
            ? t('info.roleRootOwner', 'Owner (Root)')
            : t(`roles.${effectiveRole.role}`, effectiveRole.role)}
        </span>
      </span>
      {effectiveRole.inheritedFrom && effectiveRole.inheritedFromTitle && (
        <span className="text-zinc-400 text-[11px]">
          {t('info.inheritedFrom', 'Inherited from')}{' '}
          <button
            className="text-blue-500 hover:underline"
            onClick={() => onLinkNavigate(effectiveRole.inheritedFrom!)}
          >
            {effectiveRole.inheritedFromTitle}
          </button>
          {effectiveRole.grantedBy && (
            <span> · {t('info.grantedBy', 'granted by')} {nameMap[effectiveRole.grantedBy] ?? effectiveRole.grantedBy.slice(0, 8)}</span>
          )}
        </span>
      )}
    </div>
  </div>
)}
```

- [ ] **Step 3: Fetch effective role for selected note**

This can be done in WorkspaceView and passed down as a prop, OR fetched inside InfoPanel. Since InfoPanel already fetches note-specific data, add an effect inside InfoPanel:

```typescript
const [roleInfo, setRoleInfo] = useState<EffectiveRoleInfo | null>(null);

useEffect(() => {
  if (!selectedNote) { setRoleInfo(null); return; }
  invoke<EffectiveRoleInfo>('get_effective_role', { noteId: selectedNote.id })
    .then(setRoleInfo)
    .catch(() => setRoleInfo(null));
}, [selectedNote?.id, refreshSignal]);
```

Then use `roleInfo` (fetched internally) OR `effectiveRole` (passed as prop) — whichever is set:

```typescript
const activeRole = effectiveRole ?? roleInfo;
```

This gives flexibility: WorkspaceView can pass the prop if it already has the data, or InfoPanel fetches it itself.

- [ ] **Step 4: Gate edit button for Readers**

Find the edit/delete buttons (lines 248-278, shown in view mode). Wrap or gate them:

```tsx
const canEdit = !activeRole || activeRole.role === 'owner' || activeRole.role === 'root_owner' || activeRole.role === 'writer';
```

For the edit button:
```tsx
{canEdit && (
  <button onClick={() => setIsEditing(true)} ...>
    {t('common.edit', 'Edit')}
  </button>
)}
```

For the delete button:
```tsx
{canEdit && (
  <button onClick={() => onDeleteRequest(selectedNote.id)} ...>
    {t('common.delete', 'Delete')}
  </button>
)}
```

Note: The spec says Readers cannot edit or delete. Writers can edit and delete (backend enforces authorship checks for Writers). Owners have full control.

- [ ] **Step 5: Update React.memo comparison**

InfoPanel uses `React.memo` with a custom comparison function (around line 660-665). Add `effectiveRole` to the comparison so role changes trigger re-renders:

```typescript
// In the memo comparison, add:
prev.effectiveRole?.role === next.effectiveRole?.role &&
prev.effectiveRole?.inheritedFrom === next.effectiveRole?.inheritedFrom
```

Callback props (`onShareSubtree`, `onRoleChange`, `onRevokeGrant`) don't need comparison if WorkspaceView uses stable references (`useCallback`).

- [ ] **Step 6: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 7: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(desktop): add role display and edit/delete gating to InfoPanel"
```

---

### Task 6: InfoPanel — Shared with section

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Context:** Below the "Your role" row, Owner+ users see a "Shared with" section showing:
1. **Anchored grants** (at this node) — with role dropdown to change + ✕ to revoke
2. **Inherited grants** (from parent) — read-only with "via Anchor" link
3. **"+ Share this subtree..."** button

Name resolution uses the existing `resolve_identity_name` Tauri command.

- [ ] **Step 1: Add state for permissions and name resolution**

Add state variables inside InfoPanel:

```typescript
const [anchoredGrants, setAnchoredGrants] = useState<PermissionGrantRow[]>([]);
const [inheritedGrants, setInheritedGrants] = useState<InheritedGrant[]>([]);
const [nameMap, setNameMap] = useState<Record<string, string>>({});
```

Import types:

```typescript
import type { PermissionGrantRow, InheritedGrant } from '../types';
```

- [ ] **Step 2: Fetch permissions and resolve names**

Add a `useEffect` that loads permission data when a note is selected:

```typescript
useEffect(() => {
  if (!selectedNote || !activeRole || (activeRole.role !== 'owner' && activeRole.role !== 'root_owner')) {
    setAnchoredGrants([]);
    setInheritedGrants([]);
    return;
  }
  const load = async () => {
    try {
      const [anchored, inherited] = await Promise.all([
        invoke<PermissionGrantRow[]>('get_note_permissions', { noteId: selectedNote.id }),
        invoke<InheritedGrant[]>('get_inherited_permissions', { noteId: selectedNote.id }),
      ]);
      setAnchoredGrants(anchored);
      setInheritedGrants(inherited);

      // Resolve display names for all unique public keys
      const allKeys = new Set<string>();
      anchored.forEach(g => { allKeys.add(g.userId); allKeys.add(g.grantedBy); });
      inherited.forEach(g => { allKeys.add(g.grant.userId); allKeys.add(g.grant.grantedBy); });
      // Also resolve the grantedBy from the effective role (for "granted by X" display)
      if (activeRole?.grantedBy) allKeys.add(activeRole.grantedBy);

      const names: Record<string, string> = {};
      await Promise.all(
        Array.from(allKeys).map(async (key) => {
          try {
            const name = await invoke<string>('resolve_identity_name', { publicKey: key });
            names[key] = name;
          } catch {
            names[key] = key.slice(0, 8) + '…';
          }
        })
      );
      setNameMap(names);
    } catch {
      setAnchoredGrants([]);
      setInheritedGrants([]);
    }
  };
  load();
}, [selectedNote?.id, activeRole?.role, refreshSignal]);
```

Note: Check the exact Tauri command name and parameter for `resolve_identity_name` — it's defined in `krillnotes-desktop/src-tauri/src/commands/identity.rs` at line ~44. The parameter is `publicKey` (camelCase in TS).

Note: The `currentUserKey` variable used to filter out the user's own grants in the anchored list needs to be resolved. Options:
1. Add a prop `currentUserPublicKey?: string` to InfoPanel, passed from WorkspaceView
2. Call `invoke('resolve_identity_name', { publicKey: '__self__' })` — but this doesn't give the key
3. Store the actor's public key in WorkspaceView state (fetch once from a new or existing command)

The simplest approach: the `EffectiveRoleInfo.grantedBy` for a self-grant would be the user's own key. Alternatively, add a `currentUserKey` prop passed from WorkspaceView. The implementation agent should check if there's an existing way to get the current user's public key (e.g., from `WorkspaceInfo` or a Tauri command). If not, the filter can be skipped initially and added as a follow-up.

- [ ] **Step 3: Add props for dialog callbacks**

Add callback props to `InfoPanelProps`:

```typescript
interface InfoPanelProps {
  // ... existing props ...
  onShareSubtree?: (noteId: string) => void;               // ← NEW
  onRoleChange?: (noteId: string, userId: string, newRole: string, oldRole: string) => void;  // ← NEW
  onRevokeGrant?: (noteId: string, userId: string) => void; // ← NEW
}
```

- [ ] **Step 4: Render "Shared with — anchored here" section**

After the "Your role" row in the metadata section, add:

```tsx
{(activeRole?.role === 'owner' || activeRole?.role === 'root_owner') && (
  <div className="mt-3 border-t dark:border-zinc-700 pt-2">
    {/* Anchored grants */}
    {anchoredGrants.length > 0 && (
      <div className="mb-2">
        <p className="text-xs font-medium text-zinc-500 mb-1">
          {t('info.sharedAnchored', 'Shared with — anchored here')}
        </p>
        {anchoredGrants.filter(g => g.userId !== currentUserKey).map(grant => (
          <div key={grant.userId} className="flex items-center gap-1.5 py-0.5 text-xs">
            <span className={
              grant.role === 'owner' ? 'text-green-500' :
              grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
            }>●</span>
            <span className="flex-1 truncate">{nameMap[grant.userId] ?? grant.userId.slice(0, 8)}</span>
            <select
              className="text-xs border rounded px-1 py-0.5 dark:bg-zinc-800 dark:border-zinc-600"
              value={grant.role}
              onChange={(e) => onRoleChange?.(selectedNote!.id, grant.userId, e.target.value, grant.role)}
            >
              {(activeRole?.role === 'root_owner' || activeRole?.role === 'owner') && (
                <option value="owner">{t('roles.ownerShort', 'Owner')}</option>
              )}
              <option value="writer">{t('roles.writerShort', 'Writer')}</option>
              <option value="reader">{t('roles.readerShort', 'Reader')}</option>
            </select>
            <button
              onClick={() => onRevokeGrant?.(selectedNote!.id, grant.userId)}
              className="text-red-400 hover:text-red-600 px-1"
              title={t('info.revoke', 'Revoke')}
            >✕</button>
          </div>
        ))}
      </div>
    )}

    {/* Inherited grants */}
    {inheritedGrants.length > 0 && (
      <div className="mb-2">
        <p className="text-xs font-medium text-zinc-500 mb-1">
          {t('info.accessFromParent', 'Access from parent grants')}
        </p>
        {inheritedGrants.map(ig => (
          <div key={`${ig.grant.userId}-${ig.anchorNoteId}`} className="flex items-center gap-1.5 py-0.5 text-xs opacity-60">
            <span className={
              ig.grant.role === 'owner' ? 'text-green-500' :
              ig.grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
            }>●</span>
            <span className="flex-1 truncate">{nameMap[ig.grant.userId] ?? ig.grant.userId.slice(0, 8)}</span>
            <span className="text-zinc-400">{ig.grant.role}</span>
            <button
              onClick={() => onLinkNavigate(ig.anchorNoteId)}
              className="text-blue-500 hover:underline text-[11px]"
            >
              {t('info.via', 'via')} {ig.anchorNoteTitle ?? ig.anchorNoteId.slice(0, 8)}
            </button>
          </div>
        ))}
      </div>
    )}

    {/* Share button */}
    <button
      onClick={() => onShareSubtree?.(selectedNote!.id)}
      className="text-xs text-blue-600 hover:underline mt-1"
    >
      + {t('info.shareSubtree', 'Share this subtree...')}
    </button>
  </div>
)}
```

- [ ] **Step 5: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(desktop): add shared-with section with grant management to InfoPanel"
```

---

### Task 7: ShareDialog component

**Files:**
- Create: `krillnotes-desktop/src/components/ShareDialog.tsx`

**Context:** The ShareDialog is opened from the "+ Share this subtree..." button or the context menu. It shows a searchable peer list (excluding peers who already have grants at this node), a role picker (capped at the user's own role), and a confirm button that calls `set_permission`.

- [ ] **Step 1: Create the component**

```typescript
import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { PeerInfo, PermissionGrantRow } from '../types';

interface ShareDialogProps {
  open: boolean;
  noteId: string;
  noteTitle: string;
  currentUserRole: string;  // "owner" | "root_owner" — caps available roles
  onComplete: () => void;
  onClose: () => void;
}

export function ShareDialog({
  open, noteId, noteTitle, currentUserRole, onComplete, onClose,
}: ShareDialogProps) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [existingGrants, setExistingGrants] = useState<PermissionGrantRow[]>([]);
  const [search, setSearch] = useState('');
  const [selectedPeerId, setSelectedPeerId] = useState<string | null>(null);
  const [role, setRole] = useState<string>('writer');
  const [processing, setProcessing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    Promise.all([
      invoke<PeerInfo[]>('list_workspace_peers'),
      invoke<PermissionGrantRow[]>('get_note_permissions', { noteId }),
    ]).then(([p, g]) => {
      setPeers(p);
      setExistingGrants(g);
    }).catch(() => {});
  }, [open, noteId]);

  // Filter out peers who already have an explicit grant at this node
  const availablePeers = useMemo(() => {
    const grantedKeys = new Set(existingGrants.map(g => g.userId));
    return peers.filter(p =>
      !grantedKeys.has(p.peerIdentityId) &&
      (search === '' ||
        p.displayName.toLowerCase().includes(search.toLowerCase()) ||
        p.fingerprint?.toLowerCase().includes(search.toLowerCase()))
    );
  }, [peers, existingGrants, search]);

  if (!open) return null;

  const handleShare = async () => {
    if (!selectedPeerId) return;
    const peer = peers.find(p => p.peerIdentityId === selectedPeerId);
    if (!peer) return;

    setProcessing(true);
    setError(null);
    try {
      await invoke('set_permission', {
        noteId,
        userId: peer.peerIdentityId,
        role,
      });
      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-1">
          {t('share.title', 'Share subtree')}
        </h2>
        <p className="text-xs text-zinc-500 mb-4">{noteTitle}</p>

        {/* Search */}
        <input
          type="text"
          placeholder={t('share.searchPeers', 'Search peers...')}
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="w-full border rounded px-3 py-2 text-sm mb-2 dark:bg-zinc-800 dark:border-zinc-700"
        />

        {/* Peer list */}
        <div className="max-h-40 overflow-y-auto border rounded dark:border-zinc-700 mb-3">
          {availablePeers.length === 0 ? (
            <p className="text-xs text-zinc-400 p-3 text-center">
              {t('share.noPeers', 'No available peers')}
            </p>
          ) : (
            availablePeers.map(peer => (
              <button
                key={peer.peerIdentityId}
                onClick={() => setSelectedPeerId(peer.peerIdentityId)}
                className={`w-full text-left px-3 py-2 text-sm flex items-center gap-2 ${
                  selectedPeerId === peer.peerIdentityId
                    ? 'bg-blue-50 dark:bg-blue-900/30'
                    : 'hover:bg-zinc-50 dark:hover:bg-zinc-800'
                }`}
              >
                <span className="flex-1 truncate">{peer.displayName}</span>
                <span className="text-xs text-zinc-400 font-mono">
                  {peer.fingerprint?.slice(0, 8) ?? ''}
                </span>
              </button>
            ))
          )}
        </div>
        <p className="text-xs text-zinc-400 mb-3">
          {t('share.peerCount', '{{available}} of {{total}} peers', { available: availablePeers.length, total: peers.length })}
        </p>

        {/* Role selector */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t('share.role', 'Role')}
          </label>
          <select
            className="w-full border rounded px-3 py-2 dark:bg-zinc-800 dark:border-zinc-700"
            value={role}
            onChange={e => setRole(e.target.value)}
            disabled={processing}
          >
            {(currentUserRole === 'root_owner' || currentUserRole === 'owner') && (
              <option value="owner">{t('roles.owner', 'Owner — full control of subtree')}</option>
            )}
            <option value="writer">{t('roles.writer', 'Writer — create and edit notes')}</option>
            <option value="reader">{t('roles.reader', 'Reader — view only')}</option>
          </select>
        </div>

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={processing}
            className="px-4 py-2 text-sm rounded border dark:border-zinc-700 disabled:opacity-50"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={handleShare}
            disabled={processing || !selectedPeerId}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
          >
            {processing ? t('common.saving', 'Saving…') : t('share.confirm', 'Share')}
          </button>
        </div>
      </div>
    </div>
  );
}
```

Note: Verify that `PeerInfo.peerIdentityId` is the Ed25519 public key (same format as `PermissionGrantRow.userId`). If they differ, adjust the filtering and the `userId` passed to `set_permission`. Check by reading the Rust `PeerInfo` struct in `peer_registry.rs`.

- [ ] **Step 2: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/ShareDialog.tsx
git commit -m "feat(desktop): add ShareDialog for granting subtree access to peers"
```

---

### Task 8: CascadePreviewDialog component

**Files:**
- Create: `krillnotes-desktop/src/components/CascadePreviewDialog.tsx`

**Context:** When demoting or revoking a peer who has granted downstream access, this dialog shows the impact and lets the user choose which downstream grants to also revoke. Three actions: Cancel, Demote/Revoke only (no cascade), Demote/Revoke & revoke selected.

- [ ] **Step 1: Create the component**

```typescript
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { CascadeImpactRow } from '../types';

interface CascadePreviewDialogProps {
  open: boolean;
  noteId: string;
  userId: string;
  userName: string;
  action: 'demote' | 'revoke';
  newRole?: string;          // For demote: the target role
  oldRole: string;           // Current role
  noteTitle: string;
  onConfirm: (revokeGrants: Array<{ noteId: string; userId: string }>) => void;  // Empty = no cascade
  onClose: () => void;
}

export function CascadePreviewDialog({
  open, noteId, userId, userName, action, newRole, oldRole, noteTitle,
  onConfirm, onClose,
}: CascadePreviewDialogProps) {
  const { t } = useTranslation();
  const [impacts, setImpacts] = useState<CascadeImpactRow[]>([]);
  const [nameMap, setNameMap] = useState<Record<string, string>>({});
  const [checked, setChecked] = useState<Set<string>>(new Set());  // keyed by "noteId:userId"
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    const effectiveNewRole = action === 'revoke' ? 'none' : (newRole ?? 'none');
    invoke<CascadeImpactRow[]>('preview_cascade', {
      noteId,
      userId,
      newRole: effectiveNewRole,
    })
      .then(async (rows) => {
        setImpacts(rows);
        // Pre-check all (they're all invalid under the new role)
        // Key by "noteId:userId" to handle grants at different notes
        setChecked(new Set(rows.map(r => `${r.grant.noteId}:${r.grant.userId}`)));

        // Resolve names
        const keys = new Set(rows.map(r => r.grant.userId));
        const names: Record<string, string> = {};
        await Promise.all(
          Array.from(keys).map(async (key) => {
            try {
              names[key] = await invoke<string>('resolve_identity_name', { publicKey: key });
            } catch {
              names[key] = key.slice(0, 8) + '…';
            }
          })
        );
        setNameMap(names);
      })
      .catch(() => setImpacts([]))
      .finally(() => setLoading(false));
  }, [open, noteId, userId, action, newRole]);

  if (!open) return null;

  const grantKey = (g: CascadeImpactRow) => `${g.grant.noteId}:${g.grant.userId}`;

  const toggleCheck = (key: string) => {
    setChecked(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const actionLabel = action === 'revoke'
    ? t('cascade.revoking', 'Revoking access for')
    : t('cascade.demoting', 'Demoting');

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-2">
          {actionLabel} {userName}
        </h2>
        {action === 'demote' && (
          <p className="text-sm text-zinc-500 mb-1">
            {oldRole} → {newRole} {t('cascade.on', 'on')} {noteTitle}
          </p>
        )}
        {action === 'revoke' && (
          <p className="text-sm text-zinc-500 mb-1">
            {t('cascade.revokeFrom', 'Revoking from')} {noteTitle}
          </p>
        )}

        {loading ? (
          <p className="text-sm text-zinc-400 py-4">{t('common.loading', 'Loading…')}</p>
        ) : impacts.length === 0 ? (
          <p className="text-sm text-zinc-500 py-4 mb-4">
            {t('cascade.noImpact', 'No downstream grants will be affected.')}
          </p>
        ) : (
          <>
            <p className="text-sm text-zinc-500 mb-3">
              {t('cascade.explanation', 'This user previously granted access to others. These grants would no longer be valid:')}
            </p>
            <div className="max-h-48 overflow-y-auto border rounded dark:border-zinc-700 mb-4">
              {impacts.map(impact => (
                <label
                  key={grantKey(impact)}
                  className="flex items-center gap-2 px-3 py-2 text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800 cursor-pointer"
                >
                  <input
                    type="checkbox"
                    checked={checked.has(grantKey(impact))}
                    onChange={() => toggleCheck(grantKey(impact))}
                    className="rounded"
                  />
                  <span className={
                    impact.grant.role === 'owner' ? 'text-green-500' :
                    impact.grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
                  }>●</span>
                  <span className="flex-1">
                    {nameMap[impact.grant.userId] ?? impact.grant.userId.slice(0, 8)}
                    <span className="text-zinc-400 ml-1">— {impact.grant.role}</span>
                  </span>
                  <span className="text-xs text-zinc-400">{impact.reason}</span>
                </label>
              ))}
            </div>
          </>
        )}

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded border dark:border-zinc-700"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={() => onConfirm([])}
            className="px-4 py-2 text-sm rounded border dark:border-zinc-700"
          >
            {action === 'demote'
              ? t('cascade.demoteOnly', 'Demote only')
              : t('cascade.revokeOnly', 'Revoke only')}
          </button>
          {impacts.length > 0 && (
            <button
              onClick={() => onConfirm(
                impacts
                  .filter(i => checked.has(grantKey(i)))
                  .map(i => ({ noteId: i.grant.noteId, userId: i.grant.userId }))
              )}
              className="px-4 py-2 text-sm rounded bg-red-600 text-white"
            >
              {action === 'demote'
                ? t('cascade.demoteAndRevoke', 'Demote & revoke selected')
                : t('cascade.revokeAndRevoke', 'Revoke & revoke selected')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/CascadePreviewDialog.tsx
git commit -m "feat(desktop): add CascadePreviewDialog for opt-in downstream grant revocation"
```

---

### Task 9: Wire dialogs from WorkspaceView + InfoPanel integration

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx` (add callback wiring)

**Context:** ShareDialog and CascadePreviewDialog need to be rendered in WorkspaceView and triggered from both the context menu and the InfoPanel's shared-with section.

- [ ] **Step 1: Add dialog state to WorkspaceView**

```typescript
const [shareScope, setShareScope] = useState<{ noteId: string; noteTitle: string } | null>(null);
const [cascadeState, setCascadeState] = useState<{
  noteId: string;
  userId: string;
  userName: string;
  action: 'demote' | 'revoke';
  newRole?: string;
  oldRole: string;
  noteTitle: string;
} | null>(null);
```

- [ ] **Step 2: Add handler functions**

```typescript
const handleShareSubtree = (noteId: string) => {
  const note = notes.find(n => n.id === noteId);
  setShareScope({ noteId, noteTitle: note?.title ?? noteId });
};

const handleRoleChange = async (noteId: string, userId: string, newRole: string, oldRole: string) => {
  // Check for cascade impact first
  try {
    const impacts = await invoke<CascadeImpactRow[]>('preview_cascade', {
      noteId, userId, newRole,
    });
    if (impacts.length > 0) {
      const name = await invoke<string>('resolve_identity_name', { publicKey: userId }).catch(() => userId.slice(0, 8));
      const note = notes.find(n => n.id === noteId);
      setCascadeState({
        noteId, userId, userName: name,
        action: 'demote', newRole, oldRole,
        noteTitle: note?.title ?? noteId,
      });
      return;
    }
  } catch { /* no cascade needed */ }

  // No cascade impact — apply directly
  try {
    await invoke('set_permission', { noteId, userId, role: newRole });
    loadPermissionState();
    setRefreshSignal(prev => prev + 1);  // trigger InfoPanel refresh
  } catch (e) {
    console.error('Failed to change role:', e);
  }
};

const handleRevokeGrant = async (noteId: string, userId: string) => {
  // Check for cascade impact first
  try {
    const impacts = await invoke<CascadeImpactRow[]>('preview_cascade', {
      noteId, userId, newRole: 'none',
    });
    if (impacts.length > 0) {
      const name = await invoke<string>('resolve_identity_name', { publicKey: userId }).catch(() => userId.slice(0, 8));
      const note = notes.find(n => n.id === noteId);
      setCascadeState({
        noteId, userId, userName: name,
        action: 'revoke', oldRole: 'unknown',
        noteTitle: note?.title ?? noteId,
      });
      return;
    }
  } catch { /* no cascade needed */ }

  // No cascade impact — revoke directly
  try {
    await invoke('revoke_permission', { noteId, userId });
    loadPermissionState();
    setRefreshSignal(prev => prev + 1);
  } catch (e) {
    console.error('Failed to revoke:', e);
  }
};

const handleCascadeConfirm = async (revokeGrants: Array<{ noteId: string; userId: string }>) => {
  if (!cascadeState) return;
  try {
    // Apply the primary action
    if (cascadeState.action === 'demote') {
      await invoke('set_permission', {
        noteId: cascadeState.noteId,
        userId: cascadeState.userId,
        role: cascadeState.newRole,
      });
    } else {
      await invoke('revoke_permission', {
        noteId: cascadeState.noteId,
        userId: cascadeState.userId,
      });
    }

    // Revoke selected downstream grants — each at its own anchor node
    for (const grant of revokeGrants) {
      await invoke('revoke_permission', {
        noteId: grant.noteId,
        userId: grant.userId,
      });
    }

    loadPermissionState();
    setRefreshSignal(prev => prev + 1);
  } catch (e) {
    console.error('Cascade action failed:', e);
  } finally {
    setCascadeState(null);
  }
};
```

Import at top of file:

```typescript
import { ShareDialog } from './ShareDialog';
import { CascadePreviewDialog } from './CascadePreviewDialog';
import type { CascadeImpactRow } from '../types';
```

- [ ] **Step 3: Pass callbacks to ContextMenu**

Update the ContextMenu rendering to pass `onShareSubtree`:

```tsx
<ContextMenu
  // ... existing props ...
  onShareSubtree={handleShareSubtree}
/>
```

- [ ] **Step 4: Pass callbacks to InfoPanel**

```tsx
<InfoPanel
  // ... existing props ...
  onShareSubtree={handleShareSubtree}
  onRoleChange={handleRoleChange}
  onRevokeGrant={handleRevokeGrant}
/>
```

- [ ] **Step 5: Render ShareDialog**

After the existing InviteManagerDialog block:

```tsx
{shareScope && (
  <ShareDialog
    open={true}
    noteId={shareScope.noteId}
    noteTitle={shareScope.noteTitle}
    currentUserRole={effectiveRoles[shareScope.noteId] ?? 'owner'}
    onComplete={() => {
      setShareScope(null);
      loadPermissionState();
      setRefreshSignal(prev => prev + 1);
    }}
    onClose={() => setShareScope(null)}
  />
)}
```

- [ ] **Step 6: Render CascadePreviewDialog**

```tsx
{cascadeState && (
  <CascadePreviewDialog
    open={true}
    noteId={cascadeState.noteId}
    userId={cascadeState.userId}
    userName={cascadeState.userName}
    action={cascadeState.action}
    newRole={cascadeState.newRole}
    oldRole={cascadeState.oldRole}
    noteTitle={cascadeState.noteTitle}
    onConfirm={handleCascadeConfirm}
    onClose={() => setCascadeState(null)}
  />
)}
```

- [ ] **Step 7: Ensure `refreshSignal` exists and is passed to InfoPanel**

Check if `WorkspaceView` already has a `refreshSignal` state variable that triggers InfoPanel re-fetches. The `InfoPanelProps` already has `refreshSignal?: number`. If `WorkspaceView` doesn't have it, add:

```typescript
const [refreshSignal, setRefreshSignal] = useState(0);
```

And pass it: `refreshSignal={refreshSignal}`

- [ ] **Step 8: Type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

- [ ] **Step 9: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat(desktop): wire ShareDialog and CascadePreviewDialog from WorkspaceView"
```

---

### Task 10: i18n strings

**Files:**
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`

- [ ] **Step 1: Add all new translation keys**

Merge these into the existing JSON structure (don't overwrite existing sections — add to them):

```json
{
  "info": {
    "yourRole": "Your role",
    "roleRootOwner": "Owner (Root)",
    "inheritedFrom": "Inherited from",
    "grantedBy": "granted by",
    "sharedAnchored": "Shared with — anchored here",
    "accessFromParent": "Access from parent grants",
    "shareSubtree": "Share this subtree...",
    "revoke": "Revoke",
    "via": "via"
  },
  "share": {
    "title": "Share subtree",
    "searchPeers": "Search peers...",
    "noPeers": "No available peers",
    "peerCount": "{{available}} of {{total}} peers",
    "role": "Role",
    "confirm": "Share"
  },
  "cascade": {
    "revoking": "Revoking access for",
    "demoting": "Demoting",
    "on": "on",
    "revokeFrom": "Revoking from",
    "noImpact": "No downstream grants will be affected.",
    "explanation": "This user previously granted access to others. These grants would no longer be valid:",
    "demoteOnly": "Demote only",
    "revokeOnly": "Revoke only",
    "demoteAndRevoke": "Demote & revoke selected",
    "revokeAndRevoke": "Revoke & revoke selected"
  },
  "tree": {
    "sharedSubtree": "Shared subtree"
  },
  "contextMenu": {
    "shareSubtree": "Share subtree...",
    "noAccess": "No access"
  },
  "roles": {
    "ownerShort": "Owner",
    "writerShort": "Writer",
    "readerShort": "Reader"
  }
}
```

Check for collisions with existing keys in these sections. In particular:
- `roles.owner`, `roles.writer`, `roles.reader` already exist with descriptions — the `*Short` variants are for compact display in the role dropdown
- `contextMenu.inviteToSubtree` already exists — don't overwrite
- `info` section may or may not exist — merge carefully

- [ ] **Step 2: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/en.json
git commit -m "feat(i18n): add English strings for permission management UI"
```

---

### Task 11: Cleanup + type-check + smoke test

**Files:**
- Possibly modify: `krillnotes-rbac/src/gate.rs`, `krillnotes-rbac/src/mod.rs`

- [ ] **Step 1: Fix compiler warnings**

The following compiler warnings exist and should be cleaned up:

1. `krillnotes-rbac/src/mod.rs` line 27: unused import `HashSet` — remove it
2. `krillnotes-rbac/src/gate.rs` line 128: dead `cascade_revoke` method — this was the auto-cascade that's been replaced by opt-in `preview_cascade`. **Ask the user before removing** (per project conventions about removing established methods). If approved, remove the method and its test-only public wrapper.
3. `krillnotes-desktop/src-tauri/src/commands/sync.rs` line 565: unused variable `alice_pubkey_b64` — prefix with `_`

- [ ] **Step 2: Full type-check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`

Expected: No new errors from Plan B changes.

- [ ] **Step 3: Cargo build**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib`

Expected: Compiles with no new warnings from Plan B changes.

- [ ] **Step 4: Run core tests**

Run: `cargo test -p krillnotes-core`

Expected: All tests pass.

- [ ] **Step 5: Manual smoke test**

Run: `cd krillnotes-desktop && npm update && npm run tauri dev`

Test checklist:
1. **Tree dots**: Open a workspace → verify colored dots appear next to note titles (all green if root owner)
2. **Share anchor icons**: Grant a permission to a peer on a subtree → verify 👥 icon appears on the anchor node
3. **Ghost ancestors**: As a non-root-owner peer, verify ancestor nodes are greyed out and non-interactive
4. **Context menu gating**: As Reader, right-click → verify Add Child, Add Sibling, Edit, Delete are greyed out. As non-root-owner, right-click background → verify "Add Root Note" is greyed out
5. **Info panel role**: Select a note → verify "Your role" row shows correct role with inheritance info
6. **Info panel shared-with**: As Owner, select a node with grants → verify grants listed with role dropdowns and ✕ buttons
7. **Share dialog**: Click "+ Share this subtree..." → verify peer list, search, role picker, share action
8. **Cascade preview**: Change a peer's role from Owner to Reader (where they've granted downstream) → verify cascade preview dialog appears with impacted grants
9. **Edit button gating**: As Reader, verify edit and delete buttons are hidden in the detail view

- [ ] **Step 6: Commit any fixes**

```bash
git add -A
git commit -m "fix(desktop): address type-check and smoke test issues"
```

---

## Implementation Notes

### Name resolution pattern

The existing `resolve_identity_name` Tauri command (`commands/identity.rs:44-67`) performs three-tier resolution:
1. `IdentityManager::lookup_display_name(public_key)` — local identities owned by this device
2. `ContactManager::find_by_public_key(public_key)` — remote peers in address book
3. Fallback: first 8 characters of the base64 public key + "…"

Frontend components call this command to resolve `userId` / `grantedBy` public keys to display names. Batch resolve by collecting unique keys and calling in parallel.

### Permission state lifecycle

```
WorkspaceView mount
  ├── loadNotes()
  └── loadPermissionState()
        ├── get_all_effective_roles()  → effectiveRoles state
        ├── get_share_anchor_ids()     → shareAnchorIds state
        └── is_root_owner()            → isRootOwner state

After any mutation (create/delete/move/set_permission/revoke):
  ├── loadNotes()
  └── loadPermissionState()
```

### Permissive defaults

When RBAC is not active for a workspace (no identity, no permissions set):
- `effectiveRoles` = empty map → no dots shown, no ghost styling
- `isRootOwner` = `true` → all actions allowed
- Context menu items: no disabling
- InfoPanel: no "Your role" row, no "Shared with" section

This ensures non-RBAC workspaces are unaffected.

### Name resolution typing

The `resolve_identity_name` Tauri command returns `Option<String>` (Rust), which serializes as `string | null` in TypeScript. However, the implementation always returns `Some(...)` due to the truncated-key fallback at line 66 of `identity.rs`. So `invoke<string | null>('resolve_identity_name', ...)` would be more accurate than `invoke<string>(...)`, but the result is never actually `null` in practice. The `.catch()` fallback handles any unexpected errors.

### i18n — other languages

Task 10 only adds English strings. The project supports 7 languages. Other language translations should be added as a follow-up — the `t()` calls all have English fallback defaults so the UI is functional without them.

### Peer identity key matching

`PermissionGrantRow.userId` stores Ed25519 public keys (base64). `PeerInfo.peerIdentityId` from `list_workspace_peers` also stores the identity public key (base64). These should match directly. If they don't match in testing, check whether one is hex-encoded vs base64, or whether one includes a prefix.
