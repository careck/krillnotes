# Hover Info on Tree Nodes — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Show a speech-bubble tooltip on tree node hover, driven by `show_on_hover` field flags (simple path) or an `on_hover` Rhai hook (power path).

**Architecture:** Four-layer hook registration pattern mirrors `on_view` exactly: `schema()` host fn in `mod.rs` → `SchemaRegistry` → `ScriptRegistry` → `Workspace`. Frontend: `WorkspaceView` owns debounce + state, `HoverTooltip` renders as a React portal.

**Tech Stack:** Rust (Rhai, Serde), Tauri v2, React/TypeScript, Tailwind CSS, DOMPurify

---

## Setup

### Task 0: Create worktree and branch

**Step 1: Create worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/hover-info-tree-nodes -b feat/hover-info-tree-nodes
```

**Step 2: Verify**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes/src
```

All implementation happens inside the worktree.

---

## Rust Layer

### Task 1: Add `show_on_hover` to `FieldDefinition`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

**Step 1: Add the field to the struct** (after `target_type`)

```rust
pub target_type: Option<String>,
pub show_on_hover: bool,   // controls simple-path tooltip rendering
```

**Step 2: Parse it in `parse_from_rhai`** (after `target_type` parse, before `fields.push(...)`)

```rust
let show_on_hover = field_map
    .get("show_on_hover")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(false);

// Add show_on_hover to the FieldDefinition { ... } struct literal in fields.push(...)
```

**Step 3: Write the failing test** (in `mod.rs` test block)

```rust
#[test]
fn test_field_show_on_hover_parsed() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        // @name: HoverTest
        schema("HoverTest", #{
            fields: [
                #{ name: "summary", type: "text", show_on_hover: true },
                #{ name: "internal", type: "text" },
            ],
        });
    "#, "HoverTest").unwrap();
    let schema = registry.get_schema("HoverTest").unwrap();
    assert!(schema.fields[0].show_on_hover);
    assert!(!schema.fields[1].show_on_hover);
}
```

**Step 4: Run test to verify it fails**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes
cargo test -p krillnotes-core test_field_show_on_hover_parsed 2>&1 | tail -20
```

Expected: compile error about missing `show_on_hover` field.

**Step 5: Implement, then run test**

```bash
cargo test -p krillnotes-core test_field_show_on_hover_parsed 2>&1 | tail -10
```

Expected: PASS.

**Step 6: Full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs
git commit -m "feat: add show_on_hover flag to FieldDefinition"
```

---

### Task 2: Add `on_hover` hook to `SchemaRegistry`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

**Pattern:** Mirror the `on_view_hooks` field/accessor/has/run pattern exactly. Add:
- `on_hover_hooks: Arc<Mutex<HashMap<String, HookEntry>>>` to the struct
- Initialize in `new()`
- `on_hover_hooks_arc()` accessor
- `has_hover_hook(schema_name)` — same body as `has_view_hook` but reads `on_hover_hooks`
- `run_on_hover_hook(engine, note_map)` — copy of `run_on_view_hook` but reads `on_hover_hooks`
- Add `self.on_hover_hooks.lock().unwrap().clear();` to `clear()`

**Step 1: Write tests first** (in `mod.rs` test block)

```rust
#[test]
fn test_has_hover_hook_registered() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        // @name: HoverHook
        schema("WithHover", #{
            fields: [#{ name: "body", type: "text" }],
            on_hover: |note| { "hover: " + note.title },
        });
    "#, "HoverHook").unwrap();
    assert!(registry.has_hover_hook("WithHover"));
    assert!(!registry.has_hover_hook("Nonexistent"));
}

#[test]
fn test_run_on_hover_hook_returns_html() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        // @name: HoverRun
        schema("HoverRun", #{
            fields: [#{ name: "body", type: "text" }],
            on_hover: |note| { "HOVER:" + note.title },
        });
    "#, "HoverRun").unwrap();
    let note = Note {
        id: "id1".into(), title: "Test Note".into(), node_type: "HoverRun".into(),
        parent_id: None, position: 0, created_at: 0, modified_at: 0,
        created_by: 0, modified_by: 0,
        fields: std::collections::HashMap::new(), is_expanded: false, tags: vec![],
    };
    let ctx = QueryContext {
        notes_by_id: Default::default(), children_by_id: Default::default(),
        notes_by_type: Default::default(), notes_by_tag: Default::default(),
        notes_by_link_target: Default::default(),
    };
    let html = registry.run_on_hover_hook(&note, ctx).unwrap();
    assert_eq!(html, Some("HOVER:Test Note".to_string()));
}
```

