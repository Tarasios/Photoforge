# CLAUDE.md

## Project
PhotoForge: local-first photo library tool. Rust + Tauri 2.x, Windows only. Indexes photos, finds exact/near duplicates, classifies photos vs screenshots, clusters faces. Everything runs offline; no cloud, no telemetry, no network calls.

## Owner context
I am a Java developer learning Rust with this project. This is a portfolio piece: I must be able to explain every algorithm to an interviewer. Prefer teaching over doing when I ask questions. When you introduce a Rust idiom I likely haven't seen (lifetimes, trait objects, interior mutability), add a one-line comment explaining it.

## Layout
- `photoforge-core/` — library crate: indexing, hashing, dedupe, classification, faces. NO Tauri dependencies ever.
- `photoforge-app/` (or `src-tauri/`) — Tauri shell. Thin glue only: commands call core functions and forward events. No logic here.
- Frontend: plain HTML/CSS/JS, no framework, dark theme.
- Schema lives in `photoforge-core/src/schema.sql` — it is the single source of truth; migrations go in `migrations/`.

## Commands
- Build: `cargo build` (workspace root)
- Test: `cargo test -p photoforge-core`
- Bench: `cargo bench -p photoforge-core`
- Run CLI test harness: `cargo run -p photoforge-core --bin pfcli -- <args>`
- Run app: `cargo tauri dev`

## Hard rules
1. NEVER hard-delete user files. All destructive-looking operations are moves, recorded in the undo_log table first, then executed. Every file-moving feature ships with a dry-run mode.
2. All heavy work (scanning, hashing, inference) goes in photoforge-core behind functions returning `Result<T, PfError>` (thiserror). No `unwrap()`/`expect()` outside tests and `main`.
3. Parallelism via rayon in core. The Tauri layer stays single-threaded glue; long tasks report progress via Tauri events, never block the UI thread.
4. SQLite writes are batched in transactions (~1000 rows). Indexing must stay resumable: skip files matching path+size+mtime.
5. No Python anywhere in this repo. ML runs via ONNX (ort crate). Model files live in `models/` with a README noting source and license.
6. No new dependencies without telling me why and what the alternatives were. Keep the dependency tree small; this project's brand is lean and fast.
7. Performance changes require numbers: benchmark before and after (criterion or the pfcli stopwatch mode) and state the delta. One optimization per commit.
8. Windows is the only target. Watch path handling (long paths, backslashes, reserved names) and note any DLLs the bundle must ship (ONNX Runtime).

## Learning mode
If my prompt says [YOU FIRST] or asks for review: review only — numbered issues, no rewrites, let me fix it.
If I ask you to build something outright: build it, then append a 3-sentence summary to NOTES.md (what it does, key design decision, one likely interviewer question).

## Style
- rustfmt defaults, clippy clean (`cargo clippy -- -D warnings`) before declaring anything done.
- Small functions, doc comments on public core APIs.
- Commit messages: imperative, one feature per commit, include benchmark deltas where relevant.
