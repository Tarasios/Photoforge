//! The parallel, resumable directory indexer.
//!
//! [`index_directory`] walks a tree with [`scan`](crate::scan), processes every
//! image in parallel via rayon (EXIF + sidecar + dimensions + date
//! resolution), and batch-writes the results to SQLite. It is:
//!
//!   * **resumable** — files whose path+size+mtime already match a catalog row
//!     are skipped, and writes commit every 1000 rows (see [`db`](crate::db)),
//!     so an interrupted scan can simply be re-run.
//!   * **parallel** — per-file work fans out over the rayon thread pool; the
//!     database is touched only on the calling thread (rusqlite is not `Sync`).

use crate::date;
use crate::dhash;
use crate::exif::{self, ExifData};
use crate::hash;
use crate::scan;
use crate::sidecar::{self, Sidecar};
use crate::{db, Result};
use rayon::prelude::*;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::Metadata;
use std::path::Path;
use std::time::SystemTime;

/// One fully-processed image row, ready to be written to `files`.
#[derive(Debug, Clone, Serialize)]
pub struct IndexedFile {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub exif_datetime: Option<String>,
    pub exif_create_date: Option<String>,
    pub exif_make: Option<String>,
    pub exif_model: Option<String>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub orientation: Option<i64>,
    pub sidecar_taken_time: Option<String>,
    pub sidecar_lat: Option<f64>,
    pub sidecar_lon: Option<f64>,
    pub resolved_date: Option<String>,
    pub date_source: Option<String>,
    /// BLAKE3 content hash (32 raw bytes) — exact-duplicate detection.
    #[serde(skip)]
    pub blake3: Option<Vec<u8>>,
    /// 64-bit perceptual dHash — near-duplicate detection. Stored as `i64`
    /// because SQLite INTEGER is signed; the bit pattern is what matters.
    pub dhash: Option<i64>,
}

/// Summary of an indexing run.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct IndexStats {
    /// Image files discovered on disk.
    pub scanned: usize,
    /// Rows inserted or updated.
    pub added: usize,
    /// Files skipped because path+size+mtime were unchanged.
    pub skipped: usize,
    /// Files that could not be read (e.g. stat failed).
    pub errors: usize,
}

enum Outcome {
    Added(Box<IndexedFile>),
    Skipped,
    Errored,
}

/// Index every image under `root` into the open catalog `conn`, returning stats.
pub fn index_directory(conn: &mut Connection, root: impl AsRef<Path>) -> Result<IndexStats> {
    // Snapshot existing rows so parallel workers can skip unchanged files
    // without touching the (non-Sync) database connection.
    let existing = db::load_index_state(conn)?;

    let paths = scan::enumerate_images(root);
    let scanned = paths.len();

    let outcomes: Vec<Outcome> = paths
        .par_iter()
        .map(|path| process_one(path, &existing))
        .collect();

    let mut to_insert = Vec::new();
    let mut skipped = 0;
    let mut errors = 0;
    for outcome in outcomes {
        match outcome {
            Outcome::Added(f) => to_insert.push(*f),
            Outcome::Skipped => skipped += 1,
            Outcome::Errored => errors += 1,
        }
    }

    let added = db::insert_files(conn, &to_insert)?;

    Ok(IndexStats {
        scanned,
        added,
        skipped,
        errors,
    })
}

fn mtime_secs(meta: &Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn process_one(path: &Path, existing: &HashMap<String, (i64, i64)>) -> Outcome {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Outcome::Errored,
    };
    let size = meta.len() as i64;
    let mtime = mtime_secs(&meta);
    let path_str = path.to_string_lossy().into_owned();

    // Resumable skip: unchanged path+size+mtime means we've seen this exact file.
    if let Some(&(prev_size, prev_mtime)) = existing.get(&path_str) {
        if prev_size == size && prev_mtime == mtime {
            return Outcome::Skipped;
        }
    }

    // Metadata extraction never fails the file — missing EXIF/sidecar/dims are
    // simply left NULL.
    let exif = exif::extract(path).unwrap_or_default();
    let (width, height) = dimensions(path, &exif);
    let sidecar = sidecar::find_and_parse(path).unwrap_or_default();
    let (resolved_date, date_source) = resolve_date(&exif, &sidecar, path, mtime);

    // Hashing happens here, inside the rayon worker, so it parallelizes with
    // the rest of the per-file work. Failures degrade to NULL, not errors:
    // blake3 only fails on unreadable files, dhash also on undecodable ones
    // (e.g. HEIC, which the `image` crate can't decode).
    let blake3 = hash::blake3_file(path).ok().map(|b| b.to_vec());
    let dhash = dhash::dhash_file(path, exif.orientation)
        .ok()
        .map(|h| h as i64);

    Outcome::Added(Box::new(IndexedFile {
        path: path_str,
        size,
        mtime,
        width,
        height,
        exif_datetime: exif.datetime_original,
        exif_create_date: exif.create_date,
        exif_make: exif.make,
        exif_model: exif.model,
        gps_lat: exif.gps_lat,
        gps_lon: exif.gps_lon,
        orientation: exif.orientation,
        sidecar_taken_time: sidecar.taken_time,
        sidecar_lat: sidecar.lat,
        sidecar_lon: sidecar.lon,
        resolved_date,
        date_source,
        blake3,
        dhash,
    }))
}

