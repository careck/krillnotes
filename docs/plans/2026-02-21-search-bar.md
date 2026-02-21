# Search Bar Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a search bar above the tree that filters notes by title and text-like fields, shows results in a dropdown, and reveals the selected note in the tree by expanding collapsed ancestors.

**Architecture:** A new `SearchBar` React component receives the flat `notes[]` array and filters it in-memory (no backend changes). Results appear in a dropdown overlay. Selecting a result expands collapsed ancestors via existing `toggle_note_expansion`, then selects the note. A `searchNotes` utility function handles the matching logic and is unit-testable independently.

**Tech Stack:** React 19, TypeScript, Tailwind CSS, lucide-react icons, Tauri invoke API

---

### Task 1: Create the `searchNotes` utility function

This pure function takes a query string and an array of notes, and returns matching notes with information about which field matched. This is the core search logic, separated from UI for testability.

**Files:**
- Create: `krillnotes-desktop/src/utils/search.ts`

**Step 1: Create the search utility**

Create `krillnotes-desktop/src/utils/search.ts` with this content:

```typescript
import type { Note, FieldValue } from '../types';

export interface SearchResult {
  note: Note;
  matchField: string;   // "title" or the field name that matched
  matchValue: string;    // the text value that contained the match
}

/**
 * Extracts the string value from a FieldValue if it is a text-like type.
 * Returns null for Number, Boolean, and Date fields.
 */
function textContent(fv: FieldValue): string | null {
  if ('Text' in fv) return fv.Text;
  if ('Email' in fv) return fv.Email;
  return null;
}

/**
 * Searches notes by matching a query against the title and all text-like
 * field values (Text, Email). Case-insensitive substring match.
 *
 * Returns at most one SearchResult per note (first match wins, title checked first).
 * Returns an empty array if query is empty or whitespace-only.
 */
export function searchNotes(notes: Note[], query: string): SearchResult[] {
  const trimmed = query.trim().toLowerCase();
  if (trimmed === '') return [];

  const results: SearchResult[] = [];

  for (const note of notes) {
    // Check title first
    if (note.title.toLowerCase().includes(trimmed)) {
      results.push({ note, matchField: 'title', matchValue: note.title });
      continue;
    }

    // Check text-like fields
    let matched = false;
    for (const [fieldName, fieldValue] of Object.entries(note.fields)) {
      const text = textContent(fieldValue);
      if (text !== null && text.toLowerCase().includes(trimmed)) {
        results.push({ note, matchField: fieldName, matchValue: text });
        matched = true;
        break;
      }
    }
    if (matched) continue;
  }

  return results;
}
```

**Step 2: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/search.ts
git commit -m "feat: add searchNotes utility for in-memory note filtering"
```

---

### Task 2: Create the `getAncestorIds` utility function

This function walks up the `parentId` chain and returns the IDs of all ancestors for a given note. Needed for the "reveal in tree" behavior.

**Files:**
- Modify: `krillnotes-desktop/src/utils/tree.ts`

**Step 1: Add the function to tree.ts**

Add this function at the end of `krillnotes-desktop/src/utils/tree.ts` (before the closing of the file):

```typescript
/**
 * Returns the IDs of all ancestors of the given noteId, from immediate parent
 * to the root. Returns an empty array if the note has no parent or is not found.
 */
export function getAncestorIds(notes: Note[], noteId: string): string[] {
  const noteMap = new Map(notes.map(n => [n.id, n]));
  const ancestors: string[] = [];
  let current = noteMap.get(noteId);
  while (current?.parentId) {
    ancestors.push(current.parentId);
    current = noteMap.get(current.parentId);
  }
  return ancestors;
}
```

Also add the `Note` import at the top of the file. The existing import line is:
```typescript
import type { Note, TreeNode } from '../types';
```
This already imports `Note`, so no change needed there.

**Step 2: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/utils/tree.ts
git commit -m "feat: add getAncestorIds utility for ancestor chain traversal"
```

---

### Task 3: Create the `SearchBar` component

The main UI component: a text input with a dropdown overlay showing search results.

**Files:**
- Create: `krillnotes-desktop/src/components/SearchBar.tsx`

**Step 1: Create the component**

Create `krillnotes-desktop/src/components/SearchBar.tsx` with this content:

