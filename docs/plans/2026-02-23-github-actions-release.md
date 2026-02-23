# GitHub Actions Release Automation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a single GitHub Actions workflow that builds cross-platform Tauri artifacts and creates a draft GitHub Release whenever a `v*` tag is pushed.

**Architecture:** One workflow file at `.github/workflows/release.yml` using a 3-runner matrix (Ubuntu, Windows, macOS). Each runner installs its own toolchain, builds the Tauri app in `krillnotes-desktop/`, and uploads artifacts via `tauri-apps/tauri-action@v0` which creates/updates a single draft GitHub Release.

**Tech Stack:** GitHub Actions, `tauri-apps/tauri-action@v0`, `actions/checkout@v4`, `actions/setup-node@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`

---

### Task 1: Create the git worktree

**Files:**
- No files changed — worktree scaffolding only

**Step 1: Create feature branch + worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/github-actions-release -b feat/github-actions-release
```

Expected output: `Preparing worktree (new branch 'feat/github-actions-release')`

**Step 2: Verify the worktree exists**

```bash
ls /Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release
```

Expected: the project files are present (Cargo.toml, krillnotes-desktop/, etc.)

---

### Task 2: Create the GitHub Actions workflow file

**Files:**
- Create: `.github/workflows/release.yml` (inside the worktree)

All remaining work happens inside `/Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release/`.

**Step 1: Create the workflows directory**

```bash
mkdir -p /Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release/.github/workflows
```

**Step 2: Write the workflow file**

Create `.github/workflows/release.yml` with this exact content:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  release:
    permissions:
      contents: write
    strategy:
      fail-fast: false
      matrix:
        platform: [ubuntu-22.04, windows-latest, macos-latest]

    runs-on: ${{ matrix.platform }}

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: lts/*

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Cache Rust dependencies
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: ". -> target"

      - name: Install Linux system dependencies
        if: matrix.platform == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libappindicator3-dev \
            librsvg2-dev \
            patchelf

      - name: Install frontend dependencies
        working-directory: krillnotes-desktop
        run: npm install

      - name: Build and publish release
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          projectPath: krillnotes-desktop
          tagName: ${{ github.ref_name }}
          releaseName: "Krillnotes ${{ github.ref_name }}"
          releaseBody: |
            See the assets below to download this version and install it on your platform.
          releaseDraft: true
          prerelease: false
```

**Step 3: Verify the file exists and looks correct**

Read `.github/workflows/release.yml` and confirm the content matches the above.

---

### Task 3: Commit

**Step 1: Stage and commit the workflow file**

```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release add .github/workflows/release.yml
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release commit -m "$(cat <<'EOF'
feat(ci): add cross-platform release workflow

Triggers on v* tag push. Builds macOS, Windows, and Linux artifacts
using tauri-apps/tauri-action and creates a draft GitHub Release.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

Expected: `[feat/github-actions-release <hash>] feat(ci): add cross-platform release workflow`

---

### Task 4: Merge and test

**Step 1: Merge into main (or push branch for PR)**

Option A — merge directly:
```bash
git -C /Users/careck/Source/Krillnotes checkout main
git -C /Users/careck/Source/Krillnotes merge feat/github-actions-release
git -C /Users/careck/Source/Krillnotes push origin main
```

Option B — open a PR via GitHub CLI:
```bash
git -C /Users/careck/Source/Krillnotes/.worktrees/feat/github-actions-release push -u origin feat/github-actions-release
gh pr create --title "feat(ci): cross-platform release workflow" --body "Adds GitHub Actions release workflow triggered by v* tags."
```

**Step 2: Create and push a test tag**

Once the workflow file is on `main` (or default branch), test it:

```bash
git -C /Users/careck/Source/Krillnotes tag v0.1.0
git -C /Users/careck/Source/Krillnotes push origin v0.1.0
```

**Step 3: Watch the workflow run**

```bash
gh run watch
```

Or open `https://github.com/<owner>/Krillnotes/actions` in a browser.

Expected: three parallel jobs appear (ubuntu, windows, macos). Each takes ~10-20 minutes. On success, a draft release named "Krillnotes v0.1.0" appears in the Releases tab.

**Step 4: Verify the draft release**

```bash
gh release view v0.1.0
```

Expected: draft release with `.dmg`, `.exe`/`.msi`, `.deb`/`.AppImage` assets attached.

**Step 5: Publish the release (when ready)**

In the GitHub UI: Releases → v0.1.0 → Edit → "Publish release".

Or via CLI:
```bash
gh release edit v0.1.0 --draft=false
```

---

## Notes

- `fail-fast: false` means a Windows build failure won't cancel the macOS and Linux jobs.
- `GITHUB_TOKEN` is provided automatically by GitHub Actions — no additional secrets needed.
- `releaseDraft: true` means the release won't be publicly visible until you manually publish it.
- If the workflow fails on Linux with a missing library error, check that the `apt-get install` list matches the [Tauri 2.x Linux prerequisites](https://tauri.app/start/prerequisites/#linux).
- To add Apple Silicon builds later, add `macos-14` to the matrix. To add Linux ARM, add `ubuntu-22.04-arm`.
- Rust cache via `Swatinem/rust-cache@v2` reduces build times from ~15 min to ~5 min on warm cache.
