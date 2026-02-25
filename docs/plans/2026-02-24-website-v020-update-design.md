# Website Update for v0.2.0 — Design

## Summary

Update the Krillnotes marketing homepage to reflect the v0.2.0 release, with emphasis on the new encryption features. Scope: four targeted edits to `layouts/index.html` in the website repo. No new sections or structural changes.

## Changes

### 1. Hero description

Add "encrypted" to reinforce the security-first positioning:

> A local-first, hierarchical note-taking app built with Rust. Define your own note types with scripts, organize in infinite trees, and keep everything **encrypted** on your device.

### 2. "Local-first" feature card → "Encrypted at rest"

- Icon: 🔒 (unchanged)
- New title: **Encrypted at rest**
- New copy: _All workspaces are AES-256 encrypted via SQLCipher. A password is required to create or open each workspace. Passwords can optionally be cached for the duration of a session._

### 3. "Export & Import" feature card

- Icon: 📦 (unchanged)
- Title unchanged
- New copy: _Export an entire workspace as a .zip archive, optionally password-protected with AES-256 encryption. Encrypted archives are automatically detected on import._

### 4. Tech stack pills

Add a SQLCipher pill after the SQLite pill:

> 🦀 Rust · ⚛️ React · 🪟 Tauri v2 · 🗄️ SQLite · 🔐 SQLCipher · 📜 Rhai scripting

## File changed

`krillnotes-website/layouts/index.html`