**Step 2: Run tests, verify they fail**

```bash
cargo test -p krillnotes-core test_has_hover_hook test_run_on_hover_hook 2>&1 | tail -10
```

**Step 3: Implement** all five items listed above.

**Step 4: Run tests**

```bash
cargo test -p krillnotes-core test_has_hover_hook test_run_on_hover_hook 2>&1 | tail -10
```

Expected: PASS.

**Step 5: Full test suite**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs
git commit -m "feat: add on_hover hook to SchemaRegistry"
```

---

### Task 3: Register `on_hover` in the `schema()` host function

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs`

**Pattern:** Mirror how `on_view` is extracted from the Rhai map and stored. Add:
1. `let on_hover_arc = schema_registry.on_hover_hooks_arc();` (after `on_add_child_arc`)
2. In the `schema()` closure, after the `on_add_child` block, extract `on_hover` FnPtr and insert into `on_hover_arc`
3. `pub fn has_hover_hook(&self, schema_name: &str) -> bool` delegate (after `has_view_hook`)
4. `pub fn run_on_hover_hook(&self, note: &Note, context: QueryContext) -> Result<Option<String>>` delegate
   - Build `note_map` exactly as in `run_on_view_hook` (with id, node_type, title, fields, tags)
   - Install context, call `self.schema_registry.run_on_hover_hook(...)`, clear context

**Step 1: Run tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

Expected: all pass.

**Step 2: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: register on_hover hook in schema() host function"
```

---

### Task 4: Add `run_hover_hook` to `Workspace`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Pattern:** Mirror `run_view_hook` exactly, with two differences:
- Returns `Result<Option<String>>` — `None` when no hook registered (no default renderer)
- Calls `self.script_registry.run_on_hover_hook(...)` instead of `run_on_view_hook`

Place the method after `run_view_hook`. The QueryContext construction block is identical to `run_view_hook`.

**Step 1: Run tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 2: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: add run_hover_hook to Workspace"
```

---

### Task 5: Add `get_note_hover` Tauri command and update `SchemaInfo`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add `has_hover_hook: bool` to `SchemaInfo` struct** (after `has_view_hook`)

**Step 2: Set it in both `get_schema_fields` and `get_all_schemas`** using `workspace.script_registry().has_hover_hook(...)` — same pattern as `has_view_hook`

**Step 3: Add `get_note_hover` command** after `get_note_view`:

```rust
#[tauri::command]
fn get_note_hover(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_hover_hook(&note_id).map_err(|e| e.to_string())
}
```

**Step 4: Register** `get_note_hover` in `invoke_handler` after `get_note_view`

**Step 5: Build**

```bash
cargo build -p krillnotes-desktop 2>&1 | grep "^error" | head -20
```

Expected: clean.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add get_note_hover Tauri command and has_hover_hook in SchemaInfo"
```

---

## TypeScript Layer

### Task 6: Update TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Add `showOnHover: boolean` to `FieldDefinition`** (after `targetType`)

**Step 2: Add `hasHoverHook: boolean` to `SchemaInfo`** (after `hasViewHook`)

**Step 3: Build check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes/krillnotes-desktop
npm run build 2>&1 | grep "error TS" | head -20
```

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add showOnHover and hasHoverHook to TypeScript types"
```

---

### Task 7: Add tooltip CSS

**Files:**
- Modify: `krillnotes-desktop/src/styles/globals.css`

Append at end of file:

```css
/* ── Hover Tooltip ──────────────────────────────────────────────────────── */

.kn-hover-tooltip {
  position: fixed;
  z-index: 200;
  max-width: 280px;
  min-width: 160px;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--color-background);
  border: 1px solid var(--color-border);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  font-size: 0.8125rem;
  line-height: 1.5;
  pointer-events: none;
  animation: kn-tooltip-in 120ms ease-out;
}