/// Read pixel dimensions from the image header, falling back to EXIF pixel
/// dimensions for formats `image` can't decode (e.g. HEIC).
fn dimensions(path: &Path, exif: &ExifData) -> (Option<i64>, Option<i64>) {
    if let Ok((w, h)) = image::image_dimensions(path) {
        return (Some(w as i64), Some(h as i64));
    }
    (exif.pixel_width, exif.pixel_height)
}

/// Resolve the canonical capture date and record where it came from.
///
/// Precedence: EXIF `DateTimeOriginal` → EXIF `CreateDate` → sidecar
/// `photoTakenTime` → date parsed from the filename → file mtime. EXIF wins
/// because it is the camera's authoritative capture time; mtime is the
/// last-resort floor and always yields *some* date.
fn resolve_date(
    exif: &ExifData,
    sidecar: &Sidecar,
    path: &Path,
    mtime: i64,
) -> (Option<String>, Option<String>) {
    if let Some(d) = &exif.datetime_original {
        return (Some(d.clone()), Some("exif".into()));
    }
    if let Some(d) = &exif.create_date {
        return (Some(d.clone()), Some("exif".into()));
    }
    if let Some(d) = &sidecar.taken_time {
        return (Some(d.clone()), Some("sidecar".into()));
    }
    if let Some(d) = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(date::parse_date_from_filename)
    {
        return (Some(d), Some("filename".into()));
    }
    (Some(date::unix_to_iso(mtime)), Some("mtime".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pf_idx_{}_{}", std::process::id(), nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path) {
        image::RgbImage::new(2, 2).save(path).unwrap();
    }

    #[test]
    fn indexes_resumes_and_resolves_sources() {
        let dir = tmpdir();

        // date from filename
        write_png(&dir.join("photo_20230115_120000.png"));
        // date + location from a Takeout sidecar
        write_png(&dir.join("beach.png"));
        fs::write(
            dir.join("beach.png.json"),
            r#"{"title":"beach.png",
                "photoTakenTime":{"timestamp":"1673784000","formatted":"x"},
                "geoData":{"latitude":37.5,"longitude":-122.3,"altitude":0}}"#,
        )
        .unwrap();
        // nothing to go on -> mtime
        write_png(&dir.join("plain.png"));
        // a non-image sibling that must be ignored
        fs::write(dir.join("notes.txt"), "ignore me").unwrap();

        let mut conn = db::open_in_memory().unwrap();

        let s1 = index_directory(&mut conn, &dir).unwrap();
        assert_eq!(s1.scanned, 3, "only images are scanned");
        assert_eq!(s1.added, 3);
        assert_eq!(s1.skipped, 0);
        assert_eq!(s1.errors, 0);

        // Second run is a no-op: everything is unchanged and thus skipped.
        let s2 = index_directory(&mut conn, &dir).unwrap();
        assert_eq!(s2.scanned, 3);
        assert_eq!(s2.added, 0);
        assert_eq!(s2.skipped, 3);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);

        // Dimensions come from the PNG header.
        let (w, h): (i64, i64) = conn
            .query_row(
                "SELECT width, height FROM files WHERE path LIKE '%plain.png'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((w, h), (2, 2));

        // date_source resolution across the three fallbacks.
        let src_filename: String = conn
            .query_row(
                "SELECT date_source FROM files WHERE path LIKE '%photo\\_20230115%' ESCAPE '\\'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(src_filename, "filename");

        let (src, taken, lat): (String, String, f64) = conn
            .query_row(
                "SELECT date_source, sidecar_taken_time, sidecar_lat FROM files WHERE path LIKE '%beach.png'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(src, "sidecar");
        assert_eq!(taken, "2023-01-15T12:00:00Z");
        assert!((lat - 37.5).abs() < 1e-9);

        let src_mtime: String = conn
            .query_row(
                "SELECT date_source FROM files WHERE path LIKE '%plain.png'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(src_mtime, "mtime");

        let _ = fs::remove_dir_all(&dir);
    }
}
