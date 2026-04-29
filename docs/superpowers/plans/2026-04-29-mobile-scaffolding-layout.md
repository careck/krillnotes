# Plan 1b: Tauri Mobile Scaffolding + Adaptive Layout

**Issue:** Child of #171 (Mobile support)
**Branch:** `mobile`
**Depends on:** Plan 1a (Device ID migration) — must be completed first
**Spec:** `docs/superpowers/specs/2026-04-29-mobile-support-design.md`

## Context

With the device ID migration complete, the `krillnotes-core` crate compiles cleanly for mobile targets. This plan scaffolds Tauri iOS/Android targets from the existing `krillnotes-desktop` package and implements the adaptive layout with three breakpoints.

## Prerequisites

```bash
# Rust mobile targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
rustup target add aarch64-linux-android

# Xcode (full install) — for iOS
# Android Studio + NDK + JDK — for Android
```

## Steps

### Step 1: Initialize Tauri mobile targets

**Directory:** `krillnotes-desktop/`

```bash
cd krillnotes-desktop
npm run tauri ios init
npm run tauri android init
```

This generates:
- `src-tauri/gen/apple/` — Xcode project
- `src-tauri/gen/android/` — Android Studio project

Add `gen/` to `.gitignore` if not already (generated, platform-specific).

Verify bare build compiles:
```bash
npm run tauri ios build -- --debug
npm run tauri android build -- --debug
```

### Step 2: Desktop-only guards in Rust

**File:** `krillnotes-desktop/src-tauri/src/lib.rs`

Gate desktop-only code with `#[cfg(desktop)]`:

- Native menu bar construction (the `build_menu()` call and menu event handler)
- `close_window` command
- Multi-window logic in `run()` (window creation for second workspace, etc.)

Gate the menu module:
**File:** `krillnotes-desktop/src-tauri/src/menu.rs`
- Add `#![cfg(desktop)]` at the top, or gate the import in `lib.rs`

**File:** `krillnotes-desktop/src-tauri/src/locales.rs`
- Same — only needed for native menu string translation

The mobile entry point should be minimal:
```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ... shared setup ...

    #[cfg(desktop)]
    {
        // menu, multi-window, system tray
    }

    // ... shared event loop ...
}
```

### Step 3: Mobile capabilities

**File:** `krillnotes-desktop/src-tauri/capabilities/default.json`

Review existing capabilities. May need adjustments for mobile:
- Remove file associations (desktop-only)
- Ensure `core:default` permissions work on mobile
- Add mobile-specific permissions as needed (initially minimal)

### Step 4: First mobile boot test

Run on iOS simulator and Android device/emulator:

```bash
cd krillnotes-desktop
npm run tauri ios dev    # iPad simulator
npm run tauri android dev # Android device via USB
```

Goal: app launches, existing desktop UI renders (probably badly on phone). This validates the full toolchain: Rust cross-compile, SQLCipher on mobile, React in mobile WebView.

Fix any compilation or runtime errors before proceeding to layout work.

### Step 5: `useLayout()` hook

**File:** `krillnotes-desktop/src/hooks/useLayout.ts` (new)

```typescript
export type Layout = "phone" | "tablet" | "desktop";

export function useLayout(): Layout {
  // Listen to window resize + orientation change
  // < 640px → "phone"
  // 640–1024px → "tablet"
  // > 1024px → "desktop"
}
```

Uses `window.innerWidth`, `resize` event listener, and `orientationchange` for iPad rotation. Returns reactive state via `useState`.

### Step 6: `MobileNav.tsx` — phone stack navigation

**File:** `krillnotes-desktop/src/components/MobileNav.tsx` (new)

State: `{ screen: "tree" | "note", selectedNoteId: string | null }`

Renders:
- `screen === "tree"` → full-screen `TreeView` + `SearchBar`
- `screen === "note"` → full-screen `InfoPanel` with back button
- Slide transition between screens (CSS transform + transition)
- Swipe-back gesture (touch event on left edge)

Props: receives the same props as `WorkspaceView` passes to `TreeView` and `InfoPanel` — note list, selected note, callbacks, etc.

### Step 7: Bottom navigation bar (phone)

**File:** `krillnotes-desktop/src/components/BottomNavBar.tsx` (new)

Four items: Tree (home), Search, Tags, More.

- Tree → navigates to tree screen in MobileNav
- Search → opens SearchBar overlay or navigates to tree with search focused
- Tags → slides up `TagCloudSheet` (bottom sheet component)
- More → opens dropdown/bottom sheet with workspace actions

**File:** `krillnotes-desktop/src/components/TagCloudSheet.tsx` (new)

Bottom sheet that slides up from the bottom bar. Contains the existing tag cloud content. Dismissible via swipe-down or tap-outside.

### Step 8: Toolbar hamburger menu (tablet + mobile)