```tsx
import { useState, useRef, useEffect, useCallback } from 'react';
import { Search, X } from 'lucide-react';
import { searchNotes } from '../utils/search';
import type { Note } from '../types';
import type { SearchResult } from '../utils/search';

interface SearchBarProps {
  notes: Note[];
  onSelect: (noteId: string) => void;
}

/** Truncates text around the match position to show context. */
function matchSnippet(value: string, query: string, maxLen = 60): string {
  const lower = value.toLowerCase();
  const idx = lower.indexOf(query.toLowerCase());
  if (idx === -1) return value.slice(0, maxLen);
  const start = Math.max(0, idx - 20);
  const end = Math.min(value.length, idx + query.length + 20);
  let snippet = value.slice(start, end);
  if (start > 0) snippet = '…' + snippet;
  if (end < value.length) snippet = snippet + '…';
  return snippet;
}

function SearchBar({ notes, onSelect }: SearchBarProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResult[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [isFocused, setIsFocused] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>();
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Debounced search
  useEffect(() => {
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      const found = searchNotes(notes, query);
      setResults(found);
      setActiveIndex(0);
    }, 150);
    return () => clearTimeout(debounceRef.current);
  }, [query, notes]);

  const showDropdown = isFocused && query.trim() !== '' && results.length > 0;

  const handleSelect = useCallback((noteId: string) => {
    onSelect(noteId);
    // Keep dropdown open — don't clear query
  }, [onSelect]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!showDropdown) return;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setActiveIndex(prev => Math.min(prev + 1, results.length - 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        setActiveIndex(prev => Math.max(prev - 1, 0));
        break;
      case 'Enter':
        e.preventDefault();
        if (results[activeIndex]) {
          handleSelect(results[activeIndex].note.id);
        }
        break;
      case 'Escape':
        e.preventDefault();
        setQuery('');
        inputRef.current?.blur();
        break;
    }
  };

  // Scroll active item into view within the dropdown
  useEffect(() => {
    if (!showDropdown || !dropdownRef.current) return;
    const active = dropdownRef.current.children[activeIndex] as HTMLElement | undefined;
    active?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex, showDropdown]);

  const handleClear = () => {
    setQuery('');
    inputRef.current?.focus();
  };

  return (
    <div className="relative px-2 py-2 border-b border-border">
      <div className="relative">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" />
        <input
          ref={inputRef}
          type="text"
          placeholder="Search notes…"
          value={query}
          onChange={e => setQuery(e.target.value)}
          onFocus={() => setIsFocused(true)}
          onBlur={() => {
            // Delay so click on dropdown result registers before closing
            setTimeout(() => setIsFocused(false), 200);
          }}
          onKeyDown={handleKeyDown}
          className="w-full pl-7 pr-7 py-1 text-sm bg-muted/50 border border-input rounded focus:outline-none focus:ring-1 focus:ring-primary"
        />
        {query && (
          <button
            onMouseDown={e => e.preventDefault()}
            onClick={handleClear}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      {/* Dropdown */}
      {showDropdown && (
        <div
          ref={dropdownRef}
          className="absolute left-2 right-2 top-full mt-1 bg-background border border-border rounded shadow-lg max-h-[300px] overflow-y-auto z-10"
        >
          {results.map((result, i) => (
            <div
              key={result.note.id}
              className={`px-3 py-2 cursor-pointer ${
                i === activeIndex ? 'bg-secondary' : 'hover:bg-secondary/50'
              }`}
              onMouseDown={e => e.preventDefault()}
              onClick={() => handleSelect(result.note.id)}
              onMouseEnter={() => setActiveIndex(i)}
            >
              <div className="text-sm font-medium truncate">{result.note.title}</div>
              {result.matchField !== 'title' && (
                <div className="text-xs text-muted-foreground truncate">
                  {result.matchField}: {matchSnippet(result.matchValue, query)}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* No results message */}
      {isFocused && query.trim() !== '' && results.length === 0 && (
        <div className="absolute left-2 right-2 top-full mt-1 bg-background border border-border rounded shadow-lg z-10 px-3 py-2 text-sm text-muted-foreground">
          No notes found.
        </div>
      )}
    </div>
  );
}

export default SearchBar;
```

