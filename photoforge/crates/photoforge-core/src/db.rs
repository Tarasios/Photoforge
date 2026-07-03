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

/// Full catalog schema.
///
/// This is the schema from the project spec, with two intentional deviations
/// (see the crate/PR notes):
///   * `files.exif_create_date` was added — the spec asks us to store both EXIF
///     `DateTimeOriginal` *and* `CreateDate`, but the original schema had only a
///     single `exif_datetime` column, which is structurally too small.
///   * `IF NOT EXISTS` was added throughout so opening an existing catalog is
///     idempotent (required for a resumable indexer).
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  width INTEGER, height INTEGER,
  exif_datetime TEXT, exif_create_date TEXT, exif_make TEXT, exif_model TEXT,
  gps_lat REAL, gps_lon REAL, orientation INTEGER,
  sidecar_taken_time TEXT, sidecar_lat REAL, sidecar_lon REAL,
  resolved_date TEXT, date_source TEXT,          -- 'exif'|'sidecar'|'filename'|'mtime'
  classification TEXT, class_confidence REAL,    -- Phase 2
  class_rule TEXT
);
CREATE TABLE IF NOT EXISTS hashes (
  file_id INTEGER PRIMARY KEY REFERENCES files(id),
  blake3 BLOB, dhash INTEGER                      -- dhash as i64 bit pattern
);
CREATE INDEX IF NOT EXISTS idx_hashes_blake3 ON hashes(blake3);
CREATE TABLE IF NOT EXISTS tags (id INTEGER PRIMARY KEY, name TEXT UNIQUE);
CREATE TABLE IF NOT EXISTS file_tags (file_id INTEGER REFERENCES files(id), tag_id INTEGER REFERENCES tags(id), PRIMARY KEY(file_id, tag_id));
CREATE TABLE IF NOT EXISTS people (id INTEGER PRIMARY KEY, name TEXT UNIQUE);
CREATE TABLE IF NOT EXISTS faces (
  id INTEGER PRIMARY KEY, file_id INTEGER REFERENCES files(id),
  bbox_x REAL, bbox_y REAL, bbox_w REAL, bbox_h REAL,
  embedding BLOB,                                 -- 512 x f32
  person_id INTEGER REFERENCES people(id),        -- NULL = unlabeled
  cluster_id INTEGER
);
CREATE TABLE IF NOT EXISTS undo_log (
  id INTEGER PRIMARY KEY, ts INTEGER,
  op TEXT, src_path TEXT, dst_path TEXT
);
";

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

/// Insert/upsert `files` in batches, one transaction per [`BATCH_SIZE`] rows.
/// Returns the number of rows written.
pub fn insert_files(conn: &mut Connection, files: &[IndexedFile]) -> Result<usize> {
    let mut written = 0;
    for chunk in files.chunks(BATCH_SIZE) {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(INSERT_SQL)?;
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
                written += 1;
            }
        }
        tx.commit()?;
    }
    Ok(written)
}