**File:** `krillnotes-desktop/src/components/ToolbarMenu.tsx` (new)

Hamburger button (☰) that renders in the top toolbar area. Opens a dropdown with actions that normally live in the native menu:
- New note
- Workspace settings
- Export (Phase 3)
- Script manager (Phase 3 — hidden initially)
- About

Shown when `layout !== "desktop"`. On desktop, the native menu bar handles these.

### Step 9: Integrate in WorkspaceView

**File:** `krillnotes-desktop/src/components/WorkspaceView.tsx`

```typescript
const layout = useLayout();

if (layout === "phone") {
  return <MobileNav ... />;
}

// tablet and desktop share the sidebar layout
// tablet gets: narrower default sidebar, ToolbarMenu, touch divider
// desktop gets: current layout unchanged, native menu
return (
  <div className="flex h-screen">
    <Sidebar defaultWidth={layout === "tablet" ? 200 : 280} ... />
    <Divider touchEnabled={layout === "tablet"} ... />
    <MainPanel toolbar={layout === "tablet" ? <ToolbarMenu /> : null} ... />
  </div>
);
```

The existing layout code doesn't change for desktop — `useLayout()` returns `"desktop"` on large viewports and the same code path runs.

### Step 10: Touch-enable resizable panels

**File:** `krillnotes-desktop/src/hooks/useResizablePanels.ts` (or wherever the hook lives)

Add alongside existing mouse event listeners:
- `touchstart` / `touchmove` / `touchend` handlers
- Invisible touch zone: the divider element gets `padding: 0 10px` (20px total touch area) with the visual divider as a thin inner element
- Min width: 120px. Max width: 60% of viewport.

### Step 11: Touch UX adjustments

**File:** `krillnotes-desktop/src/components/TreeNode.tsx`
- Increase minimum node height to 44px (Apple HIG)
- Long-press handler for context menu (use `touchstart` + timer, cancel on `touchmove`)
- Context menu renders as bottom sheet on phone, floating menu on tablet

**File:** `krillnotes-desktop/src/components/ContextMenu.tsx`
- Add a `mode` prop or detect layout: bottom sheet on phone, current floating menu elsewhere

**File:** `krillnotes-desktop/src/globals.css` (or Tailwind config)
- Safe area insets: `padding-top: env(safe-area-inset-top)`, etc.
- Add to root layout container

### Step 12: i18n for new components

**Files:** All 7 locale files in `krillnotes-desktop/src/i18n/locales/`

Add keys for:
- `mobile.backButton`: "Back"
- `mobile.bottomNav.tree`: "Tree" / "Notes"
- `mobile.bottomNav.search`: "Search"
- `mobile.bottomNav.tags`: "Tags"
- `mobile.bottomNav.more`: "More"
- `mobile.migrationGate`: "Please open an existing workspace first to migrate your device identity"
- `toolbar.menu`: "Menu"
- `toolbar.newNote`: "New Note"
- `toolbar.settings`: "Settings"

All 7 languages (en, de, es, fr, ja, ko, zh).

### Step 13: Test on all form factors

- **iPad simulator** (landscape + portrait rotation) — sidebar layout, touch divider, hamburger menu
- **Android phone** (USB) — stack navigation, bottom bar, tags sheet, swipe-back
- **Desktop** (`npm run tauri dev`) — verify zero regressions, native menu still works
- **Browser resize** — drag browser window small to simulate phone/tablet breakpoints during development

### Step 14: Commit sequence

1. `chore: initialize Tauri iOS and Android targets` — steps 1, 3
2. `refactor: gate desktop-only code with #[cfg(desktop)]` — step 2
3. `feat: add useLayout hook for adaptive breakpoints` — step 5
4. `feat: add MobileNav stack navigation for phone layout` — step 6
5. `feat: add bottom nav bar and tag cloud sheet` — step 7
6. `feat: add toolbar hamburger menu for tablet/mobile` — step 8
7. `refactor: integrate adaptive layout in WorkspaceView` — step 9
8. `feat: touch-enable resizable panels for tablet` — step 10
9. `feat: touch UX — tap targets, long-press context menu, safe areas` — step 11
10. `chore: add i18n keys for mobile components` — step 12

## Risks

- **WebView quirks** — WKWebView on iOS has known issues (see gotchas.md): confirm dialogs, scrollbar rendering, drag-drop. Test early.
- **SQLCipher first-open** — verify PRAGMA key works on both iOS and Android WebView contexts. The Rust side should be fine (bundled), but the Tauri bridge timing matters.
- **Keyboard behavior** — virtual keyboard pushing layout around. May need `viewport-fit=cover` meta tag and CSS adjustments. Test with text field editing.
- **gen/ directory churn** — Tauri-generated iOS/Android project files may need manual tweaks for signing, bundle ID, etc. Keep track of manual changes.
