//! File-moving operations: organize-by-date, move-to-duplicates, and undo.
//!
//! Safety contract (project hard rule #1): **nothing here ever deletes a user
//! file**. Every operation is a move or copy; every executed move/copy is
//! recorded in the `undo_log` table *before* the filesystem is touched, and
//! every mover has a dry-run planning step that changes nothing.

use crate::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether organize-by-date moves the originals or copies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizeMode {
    Move,
    Copy,
}

/// One planned file operation (the dry-run output).
#[derive(Debug, Clone, Serialize)]
pub struct PlannedMove {
    pub file_id: i64,
    pub src: String,
    pub dst: String,
}

/// Summary of an executed batch.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MoveStats {
    pub moved: usize,
    pub errors: usize,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Make `candidate` unique against both the filesystem and the set of paths
/// already claimed by this plan, by appending ` (1)`, ` (2)`, … to the stem.
fn collision_safe(candidate: PathBuf, claimed: &mut std::collections::HashSet<PathBuf>) -> PathBuf {
    let mut dst = candidate.clone();
    let mut n = 0u32;
    while dst.exists() || claimed.contains(&dst) {
        n += 1;
        let stem = candidate
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = match candidate.extension() {
            Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
            None => format!("{stem} ({n})"),
        };
        dst = candidate.with_file_name(name);
    }
    claimed.insert(dst.clone());
    dst
}

/// Plan an organize-by-date run: every catalog file lands in
/// `<target_root>/YYYY/MM/<original filename>` based on its `resolved_date`
/// (files with no resolved date go to `<target_root>/undated/`). Pure planning
/// — touches nothing on disk. Files already at their target path are omitted.
pub fn plan_organize(conn: &Connection, target_root: impl AsRef<Path>) -> Result<Vec<PlannedMove>> {
    let root = target_root.as_ref();
    let mut stmt = conn.prepare("SELECT id, path, resolved_date FROM files ORDER BY path")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut claimed = std::collections::HashSet::new();
    let mut plan = Vec::new();
    for row in rows {
        let (file_id, src, resolved) = row?;
        let src_path = PathBuf::from(&src);
        let Some(name) = src_path.file_name() else {
            continue;
        };
        // resolved_date is ISO-8601 ("2023-01-15T…"), so YYYY/MM are fixed slices.
        let subdir = match resolved.as_deref() {
            Some(d) if d.len() >= 7 && d.as_bytes()[4] == b'-' => {
                format!("{}\\{}", &d[0..4], &d[5..7])
            }
            _ => "undated".to_string(),
        };
        let candidate = root.join(subdir).join(name);
        if candidate == src_path {
            continue; // already organized
        }
        let dst = collision_safe(candidate, &mut claimed);
        plan.push(PlannedMove {
            file_id,
            src,
            dst: dst.to_string_lossy().into_owned(),
        });
    }
    Ok(plan)
}

/// Execute a plan produced by [`plan_organize`]. Each entry is logged to
/// `undo_log` first, then moved (or copied). On move, the catalog row's path
/// is updated so the index stays truthful. Per-file failures are counted, not
/// fatal — a half-finished run leaves both disk and catalog consistent.
pub fn apply_organize(
    conn: &mut Connection,
    plan: &[PlannedMove],
    mode: OrganizeMode,
) -> Result<MoveStats> {
    let op = match mode {
        OrganizeMode::Move => "move",
        OrganizeMode::Copy => "copy",
    };
    let mut stats = MoveStats::default();
    for pm in plan {
        match execute_one(conn, pm, op) {
            Ok(()) => stats.moved += 1,
            Err(_) => stats.errors += 1,
        }
    }
    Ok(stats)
}

