# Tauri v2 Migration Design

**Date:** 2026-02-17
**Status:** Approved
**Replaces:** iced UI implementation (Tasks 11-12)

## Context

The original plan used iced for the desktop UI. After implementing Tasks 1-12, we've decided to migrate to Tauri v2 for better long-term scalability, web/mobile support, and team familiarity with React.

**What's Working:**
- ✅ Complete Rust core backend (Tasks 1-10)
- ✅ SQLite storage, Rhai scripting, operation logging
- ✅ 13 passing tests
- ✅ Hierarchical note structure with CRUD operations

**What's Changing:**
- ❌ Remove iced UI (Tasks 11-12)
- ✨ Replace with Tauri v2 + React + TypeScript

## Goals

### MVP Scope (Minimal)
- Native application window with proper OS menu bar
- Menu items: File (New, Open), Edit (Add Note, Delete), View, Help
- Menu clicks trigger events that update status message
- Clean foundation for future features (workspace, tree view, detail pane)

### Long-term Vision
- Core library reusable for web and mobile platforms
- Workspace structure supports future krillnotes-web, krillnotes-mobile
- Theming system via CSS custom properties

## Architecture

### Tech Stack
- **Backend:** Rust + Tauri v2 (native menus, file dialogs, OS integration)
- **Frontend:** React 18 + TypeScript
- **Build Tool:** Vite (fast HMR, modern bundler)
- **Styling:** Tailwind CSS + CSS custom properties (theming support)
- **Core Library:** krillnotes-core (shared across desktop/web/mobile)

### Workspace Structure (Approach 2)

```
krillnotes/
├── Cargo.toml                    # Workspace root manifest
├── .gitignore
├── docs/
│   └── plans/
│
├── krillnotes-core/              # Shared core library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── core/                 # Moved from src/core/
│       │   ├── mod.rs
│       │   ├── error.rs
│       │   ├── note.rs
│       │   ├── storage.rs
│       │   ├── workspace.rs
│       │   ├── operation.rs
│       │   ├── operation_log.rs
│       │   ├── scripting.rs
│       │   └── device.rs
│       └── system_scripts/       # Moved from system_scripts/
│           └── text_note.rhai
│
└── krillnotes-desktop/           # Tauri desktop app
    ├── package.json
    ├── tsconfig.json
    ├── vite.config.ts
    ├── tailwind.config.js
    ├── postcss.config.js
    ├── index.html
    ├── src/                      # React frontend
    │   ├── main.tsx
    │   ├── App.tsx
    │   ├── styles/
    │   │   └── globals.css       # Tailwind + CSS variables
    │   └── components/
    │       └── StatusMessage.tsx
    └── src-tauri/                # Rust backend
        ├── Cargo.toml            # Depends on krillnotes-core
        ├── tauri.conf.json
        ├── build.rs
        ├── icons/
        └── src/
            ├── main.rs           # Tauri app + menu setup
            ├── lib.rs
            └── menu.rs           # Native menu builder
```

**Why Workspace Approach:**
- Core library is reusable (future web/mobile support)
- Clean separation of concerns
- Desktop app is just one consumer of the core
- Easier to test and maintain independently

## Native Menu Implementation

### Menu Structure

**macOS:** Menu appears in system menu bar
**Windows/Linux:** Menu appears at top of window

### Menu Definition (src-tauri/src/menu.rs)

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

### Menu Event Handling (src-tauri/src/main.rs)

```rust
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .menu(|app| menu::build_menu(app))
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "file_new" => {
                    app.emit("menu-action", "File > New clicked").ok();
                }
                "file_open" => {
                    app.emit("menu-action", "File > Open clicked").ok();
                }
                "edit_add_note" => {
                    app.emit("menu-action", "Edit > Add Note clicked").ok();
                }
                "edit_delete_note" => {
                    app.emit("menu-action", "Edit > Delete clicked").ok();
                }
                "view_refresh" => {
                    app.emit("menu-action", "View > Refresh clicked").ok();
                }
                "help_about" => {
                    app.emit("menu-action", "Help > About clicked").ok();
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**Communication Pattern:**
- Menu click (Rust) → Emit event → Frontend listener (React)
- Type-safe events via Tauri's event system
- For MVP: events just update status message
- Future: events trigger actual functionality (open file dialog, create note, etc.)

## Frontend Structure (React)

### Component Architecture (Minimal MVP)

```
src/
├── main.tsx                 # React entry point
├── App.tsx                  # Root component
├── styles/
│   └── globals.css          # Tailwind + CSS custom properties
└── components/
    └── StatusMessage.tsx    # Displays menu action messages
```

### App.tsx (Root Component)

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

### StatusMessage.tsx

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

**Design Principles:**
- Simple, flat structure (no premature abstractions)
- TypeScript for type safety
- Tauri event hooks for backend communication
- CSS variables for theming (via Tailwind)
- Easy to expand (add TreeView, DetailPane, etc.)

## Styling System (Tailwind + Theming)

### globals.css (Tailwind + Theme Variables)

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

### tailwind.config.js

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

**Theming Benefits:**
- HSL color format (easy to adjust lightness/saturation)
- CSS custom properties = runtime theming (no rebuild needed)
- `.dark` class switches to dark mode
- Semantic color names (background, foreground, etc.)
- Future: can add `.theme-blue`, `.theme-green`, etc.
- Follows shadcn/ui conventions (easy to add components later)

**Future Theming Implementation:**
```typescript
// Toggle dark mode
document.documentElement.classList.toggle('dark');

// Add custom themes
document.documentElement.classList.add('theme-ocean');
```

## Build and Development Workflow

### Development Commands

```bash
# Initial setup (one time)
cd krillnotes-desktop
npm install                    # Install frontend dependencies
cd src-tauri
cargo build                    # Build Rust backend