**Step 2: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/SearchBar.tsx
git commit -m "feat: add SearchBar component with dropdown results"
```

---

### Task 4: Wire SearchBar into WorkspaceView

Add the SearchBar to the left sidebar in WorkspaceView, above the TreeView. Implement the `revealAndSelect` handler that expands collapsed ancestors before selecting.

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Add imports**

At the top of `WorkspaceView.tsx`, add these imports. The existing import block starts at line 1.

Add after line 6 (`import InfoPanel from './InfoPanel';`):
```typescript
import SearchBar from './SearchBar';
```

Add `getAncestorIds` to the existing tree import. Change line 14 from:
```typescript
import { buildTree, flattenVisibleTree, findNoteInTree } from '../utils/tree';
```
to:
```typescript
import { buildTree, flattenVisibleTree, findNoteInTree, getAncestorIds } from '../utils/tree';
```

**Step 2: Add the revealAndSelect handler**

Add this handler inside the `WorkspaceView` function, after `handleToggleExpand` (around line 157):

```typescript
  const handleSearchSelect = async (noteId: string) => {
    // Expand any collapsed ancestors so the note becomes visible in the tree
    const ancestors = getAncestorIds(notes, noteId);
    const collapsedAncestors = ancestors.filter(
      id => notes.find(n => n.id === id)?.isExpanded === false
    );

    for (const ancestorId of collapsedAncestors) {
      await invoke('toggle_note_expansion', { noteId: ancestorId });
    }

    if (collapsedAncestors.length > 0) {
      await loadNotes();
    }

    await handleSelectNote(noteId);

    // Scroll the note into view in the tree
    requestAnimationFrame(() => {
      document.querySelector(`[data-note-id="${noteId}"]`)?.scrollIntoView({ block: 'nearest' });
    });
  };
```

**Step 3: Update the left sidebar JSX**

Change the left sidebar section (currently lines 343–359) from:

```tsx
      {/* Left sidebar - Tree */}
      <div
        ref={treePanelRef}
        className="shrink-0 bg-background overflow-hidden"
        style={{ width: treeWidth }}
      >
        <TreeView
          tree={tree}
          selectedNoteId={selectedNoteId}
          onSelect={handleSelectNote}
          onToggleExpand={handleToggleExpand}
          onContextMenu={handleContextMenu}
          onKeyDown={handleTreeKeyDown}
        />
      </div>
```

to:

```tsx
      {/* Left sidebar - Tree */}
      <div
        ref={treePanelRef}
        className="shrink-0 bg-background overflow-hidden flex flex-col"
        style={{ width: treeWidth }}
      >
        <SearchBar notes={notes} onSelect={handleSearchSelect} />
        <div className="flex-1 overflow-y-auto">
          <TreeView
            tree={tree}
            selectedNoteId={selectedNoteId}
            onSelect={handleSelectNote}
            onToggleExpand={handleToggleExpand}
            onContextMenu={handleContextMenu}
            onKeyDown={handleTreeKeyDown}
          />
        </div>
      </div>
```

Key changes:
- Added `flex flex-col` to the sidebar container
- Added `<SearchBar>` before TreeView
- Wrapped `<TreeView>` in a `<div className="flex-1 overflow-y-auto">` so the tree scrolls independently while SearchBar stays fixed at top

**Step 4: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: wire SearchBar into WorkspaceView with ancestor expansion"
```

---

### Task 5: Remove duplicate scroll container from TreeView

After Task 4, the TreeView's own `overflow-y-auto h-full` would conflict with the new wrapper div in WorkspaceView that provides scrolling. Remove the redundant scroll from TreeView.

**Files:**
- Modify: `krillnotes-desktop/src/components/TreeView.tsx`

**Step 1: Update TreeView container class**

In `krillnotes-desktop/src/components/TreeView.tsx`, change line 28 from:
```tsx
      className="overflow-y-auto h-full focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
```
to:
```tsx
      className="h-full focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
```

Also update the empty-state div on line 17 — change `h-full` to keep it but that's fine as the parent controls overflow.

**Step 2: Verify it compiles**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/TreeView.tsx
git commit -m "fix: remove duplicate scroll container from TreeView"
```

---

### Task 6: Build and smoke test

Verify the full application builds and runs correctly.

**Step 1: Build the frontend**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: Build succeeds with no errors

**Step 2: Build the Tauri app**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: Build succeeds

**Step 3: Run existing tests**

Run: `cd /Users/careck/Source/Krillnotes && cargo test --manifest-path krillnotes-core/Cargo.toml`
Expected: All tests pass (no backend changes were made, so existing tests should not break)
