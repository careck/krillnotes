# Website v0.2.0 Update Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Update the Krillnotes homepage to reflect v0.2.0's encryption features.

**Architecture:** Four targeted string replacements in a single Hugo template file. No new files, no structural changes.

**Tech Stack:** Hugo static site, HTML templates.

---

### Task 1: Update hero description

**Files:**
- Modify: `layouts/index.html:21`

**Step 1: Make the edit**

In `layouts/index.html`, replace:

```html
      A local-first, hierarchical note-taking app built with Rust.
      Define your own note types with scripts, organize in infinite trees,
      and keep everything on your device.
```

with:

```html
      A local-first, hierarchical note-taking app built with Rust.
      Define your own note types with scripts, organize in infinite trees,
      and keep everything encrypted on your device.
```

**Step 2: Commit**

```bash
git add layouts/index.html
git commit -m "feat: mention encryption in hero description"
```

---

### Task 2: Update "Local-first" feature card → "Encrypted at rest"

**Files:**
- Modify: `layouts/index.html:78-82`

**Step 1: Make the edit**

Replace the entire card div:

```html
    <div class="feature-card">
      <div class="feature-icon">🔒</div>
      <h3>Local-first</h3>
      <p>All data lives in a single .krillnotes file on disk. No account, no cloud dependency, no internet connection required.</p>
    </div>
```

with:

```html
    <div class="feature-card">
      <div class="feature-icon">🔒</div>
      <h3>Encrypted at rest</h3>
      <p>All workspaces are AES-256 encrypted via SQLCipher. A password is required to create or open each workspace. Passwords can optionally be cached for the duration of a session.</p>
    </div>
```

**Step 2: Commit**

```bash
git add layouts/index.html
git commit -m "feat: update Local-first card to Encrypted at rest"
```

---

### Task 3: Update "Export & Import" feature card

**Files:**
- Modify: `layouts/index.html:73-77`

**Step 1: Make the edit**

Replace the card paragraph:

```html
      <p>Export an entire workspace as a .zip archive. Import into a new workspace with version-compatibility checks.</p>
```

with:

```html
      <p>Export an entire workspace as a .zip archive, optionally password-protected with AES-256 encryption. Encrypted archives are automatically detected on import.</p>
```

**Step 2: Commit**

```bash
git add layouts/index.html
git commit -m "feat: mention encrypted exports in Export & Import card"
```

---

### Task 4: Add SQLCipher to tech stack

**Files:**
- Modify: `layouts/index.html:106`

**Step 1: Make the edit**

Replace:

```html
    <span class="tech-pill"><span class="tech-pill-icon">🗄️</span> SQLite</span>
    <span class="tech-pill"><span class="tech-pill-icon">📜</span> Rhai scripting</span>
```

with:

```html
    <span class="tech-pill"><span class="tech-pill-icon">🗄️</span> SQLite</span>
    <span class="tech-pill"><span class="tech-pill-icon">🔐</span> SQLCipher</span>
    <span class="tech-pill"><span class="tech-pill-icon">📜</span> Rhai scripting</span>
```

**Step 2: Commit**

```bash
git add layouts/index.html
git commit -m "feat: add SQLCipher to tech stack"
```
