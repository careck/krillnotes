# Docs ToC Sidebar Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a JS-generated sticky sidebar ToC with active-section highlighting to all docs pages, and remove the manual ToC from `scripting.md`.

**Architecture:** `single.html` gains a two-column CSS grid layout (`240px 1fr`). JS scans `h2`/`h3` headings at page load, builds a nested `<ul>` in the sidebar `<nav>`, and an `IntersectionObserver` adds `.active` to whichever link's heading is currently in the top of the viewport. Below 1100px the sidebar collapses away.

**Tech Stack:** Hugo static site, plain CSS (design tokens in `:root`), vanilla JS (no framework). `hugo serve` for local preview.

---

### Task 1: Update `single.html` layout

**Files:**
- Modify: `layouts/_default/single.html`

**Step 1: Replace the file content**

```html
{{ define "main" }}
<div class="docs-page">
  <nav class="docs-toc" aria-label="On this page"></nav>
  <article class="docs-layout">
    <h1>{{ .Title }}</h1>
    {{ with .Params.description }}<p class="docs-description">{{ . }}</p>{{ end }}
    <div class="docs-content">
      {{ .Content }}
    </div>
  </article>
</div>
{{ end }}
```

**Step 2: Verify in browser**

Run: `hugo serve` from the website repo root, open `http://localhost:1313/docs/scripting/`.
Expected: page looks identical to before (sidebar is empty and hidden until JS runs).

**Step 3: Commit**

```bash
git add layouts/_default/single.html
git commit -m "feat: add docs-page wrapper and empty docs-toc nav to single layout"
```

---

### Task 2: Add CSS for the two-column docs layout

**Files:**
- Modify: `static/css/style.css` — append after the existing `/* ---- Docs Layout ---- */` block (currently ends around line 923 before `/* ---- Template Gallery ---- */`)

**Step 1: Add the new rules**

Insert the following after `.docs-content img { … }` (end of the existing docs section, before `/* ---- Template Gallery ---- */`):

```css
/* ---- Docs Page (sidebar + content grid) ---- */

.docs-page {
  display: grid;
  grid-template-columns: 240px 1fr;
  column-gap: var(--space-3xl);
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 var(--space-xl);
  align-items: start;
}

.docs-toc {
  position: sticky;
  top: 5rem;
  max-height: calc(100vh - 6rem);
  overflow-y: auto;
  padding: calc(64px + var(--space-3xl)) 0 var(--space-4xl);
  font-size: 0.85rem;
}

.docs-toc:empty {
  display: none;
}

.docs-toc-label {
  font-size: 0.7rem;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--sand-400);
  margin-bottom: var(--space-md);
}

.docs-toc ul {
  list-style: none;
  padding: 0;
  margin: 0;
}

.docs-toc li {
  margin: 0;
}

.docs-toc a {
  display: block;
  padding: 0.3rem 0.5rem;
  color: var(--sand-500);
  text-decoration: none;
  border-left: 2px solid transparent;
  line-height: 1.4;
  transition: color 0.15s, border-color 0.15s;
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
}

.docs-toc a:hover {
  color: var(--ocean-deep);
}

.docs-toc a.active {
  color: var(--krill-primary);
  border-left-color: var(--krill-primary);
  font-weight: 500;
}

/* h3 items are nested and indented */
.docs-toc ul ul {
  padding-left: var(--space-md);
}

.docs-toc ul ul a {
  font-size: 0.82rem;
  padding-top: 0.2rem;
  padding-bottom: 0.2rem;
}

/* Remove the old max-width centering from .docs-layout —
   the grid now controls positioning */
.docs-layout {
  max-width: 840px;
  padding-left: 0;
  padding-right: 0;
}

@media (max-width: 1100px) {
  .docs-page {
    grid-template-columns: 1fr;
    max-width: 840px;
  }
  .docs-toc {
    display: none;
  }
  .docs-layout {
    padding-left: 0;
    padding-right: 0;
  }
}
```

**Step 2: Verify in browser**

Reload `http://localhost:1313/docs/scripting/`.
Expected: content is now left-aligned in the right column, left sidebar area is blank (JS not added yet). At viewport < 1100px, layout should be single column identical to before.

**Step 3: Commit**

```bash
git add static/css/style.css
git commit -m "feat: add docs-page grid layout and docs-toc sidebar CSS"
```

---

