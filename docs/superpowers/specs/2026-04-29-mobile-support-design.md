# Mobile Support Design Spec

## Overview

Add iOS and Android support to Krillnotes by building mobile targets from the existing `krillnotes-desktop` package (single-package approach). The core library (`krillnotes-core`) is already Tauri-independent and compiles for mobile targets with one blocker to resolve first (device ID).

**Branch:** `mobile` (exploratory — pivot to separate package if single-package hits a wall)

**Target devices:** Android phone, iPad (tablet)

## Phased Scope

### MVP (Phase 1)
- Device ID migration (prerequisite — all platforms)
- Tauri mobile scaffolding (iOS + Android targets)
- Adaptive layout (phone / tablet portrait / tablet landscape)
- Note tree browsing, note viewing/editing, search, tags
- Workspace open/create
- Bottom nav bar (phone), toolbar hamburger menu (tablet)

### Phase 2
- Multi-device sync (same identity sharing workspace across desktop + mobile)
- Attachments (with camera capture on mobile)

### Phase 3
- Script editor (mobile-friendly)
- RBAC permission UI
- Operations log
- Import/export

### Future: CI/CD & Distribution
- Target: GitHub Actions (consistent with existing desktop CI)
- **Android:** Linux or macOS runner, Android SDK + NDK, `tauri android build` → `.aab`, distribute via Google Play with `fastlane supply`. Signing keystore stored as GitHub secret.
- **iOS:** macOS runner required, Apple Developer account ($99/yr), signing certs + provisioning profiles as GitHub secrets, `tauri ios build` → `.ipa`, distribute via TestFlight with `fastlane pilot`.
- **Tooling:** `fastlane` for both platforms (signing, building, uploading).
- Not designed in this spec — to be addressed when local development stabilizes.

## Preliminary: Device ID Migration

### Problem

`device.rs` uses the `mac_address` crate to derive a stable device ID (`device-<hash>`). This doesn't work on mobile (iOS/Android don't expose MAC addresses) and caused relay sync bugs on desktop (different MAC values from different network interfaces).

### Solution

Replace MAC-based device ID with a persisted UUID. Source of truth is the workspace database, with an app-level file as seed for new workspaces.

**Lookup priority (on workspace open):**

1. `workspace_meta` has `device_id` → use it
2. `operations` table has local `device_id` → use it, write to `workspace_meta` and app-level file
3. App-level file exists → use it, write to `workspace_meta`
4. Nothing found → generate `device-{uuid}`, write to `workspace_meta` and app-level file

**Migration gate:** On "Create New Workspace", if no app-level `device_id` file exists and `list_workspaces()` returns non-empty, block creation with a message: *"Please open an existing workspace first to migrate your device identity."* This prevents generating a fresh UUID that would conflict with an existing workspace's sync identity.

**API — two functions in `device.rs`:**

```rust
/// Reads or creates the app-level device ID seed file.
/// Does NOT touch the database.
pub fn get_or_create_seed_device_id(data_dir: &Path) -> Result<String>

/// Full priority chain: workspace_meta → operations → seed file → generate.
/// Called during Workspace::open(). Writes back to workspace_meta and seed file as needed.
pub fn resolve_device_id(conn: &Connection, data_dir: &Path) -> Result<String>
```

- Desktop: `data_dir` = `home_dir()/.krillnotes/`
- Mobile: `data_dir` = app sandbox directory (passed from Tauri shell)
- Migration gate (block new workspace if existing workspaces but no seed file) lives in the Tauri command layer, not in core

**Crate changes:**
- Remove `mac_address` dependency entirely
- `device.rs` implements both functions above
- `Workspace::open()` accepts `data_dir` parameter, calls `resolve_device_id()`
- One-time migration reads existing device_id from operations table (no need to recompute MAC)

## Tauri Mobile Scaffolding

### Single-Package Approach

Build mobile from `krillnotes-desktop` rather than a separate crate. Tauri v2 supports this natively — same `src-tauri/` directory, same Cargo project.

**Existing mobile-ready config:**
- `Cargo.toml` already declares `crate-type = ["staticlib", "cdylib", "rlib"]`
- `lib.rs` supports `#[cfg_attr(mobile, tauri::mobile_entry_point)]`

**Scaffolding steps:**
- `tauri ios init` → generates `gen/apple/`
- `tauri android init` → generates `gen/android/`
- Add Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android`, `x86_64-linux-android`

### Desktop-Only Guards

Features that don't apply on mobile get `#[cfg(desktop)]`:

- Native menu bar construction (`menu.rs`)
- `close_window` command
- Multi-window workspace management logic in `run()`

All 100+ Tauri commands remain shared. Mobile simply doesn't exercise the gated code paths.

### Capabilities

New `src-tauri/capabilities/mobile.json` for mobile-specific permissions. Initially minimal — same as desktop minus file associations.

### Build Commands

