# krillnotes-desktop

The Tauri v2 + React frontend for Krillnotes.

See the root [README.md](../README.md) for build instructions and the [DEVELOPER.md](../DEVELOPER.md) for architecture details.

## Development

```bash
# Install Node dependencies (run once)
npm install

# Start dev server with hot reload
npm run tauri dev

# Build release binary
npm run tauri build
```

## Structure

```
krillnotes-desktop/
├── src-tauri/          # Rust backend (Tauri commands, AppState, menu)
└── src/                # React frontend (components, types, styles)
```
