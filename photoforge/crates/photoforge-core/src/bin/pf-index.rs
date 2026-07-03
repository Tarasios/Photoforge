//! Test/dev CLI for the indexer.
//!
//! Usage:
//!     pf-index <directory> [db_path]
//!
//! Walks `<directory>`, indexes every image into `[db_path]` (default
//! `photoforge.db`), and prints run stats. Run it twice against the same tree
//! to watch the resumable skip logic in action.

use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: pf-index <directory> [db_path]");
        return ExitCode::from(2);
    };
    let db_path = args.next().unwrap_or_else(|| "photoforge.db".to_string());

    let mut conn = match photoforge_core::db::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to open catalog {db_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let started = Instant::now();
    match photoforge_core::index_directory(&mut conn, &root) {
        Ok(stats) => {
            println!(
                "indexed {root} -> {db_path}\n  scanned {} | added {} | skipped {} | errors {}  ({:.2?})",
                stats.scanned, stats.added, stats.skipped, stats.errors, started.elapsed()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("indexing failed: {e}");
            ExitCode::FAILURE
        }
    }
}
