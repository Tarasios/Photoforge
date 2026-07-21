//! Rules-based photo vs. screenshot/asset classifier (Phase 2, heuristic tier).
//!
//! Cheap, explainable signals only — no ML. Each file gets a label
//! (`photo` / `non_photo` / `ambiguous`), a confidence, and the name of the
//! rule that fired, so every decision can be audited. Ambiguous files are the
//! ones a later ONNX model (P2.2) would arbitrate.

use crate::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

/// Labels used in `files.classification`.
pub const PHOTO: &str = "photo";
pub const NON_PHOTO: &str = "non_photo";
pub const AMBIGUOUS: &str = "ambiguous";

/// Exact pixel dimensions of common displays (desktop + phone). An image that
/// is *exactly* a screen size is very likely a screenshot; real photos come in
/// sensor sizes (4032x3024, …). Checked in both orientations.
const SCREEN_SIZES: &[(i64, i64)] = &[
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1536, 864),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 2160),
    // Phone screens (portrait as stored).
    (750, 1334),
    (828, 1792),
    (1080, 1920),
    (1080, 2340),
    (1080, 2400),
    (1170, 2532),
    (1179, 2556),
    (1284, 2778),
    (1290, 2796),
    (1440, 2960),
    (1440, 3040),
    (1440, 3200),
];

/// Path segments that mark app-generated assets rather than user photos.
const ASSET_SEGMENTS: &[&str] = &["assets", "cache", "thumbnails", ".thumbnails", "icons"];

/// The classifier's verdict for one file.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub label: &'static str,
    pub confidence: f64,
    pub rule: &'static str,
}

/// Everything the rules need to know about a file (all already in the catalog).
pub struct FileFacts<'a> {
    pub path: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub exif_make: Option<&'a str>,
    pub exif_model: Option<&'a str>,
}

/// Apply the rules in priority order and return the first that fires.
pub fn classify(facts: &FileFacts) -> Verdict {
    // Camera EXIF is the strongest photo signal: screenshots and app assets
    // never carry a Make/Model.
    if facts.exif_make.is_some() || facts.exif_model.is_some() {
        return Verdict {
            label: PHOTO,
            confidence: 0.95,
            rule: "exif_camera",
        };
    }

    let path = Path::new(facts.path);
    let lower = facts.path.to_ascii_lowercase();

    // Screenshot naming conventions (Windows, Android, iOS exports).
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            let n = n.to_ascii_lowercase();
            n.starts_with("screenshot") || n.starts_with("screen shot") || n.starts_with("scrnshot")
        })
        .unwrap_or(false)
    {
        return Verdict {
            label: NON_PHOTO,
            confidence: 0.95,
            rule: "screenshot_filename",
        };
    }

    // App-generated directories.
    if path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| ASSET_SEGMENTS.contains(&s.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    }) {
        return Verdict {
            label: NON_PHOTO,
            confidence: 0.9,
            rule: "asset_path",
        };
    }

    // Exactly a known screen size (either orientation).
    if let (Some(w), Some(h)) = (facts.width, facts.height) {
        if SCREEN_SIZES.contains(&(w, h)) || SCREEN_SIZES.contains(&(h, w)) {
            return Verdict {
                label: NON_PHOTO,
                confidence: 0.8,
                rule: "screen_resolution",
            };
        }
    }

    // PNG without camera EXIF: cameras write JPEG/HEIC; PNGs are almost always
    // screenshots, exports, or graphics.
    if lower.ends_with(".png") {
        return Verdict {
            label: NON_PHOTO,
            confidence: 0.7,
            rule: "png_no_exif",
        };
    }

    Verdict {
        label: AMBIGUOUS,
        confidence: 0.0,
        rule: "none",
    }
}

/// Counts from a [`classify_all`] run.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ClassifyStats {
    pub photo: usize,
    pub non_photo: usize,
    pub ambiguous: usize,
}

/// Classify every catalog file and store label + confidence + rule. Rows whose
/// `class_rule` is `'manual'` (user corrections from the review UI) are never
/// overwritten.
pub fn classify_all(conn: &mut Connection) -> Result<ClassifyStats> {
    // (id, path, width, height, exif_make, exif_model)
    type Row = (i64, String, Option<i64>, Option<i64>, Option<String>, Option<String>);
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT id, path, width, height, exif_make, exif_model
             FROM files WHERE class_rule IS NULL OR class_rule != 'manual'",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        mapped.collect::<std::result::Result<_, _>>()?
    };

    let mut stats = ClassifyStats::default();
    let tx = conn.transaction()?;
    {
        let mut update = tx.prepare_cached(
            "UPDATE files SET classification = ?1, class_confidence = ?2, class_rule = ?3
             WHERE id = ?4",
        )?;
        for (id, path, width, height, make, model) in &rows {
            let v = classify(&FileFacts {
                path,
                width: *width,
                height: *height,
                exif_make: make.as_deref(),
                exif_model: model.as_deref(),
            });
            match v.label {
                PHOTO => stats.photo += 1,
                NON_PHOTO => stats.non_photo += 1,
                _ => stats.ambiguous += 1,
            }
            update.execute(params![v.label, v.confidence, v.rule, id])?;
        }
    }
    tx.commit()?;
    Ok(stats)
}

