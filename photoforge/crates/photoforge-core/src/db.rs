//! The SQLite photo catalog (rusqlite, bundled SQLite — no system dependency).

use crate::{Photo, Result};
use rusqlite::Connection;
use std::path::Path;

/// Open (creating if needed) the catalog database at `path` and ensure the
/// schema exists.
pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Open an in-memory catalog, primarily useful for tests.
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS photos (
            id           INTEGER PRIMARY KEY,
            path         TEXT NOT NULL UNIQUE,
            size         INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            captured_at  TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_photos_hash ON photos(content_hash);",
    )?;
    Ok(())
}

/// Insert (or replace on path conflict) a photo record.
pub fn upsert(conn: &Connection, photo: &Photo) -> Result<()> {
    conn.execute(
        "INSERT INTO photos (path, size, content_hash, captured_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET
            size = excluded.size,
            content_hash = excluded.content_hash,
            captured_at = excluded.captured_at",
        rusqlite::params![
            photo.path.to_string_lossy(),
            photo.size,
            photo.content_hash,
            photo.captured_at,
        ],
    )?;
    Ok(())
}

/// Count rows in the catalog.
pub fn count(conn: &Connection) -> Result<i64> {
    let n = conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_roundtrip() {
        let conn = open_in_memory().unwrap();
        let photo = Photo {
            path: "/tmp/a.jpg".into(),
            size: 123,
            content_hash: "deadbeef".into(),
            captured_at: Some("2024:01:01 00:00:00".into()),
        };
        upsert(&conn, &photo).unwrap();
        upsert(&conn, &photo).unwrap(); // idempotent on path
        assert_eq!(count(&conn).unwrap(), 1);
    }
}
