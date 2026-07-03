// On Windows in release, don't spawn an extra console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;

/// Payload returned to the frontend describing an indexing run.
#[derive(Serialize)]
struct ScanSummary {
    root: String,
    scanned: usize,
    added: usize,
    skipped: usize,
    errors: usize,
    core_version: String,
}

/// Index a directory of photos and report stats. Invoked from JS via
/// `window.__TAURI__.core.invoke('scan_dir', { root })`.
///
/// This uses a throwaway in-memory catalog so the command is side-effect free;
/// the persistent catalog will be wired up in a later phase.
#[tauri::command]
fn scan_dir(root: String) -> Result<ScanSummary, String> {
    let mut conn = photoforge_core::db::open_in_memory().map_err(|e| e.to_string())?;
    let stats = photoforge_core::index_directory(&mut conn, &root).map_err(|e| e.to_string())?;
    Ok(ScanSummary {
        root,
        scanned: stats.scanned,
        added: stats.added,
        skipped: stats.skipped,
        errors: stats.errors,
        core_version: photoforge_core::version().to_string(),
    })
}

/// Simple greeting used by the starter UI to prove the JS <-> Rust bridge works.
#[tauri::command]
fn greet(name: &str) -> String {
    format!(
        "Hello, {name}! photoforge-core v{} is wired up.",
        photoforge_core::version()
    )
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet, scan_dir])
        .run(tauri::generate_context!())
        .expect("error while running the photoforge application");
}
