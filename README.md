# PhotoForge

A **local-first photo organizer for Windows**. PhotoForge indexes your photo
folders, finds exact and near duplicates, sorts photos into date folders, and
separates real photos from screenshots and app assets — entirely on your PC.
No cloud, no account, no telemetry, no network calls.

Built in **Rust** with a [Tauri 2](https://tauri.app) desktop shell.

## Highlights

- **Fast, resumable indexing** — a parallel (rayon) scanner extracts EXIF
  metadata, Google Takeout JSON sidecars, dimensions, and two content
  fingerprints per photo, batch-writing to SQLite. Re-scans skip unchanged
  files, so interrupting a scan costs nothing.
- **Exact duplicate detection** — streamed BLAKE3 content hashes; duplicate
  groups are ranked by how many bytes you'd reclaim.
- **Near-duplicate detection** — 64-bit perceptual dHash with Hamming-distance
  grouping, so recompressed, resized, or lightly edited copies are found too.
- **Organize by date** — moves (or copies) photos into `YYYY/MM` folders using
  the best available date: EXIF capture time, then Takeout metadata, then
  filename patterns (`IMG_20230115_…`, `PXL_…`, WhatsApp), then file mtime.
- **Photo vs. screenshot classifier** — explainable heuristic rules (camera
  EXIF, screenshot filenames, screen-exact resolutions, asset paths), plus a
  keyboard-driven review screen (Y confirm / N flip) for the uncertain cases.
- **Nothing is ever deleted.** Every file operation is a move, recorded in an
  undo log *before* it happens, with a dry-run mode. One click restores.
- **Skip lists** — system folders (Windows, Program Files, the Recycle Bin,
  `node_modules`, …) are never scanned, and you can add your own skip folders.

## Install & run

Prerequisites: [Rust](https://rustup.rs), the Tauri CLI
(`cargo install tauri-cli --version '^2'`), and Windows 10/11 (the app uses
the built-in WebView2 runtime).

```bash
cd photoforge

# Run in development:
cargo tauri dev

# Build a Windows installer (NSIS + MSI, in target/release/bundle/):
cargo tauri build
```

First launch shows a short getting-started guide. The catalog database lives
in `%APPDATA%` — your photos are never modified unless you explicitly use
Organize or the duplicate mover, and those are always undoable.

There is also a full-featured CLI (`pfcli`) that drives the same engine
without the GUI — see [photoforge/README.md](photoforge/README.md).

## How it works

| Problem | Approach |
|---|---|
| Byte-identical duplicates | BLAKE3 content hash, streamed in 1 MB chunks, `GROUP BY` in SQLite |
| Visually identical duplicates | 9x8 gradient dHash (EXIF-orientation-aware) + Hamming distance ≤ k, grouped transitively with union-find |
| "When was this taken?" | Precedence resolver: EXIF `DateTimeOriginal` → Takeout `photoTakenTime` → filename patterns → file mtime, with the winning source recorded |
| Photo or screenshot? | Ordered heuristic rules, each decision stored with the exact rule that fired; human corrections are sacred and exportable as training data |
| Don't lose my files | Log-then-move: every move is written to an undo table before the filesystem is touched; cross-drive moves copy first, remove only after success |

An honest engineering note: the codebase contains both a naive all-pairs
Hamming search and a BK-tree, benchmarked against each other with criterion.
**The naive scan wins** at any realistic library size (1.0 s vs 6.6 s at
50,000 hashes, k=5) — pairwise distances between unrelated 64-bit hashes
concentrate around 32, which starves the BK-tree's triangle-inequality
pruning, while the naive loop is a branch-free XOR+POPCNT sweep. The tree
stays in the codebase as the measured comparison; the naive scan is the
shipped default.

## Architecture

```
photoforge/
├── crates/photoforge-core/   # All the logic. Pure Rust, zero Tauri deps.
│   ├── src/                  #   scan, indexer, db, exif, sidecar, date,
│   │                         #   hash (BLAKE3), dhash, dedupe (BK-tree),
│   │                         #   organize (undo log), classify, thumbnail
│   ├── src/bin/pfcli.rs      #   CLI harness for everything above
│   └── benches/              #   criterion benchmarks (dhash, near-dup)
├── src-tauri/                # Thin Tauri shell: commands + progress events
└── frontend/                 # Plain HTML/CSS/JS (no framework, no bundler)
```

The split is deliberate: `photoforge-core` is a plain Rust library that can be
tested headlessly (`cargo test -p photoforge-core`) and reused outside the
app; the Tauri layer only forwards calls and streams progress events. The GUI
is intentionally plain HTML/CSS/JS rendered by the OS WebView — no Node, no
bundler, no framework — which keeps the installer small and the frontend
auditable at a glance.

## Roadmap

- ONNX-based screenshot classification for ambiguous files (offline, via the
  `ort` crate — see `photoforge/models/README.md`)
- Face detection + clustering (SCRFD / ArcFace, also offline ONNX)

## License

[MIT](LICENSE)
