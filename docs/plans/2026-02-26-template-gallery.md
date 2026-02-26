# Template Gallery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the plain link list on `/templates/` with a grid of screenshot cards that visually showcase each template.

**Architecture:** Create a Hugo section-specific list layout at `layouts/templates/list.html` that reads a `screenshot` frontmatter param from each child page and renders `.template-card` elements inside a `.template-grid`. Add corresponding CSS to `static/css/style.css`. No changes to `_default/list.html` or any other section.

**Tech Stack:** Hugo (Go templates), vanilla CSS (CSS custom properties already defined in style.css)

---

### Task 1: Add `screenshot` frontmatter to book-collection.md

**Files:**
- Modify: `content/templates/book-collection.md`

**Step 1: Add the param**

Open `content/templates/book-collection.md`. The current frontmatter is:

```yaml
---
title: "Book Collection"
description: "Track your reading list, current books, and past reads with star ratings."
---
```

Change it to:

```yaml
---
title: "Book Collection"
description: "Track your reading list, current books, and past reads with star ratings."
screenshot: "/templates/book_collection.screenshot.png"
---
```

**Step 2: Verify the file**

Check that `static/templates/book_collection.screenshot.png` exists:

```bash
ls static/templates/book_collection.screenshot.png
```

Expected: file listed with no error.

**Step 3: Commit**

```bash
git add content/templates/book-collection.md
git commit -m "feat: add screenshot frontmatter to book-collection template"
```

---

### Task 2: Create the templates section list layout

**Files:**
- Create: `layouts/templates/list.html`

**Step 1: Create the directory**

```bash
mkdir -p layouts/templates
```

**Step 2: Write the layout**

Create `layouts/templates/list.html` with:

```html
{{ define "main" }}
<article class="docs-layout">
  <h1>{{ .Title }}</h1>
  {{ with .Params.description }}<p class="docs-description">{{ . }}</p>{{ end }}
  <div class="docs-content">
    {{ .Content }}
  </div>
  <div class="template-grid">
    {{ range .Pages }}
    <a class="template-card" href="{{ .RelPermalink }}">
      <div class="template-card-thumb">
        {{ if .Params.screenshot }}
        <img src="{{ .Params.screenshot }}" alt="{{ .Title }} screenshot">
        {{ end }}
      </div>
      <div class="template-card-body">
        <h3 class="template-card-title">{{ .Title }}</h3>
        {{ with .Params.description }}
        <p class="template-card-desc">{{ . }}</p>
        {{ end }}
        <span class="template-card-link">View template →</span>
      </div>
    </a>
    {{ end }}
  </div>
</article>
{{ end }}
```

**Step 3: Verify Hugo resolves it**

Run the dev server:

```bash
hugo server -D
```

Navigate to `http://localhost:1313/templates/`. You should see the page render (even without styles yet) with card divs in the DOM instead of plain links. Check browser dev tools — Elements should show `.template-grid > .template-card` elements.

**Step 4: Commit**

```bash
git add layouts/templates/list.html
git commit -m "feat: add templates section list layout with card grid"
```

---

### Task 3: Add CSS for the template grid and cards

**Files:**
- Modify: `static/css/style.css`

**Step 1: Find the insertion point**

In `static/css/style.css`, find the `.docs-content img` block (around line 917). Insert the new styles **after** that block, before the `/* ---- Footer ---- */` comment.

**Step 2: Add the styles**

```css
/* ---- Template Gallery ---- */

.template-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-xl);
  margin-top: var(--space-2xl);
}

.template-card {
  display: flex;
  flex-direction: column;
  background: white;
  border: 1px solid var(--sand-200);
  border-radius: var(--radius-lg);
  overflow: hidden;
  text-decoration: none;
  color: inherit;
  transition: transform 0.2s, box-shadow 0.2s;
}

.template-card:hover {
  transform: translateY(-4px);
  box-shadow: var(--shadow-lg);
}

.template-card-thumb {
  height: 180px;
  background: var(--ocean-foam);
  overflow: hidden;
  flex-shrink: 0;
}

.template-card-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  object-position: top left;
  display: block;
  margin: 0;
  border-radius: 0;
  box-shadow: none;
}

.template-card-body {
  padding: var(--space-lg);
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  flex: 1;
}

.template-card-title {
  font-size: 1.05rem;
  font-weight: 650;
  color: var(--ocean-deep);
}

.template-card-desc {
  font-size: 0.9rem;
  color: var(--sand-400);
  line-height: 1.5;
  flex: 1;
}

.template-card-link {
  font-size: 0.875rem;
  color: var(--krill-primary);
  font-weight: 500;
  margin-top: var(--space-sm);
}
```

**Note on `.template-card-thumb img`:** The `static/css/style.css` has a `.docs-content img` rule that adds `margin`, `border-radius`, and `box-shadow` to all images inside docs content. The thumbnail image is outside `.docs-content`, so it won't be affected. The explicit resets (`margin: 0`, `border-radius: 0`, `box-shadow: none`) are defensive — keep them.

**Step 3: Verify in browser**

With `hugo server -D` running, navigate to `http://localhost:1313/templates/`.

Checklist:
- [ ] Card grid renders (not a plain list)
- [ ] Book Collection card shows the screenshot thumbnail
- [ ] Title and description appear below the thumbnail
- [ ] "View template →" link is orange/krill color
- [ ] Hovering a card lifts it with a shadow
- [ ] Clicking the card navigates to `/templates/book-collection/`
- [ ] Grid goes to 1 column on a narrow window (drag browser narrow to test)

**Step 4: Commit**

```bash
git add static/css/style.css
git commit -m "feat: add template grid and card CSS styles"
```

---

### Task 4: Final check — individual template page still looks correct

The `content/templates/book-collection.md` page is rendered by `layouts/_default/single.html`, which is unchanged. The screenshot is embedded inline via the markdown image tag added in a previous session.

**Step 1: Navigate to the template detail page**

Go to `http://localhost:1313/templates/book-collection/`.

Checklist:
- [ ] Screenshot renders inline in the page body (between intro paragraph and Downloads section)
- [ ] Screenshot has rounded corners and a shadow (from `.docs-content img` styles)
- [ ] Downloads links still work: `.rhai` and `.zip` files are served

**Step 2: No commit needed** — this task is verification only.