```bash
cd krillnotes-desktop
npm run tauri ios dev       # iPad simulator
npm run tauri android dev   # Android emulator or USB device
```

### Known Risk

SQLCipher on Android x86_64 emulator has a known crash (`__extenddftf2` symbol). Real ARM devices work fine. Mitigation: test on real devices or ARM64 emulator images.

## Adaptive Layout

### Breakpoints

| CSS Width | Layout | Mode |
|-----------|--------|------|
| < 640px | Phone | Stack navigation |
| 640–1024px | Tablet portrait | Compact sidebar + note panel |
| > 1024px | Tablet landscape / Desktop | Full side-by-side |

These use CSS logical pixels (not physical). Modern phones are 360–430 CSS px wide; tablets start at 744+ CSS px. The 640px boundary aligns with Tailwind's `sm:` breakpoint.

### `useLayout()` Hook

```typescript
type Layout = "phone" | "tablet" | "desktop";
function useLayout(): Layout
```

Listens to viewport resize events. Updates reactively on device rotation (iPad portrait ↔ landscape).

### Phone Layout (< 640px) — Stack Navigation

**New component: `MobileNav.tsx`**

Wraps existing `TreeView` and `InfoPanel` in a stack navigator:

- `screen: "tree"` → full-screen tree view
- Tap note → slide transition to `screen: "note"` → full-screen `InfoPanel`
- Back button / swipe-back → returns to tree

**Bottom navigation bar:**

| Icon | Label | Action |
|------|-------|--------|
| 🏠 | Tree | Navigate to tree view |
| 🔍 | Search | Open search |
| 🏷️ | Tags | Slide up tag cloud bottom sheet |
| ☰ | More | Workspace settings, future Phase 3 features |

Bottom bar is persistent across both tree and note screens.

### Tablet Portrait (640–1024px) — Compact Sidebar

Uses existing `WorkspaceView` layout with adjustments:

- Narrower default sidebar width
- Tree, search, and tag cloud in sidebar (same as desktop)
- Draggable divider with touch support (wider hit target, min/max constraints)
- Hamburger button (☰) in top toolbar for workspace actions (replaces native menu)

### Tablet Landscape (> 1024px) — Desktop-Like

Current desktop layout, with:

- Same draggable divider (touch-enabled)
- Hamburger button (☰) in toolbar (no native menu bar on mobile)
- Touch-friendly tap targets

### Navigation Summary

| Element | Phone | Tablet (both) | Desktop |
|---------|-------|---------------|---------|
| Tree | Full screen (stack) | Sidebar | Sidebar |
| Tags | Bottom sheet via bar | In sidebar | In sidebar |
| Search | Bottom bar icon | Top of sidebar | Top of sidebar |
| More/actions | Bottom bar icon | Toolbar ☰ button | Native menu bar |
| Panel divider | N/A (stack nav) | Draggable (touch) | Draggable (mouse) |

### WorkspaceView Integration

```typescript
const layout = useLayout();

if (layout === "phone") return <MobileNav ... />;
if (layout === "tablet") return <CompactSidebarLayout ... />;
return <CurrentDesktopLayout ... />;
```

Tablet compact layout may not need a separate component — could be the existing layout with different default props.

## Touch & Mobile UX

### Interaction Changes

- **Context menu:** Long-press on tree node (instead of right-click). On phone, renders as bottom sheet; on tablet, floating menu.
- **Tap targets:** Minimum 44px height on tree nodes (Apple HIG).
- **Swipe-back:** Phone stack navigation supports left-edge swipe to go back.
- **Virtual keyboard:** Fields scroll into view when keyboard appears. CSS `env(safe-area-inset-bottom)` prevents content behind keyboard.
- **Safe areas:** Padding for notches, dynamic islands, rounded corners, home indicator bars via `safe-area-inset-*`.

### Resizable Panels (Tablet)

Extend existing `useResizablePanels` hook:

- Add `touchstart/touchmove/touchend` listeners alongside mouse events
- Invisible touch zone (~20px) around the 4px visual divider
- Min width: ~120px. Max width: 60% of viewport.
- Phone layout: divider not rendered (stack navigation)

### Features Hidden on Phone

- Script editor (CodeMirror + phone keyboard = painful) — Phase 3
- Operations log dialog — Phase 3
- RBAC permission UI — Phase 3
- Resizable panel divider

### Features That Work As-Is

- Search bar, tag pills / tag cloud
- Field display (read-only) and field editing (text, number, boolean, date, select, rating)
- Add note / delete note dialogs
- Workspace open/create dialogs

## Desktop Impact

The native menu bar (`menu.rs`) is gated behind `#[cfg(desktop)]` and remains unchanged. Desktop users see no difference. The `useLayout()` hook returns `"desktop"` on large viewports, so desktop renders the existing layout.

The device ID migration improves desktop stability (no more MAC address drift) and is the only change that affects desktop behavior.