### Task 3: Add JS ToC builder and IntersectionObserver

**Files:**
- Modify: `static/js/main.js` — append to existing file

**Step 1: Append the ToC script**

The existing `main.js` is 12 lines. Append after the closing `});` of the mobile nav handler:

```js
// Docs sidebar ToC
function buildDocsToc() {
  const nav = document.querySelector('.docs-toc');
  if (!nav) return;

  const content = document.querySelector('.docs-content');
  if (!content) return;

  const headings = Array.from(content.querySelectorAll('h2, h3'));
  if (headings.length === 0) return;

  // Label
  const label = document.createElement('p');
  label.className = 'docs-toc-label';
  label.textContent = 'On this page';
  nav.appendChild(label);

  // Build nested list
  const root = document.createElement('ul');
  let currentH2Li = null;
  let currentSubList = null;

  headings.forEach(h => {
    const a = document.createElement('a');
    a.href = '#' + h.id;
    a.textContent = h.textContent;

    const li = document.createElement('li');
    li.appendChild(a);

    if (h.tagName === 'H2') {
      currentSubList = document.createElement('ul');
      li.appendChild(currentSubList);
      root.appendChild(li);
      currentH2Li = li;
    } else {
      // H3 — nest under current h2's sublist, or root if none
      (currentSubList || root).appendChild(li);
    }
  });

  nav.appendChild(root);

  // Active-section tracking via IntersectionObserver
  const links = nav.querySelectorAll('a');
  const linkMap = new Map(
    Array.from(links).map(a => [a.getAttribute('href').slice(1), a])
  );

  let activeLink = null;

  const observer = new IntersectionObserver(entries => {
    entries.forEach(entry => {
      if (entry.isIntersecting) {
        const link = linkMap.get(entry.target.id);
        if (link) {
          if (activeLink) activeLink.classList.remove('active');
          activeLink = link;
          activeLink.classList.add('active');
        }
      }
    });
  }, {
    rootMargin: '-10% 0px -80% 0px'
  });

  headings.forEach(h => observer.observe(h));
}

document.addEventListener('DOMContentLoaded', buildDocsToc);
```

**Step 2: Verify in browser**

Reload `http://localhost:1313/docs/scripting/`.
Expected:
- "On this page" label appears in the left sidebar
- All 14 numbered h2 sections listed; h3 sub-items nested beneath each
- Active link highlights in krill coral as you scroll
- At < 1100px viewport width, sidebar is hidden

**Step 3: Commit**

```bash
git add static/js/main.js
git commit -m "feat: add JS ToC builder with IntersectionObserver active-section tracking"
```

---

### Task 4: Remove manual ToC from `scripting.md`

**Files:**
- Modify: `content/docs/scripting.md`

**Step 1: Delete lines 13–30**

Lines to remove (currently):
```markdown
## Table of Contents

1. [Script structure](#1-script-structure)
2. [Defining schemas](#2-defining-schemas)
3. [Field types](#3-field-types)
4. [Schema options](#4-schema-options)
5. [on_save hook](#5-on_save-hook)
6. [on_view hook](#6-on_view-hook)
7. [on_add_child hook](#7-on_add_child-hook)
8. [add_tree_action](#8-add_tree_action)
9. [Display helpers](#9-display-helpers)
10. [Query functions](#10-query-functions)
11. [Utility functions](#11-utility-functions)
12. [Introspection functions](#12-introspection-functions)
13. [Tips and patterns](#13-tips-and-patterns)
14. [Built-in script examples](#14-built-in-script-examples)

---
```

After removal the file should flow directly from the intro paragraph and `---` separator to `## 1. Script structure`.

**Step 2: Verify in browser**

Reload `http://localhost:1313/docs/scripting/`.
Expected: no ToC list visible in the page body; sidebar ToC still present and complete.

**Step 3: Commit**

```bash
git add content/docs/scripting.md
git commit -m "docs: remove manual ToC from scripting guide (replaced by sidebar)"
```

---

### Task 5: Smoke-check other doc pages

**Step 1: Open each doc page**

- `http://localhost:1313/docs/getting-started/`
- `http://localhost:1313/docs/user-guide/`

Expected: sidebar appears if the page has `h2`/`h3` headings, is absent (hidden) if not. No layout breakage in either case.

**Step 2: Commit if any minor fixes were needed**

If no fixes needed, no commit required.