fn execute_one(conn: &mut Connection, pm: &PlannedMove, op: &str) -> Result<()> {
    let dst = Path::new(&pm.dst);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Log-then-act, atomically per file: the undo row and (on move) the path
    // update commit together only after the filesystem operation succeeds.
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO undo_log (ts, op, src_path, dst_path) VALUES (?1, ?2, ?3, ?4)",
        params![now_ts(), op, pm.src, pm.dst],
    )?;
    match op {
        "move" => {
            move_file(Path::new(&pm.src), dst)?;
            tx.execute(
                "UPDATE files SET path = ?1 WHERE id = ?2",
                params![pm.dst, pm.file_id],
            )?;
        }
        _ => {
            std::fs::copy(&pm.src, dst)?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Rename, falling back to copy-then-remove for cross-volume moves (`rename`
/// can't cross drive letters on Windows). The source is removed only after the
/// copy fully succeeds, so no state exists where the file is on neither side.
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
    }
}

/// Move `paths` into `dest_dir` (e.g. a `_Duplicates` folder), collision-safe,
/// undo-logged, catalog paths updated. Used by the dedupe UI.
pub fn move_files_to(
    conn: &mut Connection,
    paths: &[String],
    dest_dir: impl AsRef<Path>,
) -> Result<MoveStats> {
    let dest = dest_dir.as_ref();
    let mut claimed = std::collections::HashSet::new();
    let mut stats = MoveStats::default();
    for src in paths {
        let src_path = PathBuf::from(src);
        let Some(name) = src_path.file_name() else {
            stats.errors += 1;
            continue;
        };
        let dst = collision_safe(dest.join(name), &mut claimed);
        let file_id: Option<i64> = conn
            .query_row("SELECT id FROM files WHERE path = ?1", params![src], |r| {
                r.get(0)
            })
            .ok();
        let pm = PlannedMove {
            file_id: file_id.unwrap_or(-1),
            src: src.clone(),
            dst: dst.to_string_lossy().into_owned(),
        };
        match execute_one(conn, &pm, "move") {
            Ok(()) => stats.moved += 1,
            Err(_) => stats.errors += 1,
        }
    }
    Ok(stats)
}

/// Undo the most recent `count` *moves* by moving files back. Copies are
/// skipped (undoing a copy would mean deleting a file, which we never do).
/// Each successfully-reverted row is removed from the log.
pub fn undo_last(conn: &mut Connection, count: usize) -> Result<MoveStats> {
    let rows: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, src_path, dst_path FROM undo_log
             WHERE op = 'move' ORDER BY id DESC LIMIT ?1",
        )?;
        let mapped = stmt.query_map(params![count as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        mapped.collect::<std::result::Result<_, _>>()?
    };

    let mut stats = MoveStats::default();
    for (log_id, src, dst) in rows {
        // `mut` because the closure mutably borrows `conn` (FnMut in Java-ish
        // terms: a lambda that mutates captured state must itself be mutable).
        let mut back = || -> Result<()> {
            if let Some(parent) = Path::new(&src).parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tx = conn.transaction()?;
            move_file(Path::new(&dst), Path::new(&src))?;
            tx.execute(
                "UPDATE files SET path = ?1 WHERE path = ?2",
                params![src, dst],
            )?;
            tx.execute("DELETE FROM undo_log WHERE id = ?1", params![log_id])?;
            tx.commit()?;
            Ok(())
        };
        match back() {
            Ok(()) => stats.moved += 1,
            Err(_) => stats.errors += 1,
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pf_org_{tag}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_file(conn: &Connection, id: i64, path: &Path, date: Option<&str>) {
        fs::write(path, format!("content-{id}")).unwrap();
        conn.execute(
            "INSERT INTO files (id, path, size, mtime, resolved_date) VALUES (?1, ?2, 9, 0, ?3)",
            params![id, path.to_string_lossy(), date],
        )
        .unwrap();
    }

    #[test]
    fn organize_plans_moves_and_undoes() {
        let src_dir = tmpdir("src");
        let dst_dir = tmpdir("dst");
        let mut conn = crate::db::open_in_memory().unwrap();

        let a = src_dir.join("a.jpg");
        let b = src_dir.join("b.jpg"); // same name+month as c -> collision
        let c_dir = tmpdir("src2");
        let c = c_dir.join("b.jpg");
        let d = src_dir.join("d.jpg"); // no date -> undated
        seed_file(&conn, 1, &a, Some("2023-01-15T12:00:00"));
        seed_file(&conn, 2, &b, Some("2023-01-20T12:00:00"));
        seed_file(&conn, 3, &c, Some("2023-01-21T12:00:00"));
        seed_file(&conn, 4, &d, None);

        // Dry run: plan only, disk untouched.
        let plan = plan_organize(&conn, &dst_dir).unwrap();
        assert_eq!(plan.len(), 4);
        assert!(a.exists() && b.exists() && c.exists() && d.exists());
        assert!(plan.iter().any(|p| p.dst.contains("2023\\01")));
        assert!(plan.iter().any(|p| p.dst.contains("undated")));
        // The two b.jpg files must get distinct destinations.
        let bs: Vec<_> = plan.iter().filter(|p| p.dst.contains("b")).collect();
        assert_eq!(bs.len(), 2);
        assert_ne!(bs[0].dst, bs[1].dst);

        // Apply as a move.
        let stats = apply_organize(&mut conn, &plan, OrganizeMode::Move).unwrap();
        assert_eq!(stats.moved, 4);
        assert_eq!(stats.errors, 0);
        assert!(!a.exists());
        assert!(dst_dir.join("2023").join("01").join("a.jpg").exists());
        assert!(dst_dir.join("undated").join("d.jpg").exists());

        // Catalog followed the files, and everything was undo-logged.
        let db_path: String = conn
            .query_row("SELECT path FROM files WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(db_path.contains("2023"));
        let logged: i64 = conn
            .query_row("SELECT COUNT(*) FROM undo_log WHERE op='move'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(logged, 4);

        // Undo restores the originals.
        let undone = undo_last(&mut conn, 10).unwrap();
        assert_eq!(undone.moved, 4);
        assert!(a.exists() && b.exists() && c.exists() && d.exists());
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM undo_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);

        for dir in [&src_dir, &dst_dir, &c_dir] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn move_to_duplicates_is_undoable() {
        let src_dir = tmpdir("dupsrc");
        let mut conn = crate::db::open_in_memory().unwrap();
        let a = src_dir.join("x.jpg");
        seed_file(&conn, 1, &a, None);

        let dup_dir = src_dir.join("_Duplicates");
        let stats =
            move_files_to(&mut conn, &[a.to_string_lossy().into_owned()], &dup_dir).unwrap();
        assert_eq!(stats.moved, 1);
        assert!(!a.exists());
        assert!(dup_dir.join("x.jpg").exists());

        undo_last(&mut conn, 1).unwrap();
        assert!(a.exists());
        let _ = fs::remove_dir_all(&src_dir);
    }
}
