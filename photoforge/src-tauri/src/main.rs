// On Windows in release, don't spawn an extra console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell — thin glue only. Every command defers to photoforge-core and
//! forwards results/progress to the webview; no domain logic lives here.

use photoforge_core::dedupe::{DupeGroup, NearMethod};
use photoforge_core::organize::MoveStats;
use photoforge_core::{classify, dedupe, organize};
use rusqlite::Connection;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Shared handle to the persistent catalog. `Arc<Mutex<..>>` because commands
/// run concurrently on the async runtime but rusqlite connections are not
/// thread-safe to share — the mutex serializes access (like Java's
/// `synchronized`, but enforced by the type system: the connection is
/// unreachable without locking).
struct Catalog(Arc<Mutex<Connection>>);

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Payload returned to the frontend describing an indexing run.
#[derive(Serialize, Clone)]
struct ScanSummary {
    root: String,
    scanned: usize,
    added: usize,
    skipped: usize,
    errors: usize,
}

#[derive(Serialize, Clone)]
struct ScanProgress {
    done: usize,
    total: usize,
}

/// Native folder picker (tauri-plugin-dialog). Returns `None` on cancel.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> CmdResult<Option<String>> {
    // Blocking dialog on a worker thread; async commands don't run on the
    // main/UI thread, but spawn_blocking makes the intent explicit.
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().blocking_pick_folder()
    })
    .await
    .map_err(err_str)?;
    Ok(picked.map(|p| p.to_string()))
}

/// Index a directory into the persistent catalog, streaming progress to the
/// webview as `scan-progress` events.
#[tauri::command]
async fn scan_dir(app: AppHandle, state: State<'_, Catalog>, root: String) -> CmdResult<ScanSummary> {
    let conn = state.0.clone();
    let root_clone = root.clone();
    let stats = tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        photoforge_core::index_directory_with_progress(&mut conn, &root_clone, |done, total| {
            // Throttle: every 25 files plus the final tick.
            if done % 25 == 0 || done == total {
                let _ = app.emit("scan-progress", ScanProgress { done, total });
            }
        })
        .map_err(err_str)
    })
    .await
    .map_err(err_str)??;

    Ok(ScanSummary {
        root,
        scanned: stats.scanned,
        added: stats.added,
        skipped: stats.skipped,
        errors: stats.errors,
    })
}

#[tauri::command]
async fn get_exact_dupes(state: State<'_, Catalog>) -> CmdResult<Vec<DupeGroup>> {
    let conn = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        dedupe::find_exact_duplicates(&conn).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

#[tauri::command]
async fn get_near_dupes(state: State<'_, Catalog>, k: u32) -> CmdResult<Vec<DupeGroup>> {
    let conn = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        // Naive beats the BK-tree on 64-bit dHashes at realistic library
        // sizes (see the benchmark notes in dedupe.rs).
        dedupe::find_near_duplicates(&conn, k, NearMethod::Naive).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

/// JPEG thumbnail as a `data:` URL (base64), sized for the review grids.
#[tauri::command]
async fn get_thumbnail(path: String, max_edge: u32) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = photoforge_core::thumbnail::thumbnail(&path, max_edge.clamp(64, 1024))
            .map_err(err_str)?;
        Ok(format!("data:image/jpeg;base64,{}", base64(&bytes)))
    })
    .await
    .map_err(err_str)?
}

