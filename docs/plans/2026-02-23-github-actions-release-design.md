# GitHub Actions Release Automation — Design

**Date:** 2026-02-23
**Status:** Approved

## Summary

Automate cross-platform Tauri builds and GitHub Release creation via a single GitHub Actions workflow triggered by pushing a version tag.

## Trigger

Any tag matching `v*` (e.g. `v0.1.0`) pushed to the repo fires the workflow:

```
git tag v0.1.0
git push origin v0.1.0
```

## Approach

Use the official `tauri-apps/tauri-action@v0` action with a three-runner matrix. Each runner builds its native artifact, and the action creates a draft GitHub Release automatically, attaching all artifacts.

No code signing or notarization — unsigned builds for now.

## Architecture

### Workflow File

`.github/workflows/release.yml`

### Build Matrix

| Runner | Platform | Artifacts |
|--------|----------|-----------|
| `ubuntu-22.04` | Linux | `.deb`, `.AppImage` |
| `windows-latest` | Windows | `.exe` (NSIS), `.msi` |
| `macos-latest` | macOS | `.dmg`, `.app.tar.gz` |

### Steps Per Runner

1. `actions/checkout@v4`
2. `actions/setup-node@v4` — Node.js LTS (for `npm run build` in `krillnotes-desktop/`)
3. `dtolnay/rust-toolchain@stable` — Rust stable
4. Install Linux system dependencies (Ubuntu only):
   `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`
5. `tauri-apps/tauri-action@v0`:
   - `projectPath: krillnotes-desktop`
   - `tagName: ${{ github.ref_name }}`
   - `releaseName: Krillnotes ${{ github.ref_name }}`
   - `releaseDraft: true`
   - `prerelease: false`

## Release Behavior

The action creates a **draft** GitHub Release named after the tag. All three platform jobs upload their artifacts to the same release. After the workflow completes, you review the draft in the GitHub Releases UI and click "Publish" when ready.

## Artifacts Produced

| Platform | Files |
|----------|-------|
| macOS    | `krillnotes-desktop_x.y.z_x64.dmg`, `.app.tar.gz` |
| Windows  | `krillnotes-desktop_x.y.z_x64-setup.exe`, `.msi` |
| Linux    | `krillnotes-desktop_x.y.z_amd64.deb`, `.AppImage` |

## Future Considerations

- macOS notarization: add Apple Developer secrets + `APPLE_CERTIFICATE`, `APPLE_ID`, etc.
- Windows code signing: add Authenticode certificate secret
- ARM builds: add `macos-14` (Apple Silicon) and `ubuntu-22.04-arm` runners to the matrix
