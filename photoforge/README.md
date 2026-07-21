# photoforge (developer guide)

The workspace behind [PhotoForge](../README.md): a desktop photo organizer
built with **Tauri 2.x** (Rust backend) and a plain HTML/CSS/JS frontend — no
JS framework, no bundler. Targets **Windows**.

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

## Features

- **Indexing** — parallel, resumable scan of jpg/jpeg/png/heic/webp/bmp/gif:
  EXIF (dates, camera, GPS, orientation), Google Takeout JSON sidecars,
  dimensions, best-date resolution (EXIF > sidecar > filename > mtime).
- **Exact dedupe** — streamed BLAKE3 content hashes; groups sorted by
  reclaimable bytes.
- **Near dedupe** — 64-bit perceptual dHash + BK-tree Hamming search with
  union-find grouping (naive baseline kept for benchmarking).
- **Organize by date** — dry-run plan, `YYYY/MM` moves/copies, collision-safe
  renaming, every move undo-logged. Nothing is ever deleted.
- **Classifier** — rules-based photo vs. screenshot/asset labeling with an
  auditable "rule fired" per file, plus a keyboard-driven review screen
  (Y/N/arrows) whose corrections export as CSV training data.
- **Scan history & skip lists** — every indexed folder is remembered with its
  last-scan stats; built-in system skips (Windows, Program Files, Recycle Bin,
  `node_modules`, `.git`, …) plus user-managed skip folders stored in the
  catalog.

## Develop

```bash
# Compile everything (backend + core):
cargo build

# Core tests / benchmarks:
cargo test -p photoforge-core
cargo bench -p photoforge-core

# CLI harness (no Tauri needed):
cargo run -p photoforge-core --bin pfcli -- index C:\Photos --db photoforge.db
cargo run -p photoforge-core --bin pfcli -- dupes
cargo run -p photoforge-core --bin pfcli -- near-dupes --k 5 --method bktree
cargo run -p photoforge-core --bin pfcli -- organize D:\Sorted          # dry run
cargo run -p photoforge-core --bin pfcli -- organize D:\Sorted --apply  # move + undo log
cargo run -p photoforge-core --bin pfcli -- classify
cargo run -p photoforge-core --bin pfcli -- undo --count 10
cargo run -p photoforge-core --bin pfcli -- status                      # indexed folders + skip lists
cargo run -p photoforge-core --bin pfcli -- skip add "D:\Backups"

# Run the desktop app (needs the Tauri CLI: cargo install tauri-cli --version '^2'):
cargo tauri dev

# Produce a Windows installer (run on / cross-compile to Windows):
cargo tauri build
```

The `photoforge-core` crate is deliberately Tauri-free so it can be unit-tested
headlessly (`cargo test -p photoforge-core`) and reused outside the app.
