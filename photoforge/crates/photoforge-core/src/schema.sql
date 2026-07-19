-- photoforge catalog schema — single source of truth (see CLAUDE.md).
-- Intentional deviations from the original spec:
--   * files.exif_create_date added — the spec stores both EXIF DateTimeOriginal
--     and CreateDate, but had only one exif_datetime column.
--   * IF NOT EXISTS throughout so opening an existing catalog is idempotent
--     (required for a resumable indexer).

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
