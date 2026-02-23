# User Guide Page — Design

**Date:** 2026-02-23
**Status:** Approved

## Goal

Add a screenshot-driven user guide to the Krillnotes website as a single scrollable docs page, using the existing screenshots taken of the contacts demo workspace.

## Approach

Hugo page bundle at `content/docs/user-guide/`. The page is a standard markdown file (`index.md`); images live in the same directory as page resources. Plain markdown image syntax is used (`![alt](filename.png)`). One new `.screenshot` CSS class is added to `style.css` for consistent screenshot presentation.

## File Changes

| File | Action |
|------|--------|
| `content/docs/user-guide/index.md` | Create — the user guide page |
| `content/docs/user-guide/*.png` | Move from `screenshots/`, renamed to kebab-case |
| `static/css/style.css` | Add `.screenshot` CSS class |
| `hugo.toml` | Add "User Guide" nav entry between Docs and Scripting |

## Screenshot → Filename Mapping

| Original | Renamed |
|----------|---------|
| Welcome Screen.png | welcome-screen.png |
| File Menu.png | file-menu.png |
| Open Workspace.png | open-workspace.png |
| Settings Dialog.png | settings-dialog.png |
| Main Window.png | main-window.png |
| Note Context Menu.png | note-context-menu.png |
| Add Note Dialog.png | add-note-dialog.png |
| Add Note with type Select.png | add-note-type-select.png |
| Viewing a Contact.png | viewing-a-contact.png |
| Editing a Contact.png | editing-a-contact.png |
| Manage Scripts.png | manage-scripts.png |
| Editing a Script.png | editing-a-script.png |
| Operations Log.png | operations-log.png |
| Edit Menu.png | edit-menu.png |
| Tools Menu.png | tools-menu.png |

## Page Structure

1. **Your First Workspace** — Welcome screen, File menu (New/Open/Export/Import), Open Workspace dialog, Settings dialog
2. **The Main Window** — Main window overview (tree panel + detail panel)
3. **Adding Notes** — Context menu, Add Note dialog, note type selector
4. **Contacts** — Viewing a contact, editing a contact
5. **Scripts** — Manage Scripts panel, editing a script
6. **Operations Log** — Operations log viewer

## CSS Addition

```css
.screenshot {
  display: block;
  margin: var(--space-xl) auto;
  max-width: 720px;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}
```

## Nav Entry (hugo.toml)

```toml
[[menu.main]]
  name = 'User Guide'
  url = '/docs/user-guide/'
  weight = 2
```

Existing Docs entry becomes weight 1, Scripting weight 3, GitHub weight 4.