/// Move the selected files into `<dest_dir>` (undo-logged, collision-safe).
#[tauri::command]
async fn move_to_duplicates(
    state: State<'_, Catalog>,
    paths: Vec<String>,
    dest_dir: String,
) -> CmdResult<MoveStats> {
    let conn = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        organize::move_files_to(&mut conn, &paths, &dest_dir).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

#[tauri::command]
async fn undo_moves(state: State<'_, Catalog>, count: usize) -> CmdResult<MoveStats> {
    let conn = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        organize::undo_last(&mut conn, count).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

#[tauri::command]
async fn run_classifier(state: State<'_, Catalog>) -> CmdResult<classify::ClassifyStats> {
    let conn = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| "catalog lock poisoned".to_string())?;
        classify::classify_all(&mut conn).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

#[derive(Serialize, Clone)]
struct ReviewItem {
    id: i64,
    path: String,
    label: Option<String>,
    rule: Option<String>,
    confidence: Option<f64>,
}

/// Classified-but-unconfirmed files for the keyboard review screen, least
/// confident first (those benefit most from human eyes).
#[tauri::command]
fn get_review_queue(state: State<'_, Catalog>, limit: usize) -> CmdResult<Vec<ReviewItem>> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, path, classification, class_rule, class_confidence
             FROM files
             WHERE classification IS NOT NULL AND class_rule != 'manual'
             ORDER BY class_confidence ASC, id
             LIMIT ?1",
        )
        .map_err(err_str)?;
    let rows = stmt
        .query_map([limit as i64], |r| {
            Ok(ReviewItem {
                id: r.get(0)?,
                path: r.get(1)?,
                label: r.get(2)?,
                rule: r.get(3)?,
                confidence: r.get(4)?,
            })
        })
        .map_err(err_str)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err_str)
}

/// Record a human decision from the review screen.
#[tauri::command]
fn set_label(state: State<'_, Catalog>, id: i64, label: String) -> CmdResult<()> {
    if label != classify::PHOTO && label != classify::NON_PHOTO {
        return Err(format!("unknown label: {label}"));
    }
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    conn.execute(
        "UPDATE files SET classification = ?1, class_confidence = 1.0, class_rule = 'manual'
         WHERE id = ?2",
        rusqlite::params![label, id],
    )
    .map_err(err_str)?;
    Ok(())
}

/// Export human-confirmed labels as CSV training data (`path,label`).
#[tauri::command]
fn export_labels_csv(state: State<'_, Catalog>, dest: String) -> CmdResult<usize> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    let mut stmt = conn
        .prepare("SELECT path, classification FROM files WHERE class_rule = 'manual'")
        .map_err(err_str)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err_str)?;
    let mut out = String::new();
    let mut n = 0;
    for row in rows {
        let (path, label) = row.map_err(err_str)?;
        out.push_str(&format!("{path},{label}\n"));
        n += 1;
    }
    std::fs::write(&dest, out).map_err(err_str)?;
    Ok(n)
}

#[derive(Serialize, Clone)]
struct SkipLists {
    builtin: Vec<String>,
    user: Vec<String>,
}

/// Folders indexed so far, most recent first.
#[tauri::command]
fn get_scan_roots(state: State<'_, Catalog>) -> CmdResult<Vec<photoforge_core::db::ScanRoot>> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    photoforge_core::db::list_scan_roots(&conn).map_err(err_str)
}

/// Built-in system skips plus the user's own skip folders.
#[tauri::command]
fn get_skip_dirs(state: State<'_, Catalog>) -> CmdResult<SkipLists> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    Ok(SkipLists {
        builtin: photoforge_core::scan::BUILTIN_SKIP_DIRS
            .iter()
            .map(|s| s.to_string())
            .collect(),
        user: photoforge_core::db::list_skip_dirs(&conn).map_err(err_str)?,
    })
}

#[tauri::command]
fn add_skip_dir(state: State<'_, Catalog>, path: String) -> CmdResult<()> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    photoforge_core::db::add_skip_dir(&conn, &path).map_err(err_str)
}

#[tauri::command]
fn remove_skip_dir(state: State<'_, Catalog>, path: String) -> CmdResult<()> {
    let conn = state.0.lock().map_err(|_| "catalog lock poisoned".to_string())?;
    photoforge_core::db::remove_skip_dir(&conn, &path).map_err(err_str)
}

/// Minimal standard base64 encoder — 20 lines beats pulling in a crate for
/// one call site (project rule: keep the dependency tree small).
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Persistent catalog lives in the per-user app data dir.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = photoforge_core::db::open(dir.join("photoforge.db"))?;
            app.manage(Catalog(Arc::new(Mutex::new(conn))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pick_folder,
            scan_dir,
            get_exact_dupes,
            get_near_dupes,
            get_thumbnail,
            move_to_duplicates,
            undo_moves,
            run_classifier,
            get_review_queue,
            set_label,
            export_labels_csv,
            get_scan_roots,
            get_skip_dirs,
            add_skip_dir,
            remove_skip_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the photoforge application");
}