@keyframes kn-tooltip-in {
  from { opacity: 0; transform: translateX(4px); }
  to   { opacity: 1; transform: translateX(0); }
}

/* Left-pointing spike — two layered triangles to simulate a border */
.kn-hover-tooltip::before {
  content: '';
  position: absolute;
  left: -8px;
  top: var(--spike-offset, 50%);
  transform: translateY(-50%);
  width: 0;
  height: 0;
  border-top: 7px solid transparent;
  border-bottom: 7px solid transparent;
  border-right: 8px solid var(--color-border);
}

.kn-hover-tooltip::after {
  content: '';
  position: absolute;
  left: -6px;
  top: var(--spike-offset, 50%);
  transform: translateY(-50%);
  width: 0;
  height: 0;
  border-top: 6px solid transparent;
  border-bottom: 6px solid transparent;
  border-right: 7px solid var(--color-background);
}

.kn-hover-tooltip__row {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 4px 10px;
  align-items: baseline;
}

.kn-hover-tooltip__label {
  color: var(--color-muted-foreground);
  font-size: 0.75rem;
  white-space: nowrap;
}

.kn-hover-tooltip__value {
  color: var(--color-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 180px;
}
```

**Commit**

```bash
git add krillnotes-desktop/src/styles/globals.css
git commit -m "feat: add hover tooltip CSS with speech-bubble spike"
```

---

### Task 8: Create `HoverTooltip.tsx`

**Files:**
- Create: `krillnotes-desktop/src/components/HoverTooltip.tsx`

**Props:**

```typescript
interface HoverTooltipProps {
  note: Note;
  schema: SchemaInfo | null;
  hoverHtml: string | null;       // from on_hover hook; null = use field flags
  anchorY: number;                // viewport Y of hovered row center
  treeWidth: number;              // right edge of tree panel
  visible: boolean;
}
```

**Positioning logic:**
- Tooltip renders via `createPortal(element, document.body)`
- Left = `treeWidth + 2 + 12` (after the 1px divider, with a 12px gap)
- Top: center on `anchorY`, clamped to `[8, window.innerHeight - tooltipHeight - 8]`
- Spike offset: `anchorY - clampedTop` px (absolute from tooltip top edge)
- Set `--spike-offset` CSS custom property inline on the tooltip div

**Two render paths:**
1. `hoverHtml !== null`: render hook output sanitized with DOMPurify (same pattern as InfoPanel — see `InfoPanel.tsx` for the exact method used there)
2. `hoverHtml === null`: render `schema.fields.filter(f => f.showOnHover)` as label/value rows using `.kn-hover-tooltip__row` CSS

**Helper `renderFieldValue(value: FieldValue): string`:**
- `Text` → string or `—`
- `Number` → String(n)
- `Boolean` → `Yes` / `No`
- `Date` → date string or `—`
- `Email` → string or `—`
- `NoteLink` → `(linked note)` / `—`

**Build check**

```bash
npm run build 2>&1 | grep "error TS" | head -20
```

**Commit**

```bash
git add krillnotes-desktop/src/components/HoverTooltip.tsx
git commit -m "feat: add HoverTooltip component with portal and speech bubble"
```

---

### Task 9: Wire hover state into `WorkspaceView.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Import**

```tsx
import HoverTooltip from './HoverTooltip';
```

**Step 2: Add state + refs** after `draggedNoteId` state

```tsx
const [hoveredNoteId, setHoveredNoteId]   = useState<string | null>(null);
const [tooltipAnchorY, setTooltipAnchorY] = useState(0);
const [hoverHtml, setHoverHtml]           = useState<string | null>(null);
const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
```

**Step 3: Add handlers** after `handleMoveNote`

```tsx
const handleHoverStart = useCallback((noteId: string, anchorY: number) => {
  if (draggedNoteId !== null) return;
  if (hoverTimer.current) clearTimeout(hoverTimer.current);
  hoverTimer.current = setTimeout(async () => {
    const nodeType = notes.find(n => n.id === noteId)?.nodeType ?? '';
    const schema = schemas[nodeType] ?? null;
    if (schema?.hasHoverHook) {
      try {
        const html = await invoke<string | null>('get_note_hover', { noteId });
        setHoverHtml(html);
      } catch {
        setHoverHtml(null);
      }
    } else {
      setHoverHtml(null);
    }
    setHoveredNoteId(noteId);
    setTooltipAnchorY(anchorY);
  }, 600);
}, [draggedNoteId, notes, schemas]);

const handleHoverEnd = useCallback(() => {
  if (hoverTimer.current) clearTimeout(hoverTimer.current);
  hoverTimer.current = null;
  setHoveredNoteId(null);
  setHoverHtml(null);
}, []);
```

**Step 4: Clear on drag start** — `useEffect` watching `draggedNoteId`

```tsx
useEffect(() => {
  if (draggedNoteId !== null) handleHoverEnd();
}, [draggedNoteId, handleHoverEnd]);
```

**Step 5: Pass to TreeView**

```tsx
onHoverStart={handleHoverStart}
onHoverEnd={handleHoverEnd}
```

**Step 6: Render HoverTooltip** before the closing `</div>` of the outer flex container

```tsx
{hoveredNoteId && (() => {
  const note = notes.find(n => n.id === hoveredNoteId);
  const schema = note ? (schemas[note.nodeType] ?? null) : null;
  if (!note) return null;
  return (
    <HoverTooltip
      note={note}
      schema={schema}
      hoverHtml={hoverHtml}
      anchorY={tooltipAnchorY}
      treeWidth={treeWidth}
      visible={true}
    />
  );
})()}
```

---

### Task 10: Wire hover into `TreeView.tsx` and `TreeNode.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`
- Modify: `krillnotes-desktop/src/components/TreeNode.tsx`

**TreeView:** Add `onHoverStart` and `onHoverEnd` to props interface, pass through to each `<TreeNode>` (same pattern as `onContextMenu`).

**TreeNode:** Add to props interface. On the main row `<div>` (the one with `onClick`), add three handlers:

```tsx
onMouseEnter={(e) => {
  const rect = e.currentTarget.getBoundingClientRect();
  onHoverStart(node.note.id, rect.top + rect.height / 2);
}}
onMouseLeave={() => onHoverEnd()}
onMouseDown={() => onHoverEnd()}
```

Pass props down to recursive `<TreeNode>` children.

**Build check**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes/krillnotes-desktop
npm run build 2>&1 | grep "error TS" | head -20
```

Expected: clean.

**Full Rust test suite**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Commit**

```bash
git add krillnotes-desktop/src/components/TreeView.tsx \
        krillnotes-desktop/src/components/TreeNode.tsx \
        krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: wire hover tooltip into tree panel"
```

---

### Task 11: Update Zettelkasten template

**Files:**
- Modify zettelkasten template in `krillnotes-core/src/system_scripts/`

**Step 1: Find the file**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes/krillnotes-core/src/system_scripts/
```

**Step 2: Add `show_on_hover: true` to the `body` field of the Zettel schema**

**Step 3: Add lightweight `on_hover` to the Kasten schema**

```rhai
on_hover: |note| {
    let kids = get_children(note.id);
    field("Notes", kids.len().to_string())
},
```

**Step 4: Run tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -5
```

**Step 5: Commit**

```bash
git add krillnotes-core/src/system_scripts/
git commit -m "feat: add show_on_hover and on_hover demo to zettelkasten template"
```

---

## Final Verification

### Task 12: Full build + smoke test

**Step 1: Rust tests**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/hover-info-tree-nodes
cargo test 2>&1 | tail -10
```

Expected: 235+ tests pass.

**Step 2: TypeScript build**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

**Step 3: Manual smoke test** (`npm run tauri dev`)

- Hover a node 0.6s → speech-bubble tooltip appears right of tree panel
- Spike points left at the hovered node
- Mouse away → tooltip disappears immediately
- Click → tooltip never appears (mousedown clears it first)
- Drag → tooltip never appears during drag
- Schema with `show_on_hover: true` fields → those fields appear in tooltip
- Schema with `on_hover` hook → hook HTML appears in tooltip

**Step 4: Push**

```bash
git -C /Users/careck/Source/Krillnotes push -u github-https feat/hover-info-tree-nodes
```