# Development mode (hot reload)
cd krillnotes-desktop
npm run tauri:dev             # Starts Vite dev server + Tauri app
                              # Frontend changes = instant HMR
                              # Rust changes = auto-rebuild + restart

# Production build
npm run tauri:build           # Creates optimized binary
                              # Output: src-tauri/target/release/bundle/
```

### package.json Scripts

```json
{
  "name": "krillnotes-desktop",
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/plugin-shell": "^2.0.0",
    "react": "^18.3.0",
    "react-dom": "^18.3.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@types/react": "^18.3.0",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0",
    "autoprefixer": "^10.4.20",
    "postcss": "^8.4.47",
    "tailwindcss": "^3.4.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0"
  }
}
```

### Testing Strategy

```bash
# Core library tests (existing - must pass)
cd krillnotes-core
cargo test                    # All 13 existing backend tests

# Desktop app tests (future)
cd krillnotes-desktop/src-tauri
cargo test                    # Tauri-specific tests

# Frontend tests (future)
cd krillnotes-desktop
npm test                      # React component tests
```

**Key Points:**
- Single command development (`npm run tauri:dev`)
- Vite HMR for instant React updates
- Automatic Rust recompilation on changes
- TypeScript type checking
- Production builds create native installers (.dmg, .exe, .AppImage)

## Migration Steps

### What Gets Removed
```bash
# Delete iced UI code
src/ui/                       # ❌ Delete entire directory
  ├── mod.rs
  ├── app.rs
  └── menu.rs

src/main.rs                   # ❌ Delete (will be replaced)
```

### What Gets Moved
```bash
# Move core library
src/core/          →  krillnotes-core/src/core/
system_scripts/    →  krillnotes-core/src/system_scripts/

# Move dependencies
Cargo.toml         →  Split into workspace + core Cargo.toml
```

### What Gets Created
```bash
# New workspace structure
Cargo.toml                              # ✨ Workspace root
krillnotes-core/                        # ✨ Core library crate
  ├── Cargo.toml
  └── src/
      ├── lib.rs
      ├── core/                         # Moved from src/core/
      └── system_scripts/               # Moved from system_scripts/

krillnotes-desktop/                     # ✨ Tauri desktop app
  ├── package.json
  ├── tsconfig.json
  ├── vite.config.ts
  ├── tailwind.config.js
  ├── postcss.config.js
  ├── index.html
  ├── src/                              # React frontend
  │   ├── main.tsx
  │   ├── App.tsx
  │   ├── styles/globals.css
  │   └── components/StatusMessage.tsx
  └── src-tauri/                        # Rust backend
      ├── Cargo.toml
      ├── tauri.conf.json
      ├── build.rs
      ├── icons/
      └── src/
          ├── main.rs
          ├── lib.rs
          └── menu.rs
```

### Migration Sequence

1. **Create workspace structure** - Set up Cargo workspace
2. **Move core to library crate** - Relocate src/core → krillnotes-core
3. **Verify core tests** - Ensure all 13 tests still pass
4. **Initialize Tauri app** - Create krillnotes-desktop with React + TypeScript
5. **Wire up dependencies** - Link desktop app to core library
6. **Implement native menu** - Add menu.rs and event handling
7. **Create minimal UI** - Basic React app with status message
8. **Delete iced code** - Remove src/ui/, src/main.rs
9. **Update .gitignore** - Add node_modules, dist, etc.
10. **Verify builds** - Test cargo test, npm run tauri:dev
11. **Git commit** - Clear migration message with context

### Rollback Strategy
- Original iced code remains in git history (can reference if needed)
- Core library tests validate nothing broke during move
- Can keep iced code in a separate branch temporarily if desired

### Success Criteria
- ✅ All 13 core tests pass
- ✅ `npm run tauri:dev` launches app
- ✅ Native menu appears in OS menu bar
- ✅ Menu clicks update status message
- ✅ Window displays "Krillnotes" with centered layout
- ✅ Clean build with no warnings

## Future Work (Out of Scope for MVP)

### Phase 2: Functional Features
- Workspace integration (create/open .db files)
- Tree view (hierarchical note list)
- Detail pane (edit note title and fields)
- Add note dialog

### Phase 3: Advanced Features
- File picker integration
- Search functionality
- Multiple view types (Table, Kanban, Calendar)
- Keyboard shortcuts
- Undo/redo

### Phase 4: Cross-Platform
- krillnotes-web (Tauri on web or separate SPA)
- krillnotes-mobile (core + Flutter/React Native)
- Sync engine integration

## Decision Log

**Q: Why Tauri v2 instead of iced?**
A: iced is less mature, smaller ecosystem. Tauri offers web technologies, better cross-platform support, and team familiarity with React.

**Q: Why workspace structure instead of monorepo?**
A: Enables core library reuse for future web/mobile. Clean separation. Easier to test independently.

**Q: Why Tailwind + CSS custom properties?**
A: Tailwind for rapid UI development. CSS custom properties enable runtime theming without rebuilds.

**Q: Why minimal MVP scope?**
A: Prove Tauri works, establish foundation. Avoid over-building before validating the new stack.

**Q: Why React + TypeScript?**
A: Team already uses this stack. Largest Tauri ecosystem. Good TypeScript support.

## References

- [Tauri v2 Documentation](https://v2.tauri.app/)
- [Tauri Native Menus](https://v2.tauri.app/develop/menu/)
- [React + Tauri Guide](https://v2.tauri.app/start/frontend/react/)
- [Tailwind CSS](https://tailwindcss.com/)
- Original MVP Plan: `docs/plans/2026-02-17-mvp-implementation.md`

---

**Next Step:** Create implementation plan with detailed tasks
