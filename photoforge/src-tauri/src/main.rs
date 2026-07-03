// On Windows in release, don't spawn an extra console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;

/// Payload returned to the frontend describing an indexing run.
#[derive(Serialize)]
struct ScanSummary {
    root: String,
    photos: usize,
    errors: usize,
    core_version: String,
}

/// Scan a directory for photos and report a summary. Invoked from JS via
/// `window.__TAURI__.core.invoke('scan_dir', { root })`.
#[tauri::command]
fn scan_dir(root: String) -> ScanSummary {
    let results = photoforge_core::scan::scan(&root);
    let errors = results.iter().filter(|r| r.is_err()).count();
    ScanSummary {
        root,
        photos: results.len() - errors,
        errors,
        core_version: photoforge_core::version().to_string(),
    }
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
