# Tauri v2 Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Migrate from iced to Tauri v2 with React + TypeScript, establishing workspace structure for future web/mobile support.

**Architecture:** Cargo workspace with krillnotes-core library and krillnotes-desktop Tauri app. Native OS menus communicate via events to React frontend. Tailwind CSS with CSS custom properties for theming.

**Tech Stack:** Rust, Tauri v2, React 18, TypeScript, Vite, Tailwind CSS

---

## Task 1: Create Cargo Workspace Structure

**Files:**
- Modify: `Cargo.toml` (root)

**Step 1: Backup existing Cargo.toml**

```bash
cp Cargo.toml Cargo.toml.backup
```

**Step 2: Create workspace Cargo.toml**

Replace root `Cargo.toml` with:
```toml
[workspace]
members = ["krillnotes-core"]
resolver = "2"

[workspace.dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
rhai = "1.17"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
uuid = { version = "1.7", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1.0"

[workspace.dependencies.iced]
version = "0.13"
features = []

[workspace.dependencies.iced_aw]
version = "0.10"
features = ["menu"]
```

**Step 3: Verify workspace structure**

Run: `cargo metadata --format-version 1 | grep workspace_root`
Expected: Shows workspace root path

**Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "chore: convert to Cargo workspace"
```

---

## Task 2: Create krillnotes-core Library Crate

**Files:**
- Create: `krillnotes-core/Cargo.toml`
- Create: `krillnotes-core/src/lib.rs`

**Step 1: Create directory structure**

```bash
mkdir -p krillnotes-core/src
```

**Step 2: Create krillnotes-core/Cargo.toml**

Create `krillnotes-core/Cargo.toml`:
```toml
[package]
name = "krillnotes-core"
version = "0.1.0"
edition = "2021"
authors = ["Your Name"]
description = "Core library for Krillnotes - local-first note-taking"
license = "MIT"

[dependencies]
rusqlite = { workspace = true }
rhai = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
tempfile = "3.8"
```

**Step 3: Create minimal lib.rs**

Create `krillnotes-core/src/lib.rs`:
```rust
// Re-export core modules (will be populated next task)
```

**Step 4: Verify crate builds**

Run: `cargo build -p krillnotes-core`
Expected: Compilation succeeds

**Step 5: Commit**

```bash
git add krillnotes-core/
git commit -m "chore: create krillnotes-core library crate"
```

---

## Task 3: Move Core Module to Library Crate

**Files:**
- Move: `src/core/` → `krillnotes-core/src/core/`
- Modify: `krillnotes-core/src/lib.rs`

**Step 1: Move core directory**

```bash
mv src/core krillnotes-core/src/
```

**Step 2: Update lib.rs to export core**

Modify `krillnotes-core/src/lib.rs`:
```rust
pub mod core;

// Re-export commonly used types
pub use core::{
    error::{KrillnotesError, Result},
    note::{FieldValue, Note},
    operation::Operation,
    operation_log::{OperationLog, PurgeStrategy},
    scripting::{FieldDefinition, Schema, SchemaRegistry},
    storage::Storage,
    workspace::{AddPosition, Workspace},
};
```

**Step 3: Verify core builds**

Run: `cargo build -p krillnotes-core`
Expected: Compilation succeeds

**Step 4: Run core tests**

Run: `cargo test -p krillnotes-core`
Expected: All 13 tests pass

**Step 5: Commit**

```bash
git add krillnotes-core/src/
git commit -m "refactor: move core module to krillnotes-core crate"
```

---

## Task 4: Move System Scripts to Library Crate

**Files:**
- Move: `system_scripts/` → `krillnotes-core/src/system_scripts/`
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Move system_scripts directory**

```bash
mv system_scripts krillnotes-core/src/
```

**Step 2: Update scripting.rs include path**

Modify `krillnotes-core/src/core/scripting.rs` line ~913:
```rust
// Old:
registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;