/// Accuracy of the stored classifications against a hand-labeled CSV
/// (`path,label` per line, labels `photo`/`non_photo`).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AccuracyReport {
    pub total: usize,
    /// Ground-truth rows whose path wasn't in the catalog.
    pub missing: usize,
    pub correct: usize,
    pub wrong: usize,
    /// Catalog said `ambiguous` — neither right nor wrong.
    pub undecided: usize,
    pub accuracy: f64,
}

/// Compare catalog classifications to `csv_path` (lines of `path,label`; the
/// label is everything after the *last* comma so paths may contain commas).
pub fn accuracy_report(conn: &Connection, csv_path: impl AsRef<Path>) -> Result<AccuracyReport> {
    let text = std::fs::read_to_string(csv_path)?;
    let mut rep = AccuracyReport::default();
    let mut stmt = conn.prepare("SELECT classification FROM files WHERE path = ?1")?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, label)) = line.rsplit_once(',') else {
            continue;
        };
        let (path, label) = (path.trim(), label.trim().to_ascii_lowercase());
        rep.total += 1;
        let stored: Option<Option<String>> = stmt.query_row(params![path], |r| r.get(0)).ok();
        match stored.flatten().as_deref() {
            None => rep.missing += 1,
            Some(AMBIGUOUS) => rep.undecided += 1,
            Some(got) if got == label => rep.correct += 1,
            Some(_) => rep.wrong += 1,
        }
    }
    let decided = rep.correct + rep.wrong;
    rep.accuracy = if decided > 0 {
        rep.correct as f64 / decided as f64
    } else {
        0.0
    };
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(
        path: &'a str,
        dims: Option<(i64, i64)>,
        make: Option<&'a str>,
    ) -> FileFacts<'a> {
        FileFacts {
            path,
            width: dims.map(|d| d.0),
            height: dims.map(|d| d.1),
            exif_make: make,
            exif_model: None,
        }
    }

    #[test]
    fn rules_fire_in_priority_order() {
        let v = classify(&facts("C:\\p\\IMG_1.jpg", Some((4032, 3024)), Some("Canon")));
        assert_eq!((v.label, v.rule), (PHOTO, "exif_camera"));

        let v = classify(&facts("C:\\p\\Screenshot_2023.jpg", None, None));
        assert_eq!((v.label, v.rule), (NON_PHOTO, "screenshot_filename"));

        let v = classify(&facts("C:\\app\\cache\\img.jpg", None, None));
        assert_eq!((v.label, v.rule), (NON_PHOTO, "asset_path"));

        let v = classify(&facts("C:\\p\\grab.jpg", Some((1920, 1080)), None));
        assert_eq!((v.label, v.rule), (NON_PHOTO, "screen_resolution"));

        let v = classify(&facts("C:\\p\\export.png", Some((640, 480)), None));
        assert_eq!((v.label, v.rule), (NON_PHOTO, "png_no_exif"));

        let v = classify(&facts("C:\\p\\holiday.jpg", Some((640, 480)), None));
        assert_eq!((v.label, v.rule), (AMBIGUOUS, "none"));
    }

    #[test]
    fn classify_all_stores_and_respects_manual() {
        let mut conn = crate::db::open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO files (id, path, size, mtime, exif_make) VALUES
               (1, 'C:\\x\\a.jpg', 1, 0, 'Canon'),
               (2, 'C:\\x\\Screenshot_1.png', 1, 0, NULL);
             INSERT INTO files (id, path, size, mtime, classification, class_rule) VALUES
               (3, 'C:\\x\\odd.jpg', 1, 0, 'photo', 'manual');",
        )
        .unwrap();

        let stats = classify_all(&mut conn).unwrap();
        assert_eq!(stats.photo, 1);
        assert_eq!(stats.non_photo, 1);

        let manual: String = conn
            .query_row("SELECT class_rule FROM files WHERE id = 3", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(manual, "manual");
    }

    #[test]
    fn accuracy_report_counts() {
        let conn = crate::db::open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO files (id, path, size, mtime, classification) VALUES
               (1, 'a.jpg', 1, 0, 'photo'),
               (2, 'b.png', 1, 0, 'non_photo'),
               (3, 'c.jpg', 1, 0, 'ambiguous');",
        )
        .unwrap();
        let csv = std::env::temp_dir().join(format!("pf_acc_{}.csv", std::process::id()));
        std::fs::write(&csv, "a.jpg,photo\nb.png,photo\nc.jpg,photo\nmissing.jpg,photo\n").unwrap();

        let rep = accuracy_report(&conn, &csv).unwrap();
        assert_eq!(rep.total, 4);
        assert_eq!(rep.correct, 1);
        assert_eq!(rep.wrong, 1);
        assert_eq!(rep.undecided, 1);
        assert_eq!(rep.missing, 1);
        assert!((rep.accuracy - 0.5).abs() < 1e-9);
        let _ = std::fs::remove_file(&csv);
    }
}
