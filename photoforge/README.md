# photoforge

A desktop photo organizer built with **Tauri 2.x** (Rust backend) and a plain
HTML/CSS/JS frontend — no JS framework, no bundler. Targets **Windows**.

## Layout

```
photoforge/
├── Cargo.toml                 # Cargo workspace (virtual manifest)
├── crates/
│   └── photoforge-core/       # Library crate — all the real logic, no Tauri deps
│       └── src/               # scan, hash, exif, thumbnail, db, error
├── src-tauri/                 # Tauri application crate (depends on photoforge-core)
│   ├── src/main.rs            # #[tauri::command]s + Builder
│   ├── tauri.conf.json        # Tauri config (frontendDist -> ../frontend)
│   ├── capabilities/          # Tauri v2 permission capabilities
│   └── icons/                 # App icons (png + ico)
└── frontend/                  # Static UI served to the webview
    ├── index.html
    ├── styles.css
    └── main.js
```

## Develop

```bash
# Compile everything (backend + core):
cargo build

# Run the desktop app (needs the Tauri CLI: cargo install tauri-cli --version '^2'):
cargo tauri dev

# Produce a Windows installer (run on / cross-compile to Windows):
cargo tauri build
```

The `photoforge-core` crate is deliberately Tauri-free so it can be unit-tested
headlessly (`cargo test -p photoforge-core`) and reused outside the app.