// New:
registry.load_script(include_str!("../../system_scripts/text_note.rhai"))?;
```

**Step 3: Verify core tests still pass**

Run: `cargo test -p krillnotes-core`
Expected: All 13 tests pass (especially test_text_note_schema_loaded)

**Step 4: Commit**

```bash
git add krillnotes-core/src/system_scripts/
git add krillnotes-core/src/core/scripting.rs
git rm -r system_scripts/
git commit -m "refactor: move system scripts to krillnotes-core"
```

---

## Task 5: Delete Iced UI Code

**Files:**
- Delete: `src/ui/`
- Delete: `src/main.rs`
- Delete: `src/lib.rs`

**Step 1: Remove UI directory**

```bash
git rm -r src/ui/
```

**Step 2: Remove main.rs and lib.rs**

```bash
git rm src/main.rs src/lib.rs
```

**Step 3: Remove src directory (now empty)**

```bash
rmdir src
```

**Step 4: Verify workspace still compiles**

Run: `cargo build -p krillnotes-core`
Expected: Core compiles successfully

**Step 5: Commit**

```bash
git commit -m "refactor: remove iced UI code

Preparing for Tauri v2 migration. Original iced code
remains in git history if needed for reference.
"
```

---

## Task 6: Install Tauri CLI and Prerequisites

**Files:**
- None (system dependencies)

**Step 1: Check Node.js version**

Run: `node --version`
Expected: v18.0.0 or higher

**Step 2: Install Tauri CLI globally**

Run: `npm install -g @tauri-apps/cli@next`
Expected: Installation succeeds

**Step 3: Verify Tauri CLI**

Run: `tauri --version`
Expected: Shows version 2.x.x

**Step 4: Install system dependencies (macOS)**

If on macOS:
```bash
xcode-select --install
```

Expected: Xcode command line tools installed (or already present)

**Step 5: No commit needed (system-level changes)**

---

## Task 7: Initialize Tauri Desktop App

**Files:**
- Create: `krillnotes-desktop/` directory structure

**Step 1: Create desktop app with Tauri CLI**

```bash
npm create tauri-app@latest -- --name krillnotes-desktop --template react-ts --manager npm --yes
```

Expected: Creates krillnotes-desktop/ directory with React + TypeScript template

**Step 2: Verify directory structure**

Run: `ls -la krillnotes-desktop/`
Expected: See package.json, src/, src-tauri/, etc.

**Step 3: Install dependencies**

```bash
cd krillnotes-desktop
npm install
cd ..
```

Expected: node_modules/ created, dependencies installed

**Step 4: Add desktop app to workspace**

Modify root `Cargo.toml` members:
```toml
[workspace]
members = ["krillnotes-core", "krillnotes-desktop/src-tauri"]
resolver = "2"
```

**Step 5: Commit**

```bash
git add krillnotes-desktop/ Cargo.toml
git commit -m "chore: initialize Tauri desktop app with React + TypeScript"
```

---

## Task 8: Link Desktop App to Core Library

**Files:**
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add core dependency to desktop Cargo.toml**

Modify `krillnotes-desktop/src-tauri/Cargo.toml`, add to `[dependencies]`:
```toml
krillnotes-core = { path = "../../krillnotes-core" }
```

**Step 2: Re-export core from lib.rs**

Modify `krillnotes-desktop/src-tauri/src/lib.rs`:
```rust
// Re-export core library
pub use krillnotes_core::*;

// Tauri plugins and other desktop-specific code
```

**Step 3: Verify desktop app builds**

Run: `cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: Compilation succeeds

**Step 4: Verify core tests still accessible**

