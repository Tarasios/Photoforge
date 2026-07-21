//! The SQLite catalog: schema, connection setup, and batched writes.
//!
//! Uses `rusqlite` with bundled SQLite (no system dependency). Writes go
//! through [`insert_files`], which commits one transaction per 1000 rows so a
//! crash mid-scan leaves a consistent, resumable database behind.

use crate::indexer::IndexedFile;
use crate::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;

/// Full catalog schema, loaded from `schema.sql` — the single source of truth.
// `include_str!` embeds the file's text into the binary at compile time, so
// there is no runtime file lookup (think Java resource, but resolved by rustc).
const SCHEMA: &str = include_str!("schema.sql");

/// Number of rows written per transaction.
const BATCH_SIZE: usize = 1000;

/// Open (creating if needed) the catalog at `path`, tune it for bulk writes,
/// and ensure the schema exists.
pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Open an in-memory catalog, primarily useful for tests and one-shot scans.
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Load `path -> (size, mtime)` for every row already in `files`. The indexer
/// uses this to skip files whose path+size+mtime are unchanged since last run.
pub fn load_index_state(conn: &Connection) -> Result<HashMap<String, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT path, size, mtime FROM files")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, sm) = row?;
        map.insert(path, sm);
    }
    Ok(map)
}

/// A folder the user has indexed, with the stats of its most recent scan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanRoot {
    pub path: String,
    pub last_scan_ts: i64,
    pub scanned: i64,
    pub added: i64,
    pub skipped: i64,
    pub errors: i64,
}

/// Upsert the record of a completed scan of `root`.
pub fn record_scan_root(
    conn: &Connection,
    root: &str,
    ts: i64,
    stats: &crate::indexer::IndexStats,
) -> Result<()> {
    conn.execute(
        "INSERT INTO scan_roots (path, last_scan_ts, scanned, added, skipped, errors)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(path) DO UPDATE SET
           last_scan_ts = excluded.last_scan_ts,
           scanned = excluded.scanned, added = excluded.added,
           skipped = excluded.skipped, errors = excluded.errors",
        params![
            root,
            ts,
            stats.scanned as i64,
            stats.added as i64,
            stats.skipped as i64,
            stats.errors as i64
        ],
    )?;
    Ok(())
}

/// Every folder ever indexed, most recently scanned first.
pub fn list_scan_roots(conn: &Connection) -> Result<Vec<ScanRoot>> {
    let mut stmt = conn.prepare(
        "SELECT path, last_scan_ts, scanned, added, skipped, errors
         FROM scan_roots ORDER BY last_scan_ts DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ScanRoot {
            path: r.get(0)?,
            last_scan_ts: r.get(1)?,
            scanned: r.get(2)?,
            added: r.get(3)?,
            skipped: r.get(4)?,
            errors: r.get(5)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// User-defined skip folders (absolute paths the scanner never enters).
pub fn list_skip_dirs(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM skip_dirs ORDER BY path")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// Add a folder to the user skip list (no-op if already present).
pub fn add_skip_dir(conn: &Connection, path: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO skip_dirs (path) VALUES (?1)",
        params![path],
    )?;
    Ok(())
}

/// Remove a folder from the user skip list.
pub fn remove_skip_dir(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM skip_dirs WHERE path = ?1", params![path])?;
    Ok(())
}

const INSERT_SQL: &str = "
INSERT INTO files (
  path, size, mtime, width, height,
  exif_datetime, exif_create_date, exif_make, exif_model,
  gps_lat, gps_lon, orientation,
  sidecar_taken_time, sidecar_lat, sidecar_lon,
  resolved_date, date_source
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
ON CONFLICT(path) DO UPDATE SET
  size = excluded.size,
  mtime = excluded.mtime,
  width = excluded.width,
  height = excluded.height,
  exif_datetime = excluded.exif_datetime,
  exif_create_date = excluded.exif_create_date,
  exif_make = excluded.exif_make,
  exif_model = excluded.exif_model,
  gps_lat = excluded.gps_lat,
  gps_lon = excluded.gps_lon,
  orientation = excluded.orientation,
  sidecar_taken_time = excluded.sidecar_taken_time,
  sidecar_lat = excluded.sidecar_lat,
  sidecar_lon = excluded.sidecar_lon,
  resolved_date = excluded.resolved_date,
  date_source = excluded.date_source
";

const INSERT_HASHES_SQL: &str = "
INSERT INTO hashes (file_id, blake3, dhash)
VALUES ((SELECT id FROM files WHERE path = ?1), ?2, ?3)
ON CONFLICT(file_id) DO UPDATE SET
  blake3 = excluded.blake3,
  dhash = excluded.dhash
";

/// Insert/upsert `files` (and their hashes) in batches, one transaction per
/// [`BATCH_SIZE`] rows. Returns the number of file rows written.
pub fn insert_files(conn: &mut Connection, files: &[IndexedFile]) -> Result<usize> {
    let mut written = 0;
    for chunk in files.chunks(BATCH_SIZE) {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(INSERT_SQL)?;
            let mut hash_stmt = tx.prepare_cached(INSERT_HASHES_SQL)?;
            for f in chunk {
                stmt.execute(params![
                    f.path,
                    f.size,
                    f.mtime,
                    f.width,
                    f.height,
                    f.exif_datetime,
                    f.exif_create_date,
                    f.exif_make,
                    f.exif_model,
                    f.gps_lat,
                    f.gps_lon,
                    f.orientation,
                    f.sidecar_taken_time,
                    f.sidecar_lat,
                    f.sidecar_lon,
                    f.resolved_date,
                    f.date_source,
                ])?;
                if f.blake3.is_some() || f.dhash.is_some() {
                    hash_stmt.execute(params![f.path, f.blake3, f.dhash])?;
                }
                written += 1;
            }
        }
        tx.commit()?;
    }
    Ok(written)
}