Run: `cargo test -p krillnotes-core`
Expected: All 13 tests pass

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/Cargo.toml
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: link desktop app to krillnotes-core library"
```

---

## Task 9: Install and Configure Tailwind CSS

**Files:**
- Modify: `krillnotes-desktop/package.json`
- Create: `krillnotes-desktop/tailwind.config.js`
- Create: `krillnotes-desktop/postcss.config.js`
- Create: `krillnotes-desktop/src/styles/globals.css`

**Step 1: Install Tailwind dependencies**

```bash
cd krillnotes-desktop
npm install -D tailwindcss postcss autoprefixer
cd ..
```

Expected: Dependencies added to package.json

**Step 2: Initialize Tailwind config**

```bash
cd krillnotes-desktop
npx tailwindcss init -p
cd ..
```

Expected: Creates tailwind.config.js and postcss.config.js

**Step 3: Configure Tailwind with theme variables**

Modify `krillnotes-desktop/tailwind.config.js`:
```javascript
/** @type {import('tailwindcss').Config} */
export default {
  darkMode: ['class'],
  content: [
    './index.html',
    './src/**/*.{js,ts,jsx,tsx}',
  ],
  theme: {
    extend: {
      colors: {
        background: 'hsl(var(--background))',
        foreground: 'hsl(var(--foreground))',
        primary: {
          DEFAULT: 'hsl(var(--primary))',
          foreground: 'hsl(var(--primary-foreground))',
        },
        secondary: {
          DEFAULT: 'hsl(var(--secondary))',
          foreground: 'hsl(var(--secondary-foreground))',
        },
        muted: {
          DEFAULT: 'hsl(var(--muted))',
          foreground: 'hsl(var(--muted-foreground))',
        },
        accent: {
          DEFAULT: 'hsl(var(--accent))',
          foreground: 'hsl(var(--accent-foreground))',
        },
        border: 'hsl(var(--border))',
        input: 'hsl(var(--input))',
        ring: 'hsl(var(--ring))',
      },
      borderRadius: {
        lg: 'var(--radius)',
        md: 'calc(var(--radius) - 2px)',
        sm: 'calc(var(--radius) - 4px)',
      },
    },
  },
  plugins: [],
}
```

**Step 4: Create globals.css with theme variables**

Create `krillnotes-desktop/src/styles/globals.css`:
```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    /* Light theme (default) */
    --background: 0 0% 100%;
    --foreground: 222.2 84% 4.9%;

    --primary: 222.2 47.4% 11.2%;
    --primary-foreground: 210 40% 98%;

    --secondary: 210 40% 96.1%;
    --secondary-foreground: 222.2 47.4% 11.2%;

    --muted: 210 40% 96.1%;
    --muted-foreground: 215.4 16.3% 46.9%;

    --accent: 210 40% 96.1%;
    --accent-foreground: 222.2 47.4% 11.2%;

    --border: 214.3 31.8% 91.4%;
    --input: 214.3 31.8% 91.4%;
    --ring: 222.2 84% 4.9%;

    --radius: 0.5rem;
  }

  .dark {
    /* Dark theme */
    --background: 222.2 84% 4.9%;
    --foreground: 210 40% 98%;

    --primary: 210 40% 98%;
    --primary-foreground: 222.2 47.4% 11.2%;

    --secondary: 217.2 32.6% 17.5%;
    --secondary-foreground: 210 40% 98%;

    --muted: 217.2 32.6% 17.5%;
    --muted-foreground: 215 20.2% 65.1%;

    --accent: 217.2 32.6% 17.5%;
    --accent-foreground: 210 40% 98%;

    --border: 217.2 32.6% 17.5%;
    --input: 217.2 32.6% 17.5%;
    --ring: 212.7 26.8% 83.9%;
  }
}

@layer base {
  * {
    @apply border-border;
  }
  body {
    @apply bg-background text-foreground;
  }
}
```

**Step 5: Commit**

```bash
git add krillnotes-desktop/package.json
git add krillnotes-desktop/package-lock.json
git add krillnotes-desktop/tailwind.config.js
git add krillnotes-desktop/postcss.config.js
git add krillnotes-desktop/src/styles/
git commit -m "feat: configure Tailwind CSS with theme variables"
```

---

## Task 10: Create StatusMessage Component

**Files:**
- Create: `krillnotes-desktop/src/components/StatusMessage.tsx`

**Step 1: Create components directory**

```bash
mkdir -p krillnotes-desktop/src/components
```

**Step 2: Create StatusMessage component**

Create `krillnotes-desktop/src/components/StatusMessage.tsx`:
```typescript
interface StatusMessageProps {
  message: string;
}

function StatusMessage({ message }: StatusMessageProps) {
  return (
    <div className="p-4 rounded-lg bg-secondary">
      <p className="text-sm text-secondary-foreground">{message}</p>
    </div>
  );
}

export default StatusMessage;
```

**Step 3: Verify TypeScript compilation**

Run: `cd krillnotes-desktop && npm run build`
Expected: Build succeeds with no TypeScript errors

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/
git commit -m "feat: create StatusMessage component"
```

---

## Task 11: Update App Component for Minimal UI

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`
- Modify: `krillnotes-desktop/src/main.tsx`

**Step 1: Replace App.tsx with minimal UI**

Replace `krillnotes-desktop/src/App.tsx`:
```typescript
import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import StatusMessage from './components/StatusMessage';
import './styles/globals.css';

function App() {
  const [statusMessage, setStatusMessage] = useState('Welcome to Krillnotes');

  useEffect(() => {
    // Listen for menu events from Rust backend
    const unlisten = listen<string>('menu-action', (event) => {
      setStatusMessage(event.payload);
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center">
      <div className="text-center">
        <h1 className="text-4xl font-bold mb-4">Krillnotes</h1>
        <StatusMessage message={statusMessage} />
      </div>
    </div>
  );
}

export default App;
```

**Step 2: Update main.tsx to import globals.css**

Verify `krillnotes-desktop/src/main.tsx` imports styles (should be default):
```typescript
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

**Step 3: Test frontend builds**

Run: `cd krillnotes-desktop && npm run build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git add krillnotes-desktop/src/main.tsx
git commit -m "feat: implement minimal UI with Tailwind styling"
```

---

## Task 12: Create Native Menu Module

**Files:**
- Create: `krillnotes-desktop/src-tauri/src/menu.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Create menu.rs with native menu builder**

Create `krillnotes-desktop/src-tauri/src/menu.rs`:
```rust
use tauri::{menu::*, AppHandle, Runtime};

pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, tauri::Error> {
    let menu = MenuBuilder::new(app)
        // File menu
        .items(&[
            &SubmenuBuilder::new(app, "File")
                .items(&[
                    &MenuItemBuilder::with_id("file_new", "New Workspace")
                        .accelerator("CmdOrCtrl+N")
                        .build(app)?,
                    &MenuItemBuilder::with_id("file_open", "Open Workspace...")
                        .accelerator("CmdOrCtrl+O")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ])
                .build()?,

            // Edit menu
            &SubmenuBuilder::new(app, "Edit")
                .items(&[
                    &MenuItemBuilder::with_id("edit_add_note", "Add Note")
                        .accelerator("CmdOrCtrl+Shift+N")
                        .build(app)?,
                    &MenuItemBuilder::with_id("edit_delete_note", "Delete Note")
                        .accelerator("CmdOrCtrl+Backspace")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                ])
                .build()?,

            // View menu
            &SubmenuBuilder::new(app, "View")
                .items(&[
                    &PredefinedMenuItem::fullscreen(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("view_refresh", "Refresh")
                        .accelerator("CmdOrCtrl+R")
                        .build(app)?,
                ])
                .build()?,

            // Help menu
            &SubmenuBuilder::new(app, "Help")
                .items(&[
                    &MenuItemBuilder::with_id("help_about", "About Krillnotes")
                        .build(app)?,
                ])
                .build()?,
        ])
        .build()?;

    Ok(menu)
}
```

**Step 2: Export menu module in lib.rs**

Modify `krillnotes-desktop/src-tauri/src/lib.rs`, add at top:
```rust
pub mod menu;

// Re-export core library
pub use krillnotes_core::*;
```

**Step 3: Verify Rust compiles**

Run: `cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: Compilation succeeds

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: create native menu module with OS integration"
```

---

## Task 13: Wire Up Menu Events in Tauri App

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/main.rs`

**Step 1: Replace main.rs with menu integration**

Replace `krillnotes-desktop/src-tauri/src/main.rs`:
```rust
// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

mod menu;

fn main() {
    tauri::Builder::default()
        .menu(|app| menu::build_menu(app))
        .on_menu_event(|app, event| {
            let message = match event.id().as_ref() {
                "file_new" => "File > New Workspace clicked",
                "file_open" => "File > Open Workspace clicked",
                "edit_add_note" => "Edit > Add Note clicked",
                "edit_delete_note" => "Edit > Delete Note clicked",
                "view_refresh" => "View > Refresh clicked",
                "help_about" => "Help > About Krillnotes clicked",
                _ => return, // Ignore unknown events
            };

            // Emit event to frontend
            app.emit("menu-action", message).ok();
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**Step 2: Verify Rust compiles**

Run: `cd krillnotes-desktop && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: Compilation succeeds

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/main.rs
git commit -m "feat: wire up menu events to frontend via Tauri events"
```

---

## Task 14: Update .gitignore for Node and Tauri

**Files:**
- Modify: `.gitignore` (root)

**Step 1: Add Node.js and Tauri patterns to .gitignore**

Append to `.gitignore`:
```
# Node.js
node_modules/
npm-debug.log*
yarn-debug.log*
yarn-error.log*
pnpm-debug.log*
lerna-debug.log*

# Build outputs
dist/
dist-ssr/
*.local

# Tauri
krillnotes-desktop/src-tauri/target/
krillnotes-desktop/src-tauri/WixTools/

# IDE
.vscode/*
!.vscode/extensions.json
.idea/
*.swp
*.swo
*~
```

**Step 2: Verify .gitignore works**

Run: `git status`
Expected: No node_modules/ or target/ directories shown

**Step 3: Commit**

```bash
git add .gitignore
git commit -m "chore: update .gitignore for Node.js and Tauri"
```

---

## Task 15: Test Development Build

**Files:**
- None (verification only)

**Step 1: Start Tauri dev server**

```bash
cd krillnotes-desktop
npm run tauri:dev
```

Expected:
- Vite dev server starts
- Tauri app window opens
- "Krillnotes" heading visible
- "Welcome to Krillnotes" status message visible

**Step 2: Test native menu (macOS/Linux)**

In the running app:
- Click "File" menu in menu bar
- Click "New Workspace"

Expected: Status message updates to "File > New Workspace clicked"

**Step 3: Test menu keyboard shortcuts**

Press: `Cmd+N` (macOS) or `Ctrl+N` (Windows/Linux)

Expected: Status message updates to "File > New Workspace clicked"

**Step 4: Test multiple menu items**

Try each menu item:
- File > Open (`Cmd+O`)
- Edit > Add Note (`Cmd+Shift+N`)
- Edit > Delete Note (`Cmd+Backspace`)
- View > Refresh (`Cmd+R`)
- Help > About

Expected: Each updates status message accordingly

**Step 5: Stop dev server**

Press: `Ctrl+C` in terminal

Expected: Dev server stops cleanly

---

## Task 16: Run All Core Tests

**Files:**
- None (verification only)

**Step 1: Run core library tests**

```bash
cargo test -p krillnotes-core
```

Expected: All 13 tests pass
```
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Step 2: Verify specific critical tests**

Run: `cargo test -p krillnotes-core test_text_note_schema_loaded`

Expected: Test passes (ensures system scripts moved correctly)

**Step 3: Run workspace-level tests**

```bash
cargo test --workspace
```

Expected: All tests pass

**Step 4: No commit needed (verification only)**

---

## Task 17: Build Production Binary

**Files:**
- None (build verification)

**Step 1: Build production binary**

```bash
cd krillnotes-desktop
npm run tauri:build
```

Expected:
- Build completes successfully
- Binary created in `src-tauri/target/release/bundle/`

**Step 2: Check bundle output (macOS)**

Run: `ls -lh src-tauri/target/release/bundle/macos/`

Expected: See .app bundle or .dmg file

**Step 3: Check bundle output (Windows)**

Run: `ls -lh src-tauri/target/release/bundle/msi/`

Expected: See .msi installer

**Step 4: Check bundle output (Linux)**

Run: `ls -lh src-tauri/target/release/bundle/appimage/`

Expected: See .AppImage file

**Step 5: No commit needed (build artifacts not committed)**

---

## Task 18: Update CHECKPOINT.md

**Files:**
- Modify: `CHECKPOINT.md`

**Step 1: Update checkpoint with migration status**

Replace `CHECKPOINT.md`:
```markdown
# Krillnotes Tauri Migration Checkpoint

**Date:** 2026-02-17
**Session:** Tauri v2 Migration Complete
**Branch:** master

## ✅ Completed Migration

### Backend (Unchanged)
- ✅ Core library (13 tests passing)
- ✅ SQLite storage
- ✅ Rhai scripting system
- ✅ Operation logging
- ✅ CRUD operations

### Architecture Changes
- ✅ Converted to Cargo workspace
- ✅ Created krillnotes-core library crate
- ✅ Created krillnotes-desktop Tauri app
- ✅ Removed iced UI code

### Tauri Desktop App
- ✅ React 18 + TypeScript frontend
- ✅ Vite build tool
- ✅ Tailwind CSS with theme variables
- ✅ Native OS menu bar (File, Edit, View, Help)
- ✅ Menu event communication (Rust → React)
- ✅ Minimal UI (status message display)

## 📊 Current State

### Project Structure
```
krillnotes/
├── krillnotes-core/          # Shared library
│   ├── src/core/             # All backend logic
│   └── src/system_scripts/   # Rhai schemas
└── krillnotes-desktop/       # Tauri desktop app
    ├── src/                  # React frontend
    └── src-tauri/            # Rust backend
```

### Test Status
```bash
cargo test -p krillnotes-core
# All 13 tests passing
```

### Development
```bash
cd krillnotes-desktop
npm run tauri:dev
# Launches app with hot reload
```

### Production Build
```bash
cd krillnotes-desktop
npm run tauri:build
# Creates native installer
```

## 🎯 Next Steps: Functional Features

**Phase 2: Workspace Integration (Next)**
- Add Tauri commands for workspace operations
- File picker integration (create/open .db files)
- Display workspace info in UI

**Phase 3: Tree View**
- Display hierarchical note list
- Note selection handling
- Tree view component

**Phase 4: Detail View**
- Edit note title and fields
- Auto-save functionality
- Schema-driven field rendering

## 🔗 References

- Design: `docs/plans/2026-02-17-tauri-migration-design.md`
- Implementation: `docs/plans/2026-02-17-tauri-migration.md`
- Original MVP Plan: `docs/plans/2026-02-17-mvp-implementation.md`

---

**Resume command:** Continue with Phase 2 implementation
```

**Step 2: Commit checkpoint**

```bash
git add CHECKPOINT.md
git commit -m "docs: update checkpoint for completed Tauri migration"
```

---

## Task 19: Final Verification and Commit

**Files:**
- None (final verification)

**Step 1: Verify workspace builds**

```bash
cargo build --workspace
```

Expected: All workspace members build successfully

**Step 2: Verify all tests pass**

```bash
cargo test --workspace
```

Expected: All 13 tests pass

**Step 3: Verify dev mode works**

```bash
cd krillnotes-desktop
npm run tauri:dev &
sleep 5
pkill -f "tauri dev"
```

Expected: App launches and closes cleanly

**Step 4: Create final migration commit**

```bash
git add -A
git commit -m "feat: complete Tauri v2 migration

BREAKING CHANGE: Replaced iced with Tauri v2 + React

- Converted to Cargo workspace structure
- Created krillnotes-core shared library
- Built krillnotes-desktop Tauri app
- Implemented native OS menus
- Set up React + TypeScript + Tailwind
- Configured theming with CSS custom properties

All 13 core tests passing. MVP functional with menu system.

Original iced code available in git history.

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>"
```

**Step 5: Tag release**

```bash
git tag v0.2.0-tauri-mvp
```

Expected: Tag created for this milestone

---

## Summary

**MVP Complete!** 🎉

**Delivered:**
- ✅ Cargo workspace with krillnotes-core library
- ✅ Tauri v2 desktop app with React + TypeScript
- ✅ Native OS menus (File, Edit, View, Help)
- ✅ Menu event communication (Rust → Frontend)
- ✅ Tailwind CSS with theming support
- ✅ All 13 core tests passing
- ✅ Development and production builds working

**Next Phase:**
- Phase 2: Workspace integration (Tauri commands, file picker)
- Phase 3: Tree view implementation
- Phase 4: Detail view with editing

**Verification Checklist:**
- [ ] `cargo test --workspace` passes
- [ ] `npm run tauri:dev` launches app
- [ ] Native menus appear in menu bar
- [ ] Menu clicks update status message
- [ ] Keyboard shortcuts work (Cmd+N, Cmd+O, etc.)
- [ ] `npm run tauri:build` creates installer
- [ ] All files committed and tagged
